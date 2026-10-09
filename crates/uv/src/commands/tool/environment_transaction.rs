//! Owned staging and recovery for interpreter-changing tool upgrades.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

use uv_cache::Cache;
use uv_configuration::Concurrency;
use uv_fs::{LockedFile, Simplified};
use uv_installer::SitePackages;
use uv_normalize::PackageName;
use uv_python::{Interpreter, PythonEnvironment};
use uv_tool::{InstalledTools, TOOL_ENVIRONMENT_STAGING_PREFIX, Tool, ToolEntrypointLocks};
use uv_warnings::warn_user;

use super::export_transaction::{
    ExportDirectory, ExportIdentity, ExportVersion, JOURNAL_PREFIX, JournalExport, OwnedJournal,
    cleanup_anchors, file_digest, new_anchor, old_anchor, publish_export, rollback_export,
    sync_directory,
};
use super::recovery::{ToolEntrypointPlan, ToolEntrypointSnapshot};
use crate::printer::Printer;

const JOURNAL_PREFIX_ENVIRONMENT: &str = ".uv-tool-environment-";
const JOURNAL_VERSION: u8 = 1;
const REPLACEMENT: &str = "replacement";
const PREVIOUS: &str = "previous";

struct StagingOwner {
    directory: TempDir,
    // Directory cleanup and detached workers finish before admission is released.
    _root_lock: Arc<LockedFile>,
}

pub(super) struct StagedToolEnvironment {
    owner: Arc<StagingOwner>,
    original: EnvironmentSnapshot,
    destination: PathBuf,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct EnvironmentSnapshot {
    tool_root: PathBuf,
    tool: PackageName,
    identity: ExportIdentity,
    metadata: MetadataDigest,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct MetadataDigest {
    receipt: [u8; 32],
    lock: Option<[u8; 32]>,
}

impl MetadataDigest {
    fn read(directory: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            receipt: file_digest(&directory.join("uv-receipt.toml"))?
                .context("Tool receipt disappeared during replacement")?,
            lock: file_digest(&directory.join("uv.lock"))?,
        })
    }
}

impl EnvironmentSnapshot {
    pub(super) fn capture(
        installed_tools: &InstalledTools,
        name: &PackageName,
    ) -> anyhow::Result<Self> {
        let tool_root = fs_err::canonicalize(installed_tools.root())?;
        let directory = tool_root.join(name.as_ref());
        let identity = directory_identity(&directory)?.context("Tool environment disappeared")?;
        Ok(Self {
            tool_root,
            tool: name.clone(),
            identity,
            metadata: MetadataDigest::read(&directory)?,
        })
    }

    fn path(&self) -> PathBuf {
        self.tool_root.join(self.tool.as_ref())
    }

    fn check_current(&self) -> anyhow::Result<()> {
        if directory_identity(&self.path())?.as_ref() != Some(&self.identity)
            || MetadataDigest::read(&self.path())? != self.metadata
        {
            bail!(
                "Tool `{}` changed while its replacement was being prepared",
                self.tool
            );
        }
        Ok(())
    }
}

impl StagedToolEnvironment {
    pub(super) fn create(
        installed_tools: &InstalledTools,
        original: EnvironmentSnapshot,
        interpreter: Interpreter,
        root_lock: Arc<LockedFile>,
    ) -> anyhow::Result<(Self, PythonEnvironment)> {
        original.check_current()?;
        let directory = tempfile::Builder::new()
            .prefix(TOOL_ENVIRONMENT_STAGING_PREFIX)
            .tempdir_in(&original.tool_root)?;
        let environment = uv_virtualenv::create_venv(
            &directory.path().join(REPLACEMENT),
            interpreter,
            uv_virtualenv::Prompt::None,
            false,
            uv_virtualenv::OnExisting::Remove(uv_virtualenv::RemovalReason::TemporaryEnvironment),
            false,
            uv_virtualenv::Seed::Disabled,
            false,
        )?;
        let destination = installed_tools.tool_dir(&original.tool);
        let owner = Arc::new(StagingOwner {
            directory,
            _root_lock: root_lock,
        });
        let environment = environment.with_lifetime_guard(owner.clone());
        Ok((
            Self {
                owner,
                original,
                destination,
            },
            environment,
        ))
    }

    pub(super) async fn compile_bytecode(
        &self,
        environment: &PythonEnvironment,
        concurrency: &Concurrency,
        cache: &Cache,
        printer: Printer,
    ) -> anyhow::Result<()> {
        let start = std::time::Instant::now();
        let mut files = 0;
        for source in environment.site_packages() {
            if !source.exists() {
                continue;
            }
            let relative = source.strip_prefix(environment.root())?;
            files += uv_installer::compile_staged_tree(
                &source,
                &self.destination.join(relative),
                environment,
                concurrency,
                cache.root(),
            )
            .await?;
        }
        crate::commands::write_bytecode_summary(files, start, printer)?;
        Ok(())
    }

