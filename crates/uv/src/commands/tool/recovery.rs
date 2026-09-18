use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt::Write;
use std::io;
#[cfg(windows)]
use std::io::{Read, Seek};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
#[cfg(windows)]
use fs_err::File;
use itertools::Itertools;
use owo_colors::OwoColorize;
#[cfg(windows)]
use sha2::{Digest, Sha256};
use uv_distribution_types::Name;
use uv_errors::{ErrorWithHints, Hinted};
use uv_fs::Simplified;
use uv_installer::SitePackages;
use uv_normalize::PackageName;
use uv_python::PythonEnvironment;
use uv_tool::{InstalledTools, Tool, ToolEntrypoint, entrypoint_paths};

use crate::commands::tool::common::{NoExecutablesError, matching_packages};
use crate::commands::tool::uninstall::owned_entrypoints_by;
use crate::printer::Printer;

/// An existing tool's export authority, captured before its environment is changed.
///
/// This is an in-process preflight, not a transaction journal. Package installation can fail
/// after changing the environment, and export I/O can fail after changing an earlier export.
pub(super) struct ToolEntrypointSnapshot {
    receipt: Tool,
    claims: ToolEntrypointClaims,
    owned: Vec<OwnedExport>,
    inventory: Option<BTreeSet<(PackageName, OsString)>>,
    recorded_inventory: BTreeSet<(PackageName, OsString)>,
    executable_directory: PathBuf,
}

struct OwnedExport {
    entrypoint: ToolEntrypoint,
    fingerprint: ExportFingerprint,
}

/// Receipt claims, including environments whose receipt cannot establish their export set.
pub(super) struct ToolEntrypointClaims {
    receipts: Vec<(PackageName, Tool)>,
    unresolved: Vec<UnresolvedToolClaims>,
}

struct UnresolvedToolClaims {
    name: PackageName,
    receipt_error: uv_tool::Error,
    scripts: PathBuf,
    inventory: io::Result<BTreeSet<OsString>>,
}

impl ToolEntrypointClaims {
    pub(super) fn capture(installed_tools: &InstalledTools) -> anyhow::Result<Self> {
        let mut receipts = Vec::new();
        let mut unresolved = Vec::new();
        for (name, receipt) in installed_tools.tools()? {
            match receipt {
                Ok(receipt) => receipts.push((name, receipt)),
                Err(receipt_error) => {
                    let scripts = installed_tools.tool_dir(&name).join(if cfg!(windows) {
                        "Scripts"
                    } else {
                        "bin"
                    });
                    // A complete directory listing can exclude unrelated executable names. A
                    // missing or unreadable directory cannot establish that exclusion.
                    let inventory = fs_err::read_dir(&scripts).and_then(|entries| {
                        entries
                            .map(|entry| entry.map(|entry| entry.file_name()))
                            .collect()
                    });
                    unresolved.push(UnresolvedToolClaims {
                        name,
                        receipt_error,
                        scripts,
                        inventory,
                    });
                }
            }
        }
        receipts.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
        unresolved.sort_unstable_by(|left, right| left.name.cmp(&right.name));
        Ok(Self {
            receipts,
            unresolved,
        })
    }

    pub(super) fn receipts(&self) -> &[(PackageName, Tool)] {
        &self.receipts
    }

