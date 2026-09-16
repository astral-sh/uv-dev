//! Allocation preflight for the private version-1 wire schema.
//!
//! The first pass bounds and validates decoded JSON strings without materializing them. The
//! second pass uses non-collecting Serde visitors for the exact schema. Only after both passes
//! succeed may the private wire types allocate their vectors and owned atoms.

use std::fmt;

use serde::Deserializer;
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};

use super::{CaptureReadError, ReadErrorKind, usage_within};
use crate::no_solution_capture::budget::{Budget, CaptureLimits, CaptureUsage, Resource, Stop};
use crate::no_solution_capture::wire::{CaptureReason, CaptureToken, valid_request};

#[cfg(test)]
mod tests;

const MAX_DEPTH: usize = 32;
const MAX_FIELDS: usize = 16;
const JSON_BYTES_PER_WORK: usize = 16;

pub(super) fn check(
    bytes: &[u8],
    token: &CaptureToken,
    limits: CaptureLimits,
) -> Result<Budget, CaptureReadError> {
    if !limits.is_supported() {
        return Err(CaptureReadError(ReadErrorKind::InvalidLimits));
    }
    if bytes.len() > limits.json_bytes {
        return Err(CaptureReadError(ReadErrorKind::Limit(
            CaptureReason::JsonBytes,
        )));
    }
    scan_strings(bytes)?;
    let mut state = State {
        budget: Budget::new(limits),
        token,
        json_bytes: bytes.len(),
        rejection: None,
    };
    state
        .budget
        .work(bytes.len().div_ceil(JSON_BYTES_PER_WORK))?;
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let result = Seed {
        state: &mut state,
        shape: Shape::new(Kind::Object(ObjectKind::Envelope)),
        depth: 0,
    }
    .deserialize(&mut deserializer)
    .and_then(|_| deserializer.end());
    if result.is_err() {
        return Err(CaptureReadError(
            state.rejection.unwrap_or(ReadErrorKind::InvalidJson),
        ));
    }
    Ok(state.budget)
}

/// Bound Serde JSON's escape scratch before asking it to decode any string, including keys and
/// externally tagged enum names. Structural and numeric JSON grammar is checked by the schema
/// visitor in the next pass.
fn scan_strings(bytes: &[u8]) -> Result<(), CaptureReadError> {
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] != b'"' {
            cursor += 1;
            continue;
        }
        cursor += 1;
        let mut run = cursor;
        let mut decoded = 0_usize;
        loop {
            let Some(&byte) = bytes.get(cursor) else {
                return Err(CaptureReadError(ReadErrorKind::InvalidJson));
            };
            match byte {
                b'"' | b'\\' => {
                    if std::str::from_utf8(&bytes[run..cursor]).is_err() {
                        return Err(CaptureReadError(ReadErrorKind::InvalidJson));
                    }
                    decoded = decoded.saturating_add(cursor - run);
                    if byte == b'"' {
                        cursor += 1;
                        break;
                    }
                    cursor += 1;
                    let Some(&escape) = bytes.get(cursor) else {
                        return Err(CaptureReadError(ReadErrorKind::InvalidJson));
                    };
                    cursor += 1;
                    decoded = decoded.saturating_add(match escape {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => 1,
                        b'u' => {
                            let first = hex_quad(bytes, &mut cursor)?;
                            let scalar = if (0xd800..=0xdbff).contains(&first) {
                                if bytes.get(cursor..cursor + 2) != Some(b"\\u") {
                                    return Err(CaptureReadError(ReadErrorKind::InvalidJson));
                                }
                                cursor += 2;
                                let second = hex_quad(bytes, &mut cursor)?;
                                if !(0xdc00..=0xdfff).contains(&second) {
                                    return Err(CaptureReadError(ReadErrorKind::InvalidJson));
                                }
                                0x1_0000
                                    + ((u32::from(first) - 0xd800) << 10)
                                    + (u32::from(second) - 0xdc00)
                            } else {
                                u32::from(first)
                            };
                            let Some(character) = char::from_u32(scalar) else {
                                return Err(CaptureReadError(ReadErrorKind::InvalidJson));
                            };
                            character.len_utf8()
                        }
                        _ => return Err(CaptureReadError(ReadErrorKind::InvalidJson)),
                    });
                    run = cursor;
                }
                0..=0x1f => return Err(CaptureReadError(ReadErrorKind::InvalidJson)),
                _ => cursor += 1,
            }
            if decoded.saturating_add(cursor - run) > CaptureLimits::V1.atom_bytes {
                return Err(CaptureReadError(ReadErrorKind::Limit(
                    CaptureReason::AtomBytes,
                )));
            }
        }
        if decoded > CaptureLimits::V1.atom_bytes {
            return Err(CaptureReadError(ReadErrorKind::Limit(
                CaptureReason::AtomBytes,
            )));
        }
    }
    Ok(())
}

fn hex_quad(bytes: &[u8], cursor: &mut usize) -> Result<u16, CaptureReadError> {
    let mut value = 0_u16;
    for _ in 0..4 {
        let Some(&byte) = bytes.get(*cursor) else {
            return Err(CaptureReadError(ReadErrorKind::InvalidJson));
        };
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return Err(CaptureReadError(ReadErrorKind::InvalidJson)),
        };
        value = value * 16 + u16::from(digit);
        *cursor += 1;
    }
    Ok(value)
}

