//! Recoverable publication of a fresh tool's executable set.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt::Write;
use std::io::{self, Read, Write as _};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, bail};
use itertools::Itertools;
use owo_colors::OwoColorize;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use uv_distribution_types::Name;
use uv_errors::{ErrorWithHints, Hinted};
use uv_fs::Simplified;
use uv_installer::SitePackages;
use uv_normalize::PackageName;
use uv_python::PythonEnvironment;
use uv_tool::{InstalledTools, ToolEntrypoint, ToolEntrypointLocks, entrypoint_paths};
use uv_warnings::warn_user;

use crate::commands::tool::common::{NoExecutablesError, matching_packages};
use crate::commands::tool::recovery::{
    same_entrypoint_location, same_existing_entrypoint_location,
};
use crate::printer::Printer;

pub(super) const JOURNAL_PREFIX: &str = ".uv-tool-exports-";
const JOURNAL_VERSION: u8 = 1;

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "platform", rename_all = "kebab-case")]
pub(super) enum ExportIdentity {
    Unix { device: u64, inode: u64 },
    Windows { identifier: [u8; 24] },
}

impl ExportIdentity {
    pub(super) fn directory(path: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        let file = fs_err::File::open(path)?;
        #[cfg(windows)]
        let file = uv_windows::open_directory(path)?;
        Self::from_file(&file)
    }

    fn from_file(file: &fs_err::File) -> io::Result<Self> {
        #[cfg(unix)]
        {
            let metadata = file.metadata()?;
            Ok(Self::Unix {
                device: metadata.dev(),
                inode: metadata.ino(),
            })
        }
        #[cfg(windows)]
        {
            Ok(Self::Windows {
                identifier: uv_windows::FileIdentity::from_file(file)?.to_bytes(),
            })
        }
    }