    fn check_unresolved_claims(&self, name: &PackageName, target: &Path) -> anyhow::Result<()> {
        let Some(filename) = target.file_name() else {
            bail!("Invalid executable path `{}`", target.user_display());
        };
        for other in &self.unresolved {
            if other.name == *name {
                continue;
            }
            let inventory = other.inventory.as_ref().map_err(|err| {
                anyhow::anyhow!(
                    "Cannot determine whether executable `{}` belongs to `{}` because its receipt is missing or invalid and its scripts directory `{}` cannot be inspected: {err}",
                    target.user_display(),
                    other.name,
                    other.scripts.user_display(),
                )
            })?;
            #[cfg(unix)]
            let may_claim = inventory.contains(filename);
            #[cfg(windows)]
            let may_claim = {
                // Resolve current filesystem aliases, including DOS short names. Unlike a
                // valid receipt, the inventory does not assert a historical short spelling.
                let mut matches = match fs_err::symlink_metadata(other.scripts.join(filename)) {
                    Ok(_) => true,
                    Err(err) if err.kind() == io::ErrorKind::NotFound => false,
                    Err(err) => return Err(err.into()),
                };
                for installed_name in inventory {
                    if uv_windows::names_equal_ordinal(installed_name, filename)? {
                        matches = true;
                        break;
                    }
                }
                matches
            };
            if may_claim {
                bail!(
                    "Cannot restore executable `{}` because `{}` has a missing or invalid receipt and may provide the same executable: {}",
                    target.user_display(),
                    other.name,
                    other.receipt_error,
                );
            }
        }
        Ok(())
    }

    pub(super) fn check_other_claims(
        &self,
        name: &PackageName,
        target: &Path,
    ) -> anyhow::Result<()> {
        for (other_name, receipt) in &self.receipts {
            if other_name == name {
                continue;
            }
            for other in receipt.entrypoints() {
                if same_entrypoint_location(&other.install_path, target)? {
                    bail!(
                        "Cannot restore executable `{}` because it is also recorded for `{other_name}`",
                        target.user_display()
                    );
                }
            }
        }
        self.check_unresolved_claims(name, target)
    }
}