struct State<'token> {
    budget: Budget,
    token: &'token CaptureToken,
    json_bytes: usize,
    rejection: Option<ReadErrorKind>,
}

impl State<'_> {
    fn reject<E: de::Error>(&mut self, reason: ReadErrorKind) -> E {
        self.rejection.get_or_insert(reason);
        E::custom("invalid internal resolver capture")
    }

    fn stop<E: de::Error>(&mut self, stop: Stop) -> E {
        self.reject(ReadErrorKind::Limit(stop.reason))
    }

    fn work<E: de::Error>(&mut self, amount: usize) -> Result<(), E> {
        self.budget.work(amount).map_err(|stop| self.stop(stop))
    }

    fn charge<E: de::Error>(&mut self, resource: Resource, amount: usize) -> Result<(), E> {
        self.budget
            .charge(resource, amount)
            .map_err(|stop| self.stop(stop))
    }

    fn atom<E: de::Error>(&mut self, value: &str) -> Result<(), E> {
        self.budget
            .atom(value.len())
            .map_err(|stop| self.stop(stop))
    }
}

#[derive(Clone, Copy)]
struct Shape {
    kind: Kind,
    nullable: bool,
}

impl Shape {
    const fn new(kind: Kind) -> Self {
        Self {
            kind,
            nullable: false,
        }
    }

    const fn nullable(kind: Kind) -> Self {
        Self {
            kind,
            nullable: true,
        }
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Usize,
    U32,
    U16,
    Bool,
    Atom,
    LocalAtom,
    Decimal,
    Request,
    Enum(EnumKind),
    Object(ObjectKind),
    Array(ArrayKind),
    External(ExternalKind),
}

#[derive(Clone, Copy)]
enum EnumKind {
    Command,
    Scope,
    Operation,
    Metadata,
    Terminal,
    Status,
    CaptureReason,
    PythonKind,
    VersionMarkerKey,
    StringMarkerKey,
    Bound,
    Pre,
    LocalVersion,
    LocalSegment,
    PythonSource,
    Operator,
    Source,
    Listing,
    Reason,
}

impl EnumKind {
    const fn tags(self) -> &'static [&'static str] {
        match self {
            Self::Command => &["lock"],
            Self::Scope => &["workspace", "script"],
            Self::Operation => &["write", "dry_run", "locked", "frozen"],
            Self::Metadata => &["standard", "without_metadata"],
            Self::Terminal => &["no_solution"],
            Self::Status => &["complete", "truncated", "unsupported"],
            Self::CaptureReason => &[
                "derivation_nodes",
                "packages",
                "terms",
                "intervals",
                "marker_nodes",
                "marker_edges",
                "availability_entries",
                "version_components",
                "atom_bytes",
                "text_bytes",
                "work",
                "json_bytes",
                "unsupported_version",
                "unsupported_range",
                "unsupported_marker_in",
                "unsupported_marker_contains",
                "unsupported_marker_list",
                "unsupported_marker_extra",
                "specific_environment",
                "invalid_native_graph",
            ],
            Self::PythonKind => &["installed", "target"],
            Self::VersionMarkerKey => &["implementation_version", "python_full_version"],
            Self::StringMarkerKey => &[
                "os_name",
                "sys_platform",
                "platform_system",
                "platform_machine",
                "platform_python_implementation",
                "platform_release",
                "platform_version",
                "implementation_name",
            ],
            Self::Bound => &["unbounded", "included", "excluded"],
            Self::Pre => &["alpha", "beta", "rc"],
            Self::LocalVersion => &["segments", "max"],
            Self::LocalSegment => &["string", "number"],
            Self::PythonSource => &["python_version", "requires_python", "interpreter"],
            Self::Operator => &[
                "equal",
                "equal_star",
                "exact_equal",
                "not_equal",
                "not_equal_star",
                "tilde_equal",
                "less_than",
                "less_than_equal",
                "greater_than",
                "greater_than_equal",
            ],
            Self::Source => &[
                "registry",
                "explicit_index",
                "archive",
                "path",
                "directory",
                "git_directory",
                "git_path",
            ],
            Self::Listing => &[
                "found",
                "not_found",
                "no_index",
                "offline",
                "unobserved",
                "not_applicable",
            ],
            Self::Reason => &[
                "package_no_index",
                "package_offline",
                "package_not_found",
                "package_invalid_metadata",
                "package_invalid_structure",
                "package_network",
                "version_unsatisfiable_dependency",
                "version_incompatible_self_dependency",
                "version_incompatible_dist",
                "version_invalid_metadata",
                "version_inconsistent_metadata",
                "version_invalid_structure",
                "version_offline",
                "version_requires_python",
                "version_network",
                "metadata_offline",
                "metadata_invalid_metadata",
                "metadata_inconsistent_metadata",
                "metadata_invalid_structure",
                "metadata_requires_python",
                "metadata_network",
            ],
        }
    }
}

