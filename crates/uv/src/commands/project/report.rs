use std::ops::Deref;

use owo_colors::OwoColorize;
use serde::Serialize;
use uv_cli::SyncFormat;
use uv_configuration::DryRun;
use uv_distribution_types::Name;
use uv_fs::{PortablePathBuf, Simplified};
use uv_normalize::PackageName;
use uv_python::PythonEnvironment;
use uv_resolver::PythonReport;
use uv_scripts::Pep723Script;
use uv_workspace::{VirtualProject, Workspace};

use crate::commands::pip::operations::{ChangedDist, Changelog};
use crate::commands::project::lock::{LockMode, LockResult};
use crate::commands::project::lock_target::LockTarget;
use crate::commands::project::sync::{Outcome, SyncEnvironment, SyncTarget};
use crate::commands::project::{ProjectEnvironment, ScriptEnvironment};

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct WorkspaceReport {
    /// The workspace directory path.
    path: PortablePathBuf,
}

impl From<&Workspace> for WorkspaceReport {
    fn from(workspace: &Workspace) -> Self {
        Self {
            path: workspace.install_path().as_path().into(),
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct ProjectReport {
    //
    path: PortablePathBuf,
    workspace: WorkspaceReport,
}

impl From<&VirtualProject> for ProjectReport {
    fn from(project: &VirtualProject) -> Self {
        Self {
            path: project.root().into(),
            workspace: WorkspaceReport::from(project.workspace()),
        }
    }
}

impl From<&SyncTarget> for TargetName {
    fn from(target: &SyncTarget) -> Self {
        match target {
            SyncTarget::Project(_) => Self::Project,
            SyncTarget::Script(_) => Self::Script,
        }
    }
}

#[derive(Serialize, Debug)]
struct ScriptReport {
    /// The path to the script.
    path: PortablePathBuf,
}

impl From<&Pep723Script> for ScriptReport {
    fn from(script: &Pep723Script) -> Self {
        Self {
            path: script.path.as_path().into(),
        }
    }
}

#[derive(Serialize, Debug, Default)]
#[serde(rename_all = "snake_case")]
enum SchemaVersion {
    /// An unstable, experimental schema.
    #[default]
    Preview,
}

#[derive(Serialize, Debug, Default)]
struct SchemaReport {
    /// The version of the schema.
    version: SchemaVersion,
}

/// A report of the uv sync operation
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) struct Report {
    /// The schema of this report.
    schema: SchemaReport,
    /// The target of the sync operation, either a project or a script.
    target: TargetName,
    /// The report for a [`TargetName::Project`], if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<ProjectReport>,
    /// The report for a [`TargetName::Script`], if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    script: Option<ScriptReport>,
    /// The report for the sync operation.
    sync: SyncReport,
    /// The report for the lock operation.
    lock: Option<LockReport>,
    /// Whether this is a dry run.
    dry_run: bool,
}

/// The kind of target
#[derive(Debug, Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum TargetName {
    Project,
    Script,
}

impl std::fmt::Display for TargetName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Project => write!(f, "project"),
            Self::Script => write!(f, "script"),
        }
    }
}

/// Represents the action taken during a sync.
#[derive(Serialize, Debug)]
#[serde(rename_all = "snake_case")]
enum SyncAction {
    /// The environment was checked and required no updates.
    Check,
    /// The environment was updated.
    Update,
    /// The environment was replaced.
    Replace,
    /// A new environment was created.
    Create,
}

impl From<&SyncEnvironment> for SyncAction {
    fn from(env: &SyncEnvironment) -> Self {
        match &env {
            SyncEnvironment::Project(ProjectEnvironment::Existing(..)) => Self::Check,
            SyncEnvironment::Project(ProjectEnvironment::Created(..)) => Self::Create,
            SyncEnvironment::Project(ProjectEnvironment::WouldCreate(..)) => Self::Create,
            SyncEnvironment::Project(ProjectEnvironment::WouldReplace(..)) => Self::Replace,
            SyncEnvironment::Project(ProjectEnvironment::Replaced(..)) => Self::Update,
            SyncEnvironment::Script(ScriptEnvironment::Existing(..)) => Self::Check,
            SyncEnvironment::Script(ScriptEnvironment::Created(..)) => Self::Create,
            SyncEnvironment::Script(ScriptEnvironment::WouldCreate(..)) => Self::Create,
            SyncEnvironment::Script(ScriptEnvironment::WouldReplace(..)) => Self::Replace,
            SyncEnvironment::Script(ScriptEnvironment::Replaced(..)) => Self::Update,
        }
    }
}

