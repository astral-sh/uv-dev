//! Machine-readable results for `uv lock`.

use std::error::Error;
use std::fmt::Display;
use std::path::Path;

use anstream::adapter::strip_str;
use serde::Serialize;

use uv_client::{ErrorKind as ClientErrorKind, WrappedReqwestError};
use uv_configuration::DryRun;
use uv_distribution_types::Name;
use uv_fs::PortablePathBuf;
use uv_normalize::PackageName;
use uv_resolver::{ExcludeNewerChange, ExcludeNewerPackageChange, SatisfiesResult};

use crate::commands::ExitStatus;
use crate::commands::pip::operations::Error as OperationError;
use crate::commands::project::ProjectError;
use crate::commands::project::lock::{LockMode, LockResult};
use crate::settings::{FrozenSource, LockCheck};

/// This schema is intentionally experimental, like the `uv sync` JSON report.
#[derive(Debug, Serialize)]
struct Schema {
    version: &'static str,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    /// The lockfile on disk is up-to-date after the operation.
    Fresh,
    /// The lockfile on disk is missing or needs changes.
    Stale,
    /// Freshness was deliberately not checked.
    NotChecked,
    #[default]
    Indeterminate,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Use,
    Check,
    Update,
    Create,
}

#[derive(Debug, Serialize)]
pub(crate) struct LockReport {
    schema: Schema,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<PortablePathBuf>,
    status: Status,
    /// The lockfile action; create and update are proposed actions in a dry run.
    #[serde(skip_serializing_if = "Option::is_none")]
    action: Option<Action>,
    dry_run: bool,
    #[serde(skip)]
    completed: bool,
    /// Why the previous lock could not be reused, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<LockReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    validation_error: Option<ErrorReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ErrorReport>,
}

impl LockReport {
    pub(super) fn new(
        lock_check: LockCheck,
        frozen: Option<FrozenSource>,
        dry_run: DryRun,
    ) -> Self {
        let (action, dry_run) = if frozen.is_some() {
            (Some(Action::Use), false)
        } else {
            match lock_check {
                LockCheck::Enabled(_) => (Some(Action::Check), false),
                LockCheck::Disabled => (None, dry_run.enabled()),
            }
        };
        Self {
            schema: Schema { version: "preview" },
            path: None,
            status: Status::Indeterminate,
            action,
            dry_run,
            completed: false,
            reason: None,
            validation_error: None,
            error: None,
        }
    }

    pub(super) fn set_path(&mut self, path: &Path) {
        self.path = Some(path.into());
    }

    /// Record a proven mismatch, not merely a request to refresh or upgrade.
    pub(super) fn stale(&mut self, reason: LockReason) {
        self.reason = Some(reason);
    }

    /// Preserve a failed validation even if the subsequent resolution also fails.
    pub(super) fn validation_error(&mut self, error: &ProjectError) {
        self.validation_error = Some(ErrorReport::from_project(error));
    }

    pub(super) fn operation_success(&mut self, mode: &LockMode<'_>, result: &LockResult) {
        self.completed = true;
        match mode {
            LockMode::Frozen(_) => {
                self.action = Some(Action::Use);
                self.status = Status::NotChecked;
                self.reason = None;
                self.validation_error = None;
            }
            LockMode::Locked(..) => {
                self.action = Some(Action::Check);
                match result {
                    LockResult::Unchanged(_) => {
                        self.status = Status::Fresh;
                        self.reason = None;
                        self.validation_error = None;
                    }
                    LockResult::Changed(..) => {
                        self.status = Status::Stale;
                        self.reason
                            .get_or_insert_with(|| LockReason::new(ReasonCode::LockChanged));
                    }
                }
            }
            LockMode::Write(_) | LockMode::DryRun(_) => match result {
                LockResult::Unchanged(_) => {
                    self.action = Some(Action::Check);
                    self.status = Status::Fresh;
                    self.reason = None;
                    self.validation_error = None;
                }
                LockResult::Changed(previous, _) => {
                    let action = if previous.is_some() {
                        Action::Update
                    } else {
                        Action::Create
                    };
                    self.action = Some(action);
                    self.status = if self.dry_run {
                        Status::Stale
                    } else {
                        Status::Fresh
                    };
                    self.reason
                        .get_or_insert_with(|| LockReason::new(ReasonCode::LockChanged));
                }
            },
        }
    }