    fn at(path: &Path) -> io::Result<Option<Self>> {
        #[cfg(unix)]
        {
            match fs_err::symlink_metadata(path) {
                Ok(metadata) => Ok(Some(Self::Unix {
                    device: metadata.dev(),
                    inode: metadata.ino(),
                })),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
            }
        }
        #[cfg(windows)]
        {
            match uv_windows::open_file_entry(path) {
                Ok(file) => Self::from_file(&file).map(Some),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
            }
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum ExportContents {
    SymbolicLink { target: PathBuf },
    File { digest: [u8; 32] },
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct ExportVersion {
    identity: ExportIdentity,
    contents: ExportContents,
}

impl ExportVersion {
    pub(super) fn capture(path: &Path) -> anyhow::Result<Option<Self>> {
        let metadata = match fs_err::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        #[cfg(unix)]
        if metadata.is_symlink() {
            return Ok(Some(Self {
                identity: ExportIdentity::Unix {
                    device: metadata.dev(),
                    inode: metadata.ino(),
                },
                contents: ExportContents::SymbolicLink {
                    target: fs_err::read_link(path)?,
                },
            }));
        }
        if !metadata.is_file() || metadata.is_symlink() {
            bail!(
                "Executable `{}` is not a regular file or supported symbolic link",
                path.user_display()
            );
        }
        #[cfg(unix)]
        let file = fs_err::File::open(path)?;
        #[cfg(windows)]
        let file = uv_windows::open_file_entry(path)?;
        let identity = ExportIdentity::from_file(&file)?;
        if ExportIdentity::at(path)?.as_ref() != Some(&identity) {
            bail!(
                "Executable `{}` changed while being read",
                path.user_display()
            );
        }
        Ok(Some(Self {
            identity,
            contents: ExportContents::File {
                digest: digest_file(&file)?,
            },
        }))
    }

    pub(super) fn matches(&self, path: &Path) -> anyhow::Result<bool> {
        Ok(Self::capture(path)?.as_ref() == Some(self))
    }
}

fn digest_file(mut file: &fs_err::File) -> io::Result<[u8; 32]> {
    let mut digest = Sha256::new();
    let mut buffer = [0; 8192];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(digest.finalize().into())
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct JournalExport {
    pub(super) filename: PathBuf,
    pub(super) original: Option<ExportVersion>,
    pub(super) replacement: ExportVersion,
}

#[derive(Clone, Serialize, Deserialize)]
struct ExportJournal {
    version: u8,
    phase: JournalPhase,
    tool: PackageName,
    #[serde(flatten)]
    files: ExportDirectory,
    receipt_before: Option<[u8; 32]>,
    receipt: [u8; 32],
    lock: Option<[u8; 32]>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct ExportDirectory {
    pub(super) directory: PathBuf,
    pub(super) staging: PathBuf,
    pub(super) staging_identity: ExportIdentity,
    pub(super) exports: Vec<JournalExport>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum JournalPhase {
    Publishing,
    Committed,
}

pub(super) struct OwnedJournal<T> {
    pub(super) path: PathBuf,
    identity: ExportIdentity,
    // Pin the journal's identity until its contents have been checked and removed.
    _file: fs_err::File,
    bytes: Vec<u8>,
    pub(super) journal: T,
}

type JournalRecord = OwnedJournal<ExportJournal>;

impl ExportJournal {
    fn validate(&self, name: &PackageName) -> anyhow::Result<()> {
        if self.version != JOURNAL_VERSION || &self.tool != name || self.receipt_before.is_some() {
            bail!("Unsupported executable recovery journal for `{name}`");
        }
        self.files.validate(name)
    }
}

impl ExportDirectory {
    pub(super) fn validate(&self, name: &PackageName) -> anyhow::Result<()> {
        if !self.directory.is_absolute()
            || !is_filename(&self.staging)
            || !self.staging.to_string_lossy().starts_with(JOURNAL_PREFIX)
            || self
                .exports
                .iter()
                .any(|export| !is_filename(&export.filename))
        {
            bail!("Invalid executable recovery paths for `{name}`");
        }
        let mut names = BTreeSet::new();
        for export in &self.exports {
            if !names.insert(&export.filename) {
                bail!("Repeated executable recovery path for `{name}`");
            }
        }
        Ok(())
    }

    pub(super) fn staging_directory(&self) -> anyhow::Result<PathBuf> {
        let path = self.directory.join(&self.staging);
        let metadata = fs_err::symlink_metadata(&path)?;
        if !metadata.is_dir()
            || metadata.is_symlink()
            || fs_err::canonicalize(&path)? != path
            || ExportIdentity::directory(&path)? != self.staging_identity
        {
            bail!(
                "Executable recovery directory `{}` was replaced",
                path.user_display()
            );
        }
        Ok(path)
    }
}

impl OwnedJournal<ExportJournal> {
    fn read(installed_tools: &InstalledTools, name: &PackageName) -> anyhow::Result<Option<Self>> {
        let record = Self::read_from(journal_path(installed_tools, name))?;
        if let Some(record) = &record {
            record.journal.validate(name)?;
        }
        Ok(record)
    }

    fn create(installed_tools: &InstalledTools, journal: ExportJournal) -> anyhow::Result<Self> {
        journal.validate(&journal.tool)?;
        Self::create_at(journal_path(installed_tools, &journal.tool), journal)
    }

    fn mark_committed(&mut self) -> anyhow::Result<()> {
        if self.journal.phase == JournalPhase::Committed {
            return Ok(());
        }
        let mut journal = self.journal.clone();
        journal.phase = JournalPhase::Committed;
        self.replace(journal)
    }
}

impl<T: DeserializeOwned> OwnedJournal<T> {
    pub(super) fn read_from(path: PathBuf) -> anyhow::Result<Option<Self>> {
        let mut file = match fs_err::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let identity = ExportIdentity::from_file(&file)?;
        if !file.metadata()?.is_file() || ExportIdentity::at(&path)?.as_ref() != Some(&identity) {
            bail!(
                "Invalid executable recovery journal `{}`",
                path.user_display()
            );
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let journal: T = serde_json::from_slice(&bytes).with_context(|| {
            format!(
                "Invalid executable recovery journal `{}`",
                path.user_display()
            )
        })?;
        Ok(Some(Self {
            path,
            identity,
            _file: file,
            bytes,
            journal,
        }))
    }
}

impl<T: Serialize> OwnedJournal<T> {
    pub(super) fn create_at(path: PathBuf, journal: T) -> anyhow::Result<Self> {
        let bytes = serde_json::to_vec(&journal)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(JOURNAL_PREFIX)
            .tempfile_in(path.parent().context("Recovery journal has no parent")?)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        let (file, temporary) = temporary.into_parts();
        let file = fs_err::File::from_parts(file, &path);
        let identity = ExportIdentity::from_file(&file)?;
        temporary
            .persist_noclobber(&path)
            .map_err(|error| error.error)?;
        Ok(Self {
            path,
            identity,
            _file: file,
            bytes,
            journal,
        })
    }

    pub(super) fn replace(&mut self, journal: T) -> anyhow::Result<()> {
        self.check_current()?;
        let bytes = serde_json::to_vec(&journal)?;
        let parent = self
            .path
            .parent()
            .context("Recovery journal has no parent")?;
        let mut temporary = tempfile::Builder::new()
            .prefix(JOURNAL_PREFIX)
            .tempfile_in(parent)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        let (file, temporary) = temporary.into_parts();
        let file = fs_err::File::from_parts(file, &self.path);
        let identity = ExportIdentity::from_file(&file)?;
        self.check_current()?;
        #[cfg(unix)]
        temporary.persist(&self.path).map_err(|error| error.error)?;
        #[cfg(windows)]
        {
            // MoveFileEx cannot replace the journal while its identity handle is retained.
            // Clear the temporary attributes, then use the standard library's POSIX rename.
            let path = temporary.keep().map_err(|error| error.error)?;
            let mut temporary = tempfile::TempPath::try_from_path(path)?;
            let destination = uv_fs::verbatim_path(&self.path);
            uv_fs::with_retry_sync(&temporary, &destination, "rename", || {
                fs_err::rename(&temporary, &destination)
            })?;
            temporary.disable_cleanup(true);
        }
        self.identity = identity;
        self._file = file;
        self.bytes = bytes;
        self.journal = journal;
        sync_directory(parent)?;
        Ok(())
    }
}

impl<T> OwnedJournal<T> {
    pub(super) fn remove(&self) -> anyhow::Result<()> {
        self.check_current()?;
        fs_err::remove_file(&self.path)?;
        Ok(())
    }

    pub(super) fn check_current(&self) -> anyhow::Result<()> {
        if ExportIdentity::at(&self.path)?.as_ref() != Some(&self.identity)
            || fs_err::read(&self.path)? != self.bytes
        {
            bail!(
                "Executable recovery journal `{}` changed outside this installation",
                self.path.user_display()
            );
        }
        Ok(())
    }
}

fn is_filename(path: &Path) -> bool {
    let mut components = path.components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

/// Journals captured under the tool-root lock, before acquiring their destination locks.
struct PendingToolExportRecovery {
    records: Vec<JournalRecord>,
}

impl PendingToolExportRecovery {
    fn read(installed_tools: &InstalledTools, names: &[PackageName]) -> anyhow::Result<Self> {
        let names = if names.is_empty() {
            pending_export_names(installed_tools)?
        } else {
            names.to_vec()
        };
        let mut records = Vec::new();
        for name in names {
            if let Some(record) = JournalRecord::read(installed_tools, &name)? {
                records.push(record);
            }
        }
        Ok(Self { records })
    }

    fn directories(&self) -> impl Iterator<Item = &Path> {
        self.records
            .iter()
            .map(|record| record.journal.files.directory.as_path())
    }

    fn recover(self, installed_tools: &InstalledTools) -> anyhow::Result<()> {
        for directory in self.directories() {
            if fs_err::canonicalize(directory)? != directory {
                bail!(
                    "Executable recovery destination `{}` changed",
                    directory.user_display()
                );
            }
        }
        let mut errors = Vec::new();
        for mut record in self.records {
            if let Err(error) = recover_record(installed_tools, &mut record) {
                errors.push(format!("{error:#}"));
            }
        }
        if !errors.is_empty() {
            bail!("{}", errors.join("; "));
        }
        Ok(())
    }
}

/// Pending recovery records must outlive cleanup of an otherwise empty tool store.
pub(super) fn has_pending_exports(installed_tools: &InstalledTools) -> anyhow::Result<bool> {
    Ok(!pending_export_names(installed_tools)?.is_empty())
}

fn pending_export_names(installed_tools: &InstalledTools) -> anyhow::Result<Vec<PackageName>> {
    let mut names = BTreeSet::new();
    for entry in fs_err::read_dir(installed_tools.root())? {
        let entry = entry?;
        let filename = entry.file_name();
        let Some(filename) = filename.to_str() else {
            continue;
        };
        let Some(name) = filename
            .strip_prefix(JOURNAL_PREFIX)
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        let name = name.parse::<PackageName>()?;
        if entry.path() != journal_path(installed_tools, &name) {
            bail!("Invalid executable recovery journal name `{filename}`");
        }
        names.insert(name);
    }
    Ok(names.into_iter().collect())
}

fn journal_path(installed_tools: &InstalledTools, name: &PackageName) -> PathBuf {
    installed_tools
        .root()
        .join(format!("{JOURNAL_PREFIX}{name}.json"))
}

/// The complete export set, discovered before taking publication locks or changing commands.
pub(super) struct FreshToolExportPlan {
    directory: PathBuf,
    canonical_directory: PathBuf,
    exports: Vec<PreparedExport>,
}

pub(super) struct PreparedToolExports {
    plan: FreshToolExportPlan,
    staging: tempfile::TempDir,
    journal: ExportJournal,
    force: bool,
}

pub(super) struct ToolExportTransaction {
    record: JournalRecord,
    plan: FreshToolExportPlan,
    installed_tools: InstalledTools,
    finished: bool,
}

struct PreparedExport {
    entrypoint: ToolEntrypoint,
    source: PathBuf,
    provider: PackageName,
}

impl FreshToolExportPlan {
    pub(super) fn prepare(
        environment: &PythonEnvironment,
        name: &PackageName,
        providers: &[PackageName],
        printer: Printer,
    ) -> anyhow::Result<Self> {
        let site_packages = SitePackages::from_environment(environment)?;
        let environment_root = fs_err::canonicalize(environment.root())?;
        let directory = uv_tool::tool_executable_dir()?;
        fs_err::create_dir_all(&directory).context("Failed to create executable directory")?;
        let canonical_directory = fs_err::canonicalize(&directory)?;

        // A missing root command must not publish a dependency's commands or warnings first.
        let root = site_packages.get_packages(name);
        let Some(root) = root.first() else {
            return Err(NoExecutablesError::Root {
                package: name.clone(),
                matching_dependency_packages: Vec::new(),
            }
            .into());
        };
        let mut root_entries = entrypoint_paths(&site_packages, root.name(), root.version())?;
        if root_entries.is_empty() {
            return Err(NoExecutablesError::Root {
                package: name.clone(),
                matching_dependency_packages: matching_packages(name.as_ref(), &site_packages)
                    .into_iter()
                    .map(|distribution| distribution.name().clone())
                    .collect(),
            }
            .into());
        }

        let mut planned = BTreeMap::<PathBuf, PreparedExport>::new();
        let ordered = providers
            .iter()
            .filter(|provider| *provider != name)
            .collect::<BTreeSet<_>>();
        for provider in ordered.into_iter().chain(std::iter::once(name)) {
            let installed = site_packages.get_packages(provider);
            let Some(distribution) = installed.first() else {
                bail!("Expected package `{provider}` to be installed");
            };
            let entries = if provider == name {
                std::mem::take(&mut root_entries)
            } else {
                entrypoint_paths(&site_packages, distribution.name(), distribution.version())?
            };
            if entries.is_empty() {
                let error = NoExecutablesError::Dependency {
                    package: provider.clone(),
                };
                writeln!(
                    printer.stdout(),
                    "{}",
                    ErrorWithHints::new(&error, error.hints())
                )?;
                continue;
            }
            for (entry_name, source) in entries {
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
                let filename = source
                    .file_name()
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| OsString::from(&entry_name));
                let target = directory.join(filename);
                if same_entrypoint_location(&source, &target)? {
                    bail!(
                        "Cannot export executable `{}` into its tool environment",
                        target.user_display()
                    );
                }
                let mut previous_target = None;
                for previous in planned.keys() {
                    // Fresh names make no historical short-name claim. Recheck each actual
                    // destination at publication, when the filesystem has assigned any aliases.
                    if same_existing_entrypoint_location(previous, &target)? {
                        previous_target = Some(previous.clone());
                        break;
                    }
                }
                if let Some(previous_target) = previous_target {
                    planned.remove(&previous_target);
                }
                planned.insert(
                    target.clone(),
                    PreparedExport {
                        entrypoint: ToolEntrypoint::new(&entry_name, target, provider.to_string()),
                        source,
                        provider: provider.clone(),
                    },
                );
            }
        }

        Ok(Self {
            directory,
            canonical_directory,
            exports: planned.into_values().collect(),
        })
    }

    /// Directory admission belongs between preparation and the authoritative publication checks.
    fn canonical_directory(&self) -> &Path {
        &self.canonical_directory
    }

    pub(super) fn entrypoints(&self) -> Vec<ToolEntrypoint> {
        self.exports
            .iter()
            .map(|export| export.entrypoint.clone())
            .collect()
    }

    pub(super) fn check_conflicts(&self, force: bool) -> anyhow::Result<()> {
        if force {
            return Ok(());
        }
        let existing = self
            .exports
            .iter()
            .filter(|export| export.entrypoint.install_path.exists())
            .map(|export| {
                export.entrypoint.install_path.file_name().map_or_else(
                    || export.entrypoint.name.clone().into(),
                    |filename| filename.to_string_lossy(),
                )
            })
            .collect::<Vec<_>>();
        if existing.is_empty() {
            return Ok(());
        }
        let (suffix, verb) = if existing.len() == 1 {
            ("", "exists")
        } else {
            ("s", "exist")
        };
        bail!(
            "Executable{suffix} already {verb}: {} (use `--force` to overwrite)",
            existing.iter().map(|name| name.bold()).join(", ")
        )
    }

    /// Prepare data and identity anchors while the shared command directory is unlocked.
    pub(super) fn stage(
        self,
        name: &PackageName,
        receipt: &[u8],
        lock: Option<&[u8]>,
        force: bool,
    ) -> anyhow::Result<PreparedToolExports> {
        self.check_conflicts(force)?;
        if fs_err::canonicalize(&self.directory)? != self.canonical_directory {
            bail!("Executable directory changed during installation");
        }
        let staging = tempfile::Builder::new()
            .prefix(JOURNAL_PREFIX)
            .tempdir_in(&self.canonical_directory)?;
        let mut exports = Vec::with_capacity(self.exports.len());
        for (index, export) in self.exports.iter().enumerate() {
            let filename = export.entrypoint.install_path.file_name().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Executable path has no filename",
                )
            })?;
            let target = self.canonical_directory.join(filename);
            let original = ExportVersion::capture(&target)?;
            if let Some(original) = &original {
                let backup = old_anchor(staging.path(), index);
                fs_err::hard_link(&target, &backup)?;
                if !original.matches(&backup)? || !original.matches(&target)? {
                    bail!(
                        "Executable `{}` changed while preparing its backup",
                        target.user_display()
                    );
                }
            }
            let prepared = new_anchor(staging.path(), index);
            #[cfg(unix)]
            fs_err::os::unix::fs::symlink(&export.source, &prepared)?;
            #[cfg(windows)]
            {
                super::recovery::copy_executable(&export.source, &prepared)?;
                fs_err::OpenOptions::new()
                    .write(true)
                    .open(&prepared)?
                    .sync_all()?;
            }
            let replacement = ExportVersion::capture(&prepared)?.ok_or_else(|| {
                io::Error::other("Prepared executable disappeared during installation")
            })?;
            exports.push(JournalExport {
                filename: PathBuf::from(filename),
                original,
                replacement,
            });
        }
        sync_directory(staging.path())?;
        sync_directory(&self.canonical_directory)?;
        let journal = ExportJournal {
            version: JOURNAL_VERSION,
            phase: JournalPhase::Publishing,
            tool: name.clone(),
            files: ExportDirectory {
                directory: self.canonical_directory.clone(),
                staging: staging
                    .path()
                    .file_name()
                    .ok_or_else(|| io::Error::other("Recovery directory has no filename"))?
                    .into(),
                staging_identity: ExportIdentity::directory(staging.path())?,
                exports,
            },
            receipt_before: None,
            receipt: Sha256::digest(receipt).into(),
            lock: lock.map(|contents| Sha256::digest(contents).into()),
        };
        Ok(PreparedToolExports {
            plan: self,
            staging,
            journal,
            force,
        })
    }
}

impl PreparedToolExports {
    pub(super) fn canonical_directory(&self) -> &Path {
        self.plan.canonical_directory()
    }

    /// Recheck admission after locking the destination, then record intent before publishing.
    pub(super) fn begin(
        self,
        installed_tools: &InstalledTools,
    ) -> anyhow::Result<ToolExportTransaction> {
        self.plan.check_conflicts(self.force)?;
        if fs_err::canonicalize(&self.plan.directory)? != self.journal.files.directory {
            bail!("Executable directory changed during installation");
        }
        let receipt = installed_tools
            .tool_dir(&self.journal.tool)
            .join("uv-receipt.toml");
        match fs_err::symlink_metadata(&receipt) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Ok(_) => bail!("Tool receipt appeared during fresh installation"),
            Err(error) => return Err(error.into()),
        }
        for export in &self.journal.files.exports {
            let target = self.journal.files.directory.join(&export.filename);
            if ExportVersion::capture(&target)? != export.original {
                bail!(
                    "Executable `{}` changed after installation was prepared",
                    target.user_display()
                );
            }
        }
        let record = JournalRecord::create(installed_tools, self.journal)?;
        // The persisted journal now owns these anchors, including after an abrupt process exit.
        let _ = self.staging.keep();
        let transaction = ToolExportTransaction {
            record,
            plan: self.plan,
            installed_tools: installed_tools.clone(),
            finished: false,
        };
        sync_directory(installed_tools.root())?;
        Ok(transaction)
    }
}

impl ToolExportTransaction {
    pub(super) fn publish(&mut self, printer: Printer) -> anyhow::Result<()> {
        for index in 0..self.record.journal.files.exports.len() {
            publish_export(&self.record.journal.files, index)?;
        }
        let mut names = BTreeMap::<PackageName, BTreeSet<String>>::new();
        for export in &self.plan.exports {
            names
                .entry(export.provider.clone())
                .or_default()
                .insert(export.entrypoint.name.clone());
        }
        let name = &self.record.journal.tool;
        let providers = names
            .keys()
            .filter(|provider| *provider != name)
            .chain(std::iter::once(name));
        for provider in providers {
            let Some(commands) = names.get(provider) else {
                continue;
            };
            let suffix = if commands.len() == 1 { "" } else { "s" };
            let from = if provider == name {
                String::new()
            } else {
                format!(" from `{provider}`")
            };
            writeln!(
                printer.stderr(),
                "Installed {} executable{suffix}{from}: {}",
                commands.len(),
                commands.iter().map(|command| command.bold()).join(", ")
            )?;
        }
        if fs_err::canonicalize(&self.plan.directory)? != self.record.journal.files.directory {
            bail!("Executable directory changed before recording the installation");
        }
        Ok(())
    }

    /// The complete fresh receipt is the commit witness; old receipts are never accepted here.
    pub(super) fn commit(mut self) -> anyhow::Result<()> {
        if !metadata_matches(&self.installed_tools, &self.record.journal)? {
            bail!("Tool metadata does not match the completed executable installation");
        }
        self.finished = true;
        sync_metadata(&self.installed_tools, &self.record.journal)
            .context("Tool is installed, but its metadata could not be synchronized")?;
        self.record
            .mark_committed()
            .context("Tool is installed, but its recovery commit could not be saved")?;
        if let Err(error) =
            cleanup_anchors(&self.record.journal.files).and_then(|()| self.record.remove())
        {
            warn_user!(
                "Installed `{}`, but executable recovery cleanup is pending: {error:#}. Recovery information remains at `{}`",
                self.record.journal.tool,
                self.record.path.user_display()
            );
        } else if let Err(error) = sync_directory(self.installed_tools.root()) {
            warn_user!("Could not synchronize executable recovery cleanup: {error}");
        }
        Ok(())
    }
}

impl Drop for ToolExportTransaction {
    fn drop(&mut self) {
        if !self.finished
            && let Err(error) = recover_record(&self.installed_tools, &mut self.record)
        {
            warn_user!(
                "Could not recover executables for `{}`: {error:#}. Recovery information remains at `{}`",
                self.record.journal.tool,
                self.record.path.user_display()
            );
        }
    }
}

pub(super) fn publish_export(journal: &ExportDirectory, index: usize) -> anyhow::Result<()> {
    let staging = publish_export_data(journal, index)?;
    fs_err::hard_link(
        new_anchor(&staging, index),
        staging.join(format!("published-{index}")),
    )?;
    sync_directory(&staging)?;
    sync_directory(&journal.directory)?;
    Ok(())
}

pub(super) fn publish_export_data(
    journal: &ExportDirectory,
    index: usize,
) -> anyhow::Result<PathBuf> {
    let staging = journal.staging_directory()?;
    let export = &journal.exports[index];
    let target = journal.directory.join(&export.filename);
    let prepared = new_anchor(&staging, index);
    if !export.replacement.matches(&prepared)?
        || ExportVersion::capture(&target)? != export.original
    {
        bail!(
            "Executable `{}` changed before publication",
            target.user_display()
        );
    }
    if export.original.is_none() {
        fs_err::hard_link(&prepared, &target)?;
    } else {
        let candidate = staging.join(format!("publish-{index}"));
        fs_err::hard_link(&prepared, &candidate)?;
        #[cfg(unix)]
        fs_err::rename(&candidate, &target)?;
        #[cfg(windows)]
        uv_windows::replace_file_with_backup(
            &uv_fs::verbatim_path(&target),
            &uv_fs::verbatim_path(&candidate),
            &uv_fs::verbatim_path(&staging.join(format!("displaced-{index}"))),
        )?;
    }
    Ok(staging)
}

fn metadata_matches(
    installed_tools: &InstalledTools,
    journal: &ExportJournal,
) -> anyhow::Result<bool> {
    let directory = installed_tools.tool_dir(&journal.tool);
    if file_digest(&directory.join("uv-receipt.toml"))? != Some(journal.receipt) {
        return Ok(false);
    }
    Ok(file_digest(&directory.join("uv.lock"))? == journal.lock)
}

pub(super) fn file_digest(path: &Path) -> io::Result<Option<[u8; 32]>> {
    let file = match fs_err::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    digest_file(&file).map(Some)
}

fn sync_metadata(installed_tools: &InstalledTools, journal: &ExportJournal) -> io::Result<()> {
    let directory = installed_tools.tool_dir(&journal.tool);
    fs_err::OpenOptions::new()
        .write(true)
        .open(directory.join("uv-receipt.toml"))?
        .sync_all()?;
    if journal.lock.is_some() {
        fs_err::OpenOptions::new()
            .write(true)
            .open(directory.join("uv.lock"))?
            .sync_all()?;
    }
    sync_directory(&directory)
}

/// Recover the selected tool before reading its receipt or replacing its environment.
pub(super) async fn recover_tool_exports(
    installed_tools: &InstalledTools,
    name: &PackageName,
) -> anyhow::Result<()> {
    recover_selected_exports(installed_tools, std::slice::from_ref(name)).await
}

pub(super) async fn recover_selected_exports(
    installed_tools: &InstalledTools,
    names: &[PackageName],
) -> anyhow::Result<()> {
    let pending = PendingToolExportRecovery::read(installed_tools, names)?;
    let _entrypoint_locks =
        ToolEntrypointLocks::for_directories(pending.directories().map(Path::to_path_buf)).await?;
    pending.recover(installed_tools)
}

fn recover_record(
    installed_tools: &InstalledTools,
    record: &mut JournalRecord,
) -> anyhow::Result<()> {
    // A fresh receipt cannot equal an earlier receipt: publication requires it to be absent.
    // Later external edits to commands do not undo a completed installation.
    if record.journal.phase == JournalPhase::Committed {
        // Later metadata updates cannot revoke this installation's persisted commit.
    } else if metadata_matches(installed_tools, &record.journal)? {
        sync_metadata(installed_tools, &record.journal)?;
        record.mark_committed()?;
    } else {
        let mut errors = Vec::new();
        for index in (0..record.journal.files.exports.len()).rev() {
            if let Err(error) = rollback_export(&record.journal.files, index) {
                errors.push(format!("{error:#}"));
            }
        }
        if !errors.is_empty() {
            bail!("{}", errors.join("; "));
        }
    }
    cleanup_anchors(&record.journal.files)?;
    record.remove()?;
    sync_directory(installed_tools.root())?;
    Ok(())
}

pub(super) fn rollback_export(journal: &ExportDirectory, index: usize) -> anyhow::Result<()> {
    let export = &journal.exports[index];
    let target = journal.directory.join(&export.filename);
    let current = ExportVersion::capture(&target)?;
    if current == export.original {
        return Ok(());
    }
    if current.as_ref() != Some(&export.replacement) {
        #[cfg(windows)]
        if current.is_none() {
            let staging = journal.staging_directory()?;
            let displaced = staging.join(format!("displaced-{index}"));
            if let Some(original) = &export.original
                && original.matches(&displaced)?
                && ExportIdentity::at(&staging.join(format!("published-{index}")))?.is_none()
            {
                // ReplaceFileW can move the original to its backup before reporting failure.
                fs_err::hard_link(&displaced, &target)?;
                sync_directory(&journal.directory)?;
                return Ok(());
            }
        }
        bail!(
            "Executable `{}` changed outside this installation; leaving it unchanged",
            target.user_display()
        );
    }
    if let Some(original) = &export.original {
        let staging = journal.staging_directory()?;
        let backup = old_anchor(&staging, index);
        if !original.matches(&backup)? {
            bail!("Executable backup `{}` changed", backup.user_display());
        }
        let restore = staging.join(format!("restore-{index}"));
        match fs_err::hard_link(&backup, &restore) {
            Ok(()) => {}
            Err(error)
                if error.kind() == io::ErrorKind::AlreadyExists
                    && original.matches(&restore)? => {}
            Err(error) => return Err(error.into()),
        }
        #[cfg(unix)]
        fs_err::rename(&restore, &target)?;
        #[cfg(windows)]
        uv_windows::replace_file_with_backup(
            &uv_fs::verbatim_path(&target),
            &uv_fs::verbatim_path(&restore),
            &uv_fs::verbatim_path(&staging.join(format!("rolled-back-{index}"))),
        )?;
    } else {
        fs_err::remove_file(&target)?;
    }
    sync_directory(&journal.directory)?;
    Ok(())
}

pub(super) fn cleanup_anchors(journal: &ExportDirectory) -> anyhow::Result<()> {
    let path = journal.directory.join(&journal.staging);
    match fs_err::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    let staging = journal.staging_directory()?;
    for (index, export) in journal.exports.iter().enumerate() {
        for (prefix, expected) in [
            ("new", Some(&export.replacement)),
            ("publish", Some(&export.replacement)),
            ("published", Some(&export.replacement)),
            ("rolled-back", Some(&export.replacement)),
            ("old", export.original.as_ref()),
            ("restore", export.original.as_ref()),
            ("displaced", export.original.as_ref()),
        ] {
            let path = staging.join(format!("{prefix}-{index}"));
            let Some(current) = ExportVersion::capture(&path)? else {
                continue;
            };
            if Some(&current) != expected {
                bail!(
                    "Executable recovery file `{}` changed; leaving it unchanged",
                    path.user_display()
                );
            }
            #[cfg(unix)]
            fs_err::remove_file(path)?;
            #[cfg(windows)]
            uv_windows::remove_file_preserving_attributes(&path)?;
        }
    }
    // Unknown entries prevent directory removal and keep the journal available for inspection.
    fs_err::remove_dir(&staging)?;
    sync_directory(&journal.directory)?;
    Ok(())
}

pub(super) fn old_anchor(directory: &Path, index: usize) -> PathBuf {
    directory.join(format!("old-{index}"))
}

pub(super) fn new_anchor(directory: &Path, index: usize) -> PathBuf {
    directory.join(format!("new-{index}"))
}

#[cfg_attr(
    windows,
    expect(
        clippy::unnecessary_wraps,
        reason = "directory synchronization is fallible on Unix"
    )
)]
pub(super) fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    fs_err::File::open(path)?.sync_all()?;
    #[cfg(windows)]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    use anyhow::{Context, bail};
    use uv_normalize::PackageName;
    use uv_settings::ToolOptions;
    use uv_tool::{InstalledTools, PreparedToolReceipt, Tool, ToolEntrypoint, ToolEntrypointLocks};

    #[cfg(unix)]
    use super::publish_export;
    use super::{
        ExportVersion, FreshToolExportPlan, JournalRecord, PreparedExport, ToolExportTransaction,
        journal_path, publish_export_data, recover_tool_exports,
    };
    use crate::printer::Printer;

    fn isolated_root(test: &str) -> anyhow::Result<Option<PathBuf>> {
        const ROOT: &str = "UV_TEST_TOOL_EXPORT_ROOT";
        if let Some(root) = std::env::var_os(ROOT) {
            return Ok(Some(root.into()));
        }
        let directory = tempfile::tempdir()?;
        let test = format!("commands::tool::export_transaction::tests::{test}");
        let output = Command::new(std::env::current_exe()?)
            .args(["--exact", &test, "--nocapture", "--test-threads=1"])
            .env_remove("__RUST_TEST_INVOKE")
            .env(ROOT, directory.path())
            .env("UV_TOOL_DIR", directory.path().join("tools"))
            .output()?;
        anyhow::ensure!(output.status.success(), "{output:?}");
        Ok(None)
    }

    struct Installation {
        tools: InstalledTools,
        name: PackageName,
        directory: PathBuf,
        sources: PathBuf,
    }

    impl Installation {
        fn at(root: &Path) -> anyhow::Result<Self> {
            let tools = InstalledTools::from_settings()?;
            anyhow::ensure!(tools.root() == root.join("tools"));
            let tools = tools.init()?;
            let name = "example".parse()?;
            fs_err::create_dir_all(tools.tool_dir(&name))?;
            let directory = root.join("bin");
            let sources = root.join("sources");
            fs_err::create_dir_all(&directory)?;
            fs_err::create_dir_all(&sources)?;
            for command in ["alpha", "beta"] {
                fs_err::write(sources.join(command), format!("new {command}"))?;
            }
            Ok(Self {
                tools,
                name,
                directory,
                sources,
            })
        }

        fn target(&self, command: &str) -> PathBuf {
            self.directory
                .join(format!("{command}{}", std::env::consts::EXE_SUFFIX))
        }

        fn begin(
            &self,
            force: bool,
        ) -> anyhow::Result<(ToolExportTransaction, PreparedToolReceipt)> {
            let plan = FreshToolExportPlan {
                directory: self.directory.clone(),
                canonical_directory: fs_err::canonicalize(&self.directory)?,
                exports: ["alpha", "beta"]
                    .into_iter()
                    .map(|command| PreparedExport {
                        entrypoint: ToolEntrypoint::new(
                            command,
                            self.target(command),
                            self.name.to_string(),
                        ),
                        source: self.sources.join(command),
                        provider: self.name.clone(),
                    })
                    .collect(),
            };
            let tool = Tool::new(
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                None,
                plan.entrypoints(),
                ToolOptions::default(),
            );
            let receipt = self.tools.prepare_tool_receipt(&self.name, tool)?;
            let prepared = plan.stage(&self.name, receipt.as_bytes(), None, force)?;
            Ok((prepared.begin(&self.tools)?, receipt))
        }

        fn replace_alpha(&self) -> anyhow::Result<ExportVersion> {
            let foreign = self.directory.join("foreign");
            #[cfg(unix)]
            fs_err::os::unix::fs::symlink(self.sources.join("alpha"), &foreign)?;
            #[cfg(windows)]
            fs_err::copy(self.sources.join("alpha"), &foreign)?;
            fs_err::rename(&foreign, self.target("alpha"))?;
            ExportVersion::capture(&self.target("alpha"))?.context("foreign executable is missing")
        }
    }

    #[test]
    #[cfg(windows)]
    fn unlink_readonly_backup_keeps_original_attributes() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let original = directory.path().join("command.exe");
        let backup = directory.path().join("backup.exe");
        fs_err::write(&original, "original command")?;
        fs_err::hard_link(&original, &backup)?;
        let permissions = fs_err::metadata(&original)?.permissions();
        let mut readonly = permissions.clone();
        readonly.set_readonly(true);
        fs_err::set_permissions(&original, readonly)?;
        let result = uv_windows::remove_file_preserving_attributes(&backup);
        let retained = fs_err::metadata(&original)?.permissions().readonly();
        fs_err::set_permissions(&original, permissions)?;
        result?;
        assert!(retained);
        assert!(!backup.exists());
        assert_eq!(fs_err::read(&original)?, b"original command");
        Ok(())
    }