    /// Finish generated files after all build and bytecode workers have released the environment.
    pub(super) fn finalize(&self, environment: &PythonEnvironment) -> anyhow::Result<()> {
        let executable = self.destination.join(
            environment
                .python_executable()
                .strip_prefix(environment.root())?,
        );
        let updates = uv_virtualenv::finalize_activators(environment, &self.destination)?;
        let updates = updates
            .iter()
            .map(|update| uv_install_wheel::RecordUpdate {
                path: update.path(),
                before: update.before(),
                after: update.after(),
            })
            .collect::<Vec<_>>();
        let layout = environment.interpreter().layout();
        for distribution in SitePackages::from_environment(environment)?.iter() {
            uv_install_wheel::finalize_scripts(
                &layout,
                &executable,
                distribution.install_path(),
                &updates,
            )?;
        }
        Ok(())
    }

    pub(super) async fn publish(
        self,
        environment: PythonEnvironment,
        installed_tools: &InstalledTools,
        snapshot: &ToolEntrypointSnapshot,
        plan: ToolEntrypointPlan,
        tool: Tool,
        lock: Option<String>,
        cache: &Cache,
        printer: Printer,
    ) -> anyhow::Result<()> {
        let receipt = installed_tools.prepare_tool_receipt(&self.original.tool, tool)?;
        write_metadata(environment.root(), receipt.as_bytes(), lock.as_deref())?;
        let executable = self.destination.join(
            environment
                .python_executable()
                .strip_prefix(environment.root())?,
        );
        let (exports, export_staging) =
            stage_exports(&plan, environment.root(), &self.destination)?;
        let replacement = directory_identity(environment.root())?
            .context("Staged tool environment disappeared")?;
        let metadata = MetadataDigest::read(environment.root())?;
        drop(environment);

        let _entrypoint_locks = ToolEntrypointLocks::for_directories(
            exports
                .iter()
                .map(|directory| directory.files.directory.clone()),
        )
        .await?;
        self.original.check_current()?;
        plan.revalidate(snapshot, &self.original.tool)?;
        revalidate_exports(&exports)?;
        let owner = Arc::try_unwrap(self.owner).map_err(|_| {
            anyhow::anyhow!("Tool preparation is still using the staged environment")
        })?;
        let journal = EnvironmentJournal {
            version: JOURNAL_VERSION,
            phase: Phase::Prepared,
            publication: if cfg!(any(target_os = "linux", target_os = "macos")) {
                Publication::Exchange
            } else {
                Publication::Rename
            },
            original: self.original,
            staging: owner
                .directory
                .path()
                .file_name()
                .context("Missing staging filename")?
                .into(),
            staging_identity: ExportIdentity::directory(owner.directory.path())?,
            replacement_identity: replacement,
            replacement_metadata: metadata,
            exports,
        };
        journal.validate(installed_tools, &journal.original.tool)?;
        let record = OwnedJournal::create_at(
            journal_path(installed_tools, &journal.original.tool),
            journal,
        )?;
        // The persisted record owns these directories from this point, including on sync failure.
        owner.directory.keep();
        for staging in export_staging {
            staging.keep();
        }
        let mut transaction = EnvironmentTransaction {
            record,
            finished: false,
        };
        sync_directory(installed_tools.root())?;
        transaction.publish()?;
        Interpreter::clear_cache(&executable, cache)?;
        PythonEnvironment::from_root(&self.destination, cache)?;
        plan.report(&transaction.record.journal.original.tool, true, printer)?;
        transaction.commit()?;
        Ok(())
    }
}

fn write_metadata(directory: &Path, receipt: &[u8], lock: Option<&str>) -> anyhow::Result<()> {
    for (name, contents) in [
        ("uv-receipt.toml", Some(receipt)),
        ("uv.lock", lock.map(str::as_bytes)),
    ] {
        if let Some(contents) = contents {
            let mut file = fs_err::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join(name))?;
            file.write_all(contents)?;
            file.sync_all()?;
        }
    }
    sync_directory(directory)?;
    Ok(())
}