impl ToolEntrypointSnapshot {
    pub(super) fn capture(
        environment: Option<&PythonEnvironment>,
        name: &PackageName,
        receipt: &Tool,
        installed_tools: &InstalledTools,
    ) -> anyhow::Result<Self> {
        let claims = ToolEntrypointClaims::capture(installed_tools)?;
        for entrypoint in receipt.entrypoints() {
            claims.check_unresolved_claims(name, &entrypoint.install_path)?;
        }
        let owned = owned_entrypoints_by(
            name,
            receipt,
            claims.receipts(),
            installed_tools,
            same_existing_entrypoint_location,
        )?
        .into_iter()
        .map(|entrypoint| {
            Ok(OwnedExport {
                fingerprint: ExportFingerprint::capture(&entrypoint.install_path)?,
                entrypoint,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
        let inventory = if let Some(environment) = environment {
            let mut inventory = BTreeSet::new();
            let site_packages = SitePackages::from_environment(environment)?;
            for dist in site_packages.iter() {
                for (_, source) in entrypoint_paths(&site_packages, dist.name(), dist.version())? {
                    if let Some(filename) = source.file_name() {
                        inventory.insert((dist.name().clone(), filename.to_owned()));
                    }
                }
            }
            Some(inventory)
        } else {
            None
        };
        let mut recorded_inventory = BTreeSet::new();
        for entry in receipt.entrypoints() {
            let provider = entry
                .from
                .as_deref()
                .unwrap_or(name.as_ref())
                .parse::<PackageName>()?;
            let filename = entry.install_path.file_name().ok_or_else(|| {
                anyhow::anyhow!(
                    "Invalid executable path `{}`",
                    entry.install_path.user_display()
                )
            })?;
            if let (Some(environment), Some(inventory)) = (environment, &inventory) {
                for (package, installed_name) in inventory {
                    if package == &provider
                        && same_entrypoint_location(
                            &environment.scripts().join(filename),
                            &environment.scripts().join(installed_name),
                        )?
                    {
                        recorded_inventory.insert((package.clone(), installed_name.clone()));
                    }
                }
            } else {
                recorded_inventory.insert((provider, filename.to_owned()));
            }
        }
        Ok(Self {
            receipt: receipt.clone(),
            claims,
            owned,
            inventory,
            recorded_inventory,
            executable_directory: uv_tool::tool_executable_dir()?,
        })
    }

    /// Admit known destinations before any package or environment mutation, including `--force`.
    pub(super) fn admit_mutation(&self, name: &PackageName, force: bool) -> anyhow::Result<()> {
        for entrypoint in self.receipt.entrypoints() {
            let filename = entrypoint.install_path.file_name().ok_or_else(|| {
                anyhow::anyhow!(
                    "Invalid executable path `{}`",
                    entrypoint.install_path.user_display()
                )
            })?;
            self.admit_target(name, &self.executable_directory.join(filename), force)?;
        }
        Ok(())
    }

    fn admit_target(&self, name: &PackageName, target: &Path, force: bool) -> anyhow::Result<()> {
        self.claims.check_other_claims(name, target)?;
        let exists = match fs_err::symlink_metadata(target) {
            Ok(metadata) if metadata.is_dir() => {
                bail!("Executable path `{}` is a directory", target.user_display());
            }
            Ok(_) => true,
            Err(err) if err.kind() == io::ErrorKind::NotFound => false,
            Err(err) => return Err(err.into()),
        };
        for old in &self.owned {
            if same_existing_entrypoint_location(&old.entrypoint.install_path, target)? {
                if exists && !old.fingerprint.matches(target)? {
                    bail!(
                        "Executable `{}` changed while updating its tool",
                        target.user_display()
                    );
                }
                return Ok(());
            }
        }
        if exists && !force {
            bail!(
                "Executable already exists: {} (use `--force` to overwrite)",
                target.user_display().bold()
            );
        }
        Ok(())
    }

    /// Remove only exports that still have their pre-mutation identity.
    pub(super) fn remove_owned_exports(&self) -> anyhow::Result<()> {
        #[cfg(windows)]
        let itself = std::env::current_exe().ok();
        for old in &self.owned {
            if old.fingerprint.matches(&old.entrypoint.install_path)? {
                #[cfg(windows)]
                if itself.as_ref().is_some_and(|itself| {
                    std::path::absolute(&old.entrypoint.install_path)
                        .is_ok_and(|target| *itself == target)
                }) {
                    self_replace::self_delete().context("Failed to remove old executable")?;
                    continue;
                }
                match fs_err::remove_file(&old.entrypoint.install_path) {
                    Ok(()) => {}
                    Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                    Err(err) => return Err(err.into()),
                }
            }
        }
        Ok(())
    }

    /// Discover and preflight the complete final export set, then apply it.
    pub(super) fn install(
        &self,
        environment: &PythonEnvironment,
        name: &PackageName,
        providers: &[PackageName],
        force: bool,
        report_unchanged: bool,
        printer: Printer,
    ) -> anyhow::Result<Vec<ToolEntrypoint>> {
        let site_packages = SitePackages::from_environment(environment)?;
        let environment_root = fs_err::canonicalize(environment.root())?;
        let previous_providers = self
            .receipt
            .entrypoints()
            .iter()
            .map(|entry| entry.from.as_deref().unwrap_or(name.as_ref()))
            .collect::<BTreeSet<_>>();
        let mut planned = BTreeMap::<PathBuf, (String, PathBuf, PackageName, bool)>::new();
        let ordered_packages = providers
            .iter()
            .filter(|package| *package != name)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .chain(std::iter::once(name));
        for package in ordered_packages {
            let installed = site_packages.get_packages(package);
            let Some(dist) = installed.first() else {
                if package == name {
                    return Err(NoExecutablesError::Root {
                        package: name.clone(),
                        matching_dependency_packages: Vec::new(),
                    }
                    .into());
                }
                bail!("Expected package `{package}` to be installed");
            };
            let entries = entrypoint_paths(&site_packages, dist.name(), dist.version())?;
            if entries.is_empty() {
                if package != name {
                    let err = NoExecutablesError::Dependency {
                        package: package.clone(),
                    };
                    writeln!(
                        printer.stdout(),
                        "{}",
                        ErrorWithHints::new(&err, err.hints())
                    )?;
                    continue;
                }
                return Err(NoExecutablesError::Root {
                    package: name.clone(),
                    matching_dependency_packages: matching_packages(name.as_ref(), &site_packages)
                        .into_iter()
                        .map(|dist| dist.name().clone())
                        .collect(),
                }
                .into());
            }
            for (entry_name, source) in entries {
                let filename = source
                    .file_name()
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| OsString::from(&entry_name));
                let was_recorded = self
                    .recorded_inventory
                    .contains(&(package.clone(), filename.clone()));
                #[cfg(windows)]
                if self.inventory.is_none() && !was_recorded {
                    for (previous_package, previous_name) in &self.recorded_inventory {
                        if previous_package == package
                            && uv_windows::names_equal_ordinal(previous_name, &filename)?
                        {
                            bail!(
                                "Cannot compare executable names in the missing previous environment for `{name}`"
                            );
                        }
                    }
                }
                // A pruned command is not implicitly reacquired. A newly selected explicit
                // provider may add its commands, subject to the same competing-claim preflight.
                let newly_selected =
                    package != name && !previous_providers.contains(package.as_ref());
                if self.inventory.as_ref().is_none_or(|inventory| {
                    inventory.contains(&(package.clone(), filename.clone()))
                }) && !was_recorded
                    && !newly_selected
                {
                    continue;
                }
                let canonical_source = fs_err::canonicalize(&source)?;
                if !canonical_source.starts_with(&environment_root)
                    || !fs_err::metadata(&canonical_source)?.is_file()
                {
                    bail!(
                        "Executable `{}` is not a file in the tool environment",
                        source.user_display()
                    );
                }
                #[cfg(windows)]
                if fs_err::symlink_metadata(&source)?.is_symlink() {
                    bail!("Executable `{}` is a symbolic link", source.user_display());
                }
                let target = self.executable_directory.join(filename);
                if same_entrypoint_location(&source, &target)? {
                    bail!(
                        "Cannot export executable `{}` into its tool environment",
                        target.user_display()
                    );
                }
                self.admit_target(name, &target, force)?;
                let mut replace = true;
                for old in &self.owned {
                    if same_existing_entrypoint_location(&old.entrypoint.install_path, &target)?
                        && old.fingerprint.matches(&target)?
                        && ExportFingerprint::points_to(&old.fingerprint, &target, &source)?
                    {
                        replace = false;
                        break;
                    }
                }
                let mut previous_target = None;
                for previous in planned.keys() {
                    if same_planned_entrypoint_location(previous, &target)? {
                        previous_target = Some(previous.clone());
                        break;
                    }
                }
                if let Some(previous_target) = previous_target {
                    planned.remove(&previous_target);
                }
                planned.insert(target, (entry_name, source, package.clone(), replace));
            }
        }

        // All targets and old removal candidates are admitted before the first export write.
        let mut removals = Vec::new();
        for old in &self.owned {
            let mut retained = false;
            for target in planned.keys() {
                if same_existing_entrypoint_location(&old.entrypoint.install_path, target)? {
                    retained = true;
                    break;
                }
            }
            if !retained && old.fingerprint.matches(&old.entrypoint.install_path)? {
                removals.push(&old.entrypoint.install_path);
            }
        }
        if !planned.is_empty() {
            fs_err::create_dir_all(&self.executable_directory)
                .context("Failed to create executable directory")?;
        }
        #[cfg(windows)]
        let itself = std::env::current_exe().ok();
        let mut result = Vec::new();
        let mut names = BTreeMap::<PackageName, BTreeSet<String>>::new();
        for (target, (entry_name, source, package, replace)) in planned {
            if replace {
                #[cfg(unix)]
                uv_fs::replace_symlink(source, &target).context("Failed to install executable")?;
                #[cfg(windows)]
                if itself.as_ref().is_some_and(|itself| {
                    std::path::absolute(&target).is_ok_and(|target| *itself == target)
                }) {
                    self_replace::self_replace(source).context("Failed to install executable")?;
                } else {
                    copy_executable(&source, &target).context("Failed to install executable")?;
                }
            }
            let entrypoint = ToolEntrypoint::new(&entry_name, target, package.to_string());
            if replace || report_unchanged {
                names
                    .entry(package.clone())
                    .or_default()
                    .insert(entrypoint.name.clone());
            }
            result.push(entrypoint);
        }
        for old in removals {
            #[cfg(windows)]
            if itself.as_ref().is_some_and(|itself| {
                std::path::absolute(old).is_ok_and(|target| *itself == target)
            }) {
                self_replace::self_delete().context("Failed to remove old executable")?;
                continue;
            }
            match fs_err::remove_file(old) {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
        }
        let root_names = names.remove(name);
        for (package, names) in names
            .into_iter()
            .chain(root_names.map(|names| (name.clone(), names)))
        {
            let s = if names.len() == 1 { "" } else { "s" };
            let from = if package == *name {
                String::new()
            } else {
                format!(" from `{package}`")
            };
            writeln!(
                printer.stderr(),
                "Installed {} executable{s}{from}: {}",
                names.len(),
                names.iter().map(|name| name.bold()).join(", ")
            )?;
        }
        Ok(result)
    }
}

#[cfg(unix)]
struct ExportFingerprint {
    device: u64,
    inode: u64,
    target: PathBuf,
}

#[cfg(unix)]
impl ExportFingerprint {
    fn capture(path: &Path) -> anyhow::Result<Self> {
        let metadata = fs_err::symlink_metadata(path)?;
        if !metadata.is_symlink() {
            bail!(
                "Executable `{}` is not a symbolic link",
                path.user_display()
            );
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            target: fs_err::read_link(path)?,
        })
    }

    fn matches(&self, path: &Path) -> anyhow::Result<bool> {
        let metadata = match fs_err::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(err) => return Err(err.into()),
        };
        Ok(metadata.is_symlink()
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
            && fs_err::read_link(path)? == self.target)
    }

    fn points_to(_fingerprint: &Self, target: &Path, source: &Path) -> anyhow::Result<bool> {
        match fs_err::canonicalize(target) {
            Ok(target) => Ok(target == fs_err::canonicalize(source)?),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err.into()),
        }
    }
}

#[cfg(windows)]
struct ExportFingerprint {
    _file: File,
    identity: uv_windows::FileIdentity,
    digest: [u8; 32],
}

#[cfg(windows)]
impl ExportFingerprint {
    fn capture(path: &Path) -> anyhow::Result<Self> {
        let file = open_entry(path)?;
        if !file.metadata()?.is_file() || fs_err::symlink_metadata(path)?.is_symlink() {
            bail!("Executable `{}` is not a regular file", path.user_display());
        }
        let identity = uv_windows::FileIdentity::from_file(&file)?;
        let digest = digest_file(&file)?;
        Ok(Self {
            _file: file,
            identity,
            digest,
        })
    }