    #[test]
    #[cfg(windows)]
    fn forced_replacement_retains_acl_and_old_alias() -> anyhow::Result<()> {
        let Some(root) = isolated_root("forced_replacement_retains_acl_and_old_alias")? else {
            return Ok(());
        };
        let installation = Installation::at(&root)?;
        let target = installation.target("alpha");
        fs_err::write(&target, "original command")?;
        let alias = root.as_path().join("original-alias.exe");
        fs_err::hard_link(&target, &alias)?;
        let protected = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference = 'Stop'; $acl = Get-Acl -LiteralPath $env:UV_TEST_ACL_PATH; $acl.SetAccessRuleProtection($true, $true); Set-Acl -LiteralPath $env:UV_TEST_ACL_PATH -AclObject $acl"])
            .env("UV_TEST_ACL_PATH", &target)
            .output()?;
        assert!(protected.status.success(), "{protected:?}");
        let descriptor = |path: &Path| -> anyhow::Result<Vec<u8>> {
            let output = Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference = 'Stop'; (Get-Acl -LiteralPath $env:UV_TEST_ACL_PATH).Sddl"])
                .env("UV_TEST_ACL_PATH", path)
                .output()?;
            anyhow::ensure!(output.status.success(), "{output:?}");
            anyhow::ensure!(!output.stdout.is_empty());
            Ok(output.stdout)
        };
        let original = descriptor(&target)?;
        let (mut transaction, _) = installation.begin(true)?;
        transaction.publish(Printer::Silent)?;
        assert_eq!(descriptor(&target)?, original);
        assert_eq!(descriptor(&alias)?, original);
        assert_eq!(fs_err::read(&alias)?, b"original command");
        drop(transaction);
        assert_eq!(descriptor(&target)?, original);
        assert_eq!(fs_err::read(&target)?, b"original command");
        Ok(())
    }