impl SyncAction {
    fn message(&self, target: TargetName, dry_run: bool) -> Option<&'static str> {
        let message = if dry_run {
            match self {
                Self::Check => "Would use",
                Self::Update => "Would update",
                Self::Replace => "Would replace",
                Self::Create => "Would create",
            }
        } else {
            // For projects, we omit some of these messages when we're not in dry-run mode
            let is_project = matches!(target, TargetName::Project);
            match self {
                Self::Check | Self::Update | Self::Create if is_project => {
                    return None;
                }
                Self::Check => "Using",
                Self::Update => "Updating",
                Self::Replace => "Replacing",
                Self::Create => "Creating",
            }
        };
        Some(message)
    }
}

/// Represents the action taken during a lock.
#[derive(Serialize, Debug)]
#[serde(rename_all = "snake_case")]
enum LockAction {
    /// The lockfile was used without checking.
    Use,
    /// The lockfile was checked and required no updates.
    Check,
    /// The lockfile was updated.
    Update,
    /// A new lockfile was created.
    Create,
}

impl LockAction {
    fn message(&self, dry_run: bool) -> Option<&'static str> {
        let message = if dry_run {
            match self {
                Self::Use => return None,
                Self::Check => "Found up-to-date",
                Self::Update => "Would update",
                Self::Create => "Would create",
            }
        } else {
            return None;
        };
        Some(message)
    }
}

#[derive(Serialize, Debug)]
struct EnvironmentReport {
    /// The path to the environment.
    path: PortablePathBuf,
    /// The Python interpreter for the environment.
    python: PythonReport,
}

impl From<&PythonEnvironment> for EnvironmentReport {
    fn from(env: &PythonEnvironment) -> Self {
        Self {
            python: PythonReport::from(env.interpreter()),
            path: env.root().into(),
        }
    }
}

impl From<&SyncEnvironment> for EnvironmentReport {
    fn from(env: &SyncEnvironment) -> Self {
        let report = Self::from(&**env);
        // Replace the path if necessary; we construct a temporary virtual environment during dry
        // run invocations and want to report the path we _would_ use.
        if let Some(path) = env.dry_run_target() {
            report.with_path(path.into())
        } else {
            report
        }
    }
}

impl EnvironmentReport {
    /// Set the path for this environment report.
    #[must_use]
    fn with_path(mut self, path: PortablePathBuf) -> Self {
        if let Ok(python_path) = self.python.path().strip_prefix(self.path) {
            let new_path = path.as_ref().to_path_buf().join(python_path);
            self.python = self.python.with_path(new_path.as_path().into());
        }
        self.path = path;
        self
    }
}

/// The report for a sync operation.
#[derive(Serialize, Debug)]
pub(super) struct SyncReport {
    /// The environment.
    environment: EnvironmentReport,
    /// The action performed during the sync, e.g., what was done to the environment.
    action: SyncAction,
    /// The packages that changed during the sync.
    #[serde(default)]
    changes: PackageChangesReport,

    // We store these fields so the report can format itself self-contained, but the outer
    // [`Report`] is intended to include these in user-facing output
    #[serde(skip)]
    dry_run: bool,
    #[serde(skip)]
    target: TargetName,
}

impl SyncReport {
    pub(super) fn new(target: &SyncTarget, environment: &SyncEnvironment, dry_run: DryRun) -> Self {
        Self {
            dry_run: dry_run.enabled(),
            environment: EnvironmentReport::from(environment),
            action: SyncAction::from(environment),
            target: TargetName::from(target),
            changes: PackageChangesReport::default(),
        }
    }

    pub(super) fn format(&self, output_format: SyncFormat) -> Option<String> {
        match output_format {
            // This is an intermediate report, when using JSON, it's only rendered at the end
            SyncFormat::Json => None,
            SyncFormat::Text => self.to_human_readable_string(),
        }
    }

    fn to_human_readable_string(&self) -> Option<String> {
        let Self {
            environment,
            action,
            changes: _,
            dry_run,
            target,
        } = self;

        let action = action.message(*target, *dry_run)?;

        let message = format!(
            "{action} {target} environment at: {path}",
            path = environment.path.user_display().cyan(),
        );
        if *dry_run {
            return Some(message.dimmed().to_string());
        }

        Some(message)
    }
}

/// A summary of all package changes performed during sync.
#[derive(Serialize, Debug, Clone, Default)]
struct PackageChangesReport(Vec<PackageChangeReport>);