fn stage_exports(
    plan: &ToolEntrypointPlan,
    staged_environment: &Path,
    final_environment: &Path,
) -> anyhow::Result<(Vec<DirectoryChanges>, Vec<TempDir>)> {
    let mut directories = BTreeMap::<PathBuf, (DirectoryChanges, TempDir)>::new();
    for path in plan
        .exports
        .iter()
        .map(|export| &export.entrypoint.install_path)
        .chain(&plan.removals)
    {
        let directory = path.parent().context("Executable has no parent")?;
        fs_err::create_dir_all(directory)?;
        let directory = fs_err::canonicalize(directory)?;
        if !directories.contains_key(&directory) {
            let staging = tempfile::Builder::new()
                .prefix(JOURNAL_PREFIX)
                .tempdir_in(&directory)?;
            let files = ExportDirectory {
                directory: directory.clone(),
                staging: staging
                    .path()
                    .file_name()
                    .context("Missing export staging filename")?
                    .into(),
                staging_identity: ExportIdentity::directory(staging.path())?,
                exports: Vec::new(),
            };
            directories.insert(
                directory,
                (
                    DirectoryChanges {
                        files,
                        removals: Vec::new(),
                    },
                    staging,
                ),
            );
        }
    }
    for export in &plan.exports {
        let target = &export.entrypoint.install_path;
        let directory = fs_err::canonicalize(target.parent().context("Executable has no parent")?)?;
        let (changes, staging) = directories
            .get_mut(&directory)
            .context("Missing export directory")?;
        let index = changes.files.exports.len();
        let original = ExportVersion::capture(target)?;
        if let Some(original) = &original {
            let backup = old_anchor(staging.path(), index);
            fs_err::hard_link(target, &backup)?;
            if !original.matches(&backup)? || !original.matches(target)? {
                bail!(
                    "Executable `{}` changed while preparing its backup",
                    target.user_display()
                );
            }
        }
        let prepared = new_anchor(staging.path(), index);
        #[cfg(unix)]
        fs_err::os::unix::fs::symlink(
            final_environment.join(export.source.strip_prefix(staged_environment)?),
            &prepared,
        )?;
        #[cfg(windows)]
        {
            let _ = (staged_environment, final_environment);
            super::recovery::copy_executable(&export.source, &prepared)?;
            fs_err::OpenOptions::new()
                .write(true)
                .open(&prepared)?
                .sync_all()?;
        }
        changes.files.exports.push(JournalExport {
            filename: target
                .file_name()
                .context("Executable has no filename")?
                .into(),
            original,
            replacement: ExportVersion::capture(&prepared)?
                .context("Prepared executable disappeared")?,
        });
    }
    for target in &plan.removals {
        let directory = fs_err::canonicalize(target.parent().context("Executable has no parent")?)?;
        let (changes, staging) = directories
            .get_mut(&directory)
            .context("Missing removal directory")?;
        let original =
            ExportVersion::capture(target)?.context("Obsolete executable disappeared")?;
        let backup = removal_anchor(staging.path(), "backup", changes.removals.len());
        fs_err::hard_link(target, &backup)?;
        if !original.matches(&backup)? || !original.matches(target)? {
            bail!(
                "Executable `{}` changed while preparing its removal",
                target.user_display()
            );
        }
        changes.removals.push(RemovedExport {
            filename: target
                .file_name()
                .context("Executable has no filename")?
                .into(),
            original,
        });
    }
    for (changes, staging) in directories.values() {
        sync_directory(staging.path())?;
        sync_directory(&changes.files.directory)?;
    }
    Ok(directories.into_values().unzip())
}