    #[test]
    fn staging_failure_keeps_existing_exports() -> anyhow::Result<()> {
        let Some(root) = isolated_root("staging_failure_keeps_existing_exports")? else {
            return Ok(());
        };
        let installation = Installation::at(&root)?;
        fs_err::write(installation.target("alpha"), "original command")?;
        fs_err::create_dir(installation.target("beta"))?;
        let original = ExportVersion::capture(&installation.target("alpha"))?;
        assert!(installation.begin(true).is_err());
        assert_eq!(
            ExportVersion::capture(&installation.target("alpha"))?,
            original
        );
        assert!(installation.target("beta").is_dir());
        assert_eq!(fs_err::read_dir(&installation.directory)?.count(), 2);
        assert!(!journal_path(&installation.tools, &installation.name).exists());
        Ok(())
    }

    #[test]
    #[cfg(windows)]
    fn replacement_rejects_changed_integrity_label() -> anyhow::Result<()> {
        let Some(root) = isolated_root("replacement_rejects_changed_integrity_label")? else {
            return Ok(());
        };
        let installation = Installation::at(&root)?;
        fs_err::write(installation.target("alpha"), "original command")?;
        let label = Command::new("icacls")
            .arg(installation.target("alpha"))
            .args(["/setintegritylevel", "L"])
            .output()?;
        assert!(label.status.success(), "{label:?}");
        let original = ExportVersion::capture(&installation.target("alpha"))?;
        let (mut transaction, _) = installation.begin(true)?;
        assert!(transaction.publish(Printer::Silent).is_err());
        drop(transaction);
        assert_eq!(
            ExportVersion::capture(&installation.target("alpha"))?,
            original
        );
        assert!(!installation.target("beta").exists());
        assert!(!journal_path(&installation.tools, &installation.name).exists());
        Ok(())
    }