    pub(super) fn operation_error(&mut self, error: &ProjectError) {
        let reason = if let ProjectError::MissingLockfile(..) = error {
            Some(ReasonCode::MissingLockfile)
        } else if let ProjectError::LockFormat(..) = error {
            Some(ReasonCode::NonCanonicalFormatting)
        } else if let ProjectError::LockMismatch(..) = error {
            Some(ReasonCode::LockChanged)
        } else {
            None
        };
        if let Some(reason) = reason {
            self.reason.get_or_insert_with(|| LockReason::new(reason));
        } else {
            self.error = Some(ErrorReport::from_project(error));
        }
    }

    pub(super) fn finish(&mut self, result: &anyhow::Result<ExitStatus>) {
        match result {
            Ok(ExitStatus::Success) => {
                self.error = None;
            }
            Ok(ExitStatus::Failure | ExitStatus::Error | ExitStatus::External(_)) | Err(_) => {
                if !self.completed {
                    self.status = if self.reason.is_some() {
                        Status::Stale
                    } else {
                        Status::Indeterminate
                    };
                }
                if self.error.is_none()
                    && (self.reason.is_none() || self.completed)
                    && let Err(error) = result
                {
                    self.error = Some(ErrorReport::new(error.as_ref()));
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ReasonCode {
    MissingLockfile,
    NonCanonicalFormatting,
    LockChanged,
    ResolutionModeChanged,
    ForkStrategyChanged,
    ExcludeNewerChanged,
    MarkerCoverageChanged,
    PythonCoverageChanged,
    EnvironmentsChanged,
    RequiredEnvironmentsChanged,
    ConflictsChanged,
    RequiresPythonChanged,
    PrereleaseChanged,
    HashAlgorithmsChanged,
    MembersChanged,
    EditableChanged,
    VirtualChanged,
    DynamicChanged,
    VersionChanged,
    RequirementsChanged,
    ConstraintsChanged,
    OverridesChanged,
    ExcludesChanged,
    BuildConstraintsChanged,
    DependencyGroupsChanged,
    StaticMetadataChanged,
    MissingRoot,
    MissingRemoteIndex,
    MissingLocalIndex,
    PackageRequirementsChanged,
    PackageDependenciesChanged,
    PackageDependencyGroupsChanged,
    PackageExtrasChanged,
    MissingVersion,
}

#[derive(Debug, Serialize)]
pub(super) struct LockReason {
    code: ReasonCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    package: Option<PackageName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    /// The values required by the current inputs.
    #[serde(skip_serializing_if = "Option::is_none")]
    expected: Option<Vec<String>>,
    /// The values recorded in the existing lockfile.
    #[serde(skip_serializing_if = "Option::is_none")]
    actual: Option<Vec<String>>,
}

impl LockReason {
    pub(super) fn new(code: ReasonCode) -> Self {
        Self {
            code,
            package: None,
            message: None,
            expected: None,
            actual: None,
        }
    }

    fn package(mut self, package: &PackageName) -> Self {
        self.package = Some(package.clone());
        self
    }

    pub(super) fn values(
        mut self,
        expected: impl IntoIterator<Item = impl Display>,
        actual: impl IntoIterator<Item = impl Display>,
    ) -> Self {
        self.expected = Some(expected.into_iter().map(plain).collect());
        self.actual = Some(actual.into_iter().map(plain).collect());
        self
    }

    pub(super) fn exclude_newer(change: &ExcludeNewerChange) -> Self {
        let mut reason = Self::new(ReasonCode::ExcludeNewerChanged);
        reason.message = Some(plain(change));
        match change {
            ExcludeNewerChange::GlobalChanged(_)
            | ExcludeNewerChange::GlobalAdded(_)
            | ExcludeNewerChange::GlobalRemoved => {}
            ExcludeNewerChange::Package(
                ExcludeNewerPackageChange::PackageAdded(package, _)
                | ExcludeNewerPackageChange::PackageRemoved(package)
                | ExcludeNewerPackageChange::PackageChanged(package, _),
            ) => reason.package = Some(package.clone()),
        }
        reason
    }

    pub(super) fn from_satisfies(result: &SatisfiesResult<'_>) -> Option<Self> {
        Some(match result {
            SatisfiesResult::Satisfied => return None,
            SatisfiesResult::MismatchedMembers(expected, actual) => {
                Self::new(ReasonCode::MembersChanged).values(expected, *actual)
            }
            SatisfiesResult::MismatchedVirtual(package, _) => {
                Self::new(ReasonCode::VirtualChanged).package(package)
            }
            SatisfiesResult::MismatchedEditable(package, _) => {
                Self::new(ReasonCode::EditableChanged).package(package)
            }
            SatisfiesResult::MismatchedDynamic(package, _) => {
                Self::new(ReasonCode::DynamicChanged).package(package)
            }
            SatisfiesResult::MismatchedVersion(package, locked_version, current_version) => {
                Self::new(ReasonCode::VersionChanged)
                    .package(package)
                    .values(current_version, [locked_version])
            }
            SatisfiesResult::MismatchedRequirements(expected, actual) => {
                Self::new(ReasonCode::RequirementsChanged).values(expected, actual)
            }
            SatisfiesResult::MismatchedConstraints(expected, actual) => {
                Self::new(ReasonCode::ConstraintsChanged).values(expected, actual)
            }
            SatisfiesResult::MismatchedOverrides(..) => Self::new(ReasonCode::OverridesChanged),
            SatisfiesResult::MismatchedExcludes(..) => Self::new(ReasonCode::ExcludesChanged),
            SatisfiesResult::MismatchedBuildConstraints(expected, actual) => {
                Self::new(ReasonCode::BuildConstraintsChanged).values(expected, actual)
            }
            SatisfiesResult::MismatchedDependencyGroups(..) => {
                Self::new(ReasonCode::DependencyGroupsChanged)
            }
            SatisfiesResult::MismatchedStaticMetadata(..) => {
                Self::new(ReasonCode::StaticMetadataChanged)
            }
            SatisfiesResult::MissingRoot(package) => {
                Self::new(ReasonCode::MissingRoot).package(package)
            }
            SatisfiesResult::MissingRemoteIndex(package, ..) => {
                Self::new(ReasonCode::MissingRemoteIndex).package(package)
            }
            SatisfiesResult::MissingLocalIndex(package, ..) => {
                Self::new(ReasonCode::MissingLocalIndex).package(package)
            }
            SatisfiesResult::MismatchedPackageRequirements(package, _, expected, actual) => {
                Self::new(ReasonCode::PackageRequirementsChanged)
                    .package(package)
                    .values(expected, actual)
            }
            SatisfiesResult::MismatchedPackageDependencies(package, ..) => {
                Self::new(ReasonCode::PackageDependenciesChanged).package(package)
            }
            SatisfiesResult::MismatchedPackageDependencyGroups(package, ..) => {
                Self::new(ReasonCode::PackageDependencyGroupsChanged).package(package)
            }
            SatisfiesResult::MismatchedPackageProvidesExtra(package, _, expected, actual) => {
                Self::new(ReasonCode::PackageExtrasChanged)
                    .package(package)
                    .values(expected, actual)
            }
            SatisfiesResult::MissingVersion(package) => {
                Self::new(ReasonCode::MissingVersion).package(package)
            }
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ErrorCode {
    EvaluationFailed,
    MetadataUnavailable,
    OfflineCacheMiss,
    Authentication,
    AccessDenied,
    Http,
    Network,
}

#[derive(Debug, Serialize)]
struct ErrorReport {
    code: ErrorCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    package: Option<PackageName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    http_status: Option<u16>,
    message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    causes: Vec<String>,
}

impl ErrorReport {
    fn new(error: &(dyn Error + 'static)) -> Self {
        let mut report = Self {
            code: ErrorCode::EvaluationFailed,
            package: None,
            http_status: None,
            message: plain(error),
            causes: Vec::new(),
        };
        report.classify(error);
        let mut source = error.source();
        while let Some(error) = source {
            report.causes.push(plain(error));
            report.classify(error);
            source = error.source();
        }
        report
    }

    fn from_project(error: &ProjectError) -> Self {
        let mut report = Self::new(error);
        // Transparent error wrappers can omit themselves from `Error::source`.
        if let ProjectError::Operation(OperationError::Requirements(error)) = error {
            report.requirements(error);
        }
        if let ProjectError::Client(error) = error {
            report.client(error.kind());
        }
        if let ProjectError::Lock(error) = error
            && let Some(package) = error.resolution_package()
        {
            report.package = Some(package.clone());
            if let ErrorCode::EvaluationFailed = report.code {
                report.code = ErrorCode::MetadataUnavailable;
            }
        }
        report
    }

    fn classify(&mut self, error: &(dyn Error + 'static)) {
        if let Some(error) = error.downcast_ref::<uv_requirements::Error>() {
            self.requirements(error);
        }
        if let Some(error) = error.downcast_ref::<uv_distribution::Error>() {
            self.distribution(error);
        }
        if let Some(error) = error.downcast_ref::<uv_client::Error>() {
            self.client(error.kind());
        }
        if let Some(error) = error.downcast_ref::<ClientErrorKind>() {
            self.client(error);
        }
        if let Some(error) = error.downcast_ref::<WrappedReqwestError>() {
            self.network(error);
        }
    }

    fn requirements(&mut self, error: &uv_requirements::Error) {
        match error {
            uv_requirements::Error::Dist(_, distribution, error) => {
                self.package = Some(distribution.name().clone());
                if let ErrorCode::EvaluationFailed = self.code {
                    self.code = ErrorCode::MetadataUnavailable;
                }
                self.distribution(error);
            }
            uv_requirements::Error::Distribution(error) => {
                if let ErrorCode::EvaluationFailed = self.code {
                    self.code = ErrorCode::MetadataUnavailable;
                }
                self.distribution(error);
            }
            uv_requirements::Error::DistributionTypes(_)
            | uv_requirements::Error::HashStrategy(_)
            | uv_requirements::Error::WheelFilename(_)
            | uv_requirements::Error::Io(_) => {}
        }
    }

    fn distribution(&mut self, error: &uv_distribution::Error) {
        if let uv_distribution::Error::Client(error) = error {
            self.client(error.kind());
        } else if let uv_distribution::Error::Reqwest(error) = error {
            self.network(error);
        }
    }

    fn client(&mut self, error: &ClientErrorKind) {
        if let ClientErrorKind::Offline(_) = error {
            self.code = ErrorCode::OfflineCacheMiss;
        } else if let ClientErrorKind::WrappedReqwestError(_, error) = error {
            self.network(error);
        }
    }

    fn network(&mut self, error: &WrappedReqwestError) {
        self.http_status = error.status().map(|status| status.as_u16());
        self.code = match self.http_status {
            Some(401) => ErrorCode::Authentication,
            Some(403) => ErrorCode::AccessDenied,
            Some(_) => ErrorCode::Http,
            None => ErrorCode::Network,
        };
    }
}

/// Error and requirement displays can contain terminal styling even when stdout is redirected.
fn plain(value: impl Display) -> String {
    strip_str(&value.to_string()).to_string()
}