#[derive(Clone, Copy)]
enum ArrayKind {
    Nodes,
    Packages,
    Markers,
    Terms,
    EncodedIntervals,
    StringIntervals,
    VersionEdges,
    StringEdges,
    WorkspaceMembers,
    InitialForks,
    Conflicts,
    Specifiers,
    Observations,
    AvailableVersions,
    Incomplete,
    Release,
    LocalSegments,
}

impl ArrayKind {
    const fn item(self) -> Kind {
        match self {
            Self::Nodes => Kind::External(ExternalKind::Node),
            Self::Packages => Kind::External(ExternalKind::Package),
            Self::Markers => Kind::External(ExternalKind::Marker),
            Self::Terms => Kind::Object(ObjectKind::Term),
            Self::EncodedIntervals => Kind::Object(ObjectKind::EncodedInterval),
            Self::StringIntervals => Kind::Object(ObjectKind::StringInterval),
            Self::VersionEdges => Kind::Object(ObjectKind::VersionEdge),
            Self::StringEdges => Kind::Object(ObjectKind::StringEdge),
            Self::WorkspaceMembers => Kind::Atom,
            Self::InitialForks => Kind::U32,
            Self::Conflicts => Kind::External(ExternalKind::Conflict),
            Self::Specifiers => Kind::Object(ObjectKind::Specifier),
            Self::Observations => Kind::Object(ObjectKind::Observation),
            Self::AvailableVersions => Kind::Object(ObjectKind::Version),
            Self::Incomplete => Kind::Object(ObjectKind::MetadataFact),
            Self::Release => Kind::Decimal,
            Self::LocalSegments => Kind::Object(ObjectKind::LocalSegment),
        }
    }

    fn charge<E: de::Error>(self, state: &mut State<'_>, index: usize) -> Result<(), E> {
        match self {
            Self::Nodes => {
                state.charge(Resource::DerivationNodes, 1)?;
                state.work(1)
            }
            Self::Packages => {
                state.charge(Resource::Packages, 1)?;
                state.work(1)
            }
            Self::Markers => {
                state.charge(Resource::MarkerNodes, 1)?;
                state.work(1)
            }
            Self::Terms => state.charge(Resource::Terms, 1),
            Self::EncodedIntervals | Self::StringIntervals => state.charge(Resource::Intervals, 1),
            Self::VersionEdges | Self::StringEdges => state.charge(Resource::MarkerEdges, 1),
            Self::Observations | Self::AvailableVersions | Self::Incomplete => {
                state.charge(Resource::AvailabilityEntries, 1)
            }
            Self::Release | Self::LocalSegments => {
                state
                    .budget
                    .check_components(index.saturating_add(1))
                    .map_err(|stop| state.stop(stop))?;
                state.work(1)
            }
            Self::InitialForks => state.work(1),
            Self::WorkspaceMembers | Self::Conflicts | Self::Specifiers => Ok(()),
        }
    }
}

#[derive(Clone, Copy)]
enum ExternalKind {
    Node,
    Package,
    Marker,
    Conflict,
}

impl ExternalKind {
    const fn variants(self) -> &'static [(&'static str, ObjectKind)] {
        match self {
            Self::Node => &[
                ("not_root", ObjectKind::NotRoot),
                ("no_versions", ObjectKind::NoVersions),
                ("from_dependency_of", ObjectKind::FromDependencyOf),
                ("custom", ObjectKind::Custom),
                ("derived", ObjectKind::Derived),
            ],
            Self::Package => &[
                ("root", ObjectKind::RootPackage),
                ("python", ObjectKind::PythonPackage),
                ("system", ObjectKind::SystemPackage),
                ("package", ObjectKind::Package),
                ("extra", ObjectKind::ExtraPackage),
                ("group", ObjectKind::GroupPackage),
                ("marker", ObjectKind::MarkerPackage),
            ],
            Self::Marker => &[
                ("version", ObjectKind::VersionMarker),
                ("string", ObjectKind::StringMarker),
            ],
            Self::Conflict => &[
                ("project", ObjectKind::ProjectConflict),
                ("extra", ObjectKind::ExtraConflict),
                ("group", ObjectKind::GroupConflict),
            ],
        }
    }
}

#[derive(Clone, Copy)]
enum ObjectKind {
    Envelope,
    Limits,
    Usage,
    Graph,
    NotRoot,
    NoVersions,
    FromDependencyOf,
    Custom,
    Derived,
    Term,
    Range,
    Version,
    Prerelease,
    LocalVersion,
    LocalSegment,
    EncodedInterval,
    VersionBound,
    RootPackage,
    PythonPackage,
    SystemPackage,
    Package,
    ExtraPackage,
    GroupPackage,
    MarkerPackage,
    VersionMarker,
    StringMarker,
    VersionEdge,
    StringEdge,
    StringInterval,
    StringBound,
    Environment,
    ProjectConflict,
    ExtraConflict,
    GroupConflict,
    Python,
    PythonDomain,
    Specifier,
    Observation,
    MetadataFact,
    Reason,
}

#[derive(Clone, Copy)]
struct Field {
    name: &'static str,
    shape: Shape,
    required: bool,
}

impl Field {
    const fn required(name: &'static str, kind: Kind) -> Self {
        Self {
            name,
            shape: Shape::new(kind),
            required: true,
        }
    }

    const fn nullable(name: &'static str, kind: Kind) -> Self {
        Self {
            name,
            shape: Shape::nullable(kind),
            required: true,
        }
    }