    #[test]
    fn failed_install_restores_forced_command() -> anyhow::Result<()> {
        let Some(root) = isolated_root("failed_install_restores_forced_command")? else {
            return Ok(());
        };
        let installation = Installation::at(&root)?;
        fs_err::write(installation.target("alpha"), "original command")?;
        let original = ExportVersion::capture(&installation.target("alpha"))?;
        let (mut transaction, _) = installation.begin(true)?;
        transaction.publish(Printer::Silent)?;
        drop(transaction);
        assert_eq!(
            ExportVersion::capture(&installation.target("alpha"))?,
            original
        );
        assert!(!installation.target("beta").exists());
        assert!(!journal_path(&installation.tools, &installation.name).exists());
        Ok(())
    }

    #[test]
    fn identical_foreign_replacement_is_not_removed() -> anyhow::Result<()> {
        let Some(root) = isolated_root("identical_foreign_replacement_is_not_removed")? else {
            return Ok(());
        };
        let installation = Installation::at(&root)?;
        let (mut transaction, _) = installation.begin(false)?;
        transaction.publish(Printer::Silent)?;
        let foreign = installation.replace_alpha()?;
        assert_ne!(
            foreign,
            transaction.record.journal.files.exports[0].replacement
        );
        drop(transaction);
        assert!(foreign.matches(&installation.target("alpha"))?);
        assert!(!installation.target("beta").exists());
        assert!(journal_path(&installation.tools, &installation.name).exists());
        Ok(())
    }

