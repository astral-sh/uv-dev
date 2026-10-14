//! Import paths and managed files for writable overlays on parent environments.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
#[cfg(unix)]
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, bail};
use thiserror::Error;
use tracing::trace;

use uv_cache_key::cache_digest;
use uv_fs::{PythonExt, Simplified, normalize_path};
#[cfg(unix)]
use uv_install_wheel::format_shebang;
use uv_installer::SitePackages;
use uv_python_interpreter::{Interpreter, PythonEnvironment};

use crate::environment::CachedEnvironment;

/// Attach shared dependencies to a writable overlay.
///
/// The parent must be a [`CachedEnvironment`]: synchronization relies on its archive path
/// identifying immutable contents.
///
/// Configure imports before synchronizing managed commands and data. Record the parent only
/// after synchronization succeeds. An error can leave imports updated; ownership journals allow
/// a later call to recover interrupted file synchronization.
pub fn attach_shared_environment(
    environment: &PythonEnvironment,
    shared: &CachedEnvironment,
) -> anyhow::Result<()> {
    let shared = shared.as_ref();
    let parent_site_packages = shared
        .site_packages()
        .next()
        .context("Failed to find `site-packages` directory for environment")?;
    set_shared_overlay(environment, &parent_site_packages)?;
    sync_shared_environment_files(environment, shared.interpreter())?;
    set_parent_environment(environment, shared.root())
}

/// Add shared dependencies after the writable environment's editable paths.
fn set_shared_overlay(
    environment: &PythonEnvironment,
    parent_site_packages: &Path,
) -> anyhow::Result<()> {
    let site_packages = environment
        .site_packages()
        .next()
        .context("Failed to find `site-packages` directory for environment")?;
    write_overlay_file(
        &site_packages.join("_uv_shared_overlay.py"),
        include_str!("_uv_shared_overlay.py"),
    )?;
    set_overlay(
        environment,
        &format!(
            "import _uv_shared_overlay; _uv_shared_overlay.add_overlay({})",
            parent_site_packages.escape_for_python()
        ),
    )
}

/// Write the parent import setup for an overlay.
pub fn set_overlay(environment: &PythonEnvironment, contents: &str) -> anyhow::Result<()> {
    let site_packages = environment
        .site_packages()
        .next()
        .context("Failed to find `site-packages` directory for environment")?;
    write_overlay_file(&site_packages.join("_uv_ephemeral_overlay.pth"), contents)
}