    const fn optional(name: &'static str, kind: Kind) -> Self {
        Self {
            name,
            shape: Shape::new(kind),
            required: false,
        }
    }

    const fn optional_nullable(name: &'static str, kind: Kind) -> Self {
        Self {
            name,
            shape: Shape::nullable(kind),
            required: false,
        }
    }
}

macro_rules! fields {
    ($($field:expr),* $(,)?) => {
        const { &[$($field),*] }
    };
}

impl ObjectKind {
    const fn fields(self) -> &'static [Field] {
        use ArrayKind as A;
        use EnumKind as E;
        use Kind::{Array, Atom, Bool, Decimal, Enum, LocalAtom, Object, Request, U16, U32, Usize};
        use ObjectKind as O;
        match self {
            Self::Envelope => fields![
                Field::required("schema", U32),
                Field::required("request", Request),
                Field::required("producer_pid", U32),
                Field::required("command", Enum(E::Command)),
                Field::required("scope", Enum(E::Scope)),
                Field::required("operation", Enum(E::Operation)),
                Field::required("metadata", Enum(E::Metadata)),
                Field::required("terminal", Enum(E::Terminal)),
                Field::required("status", Enum(E::Status)),
                Field::optional_nullable("reason", Enum(E::CaptureReason)),
                Field::required("limits", Object(O::Limits)),
                Field::required("usage", Object(O::Usage)),
                Field::optional_nullable("graph", Object(O::Graph)),
            ],
            Self::Limits => fields![
                Field::required("derivation_nodes", Usize),
                Field::required("packages", Usize),
                Field::required("terms", Usize),
                Field::required("intervals", Usize),
                Field::required("marker_nodes", Usize),
                Field::required("marker_edges", Usize),
                Field::required("availability_entries", Usize),
                Field::required("version_components", Usize),
                Field::required("atom_bytes", Usize),
                Field::required("text_bytes", Usize),
                Field::required("work", Usize),
                Field::required("json_bytes", Usize),
            ],
            Self::Usage => fields![
                Field::required("derivation_nodes", Usize),
                Field::required("packages", Usize),
                Field::required("terms", Usize),
                Field::required("intervals", Usize),
                Field::required("marker_nodes", Usize),
                Field::required("marker_edges", Usize),
                Field::required("availability_entries", Usize),
                Field::required("max_version_components", Usize),
                Field::required("max_atom_bytes", Usize),
                Field::required("text_bytes", Usize),
                Field::required("work", Usize),
            ],
            Self::Graph => fields![
                Field::required("root", U32),
                Field::required("root_package", U32),
                Field::required("root_version", Object(O::Version)),
                Field::required("nodes", Array(A::Nodes)),
                Field::required("packages", Array(A::Packages)),
                Field::required("markers", Array(A::Markers)),
                Field::required("workspace_members", Array(A::WorkspaceMembers)),
                Field::required("environment", Object(O::Environment)),
                Field::required("original_python", Object(O::Python)),
                Field::required("effective_python", Object(O::Python)),
                Field::required("observations", Array(A::Observations)),
            ],
            Self::NotRoot => fields![
                Field::required("package", U32),
                Field::required("version", Object(O::Version)),
            ],
            Self::NoVersions => fields![
                Field::required("package", U32),
                Field::required("range", Object(O::Range)),
            ],
            Self::FromDependencyOf => fields![
                Field::required("package", U32),
                Field::required("range", Object(O::Range)),
                Field::required("dependency", U32),
                Field::required("dependency_range", Object(O::Range)),
            ],
            Self::Custom => fields![
                Field::required("package", U32),
                Field::required("range", Object(O::Range)),
                Field::required("reason", Object(O::Reason)),
            ],
            Self::Derived => fields![
                Field::required("cause1", U32),
                Field::required("cause2", U32),
                Field::required("terms", Array(A::Terms)),
            ],
            Self::Term => fields![
                Field::required("package", U32),
                Field::required("positive", Bool),
                Field::required("range", Object(O::Range)),
            ],
            Self::Range => fields![
                Field::required("encoded", Array(A::EncodedIntervals)),
                Field::optional_nullable("logical", Array(A::EncodedIntervals)),
            ],
            Self::Version => fields![
                Field::required("epoch", Decimal),
                Field::required("release", Array(A::Release)),
                Field::optional_nullable("pre", Object(O::Prerelease)),
                Field::optional_nullable("post", Decimal),
                Field::optional_nullable("dev", Decimal),
                Field::required("local", Object(O::LocalVersion)),
                Field::optional_nullable("min", Decimal),
                Field::optional_nullable("max", Decimal),
            ],
            Self::Prerelease => fields![
                Field::required("kind", Enum(E::Pre)),
                Field::required("number", Decimal),
            ],
            Self::LocalVersion => fields![
                Field::required("kind", Enum(E::LocalVersion)),
                Field::optional("segments", Array(A::LocalSegments)),
            ],
            Self::LocalSegment => fields![
                Field::required("kind", Enum(E::LocalSegment)),
                Field::required("value", LocalAtom),
            ],
            Self::EncodedInterval => fields![
                Field::required("lower", Object(O::VersionBound)),
                Field::required("upper", Object(O::VersionBound)),
            ],
            Self::VersionBound => fields![
                Field::required("kind", Enum(E::Bound)),
                Field::optional("version", Object(O::Version)),
            ],
            Self::RootPackage => fields![Field::nullable("name", Atom)],
            Self::PythonPackage => fields![Field::required("kind", Enum(E::PythonKind))],
            Self::SystemPackage => fields![Field::required("name", Atom)],
            Self::Package => fields![
                Field::required("name", Atom),
                Field::nullable("extra", Atom),
                Field::nullable("group", Atom),
                Field::required("marker", U32),
            ],
            Self::ExtraPackage => fields![
                Field::required("name", Atom),
                Field::required("extra", Atom),
                Field::required("marker", U32),
            ],
            Self::GroupPackage => fields![
                Field::required("name", Atom),
                Field::required("group", Atom),
                Field::required("marker", U32),
            ],
            Self::MarkerPackage => fields![
                Field::required("name", Atom),
                Field::required("marker", U32),
            ],
            Self::VersionMarker => fields![
                Field::required("key", Enum(E::VersionMarkerKey)),
                Field::required("edges", Array(A::VersionEdges)),
            ],
            Self::StringMarker => fields![
                Field::required("key", Enum(E::StringMarkerKey)),
                Field::required("edges", Array(A::StringEdges)),
            ],
            Self::VersionEdge => fields![
                Field::required("intervals", Array(A::EncodedIntervals)),
                Field::required("child", U32),
            ],
            Self::StringEdge => fields![
                Field::required("intervals", Array(A::StringIntervals)),
                Field::required("child", U32),
            ],
            Self::StringInterval => fields![
                Field::required("lower", Object(O::StringBound)),
                Field::required("upper", Object(O::StringBound)),
            ],
            Self::StringBound => fields![
                Field::required("kind", Enum(E::Bound)),
                Field::optional("value", Atom),
            ],
            Self::Environment => fields![
                Field::required("marker", U32),
                Field::required("initial_forks", Array(A::InitialForks)),
                Field::required("include", Array(A::Conflicts)),
                Field::required("exclude", Array(A::Conflicts)),
            ],
            Self::ProjectConflict => fields![Field::required("package", Atom)],
            Self::ExtraConflict => fields![
                Field::required("package", Atom),
                Field::required("extra", Atom),
            ],
            Self::GroupConflict => fields![
                Field::required("package", Atom),
                Field::required("group", Atom),
            ],
            Self::Python => fields![
                Field::required("source", Enum(E::PythonSource)),
                Field::required("exact", Object(O::Version)),
                Field::required("installed", Object(O::PythonDomain)),
                Field::required("target", Object(O::PythonDomain)),
                Field::required("target_marker", U32),
            ],
            Self::PythonDomain => fields![
                Field::required("lower", Object(O::VersionBound)),
                Field::required("upper", Object(O::VersionBound)),
                Field::required("specifiers", Array(A::Specifiers)),
            ],
            Self::Specifier => fields![
                Field::required("operator", Enum(E::Operator)),
                Field::required("version", Object(O::Version)),
            ],
            Self::Observation => fields![
                Field::required("name", Atom),
                Field::required("source", Enum(E::Source)),
                Field::required("has_url_policy", Bool),
                Field::required("has_index_policy", Bool),
                Field::required("listing", Enum(E::Listing)),
                Field::required("listed_versions", Array(A::AvailableVersions)),
                Field::required("known_versions", Array(A::AvailableVersions)),
                Field::nullable("unavailable", Object(O::Reason)),
                Field::required("incomplete", Array(A::Incomplete)),
            ],
            Self::MetadataFact => fields![
                Field::required("version", Object(O::Version)),
                Field::required("reason", Object(O::Reason)),
            ],
            Self::Reason => fields![
                Field::required("kind", Enum(E::Reason)),
                Field::optional_nullable("http_status", U16),
            ],
        }
    }

    fn charge<E: de::Error>(self, state: &mut State<'_>) -> Result<(), E> {
        match self {
            Self::PythonDomain => {
                state.charge(Resource::Intervals, 1)?;
                state.work(1)
            }
            Self::NotRoot
            | Self::NoVersions
            | Self::FromDependencyOf
            | Self::Custom
            | Self::Derived
            | Self::Term
            | Self::Range
            | Self::Version
            | Self::EncodedInterval
            | Self::VersionMarker
            | Self::StringMarker
            | Self::VersionEdge
            | Self::StringEdge
            | Self::StringInterval
            | Self::ProjectConflict
            | Self::ExtraConflict
            | Self::GroupConflict
            | Self::Specifier
            | Self::Observation
            | Self::MetadataFact => state.work(1),
            Self::Envelope
            | Self::Limits
            | Self::Usage
            | Self::Graph
            | Self::Prerelease
            | Self::LocalVersion
            | Self::LocalSegment
            | Self::VersionBound
            | Self::RootPackage
            | Self::PythonPackage
            | Self::SystemPackage
            | Self::Package
            | Self::ExtraPackage
            | Self::GroupPackage
            | Self::MarkerPackage
            | Self::StringBound
            | Self::Environment
            | Self::Python
            | Self::Reason => Ok(()),
        }
    }
}