    #[test]
    fn complete_receipt_keeps_other_commands_after_foreign_edit() -> anyhow::Result<()> {
        let Some(root) = isolated_root("complete_receipt_keeps_other_commands_after_foreign_edit")?
        else {
            return Ok(());
        };
        let installation = Installation::at(&root)?;
        let (mut transaction, receipt) = installation.begin(false)?;
        transaction.publish(Printer::Silent)?;
        installation
            .tools
            .publish_new_tool_receipt(&installation.name, &receipt)?;
        let foreign = installation.replace_alpha()?;
        drop(transaction);
        assert!(foreign.matches(&installation.target("alpha"))?);
        assert_eq!(fs_err::read(installation.target("beta"))?, b"new beta");
        assert!(!journal_path(&installation.tools, &installation.name).exists());
        Ok(())
    }

    #[tokio::test]
    async fn committed_cleanup_survives_later_metadata_changes() -> anyhow::Result<()> {
        let Some(root) = isolated_root("committed_cleanup_survives_later_metadata_changes")? else {
            return Ok(());
        };
        let installation = Installation::at(&root)?;
        fs_err::write(installation.target("alpha"), "original command")?;
        let (mut transaction, receipt) = installation.begin(true)?;
        transaction.publish(Printer::Silent)?;
        installation
            .tools
            .publish_new_tool_receipt(&installation.name, &receipt)?;
        let obstruction = transaction
            .record
            .journal
            .files
            .staging_directory()?
            .join("leave-alone");
        fs_err::write(&obstruction, "external recovery-directory entry")?;
        let previous_journal = fs_err::File::open(&transaction.record.path)?;
        transaction.commit()?;
        let previous: super::ExportJournal = serde_json::from_reader(previous_journal)?;
        assert_eq!(previous.phase, super::JournalPhase::Publishing);
        assert_eq!(
            fs_err::read(&obstruction)?,
            b"external recovery-directory entry"
        );
        let record = JournalRecord::read(&installation.tools, &installation.name)?
            .context("committed cleanup lost its journal")?;
        assert_eq!(record.journal.phase, super::JournalPhase::Committed);
        drop(record);

        let receipt_path = installation
            .tools
            .tool_dir(&installation.name)
            .join("uv-receipt.toml");
        fs_err::write(&receipt_path, "metadata from a later operation")?;
        let foreign = installation.replace_alpha()?;
        fs_err::remove_file(&obstruction)?;
        recover_tool_exports(&installation.tools, &installation.name).await?;
        assert!(foreign.matches(&installation.target("alpha"))?);
        assert_eq!(fs_err::read(installation.target("beta"))?, b"new beta");
        assert_eq!(
            fs_err::read(receipt_path)?,
            b"metadata from a later operation"
        );
        assert!(!journal_path(&installation.tools, &installation.name).exists());
        Ok(())
    }