fn revalidate_exports(directories: &[DirectoryChanges]) -> anyhow::Result<()> {
    for directory in directories {
        if fs_err::canonicalize(&directory.files.directory)? != directory.files.directory {
            bail!("Executable directory changed during preparation");
        }
        directory.files.staging_directory()?;
        for export in &directory.files.exports {
            if ExportVersion::capture(&directory.files.directory.join(&export.filename))?
                != export.original
            {
                bail!(
                    "Executable `{}` changed during preparation",
                    export.filename.user_display()
                );
            }
        }
        for removal in &directory.removals {
            if !removal
                .original
                .matches(&directory.files.directory.join(&removal.filename))?
            {
                bail!(
                    "Executable `{}` changed during preparation",
                    removal.filename.user_display()
                );
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Phase {
    Prepared,
    RolledBack,
    Committed,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Publication {
    Exchange,
    Rename,
}

#[derive(Clone, Serialize, Deserialize)]
struct EnvironmentJournal {
    version: u8,
    phase: Phase,
    publication: Publication,
    original: EnvironmentSnapshot,
    staging: PathBuf,
    staging_identity: ExportIdentity,
    replacement_identity: ExportIdentity,
    replacement_metadata: MetadataDigest,
    exports: Vec<DirectoryChanges>,
}

#[derive(Clone, Serialize, Deserialize)]
struct DirectoryChanges {
    files: ExportDirectory,
    removals: Vec<RemovedExport>,
}

#[derive(Clone, Serialize, Deserialize)]
struct RemovedExport {
    filename: PathBuf,
    original: ExportVersion,
}

struct EnvironmentTransaction {
    record: OwnedJournal<EnvironmentJournal>,
    finished: bool,
}

impl EnvironmentTransaction {
    fn publish(&mut self) -> anyhow::Result<()> {
        self.record.check_current()?;
        let journal = &self.record.journal;
        journal.original.check_current()?;
        let container = journal.container()?;
        let replacement = container.join(REPLACEMENT);
        if directory_identity(&replacement)?.as_ref() != Some(&journal.replacement_identity)
            || MetadataDigest::read(&replacement)? != journal.replacement_metadata
        {
            bail!("Staged tool environment changed before publication");
        }
        match journal.publication {
            Publication::Exchange => exchange(&journal.original.path(), &replacement)?,
            Publication::Rename => {
                rename_directory(&journal.original.path(), &container.join(PREVIOUS))?;
                rename_directory(&replacement, &journal.original.path())?;
            }
        }
        sync_directory(&container)?;
        sync_directory(&journal.original.tool_root)?;
        for directory in &journal.exports {
            for index in 0..directory.files.exports.len() {
                publish_export(&directory.files, index)?;
            }
            let staging = directory.files.staging_directory()?;
            for (index, removal) in directory.removals.iter().enumerate() {
                let target = directory.files.directory.join(&removal.filename);
                if !removal.original.matches(&target)? {
                    bail!(
                        "Obsolete executable `{}` changed before removal",
                        target.user_display()
                    );
                }
                // The moved entry itself witnesses removal even if the process exits immediately.
                rename_file(&target, &removal_anchor(&staging, "removed", index))?;
                sync_directory(&staging)?;
                sync_directory(&directory.files.directory)?;
            }
        }
        Ok(())
    }

    fn commit(&mut self) -> anyhow::Result<()> {
        let journal = &self.record.journal;
        if directory_identity(&journal.original.path())?.as_ref()
            != Some(&journal.replacement_identity)
            || MetadataDigest::read(&journal.original.path())? != journal.replacement_metadata
        {
            bail!("Published tool environment changed before commit");
        }
        let mut committed = journal.clone();
        committed.phase = Phase::Committed;
        let result = self.record.replace(committed);
        // A completed record replacement is the commit, including when its directory sync fails.
        self.finished = self.record.journal.phase == Phase::Committed;
        result.context("Could not persist the tool replacement commit")?;
        if let Err(error) = cleanup_record(&self.record) {
            warn_user!(
                "Tool `{}` is installed; replacement cleanup is pending: {error:#}",
                self.record.journal.original.tool
            );
        }
        Ok(())
    }
}

impl Drop for EnvironmentTransaction {
    fn drop(&mut self) {
        if !self.finished
            && let Err(error) = recover_record(&mut self.record)
        {
            warn_user!(
                "Could not restore tool `{}`: {error:#}. Recovery information remains at `{}`",
                self.record.journal.original.tool,
                self.record.path.user_display()
            );
        }
    }
}

fn removal_anchor(directory: &Path, kind: &str, index: usize) -> PathBuf {
    directory.join(format!("removal-{kind}-{index}"))
}

fn rename_file(source: &Path, target: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        // The destination is a private, previously absent transaction anchor.
        fs_err::rename(source, target)
    }
    #[cfg(windows)]
    {
        fs_err::rename(uv_fs::verbatim_path(source), uv_fs::verbatim_path(target))
    }
}

fn rename_directory(source: &Path, target: &Path) -> anyhow::Result<()> {
    if fs_err::symlink_metadata(target).is_ok() {
        bail!(
            "Tool recovery destination `{}` already exists",
            target.user_display()
        );
    }
    rename_file(source, target)?;
    Ok(())
}

fn exchange(left: &Path, right: &Path) -> anyhow::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        uv_fs::exchange_paths(left, right)?;
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (left, right);
        bail!("Tool replacement journal uses an unsupported directory exchange");
    }
}

fn restore_environment(journal: &EnvironmentJournal) -> anyhow::Result<()> {
    let container = journal.container()?;
    let destination = journal.original.path();
    let replacement = container.join(REPLACEMENT);
    let previous = container.join(PREVIOUS);
    let current = directory_identity(&destination)?;
    let staged = directory_identity(&replacement)?;
    let backup = directory_identity(&previous)?;
    match journal.publication {
        Publication::Exchange => {
            if backup.is_some() {
                bail!("Unexpected previous tool environment");
            }
            if current.as_ref() == Some(&journal.replacement_identity)
                && staged.as_ref() == Some(&journal.original.identity)
            {
                if MetadataDigest::read(&destination)? != journal.replacement_metadata
                    || MetadataDigest::read(&replacement)? != journal.original.metadata
                {
                    bail!("Tool metadata changed outside this replacement");
                }
                exchange(&destination, &replacement)?;
            } else if current.as_ref() != Some(&journal.original.identity)
                || staged
                    .as_ref()
                    .is_some_and(|identity| identity != &journal.replacement_identity)
            {
                bail!("Tool directories changed outside this replacement");
            }
        }
        Publication::Rename => {
            if current.as_ref() == Some(&journal.original.identity)
                && backup.is_none()
                && staged
                    .as_ref()
                    .is_none_or(|identity| identity == &journal.replacement_identity)
            {
                return Ok(());
            }
            if backup.as_ref() != Some(&journal.original.identity)
                || MetadataDigest::read(&previous)? != journal.original.metadata
            {
                bail!("Previous tool environment changed outside this replacement");
            }
            if current.as_ref() == Some(&journal.replacement_identity) && staged.is_none() {
                if MetadataDigest::read(&destination)? != journal.replacement_metadata {
                    bail!("Tool metadata changed outside this replacement");
                }
                rename_directory(&destination, &replacement)?;
            } else if current.is_some()
                || staged
                    .as_ref()
                    .is_some_and(|identity| identity != &journal.replacement_identity)
            {
                bail!("Tool directories changed outside this replacement");
            }
            rename_directory(&previous, &destination)?;
        }
    }
    sync_directory(&container)?;
    sync_directory(&journal.original.tool_root)?;
    Ok(())
}

fn recover_record(record: &mut OwnedJournal<EnvironmentJournal>) -> anyhow::Result<()> {
    record.check_current()?;
    let journal = &record.journal;
    if journal.phase == Phase::Prepared {
        // Restore global names before the environment they reference. Any foreign replacement
        // leaves the journal and both generations available instead of discarding the backup.
        for directory in journal.exports.iter().rev() {
            if fs_err::canonicalize(&directory.files.directory)? != directory.files.directory {
                bail!("Executable recovery directory changed");
            }
            let staging = directory.files.staging_directory()?;
            for (index, removal) in directory.removals.iter().enumerate().rev() {
                let target = directory.files.directory.join(&removal.filename);
                if removal.original.matches(&target)? {
                    continue;
                }
                let removed = removal_anchor(&staging, "removed", index);
                if ExportVersion::capture(&target)?.is_none()
                    && removal.original.matches(&removed)?
                {
                    fs_err::hard_link(&removed, &target)?;
                } else {
                    bail!(
                        "Removed executable `{}` changed outside this replacement",
                        target.user_display()
                    );
                }
            }
            for index in (0..directory.files.exports.len()).rev() {
                rollback_export(&directory.files, index)?;
            }
            sync_directory(&directory.files.directory)?;
        }
        restore_environment(journal)?;
        let mut rolled_back = journal.clone();
        rolled_back.phase = Phase::RolledBack;
        record.replace(rolled_back)?;
    }
    cleanup_record(record)
}

fn cleanup_record(record: &OwnedJournal<EnvironmentJournal>) -> anyhow::Result<()> {
    record.check_current()?;
    let journal = &record.journal;
    for directory in &journal.exports {
        let path = directory.files.directory.join(&directory.files.staging);
        if !path.try_exists()? {
            continue;
        }
        let staging = directory.files.staging_directory()?;
        for (index, removal) in directory.removals.iter().enumerate() {
            for kind in ["backup", "removed"] {
                let path = removal_anchor(&staging, kind, index);
                if let Some(current) = ExportVersion::capture(&path)? {
                    if current != removal.original {
                        bail!(
                            "Executable recovery anchor `{}` changed",
                            path.user_display()
                        );
                    }
                    #[cfg(unix)]
                    fs_err::remove_file(&path)?;
                    #[cfg(windows)]
                    uv_windows::remove_file_preserving_attributes(&path)?;
                }
            }
        }
        cleanup_anchors(&directory.files)?;
    }
    let container_path = journal.original.tool_root.join(&journal.staging);
    if directory_identity(&container_path)?.is_some() {
        let container = journal.container()?;
        let (name, expected) = if journal.phase == Phase::Committed {
            (
                match journal.publication {
                    Publication::Exchange => REPLACEMENT,
                    Publication::Rename => PREVIOUS,
                },
                &journal.original.identity,
            )
        } else {
            (REPLACEMENT, &journal.replacement_identity)
        };
        let obsolete = container.join(name);
        if let Some(identity) = directory_identity(&obsolete)? {
            if &identity != expected {
                bail!(
                    "Obsolete tool environment `{}` changed",
                    obsolete.user_display()
                );
            }
            fs_err::remove_dir_all(&obsolete)?;
        }
        // Unknown siblings keep the record and do not turn an installed generation into failure.
        fs_err::remove_dir(&container)?;
    }
    record.remove()?;
    sync_directory(&journal.original.tool_root)?;
    Ok(())
}

fn journal_path(installed_tools: &InstalledTools, name: &PackageName) -> PathBuf {
    installed_tools
        .root()
        .join(format!("{JOURNAL_PREFIX_ENVIRONMENT}{name}.json"))
}

pub(super) async fn recover_tool_environment(
    installed_tools: &InstalledTools,
    name: &PackageName,
) -> anyhow::Result<()> {
    let Some(mut record) =
        OwnedJournal::<EnvironmentJournal>::read_from(journal_path(installed_tools, name))?
    else {
        return Ok(());
    };
    record.journal.validate(installed_tools, name)?;
    let _entrypoint_locks = ToolEntrypointLocks::for_directories(
        record
            .journal
            .exports
            .iter()
            .map(|directory| directory.files.directory.clone()),
    )
    .await?;
    recover_record(&mut record)
}

pub(super) async fn recover_selected_environments(
    installed_tools: &InstalledTools,
    names: &[PackageName],
) -> anyhow::Result<()> {
    let names = if names.is_empty() {
        pending_environment_names(installed_tools)?
    } else {
        names.to_vec()
    };
    for name in names {
        recover_tool_environment(installed_tools, &name).await?;
    }
    Ok(())
}

pub(super) fn has_pending_environments(installed_tools: &InstalledTools) -> anyhow::Result<bool> {
    Ok(!pending_environment_names(installed_tools)?.is_empty())
}

fn pending_environment_names(installed_tools: &InstalledTools) -> anyhow::Result<Vec<PackageName>> {
    let mut names = BTreeSet::new();
    for entry in fs_err::read_dir(installed_tools.root())? {
        let entry = entry?;
        let filename = entry.file_name();
        let Some(filename) = filename.to_str() else {
            continue;
        };
        let Some(name) = filename
            .strip_prefix(JOURNAL_PREFIX_ENVIRONMENT)
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        let name = name.parse::<PackageName>()?;
        if entry.path() != journal_path(installed_tools, &name) {
            bail!("Invalid tool replacement journal name `{filename}`");
        }
        names.insert(name);
    }
    Ok(names.into_iter().collect())
}

impl EnvironmentJournal {
    fn container(&self) -> anyhow::Result<PathBuf> {
        let directory = self.original.tool_root.join(&self.staging);
        if directory_identity(&directory)?.as_ref() != Some(&self.staging_identity) {
            bail!(
                "Tool replacement directory `{}` changed",
                directory.user_display()
            );
        }
        Ok(directory)
    }

    fn validate(&self, installed_tools: &InstalledTools, name: &PackageName) -> anyhow::Result<()> {
        if self.version != JOURNAL_VERSION
            || &self.original.tool != name
            || fs_err::canonicalize(installed_tools.root())? != self.original.tool_root
            || !is_filename(&self.staging)
            || !self
                .staging
                .to_string_lossy()
                .starts_with(TOOL_ENVIRONMENT_STAGING_PREFIX)
        {
            bail!("Invalid tool replacement journal for `{name}`");
        }
        for directory in &self.exports {
            directory.files.validate(name)?;
            let mut names = directory
                .files
                .exports
                .iter()
                .map(|export| &export.filename)
                .collect::<BTreeSet<_>>();
            for removal in &directory.removals {
                if !is_filename(&removal.filename) || !names.insert(&removal.filename) {
                    bail!("Invalid tool removal path for `{name}`");
                }
            }
        }
        Ok(())
    }
}

fn is_filename(path: &Path) -> bool {
    let mut parts = path.components();
    matches!(parts.next(), Some(Component::Normal(_))) && parts.next().is_none()
}

fn directory_identity(path: &Path) -> anyhow::Result<Option<ExportIdentity>> {
    match fs_err::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
        Ok(metadata) if metadata.is_dir() && !metadata.is_symlink() => {
            ExportIdentity::directory(path)
                .map(Some)
                .map_err(Into::into)
        }
        Ok(_) => bail!(
            "Tool environment `{}` is not an owned directory",
            path.user_display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _root: TempDir,
        root: PathBuf,
        destination: PathBuf,
        container: PathBuf,
        bin: PathBuf,
        record: PathBuf,
    }

    impl Fixture {
        fn new(
            publication: Publication,
            identical_metadata: bool,
        ) -> anyhow::Result<(Self, EnvironmentTransaction)> {
            let temporary = tempfile::tempdir()?;
            let root = fs_err::canonicalize(temporary.path())?;
            let destination = root.join("example");
            let container = tempfile::Builder::new()
                .prefix(TOOL_ENVIRONMENT_STAGING_PREFIX)
                .tempdir_in(&root)?
                .keep();
            let replacement = container.join(REPLACEMENT);
            for (directory, value) in [(&destination, "old"), (&replacement, "new")] {
                fs_err::create_dir(directory)?;
                fs_err::write(directory.join("generation"), value)?;
                let metadata = if identical_metadata { "same" } else { value };
                write_metadata(directory, metadata.as_bytes(), Some(metadata))?;
            }
            let original = EnvironmentSnapshot {
                tool_root: root.clone(),
                tool: "example".parse()?,
                identity: directory_identity(&destination)?.context("Missing original")?,
                metadata: MetadataDigest::read(&destination)?,
            };
            let bin = root.join("bin");
            fs_err::create_dir(&bin)?;
            fs_err::write(bin.join("alpha"), "old alpha")?;
            fs_err::write(bin.join("obsolete"), "old obsolete")?;
            let staging = tempfile::Builder::new()
                .prefix(JOURNAL_PREFIX)
                .tempdir_in(&bin)?
                .keep();
            let mut exports = Vec::new();
            for (index, name) in ["alpha", "beta"].into_iter().enumerate() {
                let target = bin.join(name);
                let original = ExportVersion::capture(&target)?;
                if original.is_some() {
                    fs_err::hard_link(&target, old_anchor(&staging, index))?;
                }
                fs_err::write(new_anchor(&staging, index), format!("new {name}"))?;
                exports.push(JournalExport {
                    filename: name.into(),
                    original,
                    replacement: ExportVersion::capture(&new_anchor(&staging, index))?
                        .context("Missing new export")?,
                });
            }
            let obsolete = ExportVersion::capture(&bin.join("obsolete"))?
                .context("Missing obsolete export")?;
            fs_err::hard_link(bin.join("obsolete"), removal_anchor(&staging, "backup", 0))?;
            let journal = EnvironmentJournal {
                version: JOURNAL_VERSION,
                phase: Phase::Prepared,
                publication,
                original,
                staging: container
                    .file_name()
                    .context("Missing container filename")?
                    .into(),
                staging_identity: ExportIdentity::directory(&container)?,
                replacement_identity: directory_identity(&replacement)?
                    .context("Missing replacement")?,
                replacement_metadata: MetadataDigest::read(&replacement)?,
                exports: vec![DirectoryChanges {
                    files: ExportDirectory {
                        directory: bin.clone(),
                        staging: staging
                            .file_name()
                            .context("Missing anchor filename")?
                            .into(),
                        staging_identity: ExportIdentity::directory(&staging)?,
                        exports,
                    },
                    removals: vec![RemovedExport {
                        filename: "obsolete".into(),
                        original: obsolete,
                    }],
                }],
            };
            let record = root.join(".uv-tool-environment-example.json");
            let owned = OwnedJournal::create_at(record.clone(), journal)?;
            Ok((
                Self {
                    _root: temporary,
                    root,
                    destination,
                    container,
                    bin,
                    record,
                },
                EnvironmentTransaction {
                    record: owned,
                    finished: false,
                },
            ))
        }

        fn assert_original(&self) -> anyhow::Result<()> {
            assert_eq!(
                fs_err::read_to_string(self.destination.join("generation"))?,
                "old"
            );
            assert_eq!(fs_err::read_to_string(self.bin.join("alpha"))?, "old alpha");
            assert_eq!(
                fs_err::read_to_string(self.bin.join("obsolete"))?,
                "old obsolete"
            );
            assert!(!self.bin.join("beta").try_exists()?);
            assert!(!self.record.try_exists()?);
            assert!(!self.container.try_exists()?);
            Ok(())
        }

        fn replay(&self) -> anyhow::Result<()> {
            let mut record = OwnedJournal::<EnvironmentJournal>::read_from(self.record.clone())?
                .context("Missing journal")?;
            recover_record(&mut record)
        }
    }

    #[test]
    fn failed_replacement_restores_environment_exports_and_metadata() -> anyhow::Result<()> {
        let (fixture, mut transaction) = Fixture::new(Publication::Rename, false)?;
        transaction.publish()?;
        assert_eq!(
            fs_err::read_to_string(fixture.destination.join("generation"))?,
            "new"
        );
        assert_eq!(
            fs_err::read_to_string(fixture.bin.join("alpha"))?,
            "new alpha"
        );
        drop(transaction);
        fixture.assert_original()?;
        assert_eq!(
            fs_err::read_to_string(fixture.destination.join("uv-receipt.toml"))?,
            "old"
        );
        assert_eq!(
            fs_err::read_to_string(fixture.destination.join("uv.lock"))?,
            "old"
        );
        Ok(())
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn exchanged_environment_rolls_back_after_late_failure() -> anyhow::Result<()> {
        let (fixture, mut transaction) = Fixture::new(Publication::Exchange, false)?;
        transaction.publish()?;
        drop(transaction);
        fixture.assert_original()
    }

    #[test]
    fn replay_restores_the_gap_between_directory_renames() -> anyhow::Result<()> {
        let (fixture, mut transaction) = Fixture::new(Publication::Rename, false)?;
        rename_directory(&fixture.destination, &fixture.container.join(PREVIOUS))?;
        transaction.finished = true;
        drop(transaction);
        fixture.replay()?;
        fixture.assert_original()
    }

    #[test]
    fn identical_metadata_does_not_commit_an_unfinished_generation() -> anyhow::Result<()> {
        let (fixture, mut transaction) = Fixture::new(Publication::Rename, true)?;
        transaction.publish()?;
        transaction.finished = true;
        drop(transaction);
        fixture.replay()?;
        fixture.assert_original()
    }

    #[test]
    fn committed_cleanup_survives_subsequent_metadata_changes() -> anyhow::Result<()> {
        let (fixture, mut transaction) = Fixture::new(Publication::Rename, false)?;
        transaction.publish()?;
        fs_err::write(fixture.container.join("external"), "keep")?;
        transaction.commit()?;
        drop(transaction);
        assert!(fixture.record.try_exists()?);
        fs_err::write(
            fixture.destination.join("uv-receipt.toml"),
            "later legitimate receipt",
        )?;
        assert!(fixture.replay().is_err());
        assert_eq!(
            fs_err::read_to_string(fixture.destination.join("generation"))?,
            "new"
        );
        assert_eq!(
            fs_err::read_to_string(fixture.container.join("external"))?,
            "keep"
        );
        fs_err::remove_file(fixture.container.join("external"))?;
        fixture.replay()?;
        assert_eq!(
            fs_err::read_to_string(fixture.destination.join("uv-receipt.toml"))?,
            "later legitimate receipt"
        );
        assert!(!fixture.record.try_exists()?);
        Ok(())
    }

    #[test]
    fn rollback_cleanup_can_resume_after_an_obstruction() -> anyhow::Result<()> {
        let (fixture, mut transaction) = Fixture::new(Publication::Rename, false)?;
        transaction.publish()?;
        fs_err::write(fixture.container.join("external"), "keep")?;
        drop(transaction);
        assert!(fixture.record.try_exists()?);
        assert_eq!(
            fs_err::read_to_string(fixture.destination.join("generation"))?,
            "old"
        );
        fs_err::remove_file(fixture.container.join("external"))?;
        fixture.replay()?;
        fixture.assert_original()
    }

    #[test]
    fn foreign_export_keeps_both_generations_and_recovery_record() -> anyhow::Result<()> {
        let (fixture, mut transaction) = Fixture::new(Publication::Rename, false)?;
        transaction.publish()?;
        let foreign = fixture.root.join("foreign");
        fs_err::write(&foreign, "external replacement")?;
        fs_err::rename(foreign, fixture.bin.join("alpha"))?;
        drop(transaction);
        assert_eq!(
            fs_err::read_to_string(fixture.bin.join("alpha"))?,
            "external replacement"
        );
        assert_eq!(
            fs_err::read_to_string(fixture.destination.join("generation"))?,
            "new"
        );
        assert_eq!(
            fs_err::read_to_string(fixture.container.join(PREVIOUS).join("generation"))?,
            "old"
        );
        assert!(fixture.record.try_exists()?);
        assert!(fixture.replay().is_err());
        Ok(())
    }

    #[test]
    fn foreign_environment_is_not_overwritten_by_recovery() -> anyhow::Result<()> {
        let (fixture, mut transaction) = Fixture::new(Publication::Rename, false)?;
        transaction.publish()?;
        fs_err::rename(&fixture.destination, fixture.root.join("retained"))?;
        fs_err::create_dir(&fixture.destination)?;
        fs_err::write(fixture.destination.join("generation"), "external")?;
        drop(transaction);
        assert_eq!(
            fs_err::read_to_string(fixture.destination.join("generation"))?,
            "external"
        );
        assert_eq!(
            fs_err::read_to_string(fixture.container.join(PREVIOUS).join("generation"))?,
            "old"
        );
        assert!(fixture.record.try_exists()?);
        Ok(())
    }
}