#[derive(Clone, Copy)]
enum Value {
    Missing,
    Null,
    Unit,
    Number(u64),
    Tag(usize),
    Count(usize),
    LocalAtom {
        decimal: bool,
        normalized_string: bool,
    },
    LocalEmpty(bool),
    Limits(CaptureLimits),
    Usage(CaptureUsage),
}

impl Value {
    const fn present(self) -> bool {
        !matches!(self, Self::Missing | Self::Null)
    }

    const fn number(self) -> Option<u64> {
        match self {
            Self::Number(value) => Some(value),
            _ => None,
        }
    }
}

struct Seed<'state, 'token> {
    state: &'state mut State<'token>,
    shape: Shape,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for Seed<'_, '_> {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        if self.depth > MAX_DEPTH {
            return Err(self.state.reject(ReadErrorKind::InvalidSchema));
        }
        deserializer.deserialize_any(ShapeVisitor(self))
    }
}

struct ShapeVisitor<'state, 'token>(Seed<'state, 'token>);

impl<'de> Visitor<'de> for ShapeVisitor<'_, '_> {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the bounded internal resolver capture schema")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        if self.0.shape.nullable {
            Ok(Value::Null)
        } else {
            Err(self.0.state.reject(ReadErrorKind::InvalidSchema))
        }
    }

    fn visit_bool<E: de::Error>(self, _value: bool) -> Result<Value, E> {
        if matches!(self.0.shape.kind, Kind::Bool) {
            Ok(Value::Unit)
        } else {
            Err(self.0.state.reject(ReadErrorKind::InvalidSchema))
        }
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Value, E> {
        let valid = match self.0.shape.kind {
            Kind::Usize => usize::try_from(value).is_ok(),
            Kind::U32 => u32::try_from(value).is_ok(),
            Kind::U16 => u16::try_from(value).is_ok(),
            _ => false,
        };
        if valid {
            Ok(Value::Number(value))
        } else {
            Err(self.0.state.reject(ReadErrorKind::InvalidSchema))
        }
    }

    fn visit_i64<E: de::Error>(self, _value: i64) -> Result<Value, E> {
        Err(self.0.state.reject(ReadErrorKind::InvalidSchema))
    }

    fn visit_f64<E: de::Error>(self, _value: f64) -> Result<Value, E> {
        Err(self.0.state.reject(ReadErrorKind::InvalidSchema))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Value, E> {
        let state = self.0.state;
        match self.0.shape.kind {
            Kind::Atom => {
                state.atom(value)?;
                Ok(Value::Unit)
            }
            Kind::LocalAtom => {
                state.atom(value)?;
                Ok(Value::LocalAtom {
                    decimal: decimal(value).is_some(),
                    normalized_string: !value.is_empty()
                        && value
                            .bytes()
                            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
                        && value.parse::<u64>().is_err(),
                })
            }
            Kind::Decimal => {
                let Some(number) = decimal(value) else {
                    return Err(state.reject(ReadErrorKind::InvalidSchema));
                };
                state.atom(value)?;
                Ok(Value::Number(number))
            }
            Kind::Request => {
                if !valid_request(value) || value != state.token.request() {
                    return Err(state.reject(ReadErrorKind::MismatchedRequest));
                }
                Ok(Value::Unit)
            }
            Kind::Enum(kind) => kind
                .tags()
                .iter()
                .position(|tag| *tag == value)
                .map(Value::Tag)
                .ok_or_else(|| state.reject(ReadErrorKind::InvalidSchema)),
            Kind::External(ExternalKind::Marker) if value == "true" || value == "false" => {
                Ok(Value::Unit)
            }
            _ => Err(state.reject(ReadErrorKind::InvalidSchema)),
        }
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Value, A::Error> {
        let Kind::Array(kind) = self.0.shape.kind else {
            return Err(self.0.state.reject(ReadErrorKind::InvalidSchema));
        };
        let mut count = 0;
        while sequence
            .next_element_seed(ItemSeed {
                state: self.0.state,
                kind,
                index: count,
                depth: self.0.depth + 1,
            })?
            .is_some()
        {
            count += 1;
        }
        Ok(Value::Count(count))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        match self.0.shape.kind {
            Kind::Object(kind) => {
                kind.charge(self.0.state)?;
                let fields = kind.fields();
                if fields.len() > MAX_FIELDS {
                    return Err(self.0.state.reject(ReadErrorKind::InvalidSchema));
                }
                let mut values = [Value::Missing; MAX_FIELDS];
                let mut seen = 0_u32;
                while let Some(index) = map.next_key_seed(KeySeed {
                    state: self.0.state,
                    names: Names::Fields(fields),
                })? {
                    let bit = 1_u32 << index;
                    if seen & bit != 0 {
                        return Err(self.0.state.reject(ReadErrorKind::InvalidSchema));
                    }
                    seen |= bit;
                    values[index] = map.next_value_seed(Seed {
                        state: self.0.state,
                        shape: fields[index].shape,
                        depth: self.0.depth + 1,
                    })?;
                }
                for (index, field) in fields.iter().enumerate() {
                    if field.required && seen & (1_u32 << index) == 0 {
                        return Err(self.0.state.reject(ReadErrorKind::InvalidSchema));
                    }
                }
                finish_object(kind, fields, &values, self.0.state)
            }
            Kind::External(kind) => {
                let variants = kind.variants();
                let Some(index) = map.next_key_seed(KeySeed {
                    state: self.0.state,
                    names: Names::Variants(variants),
                })?
                else {
                    return Err(self.0.state.reject(ReadErrorKind::InvalidSchema));
                };
                map.next_value_seed(Seed {
                    state: self.0.state,
                    shape: Shape::new(Kind::Object(variants[index].1)),
                    depth: self.0.depth + 1,
                })?;
                if map.next_key::<de::IgnoredAny>()?.is_some() {
                    return Err(self.0.state.reject(ReadErrorKind::InvalidSchema));
                }
                Ok(Value::Unit)
            }
            _ => Err(self.0.state.reject(ReadErrorKind::InvalidSchema)),
        }
    }
}

struct ItemSeed<'state, 'token> {
    state: &'state mut State<'token>,
    kind: ArrayKind,
    index: usize,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for ItemSeed<'_, '_> {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        self.kind.charge(self.state, self.index)?;
        Seed {
            state: self.state,
            shape: Shape::new(self.kind.item()),
            depth: self.depth,
        }
        .deserialize(deserializer)
    }
}

enum Names {
    Fields(&'static [Field]),
    Variants(&'static [(&'static str, ObjectKind)]),
}

struct KeySeed<'state, 'token> {
    state: &'state mut State<'token>,
    names: Names,
}

impl<'de> DeserializeSeed<'de> for KeySeed<'_, '_> {
    type Value = usize;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<usize, D::Error> {
        deserializer.deserialize_str(self)
    }
}

impl Visitor<'_> for KeySeed<'_, '_> {
    type Value = usize;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a known capture field")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<usize, E> {
        let index = match self.names {
            Names::Fields(fields) => fields.iter().position(|field| field.name == value),
            Names::Variants(variants) => variants.iter().position(|(name, _)| *name == value),
        };
        index.ok_or_else(|| self.state.reject(ReadErrorKind::InvalidSchema))
    }
}

