use serde::{Deserialize, Serialize};
use uv_pep440::{EncodedVersion, EncodedVersionRanges};

use super::budget::{CaptureLimits, CaptureUsage};

/// A validated, invocation-local request. It contains no filesystem destination.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureToken {
    request: String,
    producer_pid: u32,
}

impl CaptureToken {
    pub fn new(request: &str, producer_pid: u32) -> Option<Self> {
        (valid_request(request) && producer_pid != 0).then(|| Self {
            request: request.to_owned(),
            producer_pid,
        })
    }

    pub(super) fn request(&self) -> &str {
        &self.request
    }

    pub(super) const fn producer_pid(&self) -> u32 {
        self.producer_pid
    }

    pub fn for_lock(
        self,
        scope: CaptureScope,
        operation: CaptureOperation,
        metadata: CaptureMetadata,
    ) -> CaptureOptions {
        CaptureOptions {
            token: self,
            scope,
            operation,
            metadata,
        }
    }
}

pub(super) fn valid_request(request: &str) -> bool {
    request.len() == 32
        && request
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug)]
pub struct CaptureOptions {
    pub(super) token: CaptureToken,
    pub(super) scope: CaptureScope,
    pub(super) operation: CaptureOperation,
    pub(super) metadata: CaptureMetadata,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureScope {
    Workspace,
    Script,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureOperation {
    Write,
    DryRun,
    Locked,
    Frozen,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMetadata {
    Standard,
    WithoutMetadata,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CaptureCommand {
    Lock,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CaptureTerminal {
    NoSolution,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CaptureStatus {
    Complete,
    Truncated,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CaptureReason {
    DerivationNodes,
    Packages,
    Terms,
    Intervals,
    MarkerNodes,
    MarkerEdges,
    AvailabilityEntries,
    VersionComponents,
    AtomBytes,
    TextBytes,
    Work,
    JsonBytes,
    UnsupportedVersion,
    UnsupportedRange,
    UnsupportedMarkerIn,
    UnsupportedMarkerContains,
    UnsupportedMarkerList,
    UnsupportedMarkerExtra,
    SpecificEnvironment,
    InvalidNativeGraph,
}

/// Private transport representation. Deserialization is preceded by the schema-aware preflight.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EvidenceWire {
    pub schema: u32,
    pub request: String,
    pub producer_pid: u32,
    pub command: CaptureCommand,
    pub scope: CaptureScope,
    pub operation: CaptureOperation,
    pub metadata: CaptureMetadata,
    pub terminal: CaptureTerminal,
    pub status: CaptureStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<CaptureReason>,
    pub limits: CaptureLimits,
    pub usage: CaptureUsage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<CapturedGraph>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapturedGraph {
    pub root: u32,
    pub root_package: u32,
    pub root_version: EncodedVersion,
    pub nodes: Vec<CapturedNode>,
    pub packages: Vec<CapturedPackage>,
    pub markers: Vec<CapturedMarker>,
    pub workspace_members: Vec<String>,
    pub environment: CapturedEnvironment,
    pub original_python: CapturedPython,
    pub effective_python: CapturedPython,
    pub index_authentication: CapturedIndexAuthentication,
    pub observations: Vec<CapturedObservation>,
}

/// Authentication failures observed for the configured or selected indexes as a whole.
///
/// These flags do not attribute a failure to an individual package or establish causality.
#[derive(Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapturedIndexAuthentication {
    pub unauthorized: bool,
    pub forbidden: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum CapturedNode {
    NotRoot {
        package: u32,
        version: EncodedVersion,
    },
    NoVersions {
        package: u32,
        range: CapturedRange,
    },
    FromDependencyOf {
        package: u32,
        range: CapturedRange,
        dependency: u32,
        dependency_range: CapturedRange,
    },
    Custom {
        package: u32,
        range: CapturedRange,
        reason: CapturedReason,
    },
    Derived {
        cause1: u32,
        cause2: u32,
        terms: Vec<CapturedTerm>,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapturedTerm {
    pub package: u32,
    pub positive: bool,
    pub range: CapturedRange,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapturedRange {
    pub encoded: EncodedVersionRanges,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical: Option<EncodedVersionRanges>,
}

#[derive(Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum CapturedPackage {
    Root {
        name: Option<String>,
    },
    Python {
        kind: CapturedPythonKind,
    },
    System {
        name: String,
    },
    Package {
        name: String,
        extra: Option<String>,
        group: Option<String>,
        marker: u32,
    },
    Extra {
        name: String,
        extra: String,
        marker: u32,
    },
    Group {
        name: String,
        group: String,
        marker: u32,
    },
    Marker {
        name: String,
        marker: u32,
    },
}

impl CapturedPackage {
    pub(super) fn name(&self) -> Option<&str> {
        match self {
            Self::Root { name } => name.as_deref(),
            Self::Python { .. } => None,
            Self::System { name }
            | Self::Package { name, .. }
            | Self::Extra { name, .. }
            | Self::Group { name, .. }
            | Self::Marker { name, .. } => Some(name),
        }
    }

    pub(super) fn marker(&self) -> Option<u32> {
        match self {
            Self::Root { .. } | Self::Python { .. } | Self::System { .. } => None,
            Self::Package { marker, .. }
            | Self::Extra { marker, .. }
            | Self::Group { marker, .. }
            | Self::Marker { marker, .. } => Some(*marker),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CapturedPythonKind {
    Installed,
    Target,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum CapturedMarker {
    True,
    False,
    Version {
        key: VersionMarkerKey,
        edges: Vec<VersionMarkerEdge>,
    },
    String {
        key: StringMarkerKey,
        edges: Vec<StringMarkerEdge>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum VersionMarkerKey {
    ImplementationVersion,
    PythonFullVersion,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StringMarkerKey {
    OsName,
    SysPlatform,
    PlatformSystem,
    PlatformMachine,
    PlatformPythonImplementation,
    PlatformRelease,
    PlatformVersion,
    ImplementationName,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VersionMarkerEdge {
    pub intervals: EncodedVersionRanges,
    pub child: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StringMarkerEdge {
    pub intervals: Vec<StringInterval>,
    pub child: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StringInterval {
    pub lower: StringBound,
    pub upper: StringBound,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(super) enum StringBound {
    Unbounded,
    Included(String),
    Excluded(String),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapturedEnvironment {
    pub marker: u32,
    pub initial_forks: Vec<u32>,
    pub include: Vec<CapturedConflict>,
    pub exclude: Vec<CapturedConflict>,
}

#[derive(Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum CapturedConflict {
    Project { package: String },
    Extra { package: String, extra: String },
    Group { package: String, group: String },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapturedPython {
    pub source: CapturedPythonSource,
    pub exact: EncodedVersion,
    pub installed: CapturedPythonDomain,
    pub target: CapturedPythonDomain,
    pub target_marker: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CapturedPythonSource {
    PythonVersion,
    RequiresPython,
    Interpreter,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapturedPythonDomain {
    pub lower: VersionBound,
    pub upper: VersionBound,
    pub specifiers: Vec<CapturedSpecifier>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    content = "version",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(super) enum VersionBound {
    Unbounded,
    Included(EncodedVersion),
    Excluded(EncodedVersion),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapturedSpecifier {
    pub operator: CapturedOperator,
    pub version: EncodedVersion,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CapturedOperator {
    Equal,
    EqualStar,
    ExactEqual,
    NotEqual,
    NotEqualStar,
    TildeEqual,
    LessThan,
    LessThanEqual,
    GreaterThan,
    GreaterThanEqual,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapturedObservation {
    pub name: String,
    pub source: CapturedSource,
    pub has_url_policy: bool,
    pub has_index_policy: bool,
    pub listing: CapturedListing,
    pub listed_versions: Vec<EncodedVersion>,
    pub known_versions: Vec<EncodedVersion>,
    pub unavailable: Option<CapturedReason>,
    pub incomplete: Vec<CapturedMetadataFact>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CapturedSource {
    Registry,
    ExplicitIndex,
    Archive,
    Path,
    Directory,
    GitDirectory,
    GitPath,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CapturedListing {
    Found,
    NotFound,
    NoIndex,
    Offline,
    Unobserved,
    NotApplicable,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapturedMetadataFact {
    pub version: EncodedVersion,
    pub reason: CapturedReason,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapturedReason {
    pub kind: CapturedReasonKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CapturedReasonKind {
    PackageNoIndex,
    PackageOffline,
    PackageNotFound,
    PackageInvalidMetadata,
    PackageInvalidStructure,
    PackageNetwork,
    VersionUnsatisfiableDependency,
    VersionIncompatibleSelfDependency,
    VersionIncompatibleDist,
    VersionInvalidMetadata,
    VersionInconsistentMetadata,
    VersionInvalidStructure,
    VersionOffline,
    VersionRequiresPython,
    VersionNetwork,
    MetadataOffline,
    MetadataInvalidMetadata,
    MetadataInconsistentMetadata,
    MetadataInvalidStructure,
    MetadataRequiresPython,
    MetadataNetwork,
}
