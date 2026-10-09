//! Machine-readable results for `uv lock`.

use std::error::Error;
use std::fmt::{Display, Write as _};
use std::path::Path;

use anstream::adapter::strip_str;
use serde::Serialize;

use uv_client::{ErrorKind as ClientErrorKind, WrappedReqwestError};
use uv_command_support::ExitStatus;
use uv_configuration::DryRun;
use uv_distribution_types::{Name, RequirementSource};
use uv_fs::PortablePathBuf;
use uv_lock_operations::{
    LockError, LockMode, LockReporter, LockResult, LockValidationError, LockValidationReason,
    LockValidationReasonCode, LockValidationValues,
};
use uv_normalize::PackageName;
use uv_resolver::{NoSolutionError, PubGrubHint};
use uv_settings::{FrozenSource, LockCheck};

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
                    self.action = Some(if previous.is_some() {
                        Action::Update
                    } else {
                        Action::Create
                    });
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

    pub(super) fn operation_error(&mut self, error: &LockError) {
        let reason = if let LockError::MissingLockfile(..) = error {
            Some(ReasonCode::MissingLockfile)
        } else if let LockError::LockFormat(..) = error {
            Some(ReasonCode::NonCanonicalFormatting)
        } else if let LockError::LockMismatch(..) = error {
            Some(ReasonCode::LockChanged)
        } else {
            None
        };
        if let Some(reason) = reason {
            self.reason.get_or_insert_with(|| LockReason::new(reason));
        } else {
            self.error = Some(ErrorReport::from_lock(error));
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
    MinimumLibcVersionChanged,
    ConflictsChanged,
    RequiresPythonChanged,
    PrereleaseChanged,
    HashAlgorithmsChanged,
    MembersChanged,
    MemberDefaultGroupsChanged,
    WorkspaceGroupMetadataChanged,
    WorkspaceDefaultGroupsChanged,
    MemberGroupMetadataChanged,
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
    fn new(code: ReasonCode) -> Self {
        Self {
            code,
            package: None,
            message: None,
            expected: None,
            actual: None,
        }
    }
}

impl From<LockValidationReason> for LockReason {
    fn from(reason: LockValidationReason) -> Self {
        Self {
            code: reason.code.into(),
            package: reason.package,
            message: reason.message,
            expected: reason.expected.map(render_values),
            actual: reason.actual.map(render_values),
        }
    }
}

fn render_values(values: LockValidationValues) -> Vec<String> {
    match values {
        LockValidationValues::Strings(values) => values,
        LockValidationValues::Requirements(requirements) => requirements
            .into_iter()
            .map(|requirement| {
                let mut value = requirement.to_string();
                if let RequirementSource::Directory {
                    editable,
                    r#virtual,
                    ..
                } = requirement.source
                {
                    if let Some(editable) = editable {
                        let _ = write!(value, " (editable: {editable})");
                    }
                    if let Some(r#virtual) = r#virtual {
                        let _ = write!(value, " (virtual: {virtual})");
                    }
                }
                value
            })
            .collect(),
        LockValidationValues::BuildConstraints(constraints) => constraints
            .into_iter()
            .map(|constraint| {
                let mut value = constraint.requirement.to_string();
                for hash in constraint.hashes {
                    value.push_str(" --hash=");
                    value.push_str(&hash);
                }
                value
            })
            .collect(),
    }
}

impl From<LockValidationReasonCode> for ReasonCode {
    fn from(code: LockValidationReasonCode) -> Self {
        match code {
            LockValidationReasonCode::MissingLockfile => Self::MissingLockfile,
            LockValidationReasonCode::ResolutionModeChanged => Self::ResolutionModeChanged,
            LockValidationReasonCode::ForkStrategyChanged => Self::ForkStrategyChanged,
            LockValidationReasonCode::ExcludeNewerChanged => Self::ExcludeNewerChanged,
            LockValidationReasonCode::MarkerCoverageChanged => Self::MarkerCoverageChanged,
            LockValidationReasonCode::PythonCoverageChanged => Self::PythonCoverageChanged,
            LockValidationReasonCode::EnvironmentsChanged => Self::EnvironmentsChanged,
            LockValidationReasonCode::RequiredEnvironmentsChanged => {
                Self::RequiredEnvironmentsChanged
            }
            LockValidationReasonCode::MinimumLibcVersionChanged => Self::MinimumLibcVersionChanged,
            LockValidationReasonCode::ConflictsChanged => Self::ConflictsChanged,
            LockValidationReasonCode::RequiresPythonChanged => Self::RequiresPythonChanged,
            LockValidationReasonCode::PrereleaseChanged => Self::PrereleaseChanged,
            LockValidationReasonCode::HashAlgorithmsChanged => Self::HashAlgorithmsChanged,
            LockValidationReasonCode::MembersChanged => Self::MembersChanged,
            LockValidationReasonCode::MemberDefaultGroupsChanged => {
                Self::MemberDefaultGroupsChanged
            }
            LockValidationReasonCode::WorkspaceGroupMetadataChanged => {
                Self::WorkspaceGroupMetadataChanged
            }
            LockValidationReasonCode::WorkspaceDefaultGroupsChanged => {
                Self::WorkspaceDefaultGroupsChanged
            }
            LockValidationReasonCode::MemberGroupMetadataChanged => {
                Self::MemberGroupMetadataChanged
            }
            LockValidationReasonCode::EditableChanged => Self::EditableChanged,
            LockValidationReasonCode::VirtualChanged => Self::VirtualChanged,
            LockValidationReasonCode::DynamicChanged => Self::DynamicChanged,
            LockValidationReasonCode::VersionChanged => Self::VersionChanged,
            LockValidationReasonCode::RequirementsChanged => Self::RequirementsChanged,
            LockValidationReasonCode::ConstraintsChanged => Self::ConstraintsChanged,
            LockValidationReasonCode::OverridesChanged => Self::OverridesChanged,
            LockValidationReasonCode::ExcludesChanged => Self::ExcludesChanged,
            LockValidationReasonCode::BuildConstraintsChanged => Self::BuildConstraintsChanged,
            LockValidationReasonCode::DependencyGroupsChanged => Self::DependencyGroupsChanged,
            LockValidationReasonCode::StaticMetadataChanged => Self::StaticMetadataChanged,
            LockValidationReasonCode::MissingRoot => Self::MissingRoot,
            LockValidationReasonCode::MissingRemoteIndex => Self::MissingRemoteIndex,
            LockValidationReasonCode::MissingLocalIndex => Self::MissingLocalIndex,
            LockValidationReasonCode::PackageRequirementsChanged => {
                Self::PackageRequirementsChanged
            }
            LockValidationReasonCode::PackageDependenciesChanged => {
                Self::PackageDependenciesChanged
            }
            LockValidationReasonCode::PackageDependencyGroupsChanged => {
                Self::PackageDependencyGroupsChanged
            }
            LockValidationReasonCode::PackageExtrasChanged => Self::PackageExtrasChanged,
            LockValidationReasonCode::MissingVersion => Self::MissingVersion,
        }
    }
}

impl LockReporter for LockReport {
    fn stale(&mut self, reason: LockValidationReason) {
        self.reason = Some(reason.into());
    }

    fn validation_error(&mut self, error: &LockValidationError) {
        self.validation_error = Some(ErrorReport::from_validation(error));
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    hints: Vec<String>,
}

impl ErrorReport {
    fn new(error: &(dyn Error + 'static)) -> Self {
        let mut report = Self {
            code: ErrorCode::EvaluationFailed,
            package: None,
            http_status: None,
            message: plain(error),
            causes: Vec::new(),
            hints: Vec::new(),
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

    fn from_lock(error: &LockError) -> Self {
        let mut report = Self::new(error);
        if let LockError::Resolve(error) = error {
            report.resolution(error);
        }
        if let LockError::Lock(error) = error
            && let Some(package) = error.resolution_package()
        {
            report.package = Some(package.clone());
            if let ErrorCode::EvaluationFailed = report.code {
                report.code = ErrorCode::MetadataUnavailable;
            }
        }
        report
    }

    fn from_validation(error: &LockValidationError) -> Self {
        let mut report = Self::new(error);
        if let LockValidationError::Lock(error) = error
            && let Some(package) = error.resolution_package()
        {
            report.package = Some(package.clone());
            if let ErrorCode::EvaluationFailed = report.code {
                report.code = ErrorCode::MetadataUnavailable;
            }
        }
        report
    }

    fn resolution(&mut self, error: &uv_resolve_operations::Error) {
        if let Some(error) = error.as_no_solution() {
            self.resolver_hints(error);
        }
        if let uv_resolve_operations::Error::Requirements(error)
        | uv_resolve_operations::Error::RequirementsWithContext { source: error, .. } = error
        {
            self.requirements(error);
        }
    }

    fn resolver_hints(&mut self, error: &NoSolutionError) {
        self.hints = error.resolution_hints().map(plain).collect();
        for hint in error.resolution_hints() {
            if let PubGrubHint::Offline = hint {
                self.code = ErrorCode::OfflineCacheMiss;
            }
            if let PubGrubHint::UnauthorizedIndex { .. } = hint {
                self.code = ErrorCode::Authentication;
                self.http_status = Some(401);
                break;
            }
            if let PubGrubHint::ForbiddenIndex { .. } = hint {
                self.code = ErrorCode::AccessDenied;
                self.http_status = Some(403);
                break;
            }
        }
    }

    fn classify(&mut self, error: &(dyn Error + 'static)) {
        if let Some(error) = error.downcast_ref::<uv_resolve_operations::Error>() {
            self.resolution(error);
        }
        if let Some(error) = error.downcast_ref::<NoSolutionError>() {
            self.resolver_hints(error);
        }
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
            | uv_requirements::Error::FlatIndex(_)
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