fn write_overlay_file(path: &Path, contents: &str) -> anyhow::Result<()> {
    match fs_err::read(path) {
        Ok(current) if current == contents.as_bytes() => return Ok(()),
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    uv_fs::write_atomic_sync(path, contents)?;
    Ok(())
}

/// Record the parent environment in the overlay's `pyvenv.cfg`.
///
/// Shared and ephemeral overlays add the parent's site packages through a `.pth` file. Static
/// analysis tools cannot reliably infer this dynamic import relationship, so `extends-environment`
/// also records the parent's `sys.prefix`. The parent may be a virtual or system environment.
pub fn set_parent_environment(
    environment: &PythonEnvironment,
    parent_environment_sys_prefix: &Path,
) -> anyhow::Result<()> {
    let parent_environment_sys_prefix = parent_environment_sys_prefix.to_str().context(
        "Cannot write parent environment path to `pyvenv.cfg` because it is not valid UTF-8",
    )?;
    environment.set_pyvenv_cfg("extends-environment", parent_environment_sys_prefix)?;
    Ok(())
}

#[derive(Error, Debug)]
enum CopyEntrypointError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[cfg(windows)]
    #[error(transparent)]
    Trampoline(#[from] uv_trampoline_builder::Error),
}

/// Copy Python entrypoints directly into a disposable overlay using its interpreter.
pub fn copy_environment_entrypoints(
    source: &Interpreter,
    target: &PythonEnvironment,
) -> anyhow::Result<()> {
    let scripts = match fs_err::read_dir(source.scripts()) {
        Ok(scripts) => scripts,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    for entry in scripts {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let target_path = target.scripts().join(entry.file_name());
        if fs_err::symlink_metadata(&target_path).is_ok() {
            continue;
        }
        let result = (|| -> Result<(), CopyEntrypointError> {
            let Some(prepared) = prepare_entrypoint(
                &entry.path(),
                source.sys_executable(),
                target.interpreter().sys_executable(),
            )?
            else {
                return Ok(());
            };
            prepared.write(&target_path)
        })();
        match result {
            Ok(()) => {}
            Err(CopyEntrypointError::Io(err))
                if matches!(
                    err.kind(),
                    io::ErrorKind::AlreadyExists | io::ErrorKind::PermissionDenied
                ) =>
            {
                trace!(
                    "Skipping copy of entrypoint `{}`: {err}",
                    entry.path().display()
                );
            }
            Err(err) => return Err(err.into()),
        }
    }
    Ok(())
}

/// Stage rewritten launchers and record their ownership before persistent publication.
fn publish_shared_entrypoints(
    source: &Interpreter,
    target: &PythonEnvironment,
    owned_files: &BTreeSet<PathBuf>,
    pending_path: &Path,
    mut pending: BTreeMap<PathBuf, String>,
) -> anyhow::Result<(Vec<PathBuf>, bool)> {
    let scripts = match fs_err::read_dir(source.scripts()) {
        Ok(scripts) => scripts,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok((Vec::new(), false)),
        Err(err) => return Err(err.into()),
    };
    let mut copied = Vec::new();
    let mut staging: Option<tempfile::TempDir> = None;
    let mut staged = Vec::new();
    for entry in scripts {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = PathBuf::from(entry.file_name());
        let Some(target_path) = shared_entrypoint_path(target, &name)? else {
            continue;
        };
        if owned_files.contains(normalize_path(&target_path).as_ref())
            || fs_err::symlink_metadata(&target_path).is_ok()
        {
            continue;
        }
        let result = (|| -> Result<(), CopyEntrypointError> {
            let Some(prepared) = prepare_entrypoint(
                &entry.path(),
                source.sys_executable(),
                target.interpreter().sys_executable(),
            )?
            else {
                return Ok(());
            };
            // Stage only recognized launchers, keeping cache hits free of temporary files.
            let directory = if let Some(directory) = staging.as_ref() {
                directory.path().to_path_buf()
            } else {
                let directory = tempfile::Builder::new()
                    .prefix(".uv-shared-")
                    .tempdir_in(uv_fs::verbatim_path(target.scripts()))?;
                let path = directory.path().to_path_buf();
                staging = Some(directory);
                path
            };
            let staged_path = directory.join(&name);
            prepared.write(&staged_path)?;
            pending.insert(name.clone(), cache_digest(&fs_err::read(&staged_path)?));
            staged.push((name.clone(), staged_path));
            Ok(())
        })();
        match result {
            Ok(()) => {}
            Err(CopyEntrypointError::Io(err))
                if matches!(
                    err.kind(),
                    io::ErrorKind::AlreadyExists | io::ErrorKind::PermissionDenied
                ) =>
            {
                trace!(
                    "Skipping copy of entrypoint `{}`: {err}",
                    entry.path().display()
                );
            }
            Err(err) => return Err(err.into()),
        }
    }
    let mut journal_written = false;
    if !staged.is_empty() {
        // Persist ownership of the exact rewritten bytes before publishing any launcher.
        uv_fs::write_atomic_sync(pending_path, serde_json::to_vec(&pending)?)?;
        journal_written = true;
        for (name, path) in staged {
            let Some(target_path) = shared_entrypoint_path(target, &name)? else {
                continue;
            };
            let path = tempfile::TempPath::try_from_path(uv_fs::verbatim_path(&path).into_owned())?;
            match path.persist_noclobber(uv_fs::verbatim_path(&target_path)) {
                Ok(()) => copied.push(name),
                Err(err)
                    if matches!(
                        err.error.kind(),
                        io::ErrorKind::AlreadyExists | io::ErrorKind::PermissionDenied
                    ) =>
                {
                    trace!(
                        "Skipping copy of entrypoint `{}`: {}",
                        target_path.display(),
                        err.error
                    );
                }
                Err(err) => return Err(err.error.into()),
            }
        }
    }
    copied.sort();
    Ok((copied, journal_written))
}

#[derive(PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct SharedEnvironmentFiles {
    source: PathBuf,
    names: Vec<PathBuf>,
    data: BTreeMap<PathBuf, String>,
    owned_data: BTreeSet<PathBuf>,
}

impl SharedEnvironmentFiles {
    /// Load and validate paths before any managed files are synchronized.
    fn load(path: &Path) -> anyhow::Result<Option<Self>> {
        let bytes = match fs_err::read(path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        let manifest: Self = serde_json::from_slice(&bytes).with_context(|| {
            format!(
                "Failed to parse shared environment manifest at: {}",
                path.user_display()
            )
        })?;
        for name in &manifest.names {
            validate_shared_entrypoint_name(name)?;
        }
        validate_shared_data_paths(&manifest.data)?;
        Ok(Some(manifest))
    }
}

/// Read files owned by distributions installed directly into the writable overlay.
fn installed_environment_files(
    environment: &PythonEnvironment,
) -> anyhow::Result<BTreeSet<PathBuf>> {
    let mut files = BTreeSet::new();
    for distribution in SitePackages::from_environment(environment)?.iter() {
        files.extend(distribution.read_files()?);
    }
    Ok(files)
}

/// Synchronize shared commands and data while retaining overlay installations and edited data files.
fn sync_shared_environment_files(
    environment: &PythonEnvironment,
    shared: &Interpreter,
) -> anyhow::Result<()> {
    let manifest = environment.root().join(".uv-shared-entrypoints.json");
    let previous = SharedEnvironmentFiles::load(&manifest)?;
    let pending_path = environment.root().join(".uv-shared-data.pending.json");
    let pending = load_pending_shared_data(&pending_path)?;
    let pending_entrypoints_path = environment
        .root()
        .join(".uv-shared-entrypoints.pending.json");
    let pending_entrypoints = load_pending_shared_entrypoints(&pending_entrypoints_path)?;
    let names = previous
        .as_ref()
        .map_or(&[][..], |previous| previous.names.as_slice());
    let source_changed = previous
        .as_ref()
        .is_none_or(|previous| previous.source != shared.sys_prefix());
    let owned_files = installed_environment_files(environment)?;
    if let Some(pending) = pending.as_ref() {
        // An interrupted update can leave bytes from the next base alongside the old manifest.
        // Reconcile those copies before replacing their ownership record with another update.
        remove_shared_data(environment.root(), pending, &owned_files)?;
    }
    let protected_entrypoints = if let Some(pending) = pending_entrypoints.as_ref() {
        remove_pending_shared_entrypoints(environment, pending, &owned_files)?
    } else {
        BTreeSet::new()
    };
    let owned_data = owned_files
        .iter()
        .filter_map(|path| path.strip_prefix(environment.root()).ok())
        .filter(|path| {
            SHARED_DATA_DIRECTORIES
                .iter()
                .any(|directory| path.starts_with(directory))
        })
        .map(Path::to_path_buf)
        .collect::<BTreeSet<_>>();
    let mut retained = Vec::new();
    for name in names {
        if protected_entrypoints.contains(name) {
            continue;
        }
        let Some(path) = shared_entrypoint_path(environment, name)? else {
            continue;
        };
        if owned_files.contains(normalize_path(&path).as_ref()) {
            continue;
        }
        if source_changed {
            match fs_err::remove_file(&path) {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
        } else if path.is_file() {
            retained.push(name.clone());
        }
    }
    // Keep edited commands protected if another update fails before the manifest commits.
    let preserved_entrypoints = pending_entrypoints
        .as_ref()
        .into_iter()
        .flat_map(|pending| pending.iter())
        .filter(|(name, _)| protected_entrypoints.contains(*name))
        .map(|(name, digest)| (name.clone(), digest.clone()))
        .collect();
    let (copied_entrypoints, entrypoints_journal_written) = publish_shared_entrypoints(
        shared,
        environment,
        &owned_files,
        &pending_entrypoints_path,
        preserved_entrypoints,
    )?;
    retained.extend(copied_entrypoints);
    retained.sort();
    retained.dedup();
    let (data, synchronized) = if pending.is_none()
        && let Some(previous) = previous.as_ref()
        && !source_changed
        && previous.owned_data == owned_data
        && shared_data_is_present(environment.root(), &previous.data)?
    {
        (previous.data.clone(), false)
    } else {
        (
            sync_shared_data(
                environment,
                shared,
                previous.as_ref().map(|previous| &previous.data),
                &owned_files,
                &pending_path,
            )?,
            true,
        )
    };
    let copied = SharedEnvironmentFiles {
        source: shared.sys_prefix().to_path_buf(),
        names: retained,
        data,
        owned_data,
    };
    if previous.as_ref() != Some(&copied) {
        uv_fs::write_atomic_sync(manifest, serde_json::to_vec(&copied)?)?;
    }
    if pending_entrypoints.is_some() || entrypoints_journal_written {
        fs_err::remove_file(pending_entrypoints_path)?;
    }
    if synchronized {
        // Keep pending ownership until the final manifest commits, including on write failures.
        fs_err::remove_file(pending_path)?;
    }
    Ok(())
}

/// Resolve a shared launcher without following directory links out of the overlay.
fn shared_entrypoint_path(
    environment: &PythonEnvironment,
    name: &Path,
) -> anyhow::Result<Option<PathBuf>> {
    let relative = environment
        .scripts()
        .strip_prefix(environment.root())
        .context("Script directory is outside its environment")?
        .join(name);
    shared_environment_path(environment.root(), &relative)
}

/// Validate a command name before resolving it inside the overlay's scripts directory.
fn validate_shared_entrypoint_name(name: &Path) -> anyhow::Result<()> {
    if name.components().count() != 1 || name.file_name().is_none() {
        bail!("Invalid shared entrypoint name: {}", name.display());
    }
    Ok(())
}

/// Read the exact launcher bytes recorded before an interrupted publication.
fn load_pending_shared_entrypoints(
    path: &Path,
) -> anyhow::Result<Option<BTreeMap<PathBuf, String>>> {
    let bytes = match fs_err::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let pending: BTreeMap<PathBuf, String> = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "Failed to parse shared entrypoint journal at: {}",
            path.user_display()
        )
    })?;
    for name in pending.keys() {
        validate_shared_entrypoint_name(name)?;
    }
    Ok(Some(pending))
}