impl PackageChangesReport {
    fn from_changelog(changelog: &Changelog) -> Self {
        let mut changes: Vec<_> =
            changelog
                .uninstalled
                .iter()
                .map(|dist| PackageChangeReport::from_dist(dist, PackageChangeAction::Uninstalled))
                .chain(changelog.installed.iter().map(|dist| {
                    PackageChangeReport::from_dist(dist, PackageChangeAction::Installed)
                }))
                .chain(changelog.reinstalled.iter().map(|dist| {
                    PackageChangeReport::from_dist(dist, PackageChangeAction::Reinstalled)
                }))
                .collect();

        changes.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| a.action.cmp(&b.action))
                .then_with(|| a.version.cmp(&b.version))
        });
        Self(changes)
    }
}

/// A summary of a single package change performed during sync.
#[derive(Serialize, Debug, Clone)]
struct PackageChangeReport {
    /// The normalized package name.
    name: PackageName,
    /// The resolved version of the package.
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<uv_pep440::Version>,
    /// The action that was taken for the package.
    action: PackageChangeAction,
}

impl PackageChangeReport {
    fn from_dist(dist: &ChangedDist, action: PackageChangeAction) -> Self {
        Self {
            name: dist.name().clone(),
            version: dist.version().cloned(),
            action,
        }
    }
}

/// The action taken on an individual package during sync.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum PackageChangeAction {
    Uninstalled,
    Installed,
    Reinstalled,
}

/// The report for a lock operation.
#[derive(Debug, Serialize)]
pub(super) struct LockReport {
    /// The path to the lockfile
    path: PortablePathBuf,
    /// Whether the lockfile was preserved, created, or updated.
    action: LockAction,

    // We store this field so the report can format itself self-contained, but the outer
    // [`Report`] is intended to include this in user-facing output
    #[serde(skip)]
    dry_run: bool,
}

impl From<(&LockTarget<'_>, &LockMode<'_>, &Outcome)> for LockReport {
    fn from((target, mode, outcome): (&LockTarget, &LockMode, &Outcome)) -> Self {
        Self {
            path: target.lock_path().deref().into(),
            action: match outcome {
                Outcome::Success(result) => {
                    match result {
                        LockResult::Unchanged(..) => match mode {
                            // When `--frozen` is used, we don't check the lockfile.
                            LockMode::Frozen(_) => LockAction::Use,
                            LockMode::DryRun(_) | LockMode::Locked(_, _) | LockMode::Write(_) => {
                                LockAction::Check
                            }
                        },
                        LockResult::Changed(None, ..) => LockAction::Create,
                        LockResult::Changed(Some(_), ..) => LockAction::Update,
                    }
                }
                // TODO(zanieb): We don't have a way to report the outcome of the lock yet
                Outcome::LockMismatch(..) => LockAction::Check,
            },
            dry_run: matches!(mode, LockMode::DryRun(_)),
        }
    }
}

impl LockReport {
    pub(super) fn format(&self, output_format: SyncFormat) -> Option<String> {
        match output_format {
            SyncFormat::Json => None,
            SyncFormat::Text => self.to_human_readable_string(),
        }
    }

    fn to_human_readable_string(&self) -> Option<String> {
        let Self {
            path,
            action,
            dry_run,
        } = self;

        let action = action.message(*dry_run)?;

        let message = format!(
            "{action} lockfile at: {path}",
            path = path.user_display().cyan(),
        );
        if *dry_run {
            return Some(message.dimmed().to_string());
        }

        Some(message)
    }
}

impl Report {
    pub(super) fn new(
        target: &SyncTarget,
        environment: &SyncEnvironment,
        changelog: &Changelog,
        lock: Option<LockReport>,
        dry_run: DryRun,
    ) -> Self {
        Self {
            schema: SchemaReport::default(),
            target: TargetName::from(target),
            project: target.project().map(ProjectReport::from),
            script: target.script().map(ScriptReport::from),
            sync: SyncReport {
                environment: EnvironmentReport::from(environment),
                action: SyncAction::from(environment),
                changes: PackageChangesReport::from_changelog(changelog),
                dry_run: dry_run.enabled(),
                target: TargetName::from(target),
            },
            lock,
            dry_run: dry_run.enabled(),
        }
    }

    pub(super) fn format(&self, output_format: SyncFormat) -> Option<String> {
        match output_format {
            SyncFormat::Json => serde_json::to_string_pretty(self).ok(),
            SyncFormat::Text => None,
        }
    }
}