    fn matches(&self, path: &Path) -> anyhow::Result<bool> {
        let file = match open_entry(path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(err) => return Err(err.into()),
        };
        Ok(file.metadata()?.is_file()
            && !fs_err::symlink_metadata(path)?.is_symlink()
            && uv_windows::FileIdentity::from_file(&file)? == self.identity
            && digest_file(&file)? == self.digest)
    }

    fn points_to(fingerprint: &Self, _target: &Path, source: &Path) -> anyhow::Result<bool> {
        Ok(digest_file(&open_entry(source)?)? == fingerprint.digest)
    }
}

#[cfg(windows)]
fn digest_file(mut file: &File) -> io::Result<[u8; 32]> {
    file.rewind()?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 8192];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash.finalize().into())
}

#[cfg(windows)]
fn open_entry(path: &Path) -> io::Result<File> {
    uv_windows::open_file_entry(path)
}

/// Atomically replace an export while its previous file identity remains open.
#[cfg(windows)]
pub(super) fn copy_executable(source: &Path, target: &Path) -> io::Result<()> {
    let parent = target.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "Executable path has no parent")
    })?;
    let temporary = tempfile::Builder::new().tempfile_in(uv_fs::verbatim_path(parent))?;
    fs_err::copy(source, temporary.path())?;

    // `keep` clears the temporary and read-only attributes before the standard library's rename
    // can use POSIX replacement semantics for a destination with an open identity handle.
    let (_file, path) = temporary.keep().map_err(|error| error.error)?;
    let mut temporary = tempfile::TempPath::try_from_path(path)?;
    let target = uv_fs::verbatim_path(target);
    uv_fs::with_retry_sync(&temporary, &target, "rename", || {
        fs_err::rename(&temporary, &target)
    })?;
    temporary.disable_cleanup(true);
    Ok(())
}