/// Reconcile interrupted launcher copies without claiming local or directly installed commands.
fn remove_pending_shared_entrypoints(
    environment: &PythonEnvironment,
    pending: &BTreeMap<PathBuf, String>,
    owned_files: &BTreeSet<PathBuf>,
) -> anyhow::Result<BTreeSet<PathBuf>> {
    let mut protected = BTreeSet::new();
    for (name, digest) in pending {
        let Some(path) = shared_entrypoint_path(environment, name)? else {
            protected.insert(name.clone());
            continue;
        };
        if owned_files.contains(normalize_path(&path).as_ref()) {
            continue;
        }
        match fs_err::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => {
                if cache_digest(&fs_err::read(&path)?) == *digest {
                    fs_err::remove_file(&path)?;
                } else {
                    protected.insert(name.clone());
                }
            }
            Ok(_) => {
                protected.insert(name.clone());
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }
    Ok(protected)
}

const SHARED_DATA_DIRECTORIES: [&str; 2] = ["etc/jupyter", "share/jupyter"];

/// Validate persisted ownership before using any of its paths for cleanup.
fn validate_shared_data_paths(data: &BTreeMap<PathBuf, String>) -> anyhow::Result<()> {
    for relative in data.keys() {
        if !relative.components().all(|component| match component {
            Component::Normal(_) => true,
            Component::Prefix(_)
            | Component::RootDir
            | Component::CurDir
            | Component::ParentDir => false,
        }) || !SHARED_DATA_DIRECTORIES
            .iter()
            .any(|directory| relative.starts_with(directory) && relative != Path::new(directory))
        {
            bail!("Invalid shared data path: {}", relative.display());
        }
    }
    Ok(())
}

/// Read the ownership recorded before an interrupted data update.
fn load_pending_shared_data(path: &Path) -> anyhow::Result<Option<BTreeMap<PathBuf, String>>> {
    let bytes = match fs_err::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let data = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "Failed to parse shared data journal at: {}",
            path.user_display()
        )
    })?;
    validate_shared_data_paths(&data)?;
    Ok(Some(data))
}