fn decimal(value: &str) -> Option<u64> {
    if value.is_empty()
        || value.len() > 20
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    value.parse().ok()
}

fn field_value(fields: &[Field], values: &[Value; MAX_FIELDS], name: &str) -> Value {
    fields
        .iter()
        .position(|field| field.name == name)
        .map_or(Value::Missing, |index| values[index])
}

fn numeric_fields<const N: usize>(values: &[Value; MAX_FIELDS]) -> Option<[usize; N]> {
    let mut result = [0; N];
    for (index, target) in result.iter_mut().enumerate() {
        *target = usize::try_from(values[index].number()?).ok()?;
    }
    Some(result)
}

fn finish_object<E: de::Error>(
    kind: ObjectKind,
    fields: &[Field],
    values: &[Value; MAX_FIELDS],
    state: &mut State<'_>,
) -> Result<Value, E> {
    let field = |name| field_value(fields, values, name);
    match kind {
        ObjectKind::Limits => {
            let Some(
                [
                    derivation_nodes,
                    packages,
                    terms,
                    intervals,
                    marker_nodes,
                    marker_edges,
                    availability_entries,
                    version_components,
                    atom_bytes,
                    text_bytes,
                    work,
                    json_bytes,
                ],
            ) = numeric_fields(values)
            else {
                return Err(state.reject(ReadErrorKind::InvalidSchema));
            };
            Ok(Value::Limits(CaptureLimits {
                derivation_nodes,
                packages,
                terms,
                intervals,
                marker_nodes,
                marker_edges,
                availability_entries,
                version_components,
                atom_bytes,
                text_bytes,
                work,
                json_bytes,
            }))
        }
        ObjectKind::Usage => {
            let Some(
                [
                    derivation_nodes,
                    packages,
                    terms,
                    intervals,
                    marker_nodes,
                    marker_edges,
                    availability_entries,
                    max_version_components,
                    max_atom_bytes,
                    text_bytes,
                    work,
                ],
            ) = numeric_fields(values)
            else {
                return Err(state.reject(ReadErrorKind::InvalidSchema));
            };
            Ok(Value::Usage(CaptureUsage {
                derivation_nodes,
                packages,
                terms,
                intervals,
                marker_nodes,
                marker_edges,
                availability_entries,
                max_version_components,
                max_atom_bytes,
                text_bytes,
                work,
            }))
        }
        ObjectKind::Envelope => {
            if field("schema").number() != Some(1) {
                return Err(state.reject(ReadErrorKind::InvalidSchema));
            }
            if field("producer_pid").number() != Some(u64::from(state.token.producer_pid())) {
                return Err(state.reject(ReadErrorKind::MismatchedRequest));
            }
            let (Value::Limits(limits), Value::Usage(usage), Value::Tag(status)) =
                (field("limits"), field("usage"), field("status"))
            else {
                return Err(state.reject(ReadErrorKind::InvalidSchema));
            };
            if !limits.is_supported() {
                return Err(state.reject(ReadErrorKind::InvalidLimits));
            }
            if state.json_bytes > limits.json_bytes {
                return Err(state.reject(ReadErrorKind::Limit(CaptureReason::JsonBytes)));
            }
            let reason = field("reason");
            let graph = field("graph").present();
            let valid = match (status, reason) {
                (0, Value::Missing | Value::Null) => graph && usage_within(usage, limits, 0),
                (1, Value::Tag(reason)) => !graph && reason <= 11 && usage_within(usage, limits, 1),
                (2, Value::Tag(reason)) => !graph && reason >= 12 && usage_within(usage, limits, 0),
                _ => false,
            };
            if !valid {
                return Err(state.reject(ReadErrorKind::InvalidSchema));
            }
            if status == 0 {
                let observed = state.budget.usage;
                if usage.derivation_nodes != observed.derivation_nodes
                    || usage.packages != observed.packages
                    || usage.terms != observed.terms
                    || usage.intervals != observed.intervals
                    || usage.marker_nodes != observed.marker_nodes
                    || usage.marker_edges != observed.marker_edges
                    || usage.availability_entries != observed.availability_entries
                    || usage.max_version_components != observed.max_version_components
                    || usage.max_atom_bytes < observed.max_atom_bytes
                    // Producers may reserve the optional zero-sentinel byte for every version.
                    || usage.text_bytes < observed.text_bytes
                {
                    return Err(state.reject(ReadErrorKind::InvalidSchema));
                }
            }
            Ok(Value::Unit)
        }
        ObjectKind::Version => {
            if !matches!(field("release"), Value::Count(count) if count != 0) {
                return Err(state.reject(ReadErrorKind::InvalidSchema));
            }
            let Value::LocalEmpty(local_empty) = field("local") else {
                return Err(state.reject(ReadErrorKind::InvalidSchema));
            };
            let min = field("min").number();
            let max = field("max").number();
            let valid = match (min, max) {
                (None, None) => true,
                (Some(0), None) => {
                    !field("pre").present()
                        && !field("post").present()
                        && !field("dev").present()
                        && local_empty
                }
                (None, Some(0)) => {
                    !field("post").present() && !field("dev").present() && local_empty
                }
                _ => false,
            };
            if !valid {
                return Err(state.reject(ReadErrorKind::InvalidSchema));
            }
            Ok(Value::Unit)
        }
        ObjectKind::LocalVersion => match (field("kind"), field("segments")) {
            (Value::Tag(0), Value::Count(count)) => Ok(Value::LocalEmpty(count == 0)),
            (Value::Tag(1), Value::Missing) => Ok(Value::LocalEmpty(false)),
            _ => Err(state.reject(ReadErrorKind::InvalidSchema)),
        },
        ObjectKind::LocalSegment => {
            let valid = match (field("kind"), field("value")) {
                (
                    Value::Tag(0),
                    Value::LocalAtom {
                        normalized_string, ..
                    },
                ) => normalized_string,
                (Value::Tag(1), Value::LocalAtom { decimal, .. }) => decimal,
                _ => false,
            };
            if valid {
                Ok(Value::Unit)
            } else {
                Err(state.reject(ReadErrorKind::InvalidSchema))
            }
        }
        ObjectKind::VersionBound | ObjectKind::StringBound => {
            let value = if matches!(kind, ObjectKind::VersionBound) {
                field("version")
            } else {
                field("value")
            };
            let valid = match (field("kind"), value) {
                (Value::Tag(0), Value::Missing) => true,
                (Value::Tag(1 | 2), value) => value.present(),
                _ => false,
            };
            if valid {
                Ok(Value::Unit)
            } else {
                Err(state.reject(ReadErrorKind::InvalidSchema))
            }
        }
        ObjectKind::Reason => {
            let (Value::Tag(kind), status) = (field("kind"), field("http_status")) else {
                return Err(state.reject(ReadErrorKind::InvalidSchema));
            };
            let valid = if matches!(kind, 5 | 14 | 20) {
                status
                    .number()
                    .is_some_and(|status| (100..=999).contains(&status))
            } else {
                !status.present()
            };
            if valid {
                Ok(Value::Unit)
            } else {
                Err(state.reject(ReadErrorKind::InvalidSchema))
            }
        }
        ObjectKind::Graph
        | ObjectKind::NotRoot
        | ObjectKind::NoVersions
        | ObjectKind::FromDependencyOf
        | ObjectKind::Custom
        | ObjectKind::Derived
        | ObjectKind::Term
        | ObjectKind::Range
        | ObjectKind::Prerelease
        | ObjectKind::EncodedInterval
        | ObjectKind::RootPackage
        | ObjectKind::PythonPackage
        | ObjectKind::SystemPackage
        | ObjectKind::Package
        | ObjectKind::ExtraPackage
        | ObjectKind::GroupPackage
        | ObjectKind::MarkerPackage
        | ObjectKind::VersionMarker
        | ObjectKind::StringMarker
        | ObjectKind::VersionEdge
        | ObjectKind::StringEdge
        | ObjectKind::StringInterval
        | ObjectKind::Environment
        | ObjectKind::ProjectConflict
        | ObjectKind::ExtraConflict
        | ObjectKind::GroupConflict
        | ObjectKind::Python
        | ObjectKind::PythonDomain
        | ObjectKind::Specifier
        | ObjectKind::Observation
        | ObjectKind::MetadataFact => Ok(Value::Unit),
    }
}