/// Compare exported directory entries, not the file objects to which hardlinks refer.
pub(super) fn same_entrypoint_location(left: &Path, right: &Path) -> anyhow::Result<bool> {
    compare_entrypoint_location(left, right, false)
}

/// Missing old exports grant no removal authority, even when their absent parent aliases cannot
/// be resolved. Competing receipt claims must use the strict comparison instead.
pub(super) fn same_existing_entrypoint_location(left: &Path, right: &Path) -> anyhow::Result<bool> {
    compare_entrypoint_location(left, right, true)
}

/// A newly planned name can be distinguished from a present entry by current alias lookup.
/// Two absent destinations still require the strict historical short-name check.
pub(super) fn same_planned_entrypoint_location(left: &Path, right: &Path) -> anyhow::Result<bool> {
    #[cfg(windows)]
    let missing_are_distinct = {
        let exists = |path: &Path| match fs_err::symlink_metadata(path) {
            Ok(_) => Ok(true),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err),
        };
        exists(left)? != exists(right)?
    };
    #[cfg(unix)]
    let missing_are_distinct = false;
    compare_entrypoint_location(left, right, missing_are_distinct)
}

fn compare_entrypoint_location(
    left: &Path,
    right: &Path,
    missing_are_distinct: bool,
) -> anyhow::Result<bool> {
    if left == right {
        return Ok(true);
    }
    let (Some(left_parent), Some(right_parent), Some(left_name), Some(right_name)) = (
        left.parent(),
        right.parent(),
        left.file_name(),
        right.file_name(),
    ) else {
        return Ok(false);
    };
    #[cfg(unix)]
    {
        let _ = missing_are_distinct;
        if left_name != right_name {
            return Ok(false);
        }
        for parent in [left_parent, right_parent] {
            match fs_err::metadata(parent) {
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => return Ok(false),
                Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
                Err(err) => return Err(err.into()),
            }
        }
        uv_fs::is_same_file_allow_missing(left_parent, right_parent).ok_or_else(|| {
            anyhow::anyhow!(
                "Cannot compare executable directories `{}` and `{}`",
                left_parent.user_display(),
                right_parent.user_display()
            )
        })
    }
    #[cfg(windows)]
    {
        let left_directory = uv_windows::open_directory(left_parent);
        let right_directory = uv_windows::open_directory(right_parent);
        let (left_directory, right_directory) = match (left_directory, right_directory) {
            (Ok(left), Ok(right)) => (left, right),
            (Err(left), Err(right))
                if left.kind() == io::ErrorKind::NotFound
                    && right.kind() == io::ErrorKind::NotFound =>
            {
                if !missing_are_distinct
                    && (uv_windows::names_equal_ordinal(left_name, right_name)?
                        || uv_windows::could_be_dos_short_name(left_name)?
                        || uv_windows::could_be_dos_short_name(right_name)?)
                {
                    bail!(
                        "Cannot compare missing executable directories `{}` and `{}`",
                        left_parent.user_display(),
                        right_parent.user_display()
                    );
                }
                return Ok(false);
            }
            (Err(left), Err(right)) => {
                return Err(if left.kind() == io::ErrorKind::NotFound {
                    right
                } else {
                    left
                }
                .into());
            }
            (Err(err), Ok(_)) | (Ok(_), Err(err)) if err.kind() == io::ErrorKind::NotFound => {
                return Ok(false);
            }
            (Err(err), Ok(_)) | (Ok(_), Err(err)) => return Err(err.into()),
        };
        if !left_directory.metadata()?.is_dir() || !right_directory.metadata()?.is_dir() {
            return Ok(false);
        }
        if uv_windows::FileIdentity::from_file(&left_directory)?
            != uv_windows::FileIdentity::from_file(&right_directory)?
        {
            return Ok(false);
        }
        if left_name == right_name {
            return Ok(true);
        }
        let case_sensitive = uv_windows::directory_is_case_sensitive(&left_directory)?;
        let actual_names = fs_err::read_dir(left_parent)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<io::Result<BTreeSet<_>>>()?;
        if actual_names.contains(left_name) && actual_names.contains(right_name) {
            return Ok(false);
        }
        let case_equal = uv_windows::names_equal_ordinal(left_name, right_name)?;
        if !case_sensitive && case_equal {
            return Ok(true);
        }
        // Non-case aliases (for example short names) need an exact directory-entry identity,
        // not equality of file IDs, which would also collapse distinct hardlinks.
        let exists = |path: &Path| match fs_err::symlink_metadata(path) {
            Ok(_) => Ok(true),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err),
        };
        let left_exists = exists(left)?;
        let right_exists = exists(right)?;
        if case_sensitive && case_equal && (!left_exists || !right_exists) {
            return Ok(false);
        }
        if (!left_exists || !right_exists)
            && !missing_are_distinct
            && (uv_windows::could_be_dos_short_name(left_name)?
                || uv_windows::could_be_dos_short_name(right_name)?)
        {
            bail!(
                "Cannot compare executable directory entries `{}` and `{}` while a possible short-name alias is missing",
                left.user_display(),
                right.user_display()
            );
        }
        if left_exists
            && right_exists
            && uv_windows::FileIdentity::from_file(&open_entry(left)?)?
                == uv_windows::FileIdentity::from_file(&open_entry(right)?)?
        {
            bail!(
                "Cannot compare executable directory entries `{}` and `{}`",
                left.user_display(),
                right.user_display()
            );
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use std::io::{Read, Seek};
    #[cfg(windows)]
    use std::os::windows::fs::MetadataExt;

    use super::ExportFingerprint;
    #[cfg(windows)]
    use super::{
        copy_executable, same_entrypoint_location, same_existing_entrypoint_location,
        same_planned_entrypoint_location,
    };

    #[cfg(windows)]
    #[test]
    fn replace_export_with_open_identity_handle() -> anyhow::Result<()> {
        const FILE_ATTRIBUTE_TEMPORARY: u32 = 0x100;

        let directory = tempfile::tempdir()?;
        let source = directory.path().join("source.exe");
        let export = directory.path().join("export.exe");
        let peer = directory.path().join("peer.exe");
        fs_err::write(&source, "new executable")?;
        fs_err::write(&export, "old executable")?;
        fs_err::hard_link(&export, &peer)?;
        let mut old_export = uv_windows::open_file_entry(&export)?;
        let identity = uv_windows::FileIdentity::from_file(&old_export)?;
        let source_permissions = fs_err::metadata(&source)?.permissions();
        let mut readonly = source_permissions.clone();
        readonly.set_readonly(true);
        fs_err::set_permissions(&source, readonly)?;
        let result = copy_executable(&source, &export);
        fs_err::set_permissions(&source, source_permissions)?;
        result?;

        assert_eq!(fs_err::read(&export)?, b"new executable");
        assert_eq!(fs_err::read(&peer)?, b"old executable");
        assert_ne!(
            uv_windows::FileIdentity::from_file(&uv_windows::open_file_entry(&export)?)?,
            identity
        );
        assert_eq!(
            uv_windows::FileIdentity::from_file(&uv_windows::open_file_entry(&peer)?)?,
            identity
        );
        assert_eq!(uv_windows::FileIdentity::from_file(&old_export)?, identity);
        old_export.rewind()?;
        let mut retained = Vec::new();
        old_export.read_to_end(&mut retained)?;
        assert_eq!(retained, b"old executable");
        let metadata = fs_err::metadata(&export)?;
        assert!(!metadata.permissions().readonly());
        assert_eq!(metadata.file_attributes() & FILE_ATTRIBUTE_TEMPORARY, 0);
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn missing_short_names_require_admission() -> anyhow::Result<()> {
        use std::ffi::OsStr;
        assert!(uv_windows::could_be_dos_short_name(OsStr::new(
            "CUSTOM.EXE"
        ))?);
        assert!(uv_windows::could_be_dos_short_name(OsStr::new(
            "LONGFI~1.EXE"
        ))?);
        assert!(!uv_windows::could_be_dos_short_name(OsStr::new(
            "unambiguously-long.exe"
        ))?);
        let directory = tempfile::tempdir()?;
        let long = directory.path().join("unambiguously-long.exe");
        let short = directory.path().join("CUSTOM.EXE");
        assert!(same_entrypoint_location(&long, &short).is_err());
        assert!(same_planned_entrypoint_location(&long, &short).is_err());
        assert!(!same_existing_entrypoint_location(&long, &short)?);
        let missing_long = directory
            .path()
            .join("missing")
            .join("unambiguously-long.exe");
        let missing_short = directory.path().join("missing").join("CUSTOM.EXE");
        assert!(same_entrypoint_location(&missing_long, &missing_short).is_err());
        assert!(same_planned_entrypoint_location(&missing_long, &missing_short).is_err());
        assert!(!same_existing_entrypoint_location(
            &missing_long,
            &missing_short
        )?);
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn present_and_missing_planned_names_are_distinct() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let present = directory.path().join("black.exe");
        let missing = directory.path().join("blackd.exe");
        fs_err::write(&present, "existing executable")?;
        assert!(same_entrypoint_location(&present, &missing).is_err());
        assert!(!same_planned_entrypoint_location(&present, &missing)?);
        assert!(!same_planned_entrypoint_location(&missing, &present)?);
        let alias = directory.path().join("BLACK.exe");
        assert!(same_planned_entrypoint_location(&present, &alias)?);
        fs_err::remove_file(&present)?;
        assert!(same_planned_entrypoint_location(&present, &missing).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn changed_symlink_loses_removal_authority() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("source");
        let export = directory.path().join("export");
        fs_err::write(&source, "before")?;
        fs_err::os::unix::fs::symlink(&source, &export)?;
        let fingerprint = ExportFingerprint::capture(&export)?;
        fs_err::write(&source, "after")?;
        assert!(fingerprint.matches(&export)?);
        fs_err::remove_file(&export)?;
        fs_err::os::unix::fs::symlink(directory.path().join("foreign"), &export)?;
        assert!(!fingerprint.matches(&export)?);
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn case_insensitive_names_and_distinct_hardlinks() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        assert!(!uv_windows::directory_is_case_sensitive(
            &uv_windows::open_directory(directory.path())?
        )?);
        let first = directory.path().join("first.exe");
        let alias = directory.path().join("FIRST.exe");
        let second = directory.path().join("second.exe");
        fs_err::write(&first, "original")?;
        fs_err::hard_link(&first, &second)?;
        assert!(same_entrypoint_location(&first, &alias)?);
        assert!(!same_entrypoint_location(&first, &second)?);
        let fingerprint = ExportFingerprint::capture(&first)?;
        fs_err::write(&first, "changed")?;
        assert!(!fingerprint.matches(&first)?);
        fs_err::remove_file(&first)?;
        assert!(same_entrypoint_location(&first, &alias)?);
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn case_sensitive_names_are_distinct() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let status = std::process::Command::new("fsutil.exe")
            .args(["file", "setCaseSensitiveInfo"])
            .arg(directory.path())
            .arg("enable")
            .status()?;
        anyhow::ensure!(
            status.success(),
            "Failed to create a case-sensitive test directory"
        );
        assert!(uv_windows::directory_is_case_sensitive(
            &uv_windows::open_directory(directory.path())?
        )?);
        let first = directory.path().join("first.exe");
        let second = directory.path().join("FIRST.exe");
        fs_err::write(&first, "first")?;
        fs_err::hard_link(&first, &second)?;
        assert!(!same_entrypoint_location(&first, &second)?);
        fs_err::remove_file(&second)?;
        assert!(!same_entrypoint_location(&first, &second)?);
        Ok(())
    }
}