/// Remove managed copies only when their contents still match their ownership record.
fn remove_shared_data(
    root: &Path,
    data: &BTreeMap<PathBuf, String>,
    owned_files: &BTreeSet<PathBuf>,
) -> anyhow::Result<()> {
    for (relative, digest) in data {
        let Some(target) = shared_environment_path(root, relative)? else {
            continue;
        };
        if owned_files.contains(normalize_path(&target).as_ref()) {
            continue;
        }
        match fs_err::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_file() => {
                if cache_digest(&fs_err::read(&target)?) == *digest {
                    fs_err::remove_file(&target)?;
                }
            }
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }
    Ok(())
}

/// Resolve a managed file path without following directory links out of the overlay.
fn shared_environment_path(root: &Path, relative: &Path) -> anyhow::Result<Option<PathBuf>> {
    let mut directory = root.to_path_buf();
    if let Some(parent) = relative.parent() {
        for component in parent.components() {
            directory.push(component);
            match fs_err::symlink_metadata(&directory) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
                Ok(_) => return Ok(None),
                Err(err) if err.kind() == io::ErrorKind::NotFound => break,
                Err(err) => return Err(err.into()),
            }
        }
    }
    Ok(Some(root.join(relative)))
}

/// Check for removals that occurred without changing the overlay's installed distributions.
fn shared_data_is_present(root: &Path, data: &BTreeMap<PathBuf, String>) -> anyhow::Result<bool> {
    for relative in data.keys() {
        let Some(path) = shared_environment_path(root, relative)? else {
            continue;
        };
        match fs_err::symlink_metadata(path) {
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(err) => return Err(err.into()),
        }
    }
    Ok(true)
}

