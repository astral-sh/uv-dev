use std::fmt::Display;

use uv_configuration::{ExcludeNewerChange, ExcludeNewerPackageChange};
use uv_distribution_types::NameRequirementSpecification;
use uv_lock::SatisfiesResult;
use uv_normalize::PackageName;

use crate::LockValidationError;

/// Receives structured diagnostics while an existing lockfile is validated.
pub trait LockReporter {
    /// Record a proven mismatch between the lockfile and the current inputs.
    fn stale(&mut self, reason: LockValidationReason);

    /// Preserve a validation failure even if a subsequent resolution also fails.
    fn validation_error(&mut self, error: &LockValidationError);
}

/// A structured reason that an existing lockfile could not be reused.
#[derive(Debug)]
pub struct LockValidationReason {
    pub code: LockValidationReasonCode,
    pub package: Option<PackageName>,
    pub message: Option<String>,
    pub expected: Option<LockValidationValues>,
    pub actual: Option<LockValidationValues>,
}

/// Values retained until the command chooses a diagnostic output format.
#[derive(Debug)]
pub enum LockValidationValues {
    Strings(Vec<String>),
    BuildConstraints(Vec<NameRequirementSpecification>),
}

impl LockValidationReason {
    pub(crate) fn new(code: LockValidationReasonCode) -> Self {
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

    pub(crate) fn values(
        mut self,
        expected: impl IntoIterator<Item = impl Display>,
        actual: impl IntoIterator<Item = impl Display>,
    ) -> Self {
        self.expected = Some(LockValidationValues::Strings(
            expected
                .into_iter()
                .map(|value| value.to_string())
                .collect(),
        ));
        self.actual = Some(LockValidationValues::Strings(
            actual.into_iter().map(|value| value.to_string()).collect(),
        ));
        self
    }

    pub(crate) fn exclude_newer(change: &ExcludeNewerChange) -> Self {
        let mut reason = Self::new(LockValidationReasonCode::ExcludeNewerChanged);
        reason.message = Some(change.to_string());
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

    pub(crate) fn from_satisfies(result: &SatisfiesResult<'_>) -> Option<Self> {
        Some(match result {
            SatisfiesResult::Satisfied => return None,
            SatisfiesResult::MismatchedMembers(expected, actual) => {
                Self::new(LockValidationReasonCode::MembersChanged).values(expected, *actual)
            }
            SatisfiesResult::MismatchedMemberDefaultGroups(..) => {
                Self::new(LockValidationReasonCode::MemberDefaultGroupsChanged)
            }
            SatisfiesResult::MismatchedWorkspaceGroupMetadata(..) => {
                Self::new(LockValidationReasonCode::WorkspaceGroupMetadataChanged)
            }
            SatisfiesResult::MismatchedWorkspaceDefaultGroups(..) => {
                Self::new(LockValidationReasonCode::WorkspaceDefaultGroupsChanged)
            }
            SatisfiesResult::MismatchedMemberGroupMetadata(..) => {
                Self::new(LockValidationReasonCode::MemberGroupMetadataChanged)
            }
            SatisfiesResult::MismatchedVirtual(package, _) => {
                Self::new(LockValidationReasonCode::VirtualChanged).package(package)
            }
            SatisfiesResult::MismatchedEditable(package, _) => {
                Self::new(LockValidationReasonCode::EditableChanged).package(package)
            }
            SatisfiesResult::MismatchedDynamic(package, _) => {
                Self::new(LockValidationReasonCode::DynamicChanged).package(package)
            }
            SatisfiesResult::MismatchedVersion(package, locked, current) => {
                Self::new(LockValidationReasonCode::VersionChanged)
                    .package(package)
                    .values(current, [locked])
            }
            SatisfiesResult::MismatchedRequirements(expected, actual) => {
                Self::new(LockValidationReasonCode::RequirementsChanged).values(expected, actual)
            }
            SatisfiesResult::MismatchedConstraints(expected, actual) => {
                Self::new(LockValidationReasonCode::ConstraintsChanged).values(expected, actual)
            }
            SatisfiesResult::MismatchedOverrides(..) => {
                Self::new(LockValidationReasonCode::OverridesChanged)
            }
            SatisfiesResult::MismatchedExcludes(..) => {
                Self::new(LockValidationReasonCode::ExcludesChanged)
            }
            SatisfiesResult::MismatchedBuildConstraints(expected, actual) => {
                let mut reason = Self::new(LockValidationReasonCode::BuildConstraintsChanged);
                reason.expected = Some(LockValidationValues::BuildConstraints(
                    expected.iter().cloned().collect(),
                ));
                reason.actual = Some(LockValidationValues::BuildConstraints(
                    actual.iter().cloned().collect(),
                ));
                reason
            }
            SatisfiesResult::MismatchedDependencyGroups(..) => {
                Self::new(LockValidationReasonCode::DependencyGroupsChanged)
            }
            SatisfiesResult::MismatchedStaticMetadata(..) => {
                Self::new(LockValidationReasonCode::StaticMetadataChanged)
            }
            SatisfiesResult::MissingRoot(package) => {
                Self::new(LockValidationReasonCode::MissingRoot).package(package)
            }
            SatisfiesResult::MissingRemoteIndex(package, ..) => {
                Self::new(LockValidationReasonCode::MissingRemoteIndex).package(package)
            }
            SatisfiesResult::MissingLocalIndex(package, ..) => {
                Self::new(LockValidationReasonCode::MissingLocalIndex).package(package)
            }
            SatisfiesResult::MismatchedPackageRequirements(package, _, expected, actual) => {
                Self::new(LockValidationReasonCode::PackageRequirementsChanged)
                    .package(package)
                    .values(expected, actual)
            }
            SatisfiesResult::MismatchedPackageDependencies(package, ..) => {
                Self::new(LockValidationReasonCode::PackageDependenciesChanged).package(package)
            }
            SatisfiesResult::MismatchedPackageDependencyGroups(package, ..) => {
                Self::new(LockValidationReasonCode::PackageDependencyGroupsChanged).package(package)
            }
            SatisfiesResult::MismatchedPackageProvidesExtra(package, _, expected, actual) => {
                Self::new(LockValidationReasonCode::PackageExtrasChanged)
                    .package(package)
                    .values(expected, actual)
            }
            SatisfiesResult::MissingVersion(package) => {
                Self::new(LockValidationReasonCode::MissingVersion).package(package)
            }
        })
    }
}

/// The kind of mismatch found while validating an existing lockfile.
#[derive(Debug, Clone, Copy)]
pub enum LockValidationReasonCode {
    MissingLockfile,
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