    #[test]
    fn receipt_publication_failure_restores_exports() -> anyhow::Result<()> {
        let Some(root) = isolated_root("receipt_publication_failure_restores_exports")? else {
            return Ok(());
        };
        let installation = Installation::at(&root)?;
        fs_err::write(installation.target("alpha"), "original command")?;
        let original = ExportVersion::capture(&installation.target("alpha"))?;
        let (mut transaction, receipt) = installation.begin(true)?;
        transaction.publish(Printer::Silent)?;
        let receipt_path = installation
            .tools
            .tool_dir(&installation.name)
            .join("uv-receipt.toml");
        fs_err::write(&receipt_path, "foreign receipt")?;
        assert!(
            installation
                .tools
                .publish_new_tool_receipt(&installation.name, &receipt)
                .is_err()
        );
        drop(transaction);
        assert_eq!(
            ExportVersion::capture(&installation.target("alpha"))?,
            original
        );
        assert!(!installation.target("beta").exists());
        assert_eq!(fs_err::read(receipt_path)?, b"foreign receipt");
        assert!(!journal_path(&installation.tools, &installation.name).exists());
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn second_publication_io_failure_restores_first() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let Some(root) = isolated_root("second_publication_io_failure_restores_first")? else {
            return Ok(());
        };
        let installation = Installation::at(&root)?;
        fs_err::write(installation.target("alpha"), "original command")?;
        let original = ExportVersion::capture(&installation.target("alpha"))?;
        let (transaction, _) = installation.begin(true)?;
        publish_export(&transaction.record.journal.files, 0)?;
        let permissions = fs_err::metadata(&installation.directory)?.permissions();
        fs_err::set_permissions(
            &installation.directory,
            std::fs::Permissions::from_mode(0o500),
        )?;
        let result = publish_export(&transaction.record.journal.files, 1);
        let privileged =
            fs_err::write(installation.directory.join("authorization-probe"), "").is_ok();
        fs_err::set_permissions(&installation.directory, permissions)?;
        assert_eq!(result.is_ok(), privileged);
        drop(transaction);
        assert_eq!(
            ExportVersion::capture(&installation.target("alpha"))?,
            original
        );
        assert!(!installation.target("beta").exists());
        assert!(!journal_path(&installation.tools, &installation.name).exists());
        Ok(())
    }

    #[test]
    #[cfg(windows)]
    fn foreign_in_place_edit_keeps_recovery_backups() -> anyhow::Result<()> {
        let Some(root) = isolated_root("foreign_in_place_edit_keeps_recovery_backups")? else {
            return Ok(());
        };
        let installation = Installation::at(&root)?;
        fs_err::write(installation.target("alpha"), "original command")?;
        let (mut transaction, _) = installation.begin(true)?;
        transaction.publish(Printer::Silent)?;
        fs_err::write(installation.target("alpha"), "foreign edit")?;
        drop(transaction);
        assert_eq!(fs_err::read(installation.target("alpha"))?, b"foreign edit");
        let record = JournalRecord::read(&installation.tools, &installation.name)?
            .context("recovery journal is missing")?;
        let backup = super::old_anchor(&record.journal.files.staging_directory()?, 0);
        assert_eq!(fs_err::read(backup)?, b"original command");
        Ok(())
    }

    #[test]
    #[cfg(windows)]
    fn interrupted_replacement_restores_displaced_original() -> anyhow::Result<()> {
        let Some(root) = isolated_root("interrupted_replacement_restores_displaced_original")?
        else {
            return Ok(());
        };
        let installation = Installation::at(&root)?;
        fs_err::write(installation.target("alpha"), "original command")?;
        let original = ExportVersion::capture(&installation.target("alpha"))?;
        let (transaction, _) = installation.begin(true)?;
        let staging = transaction.record.journal.files.staging_directory()?;
        // ReplaceFileW documents this intermediate state when moving the replacement fails.
        fs_err::rename(installation.target("alpha"), staging.join("displaced-0"))?;
        drop(transaction);
        assert_eq!(
            ExportVersion::capture(&installation.target("alpha"))?,
            original
        );
        assert!(!journal_path(&installation.tools, &installation.name).exists());
        Ok(())
    }