/// Materialize independent data files: overlay installs must not write through to the shared base.
fn sync_shared_data(
    environment: &PythonEnvironment,
    shared: &Interpreter,
    previous: Option<&BTreeMap<PathBuf, String>>,
    owned_files: &BTreeSet<PathBuf>,
    pending_path: &Path,
) -> anyhow::Result<BTreeMap<PathBuf, String>> {
    if let Some(previous) = previous {
        remove_shared_data(environment.root(), previous, owned_files)?;
    }

    let mut copied = BTreeMap::new();
    let mut planned = Vec::new();
    let mut directories = SHARED_DATA_DIRECTORIES
        .iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    while let Some(directory) = directories.pop() {
        let entries = match fs_err::read_dir(shared.sys_prefix().join(&directory)) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err.into()),
        };
        for entry in entries {
            let entry = entry?;
            let relative = directory.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                directories.push(relative);
                continue;
            }
            let Some(target) = shared_environment_path(environment.root(), &relative)? else {
                continue;
            };
            if owned_files.contains(normalize_path(&target).as_ref()) {
                continue;
            }
            match fs_err::symlink_metadata(&target) {
                Ok(_) => continue,
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
            let bytes = fs_err::read(entry.path())?;
            copied.insert(relative.clone(), cache_digest(&bytes));
            planned.push(relative);
        }
    }

    // Record only files eligible for a managed copy. Existing local files and files owned by an
    // overlay installation must never be adopted by an interrupted update's recovery record.
    uv_fs::write_atomic_sync(pending_path, serde_json::to_vec(&copied)?)?;
    for relative in planned {
        let Some(target) = shared_environment_path(environment.root(), &relative)? else {
            copied.remove(&relative);
            continue;
        };
        match fs_err::symlink_metadata(&target) {
            Ok(_) => {
                copied.remove(&relative);
                continue;
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
        if let Some(parent) = target.parent() {
            fs_err::create_dir_all(parent)?;
        }
        uv_fs::copy_atomic_sync(shared.sys_prefix().join(relative), target)?;
    }
    Ok(copied)
}

/// A recognized launcher whose interpreter has already been rewritten.
struct PreparedEntrypoint {
    #[cfg(unix)]
    contents: String,
    #[cfg(unix)]
    mode: u32,
    #[cfg(windows)]
    launcher: uv_trampoline_builder::Launcher,
    #[cfg(windows)]
    is_gui: bool,
}

impl PreparedEntrypoint {
    /// Write a launcher without replacing an existing destination.
    fn write(self, target: &Path) -> Result<(), CopyEntrypointError> {
        #[cfg(unix)]
        {
            use fs_err::os::unix::fs::OpenOptionsExt;
            use std::io::Write;

            let mut file = fs_err::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(self.mode)
                .open(target)?;
            file.write_all(self.contents.as_bytes())?;
        }
        #[cfg(windows)]
        {
            let mut file = fs_err::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(target)?;
            self.launcher.write_to_file(&mut file, self.is_gui)?;
        }
        trace!("Updated entrypoint at `{}`", target.user_display());
        Ok(())
    }
}

/// Prepare a Python entrypoint with the target environment's interpreter.
#[cfg(unix)]
fn prepare_entrypoint(
    source: &Path,
    previous_executable: &Path,
    python_executable: &Path,
) -> Result<Option<PreparedEntrypoint>, CopyEntrypointError> {
    use std::io::Seek;
    use std::os::unix::fs::PermissionsExt;

    let mut file = fs_err::File::open(source)?;
    let mut buffer = [0u8; 2];
    if file.read_exact(&mut buffer).is_err() {
        // File is too small to have a shebang
        trace!(
            "Skipping copy of entrypoint `{}`: file is too small to contain a shebang",
            source.user_display()
        );
        return Ok(None);
    }

    // Check if it starts with `#!` to avoid reading binary files and such into memory
    if &buffer != b"#!" {
        trace!(
            "Skipping copy of entrypoint `{}`: does not start with #!",
            source.user_display()
        );
        return Ok(None);
    }

    let mut contents = String::new();
    file.seek(std::io::SeekFrom::Start(0))?;
    match file.read_to_string(&mut contents) {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::InvalidData => {
            // If the file is not valid UTF-8, we skip it in case it was a binary file with `#!` at
            // the start (which seems pretty niche, but being defensive here seems safe)
            trace!(
                "Skipping copy of entrypoint `{}`: is not valid UTF-8",
                source.user_display()
            );
            return Ok(None);
        }
        Err(err) => return Err(err.into()),
    }

    let Some(contents) = contents
        // Check for a relative path or relocatable shebang
        .strip_prefix(
            r#"#!/bin/sh
'''exec' "$(dirname -- "$(realpath -- "$0")")"/'python' "$0" "$@"
' '''
"#,
        )
        // Or, an absolute path shebang, including a shell wrapper for long or spaced paths.
        .or_else(|| contents.strip_prefix(&format!("#!{}\n", previous_executable.display())))
        .or_else(|| {
            contents.strip_prefix(&format!(
                "{}\n",
                format_shebang(previous_executable, "posix", false)
            ))
        })
        // If the previous executable ends with `python3`, check for a shebang with `python` too
        .or_else(|| {
            previous_executable
                .to_str()
                .and_then(|path| path.strip_suffix("3"))
                .and_then(|path| {
                    contents.strip_prefix(&format!("#!{path}\n")).or_else(|| {
                        contents
                            .strip_prefix(&format!("{}\n", format_shebang(path, "posix", false)))
                    })
                })
        })
    else {
        // If it's not a Python shebang, we'll skip it
        trace!(
            "Skipping copy of entrypoint `{}`: does not start with expected shebang",
            source.user_display()
        );
        return Ok(None);
    };

    let contents = format!(
        "{}\n{contents}",
        format_shebang(python_executable, "posix", false)
    );
    let mode = fs_err::metadata(source)?.permissions().mode();
    Ok(Some(PreparedEntrypoint { contents, mode }))
}

/// Prepare a Windows launcher with the target environment's interpreter.
#[cfg(windows)]
fn prepare_entrypoint(
    source: &Path,
    _previous_executable: &Path,
    python_executable: &Path,
) -> Result<Option<PreparedEntrypoint>, CopyEntrypointError> {
    use uv_trampoline_builder::Launcher;

    let Some(launcher) = Launcher::try_from_path(source)? else {
        return Ok(None);
    };

    let is_gui = launcher.python_path.ends_with("pythonw.exe");

    let python_path = if is_gui {
        python_executable.with_file_name("pythonw.exe")
    } else {
        python_executable.to_path_buf()
    };

    Ok(Some(PreparedEntrypoint {
        launcher: launcher.with_python_path(python_path),
        is_gui,
    }))
}