    #[test]
    fn deleted_acknowledged_export_is_not_recreated() -> anyhow::Result<()> {
        let Some(root) = isolated_root("deleted_acknowledged_export_is_not_recreated")? else {
            return Ok(());
        };
        let installation = Installation::at(&root)?;
        fs_err::write(installation.target("alpha"), "original command")?;
        let (mut transaction, _) = installation.begin(true)?;
        transaction.publish(Printer::Silent)?;
        fs_err::remove_file(installation.target("alpha"))?;
        drop(transaction);
        assert!(!installation.target("alpha").exists());
        assert!(!installation.target("beta").exists());
        assert!(journal_path(&installation.tools, &installation.name).exists());
        Ok(())
    }

    #[tokio::test]
    #[cfg(windows)]
    async fn running_export_remains_recoverable() -> anyhow::Result<()> {
        const CHILD: &str = "UV_TEST_RUNNING_TOOL_EXPORT";
        const TEST: &str =
            "commands::tool::export_transaction::tests::running_export_remains_recoverable";
        if let Some(root) = std::env::var_os(CHILD) {
            let root = PathBuf::from(root);
            let installation = Installation::at(&root)?;
            let (mut transaction, _) = installation.begin(true)?;
            // Publish over this process's own executable, as a fresh uv self-install does.
            transaction.publish(Printer::Silent)?;
            fs_err::write(root.join("running"), "ready")?;
            loop {
                thread::park();
            }
        }
        let Some(root) = isolated_root("running_export_remains_recoverable")? else {
            return Ok(());
        };
        let installation = Installation::at(&root)?;
        fs_err::copy(std::env::current_exe()?, installation.target("alpha"))?;
        let original = ExportVersion::capture(&installation.target("alpha"))?;
        let mut child = Command::new(installation.target("alpha"))
            .args([TEST, "--exact", "--nocapture", "--test-threads=1"])
            .env("__RUST_TEST_INVOKE", TEST)
            .env(CHILD, &root)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(30);
        while !root.join("running").exists() {
            if let Some(status) = child.try_wait()? {
                let output = child.wait_with_output()?;
                bail!(
                    "running export exited before readiness: {status}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            if Instant::now() >= deadline {
                child.kill()?;
                child.wait()?;
                bail!("running export did not reach readiness");
            }
            thread::sleep(Duration::from_millis(10));
        }
        let published_alpha = fs_err::read(installation.target("alpha"));
        let published_beta = fs_err::read(installation.target("beta"));
        child.kill()?;
        child.wait()?;
        assert_eq!(published_alpha?, b"new alpha");
        assert_eq!(published_beta?, b"new beta");
        recover_tool_exports(&installation.tools, &installation.name).await?;
        assert_eq!(
            ExportVersion::capture(&installation.target("alpha"))?,
            original
        );
        assert!(!installation.target("beta").exists());
        assert!(!journal_path(&installation.tools, &installation.name).exists());
        Ok(())
    }

    #[tokio::test]
    async fn unrelated_uninstall_retains_pending_recovery() -> anyhow::Result<()> {
        let Some(root) = isolated_root("unrelated_uninstall_retains_pending_recovery")? else {
            return Ok(());
        };
        let installation = Installation::at(&root)?;
        let (mut transaction, _) = installation.begin(false)?;
        transaction.publish(Printer::Silent)?;
        // Leave the persisted journal responsible for recovery, as it is after process exit.
        transaction.finished = true;
        drop(transaction);
        fs_err::remove_dir_all(installation.tools.tool_dir(&installation.name))?;
        let unrelated = "unrelated".parse()?;
        fs_err::create_dir_all(installation.tools.tool_dir(&unrelated))?;
        super::super::uninstall::uninstall(vec![unrelated], Printer::Silent).await?;
        assert!(journal_path(&installation.tools, &installation.name).exists());
        assert!(installation.target("alpha").exists());
        recover_tool_exports(&installation.tools, &installation.name).await?;
        assert!(!installation.target("alpha").exists());
        assert!(!installation.target("beta").exists());
        assert!(!journal_path(&installation.tools, &installation.name).exists());
        Ok(())
    }

    #[tokio::test]
    async fn interruption_before_ack_is_recovered() -> anyhow::Result<()> {
        const CHILD: &str = "UV_TEST_TOOL_EXPORT_INTERRUPTION";
        if let Some(root) = std::env::var_os(CHILD) {
            let root = PathBuf::from(root);
            let installation = Installation::at(&root)?;
            let (transaction, _) = installation.begin(true)?;
            publish_export_data(&transaction.record.journal.files, 0)?;
            fs_err::write(root.join("published"), "ready")?;
            loop {
                thread::park();
            }
        }

        let Some(root) = isolated_root("interruption_before_ack_is_recovered")? else {
            return Ok(());
        };
        let installation = Installation::at(&root)?;
        fs_err::write(installation.target("alpha"), "original command")?;
        let original = ExportVersion::capture(&installation.target("alpha"))?;
        let mut child = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "commands::tool::export_transaction::tests::interruption_before_ack_is_recovered",
                "--nocapture",
                "--test-threads=1",
            ])
            // Invoke the fixture directly when libtest uses a panic-abort subprocess, so the
            // process we interrupt owns the transaction and cannot leave a parked grandchild.
            .env(
                "__RUST_TEST_INVOKE",
                "commands::tool::export_transaction::tests::interruption_before_ack_is_recovered",
            )
            .env(CHILD, root.as_path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(30);
        while !root.as_path().join("published").exists() {
            if let Some(status) = child.try_wait()? {
                bail!("publication child exited before the barrier: {status}");
            }
            if Instant::now() >= deadline {
                child.kill()?;
                child.wait()?;
                bail!("publication child did not reach the barrier");
            }
            thread::sleep(Duration::from_millis(10));
        }
        #[cfg(unix)]
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(i32::try_from(child.id())?),
            nix::sys::signal::Signal::SIGINT,
        )?;
        #[cfg(windows)]
        child.kill()?;
        assert!(!child.wait()?.success());
        let record = JournalRecord::read(&installation.tools, &installation.name)?
            .context("interrupted publication lost its journal")?;
        assert!(
            !record
                .journal
                .files
                .staging_directory()?
                .join("published-0")
                .exists()
        );
        drop(record);
        // Recovery must wait for the historical destination even if it is no longer configured.
        let admission =
            ToolEntrypointLocks::for_directories([installation.directory.clone()]).await?;
        assert!(
            tokio::time::timeout(
                Duration::from_millis(100),
                recover_tool_exports(&installation.tools, &installation.name),
            )
            .await
            .is_err()
        );
        assert_ne!(
            ExportVersion::capture(&installation.target("alpha"))?,
            original
        );
        drop(admission);
        recover_tool_exports(&installation.tools, &installation.name).await?;
        assert_eq!(
            ExportVersion::capture(&installation.target("alpha"))?,
            original
        );
        assert!(!installation.target("beta").exists());
        assert!(!journal_path(&installation.tools, &installation.name).exists());
        Ok(())
    }
}
