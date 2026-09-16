use std::collections::BTreeSet;
use std::sync::Arc;

use itertools::Itertools;
use rustc_hash::FxHashSet;
use tracing::trace;

use uv_distribution_types::{RequiresPython, RequiresPythonRange};
use uv_normalize::PackageName;
use uv_pep440::VersionSpecifiers;
use uv_pep508::{MarkerEnvironment, MarkerTree, MarkerTreeKind};
use uv_pypi_types::{
    ConflictItem, ConflictItemRef, ConflictKind, ConflictKindRef, ResolverMarkerEnvironment,
};

use crate::error::{ErrorTree, derivation_tree_has_metadata_failure, derivation_tree_packages};
use crate::pubgrub::{PubGrubDependency, PubGrubPackage};
use crate::resolver::ForkState;
use crate::universal_marker::{ConflictMarker, UniversalMarker};
use crate::{PythonRequirement, ResolveError};

/// Represents one or more marker environments for a resolution.
///
/// Dependencies outside of the marker environments represented by this value
/// are ignored for that particular resolution.
///
/// In normal "pip"-style resolution, one resolver environment corresponds to
/// precisely one marker environment. In universal resolution, multiple marker
/// environments may be specified via a PEP 508 marker expression. In either
/// case, as mentioned above, dependencies not in these marker environments are
/// ignored for the corresponding resolution.
///
/// Callers must provide this to the resolver to indicate, broadly, what kind
/// of resolution it will produce. Generally speaking, callers should provide
/// a specific marker environment for `uv pip`-style resolutions and ask for a
/// universal resolution for uv's project based commands like `uv lock`.
///
/// Callers can rely on this type being reasonably cheap to clone.
///
/// # Internals
///
/// Inside the resolver, when doing a universal resolution, it may create
/// many "forking" states to deal with the fact that there may be multiple
/// incompatible dependency specifications. Specifically, in the Python world,
/// the main constraint is that for any one *specific* marker environment,
/// there must be only one version of a package in a corresponding resolution.
/// But when doing a universal resolution, we want to support many marker
/// environments, and in this context, the "universal" resolution may contain
/// multiple versions of the same package. This is allowed so long as, for
/// any marker environment supported by this resolution, an installation will
/// select at most one version of any given package.
///
/// During resolution, a `ResolverEnvironment` is attached to each internal
/// fork. For non-universal or "specific" resolution, there is only ever one
/// fork because a `ResolverEnvironment` corresponds to one and exactly one
/// marker environment. For universal resolution, the resolver may choose
/// to split its execution into multiple branches. Each of those branches
/// (also called "forks" or "splits") will get its own marker expression that
/// represents a set of marker environments that is guaranteed to be disjoint
/// with the marker environments described by the marker expressions of all
/// other branches.
///
/// Whether it's universal resolution or not, and whether it's one of many
/// forks or one fork, this type represents the set of possible dependency
/// specifications allowed in the resolution produced by a single fork.
///
/// An exception to this is `requires-python`. That is handled separately and
/// explicitly by the resolver. (Perhaps a future refactor can incorporate
/// `requires-python` into this type as well, but it's not totally clear at
/// time of writing if that's a good idea or not.)
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolverEnvironment {
    kind: Kind,
}

/// Already-materialized universal state borrowed for bounded no-solution capture.
pub(crate) struct UniversalEnvironmentRef<'a> {
    pub(crate) marker: MarkerTree,
    pub(crate) initial_forks: &'a [MarkerTree],
    pub(crate) include: &'a crate::FxHashbrownSet<ConflictItem>,
    pub(crate) exclude: &'a crate::FxHashbrownSet<ConflictItem>,
}

/// The specific kind of resolver environment.
///
/// Note that it is explicitly intended that this type remain unexported from
/// this module. The motivation for this design is to discourage repeated case
/// analysis on this type, and instead try to encapsulate the case analysis via
/// higher level routines on `ResolverEnvironment` itself. (This goal may prove
/// intractable, so don't treat it like gospel.)
#[derive(Clone, Debug, Eq, PartialEq)]
enum Kind {
    /// We're solving for one specific marker environment only.
    ///
    /// Generally, this is what's done for `uv pip`. For the project based
    /// commands, like `uv lock`, we do universal resolution.
    Specific {
        /// The marker environment being resolved for.
        ///
        /// Any dependency specification that isn't satisfied by this marker
        /// environment is ignored.
        marker_env: ResolverMarkerEnvironment,
    },
    /// We're solving for all possible marker environments.
    Universal {
        /// The initial set of "fork preferences." These will come from the
        /// lock file when available, or the list of supported environments
        /// explicitly written into the `pyproject.toml`.
        ///
        /// Note that this may be empty, which means resolution should begin
        /// with no forks. Or equivalently, a single fork whose marker
        /// expression matches all marker environments.
        initial_forks: Arc<[MarkerTree]>,
        /// The markers associated with this resolver fork.
        markers: MarkerTree,
        /// Conflicting group inclusions.
        ///
        /// Inclusions are checked in `included_by_group` only when
        /// a project-level exclusion exists for the same package:
        /// an explicit inclusion overrides the project-level
        /// exclusion, allowing a specific extra/group to remain
        /// active even when the project as a whole is excluded.
        ///
        /// We also record inclusions because if we somehow wind up
        /// with an inclusion and exclusion rule for the same conflict
        /// item, then we treat the resulting fork as impossible.
        /// (You cannot require that an extra is both included and
        /// excluded. Such a rule can never be satisfied.) Finally,
        /// we use the inclusion rules to write conflict markers
        /// after resolution is finished.
        include: Arc<crate::FxHashbrownSet<ConflictItem>>,
        /// Conflicting group exclusions.
        exclude: Arc<crate::FxHashbrownSet<ConflictItem>>,
    },
}

impl ResolverEnvironment {
    /// Create a resolver environment that is fixed to one and only one marker
    /// environment.
    ///
    /// This enables `uv pip`-style resolutions. That is, the resolution
    /// returned is only guaranteed to be installable for this specific marker
    /// environment.
    pub fn specific(marker_env: ResolverMarkerEnvironment) -> Self {
        let kind = Kind::Specific { marker_env };
        Self { kind }
    }

    /// Create a resolver environment for producing a multi-platform
    /// resolution.
    ///
    /// The set of marker expressions given corresponds to an initial
    /// seeded set of resolver branches. This might come from a lock file
    /// corresponding to the set of forks produced by a previous resolution, or
    /// it might come from a human crafted set of marker expressions.
    ///
    /// The "normal" case is that the initial forks are empty. When empty,
    /// resolution will create forks as needed to deal with potentially
    /// conflicting dependency specifications across distinct marker
    /// environments.
    ///
    /// Initial forks with distinct lower Python bounds are ordered by the fork
    /// strategy and resolution mode, the same way forks created during
    /// resolution are. The given order still decides between forks that tie,
    /// although we don't guarantee any specific treatment (similar to, at time
    /// of writing, how the order of dependencies specified is also significant
    /// but has no specific guarantees around it).
    pub fn universal(initial_forks: Vec<MarkerTree>) -> Self {
        let kind = Kind::Universal {
            initial_forks: initial_forks.into(),
            markers: MarkerTree::TRUE,
            include: Arc::new(crate::FxHashbrownSet::default()),
            exclude: Arc::new(crate::FxHashbrownSet::default()),
        };
        Self { kind }
    }

    /// Returns the marker environment corresponding to this resolver
    /// environment.
    ///
    /// This only returns a marker environment when resolving for a specific
    /// marker environment. i.e., A non-universal or "pip"-style resolution.
    pub fn marker_environment(&self) -> Option<&MarkerEnvironment> {
        match self.kind {
            Kind::Specific { ref marker_env } => Some(marker_env),
            Kind::Universal { .. } => None,
        }
    }

    /// Returns `false` only when this environment is a fork and it is disjoint
    /// with the given marker.
    pub(crate) fn included_by_marker(&self, marker: MarkerTree) -> bool {
        match self.kind {
            Kind::Specific { .. } => true,
            Kind::Universal { ref markers, .. } => !markers.is_disjoint(marker),
        }
    }

    /// Returns true if the dependency represented by this forker may be
    /// included in the given resolver environment.
    pub(crate) fn included_by_group(&self, group: ConflictItemRef<'_>) -> bool {
        match self.kind {
            Kind::Specific { .. } => true,
            Kind::Universal {
                ref include,
                ref exclude,
                ..
            } => {
                if exclude.contains(&group) {
                    return false;
                }
                // When a project-level conflict item is excluded, the
                // project's extras should be excluded too (unless they
                // are explicitly included). This is because extras
                // transitively depend on the base package, so leaving
                // them in a fork that excludes the project would pull
                // the project's dependencies back in.
                //
                // Groups, on the other hand, do NOT depend on the base
                // package — they are independent dependency sets — so
                // they can safely remain active even when the project
                // itself is excluded.
                if matches!(group.kind(), ConflictKindRef::Extra(_)) {
                    if exclude.contains(&ConflictItemRef::from(group.package())) {
                        // But if this specific extra is explicitly
                        // included (e.g., in a conflict between a project
                        // and one of its own extras), respect the inclusion.
                        return include.contains(&group);
                    }
                }
                true
            }
        }
    }

    /// Returns the bounding Python versions that can satisfy this
    /// resolver environment's marker, if it's constrained.
    pub(crate) fn requires_python(&self) -> Option<RequiresPythonRange> {
        let Kind::Universal {
            markers: pep508_marker,
            ..
        } = self.kind
        else {
            return None;
        };
        crate::marker::requires_python(pep508_marker)
    }

    /// For a universal resolution, return the markers of the current fork.
    pub(crate) fn fork_markers(&self) -> Option<MarkerTree> {
        match self.kind {
            Kind::Specific { .. } => None,
            Kind::Universal { markers, .. } => Some(markers),
        }
    }

    /// Borrow the already-materialized universal state without combining conflict markers.
    pub(crate) fn capture_parts(&self) -> Option<UniversalEnvironmentRef<'_>> {
        match &self.kind {
            Kind::Specific { .. } => None,
            Kind::Universal {
                initial_forks,
                markers,
                include,
                exclude,
            } => Some(UniversalEnvironmentRef {
                marker: *markers,
                initial_forks,
                include,
                exclude,
            }),
        }
    }

    /// Narrow this environment given the forking markers.
    ///
    /// This effectively intersects any markers in this environment with the
    /// markers given, and returns the new resulting environment.
    ///
    /// This is also useful in tests to generate a "forked" marker environment.
    ///
    /// # Panics
    ///
    /// This panics if the resolver environment corresponds to one and only one
    /// specific marker environment. i.e., "pip"-style resolution.
    fn narrow_environment(&self, rhs: MarkerTree) -> Self {
        match self.kind {
            Kind::Specific { .. } => {
                unreachable!("environment narrowing only happens in universal resolution")
            }
            Kind::Universal {
                ref initial_forks,
                markers: ref lhs,
                ref include,
                ref exclude,
            } => {
                let mut markers = *lhs;
                markers = markers.and(rhs);
                let kind = Kind::Universal {
                    initial_forks: Arc::clone(initial_forks),
                    markers,
                    include: Arc::clone(include),
                    exclude: Arc::clone(exclude),
                };
                Self { kind }
            }
        }
    }

    /// Returns a new resolver environment with the given groups included or
    /// excluded from it. An `Ok` variant indicates an include rule while an
    /// `Err` variant indicates en exclude rule.
    ///
    /// When a group is excluded from a resolver environment,
    /// `ResolverEnvironment::included_by_group` will return false. The idea
    /// is that a dependency with a corresponding group should be excluded by
    /// forks in the resolver with this environment. (Include rules also
    /// affect `included_by_group`: when a project-level exclusion exists,
    /// an explicit inclusion for a specific extra overrides it.)
    ///
    /// If calling this routine results in the same conflict item being both
    /// included and excluded, then this returns `None` (since it would
    /// otherwise result in a fork that can never be satisfied).
    ///
    /// # Panics
    ///
    /// This panics if the resolver environment corresponds to one and only one
    /// specific marker environment. i.e., "pip"-style resolution.
    pub(crate) fn filter_by_group(
        &self,
        rules: impl IntoIterator<Item = Result<ConflictItem, ConflictItem>>,
    ) -> Option<Self> {
        match self.kind {
            Kind::Specific { .. } => {
                unreachable!("environment narrowing only happens in universal resolution")
            }
            Kind::Universal {
                ref initial_forks,
                ref markers,
                ref include,
                ref exclude,
            } => {
                let mut include: crate::FxHashbrownSet<_> = (**include).clone();
                let mut exclude: crate::FxHashbrownSet<_> = (**exclude).clone();
                for rule in rules {
                    match rule {
                        Ok(item) => {
                            if exclude.contains(&item) {
                                return None;
                            }
                            include.insert(item);
                        }
                        Err(item) => {
                            if include.contains(&item) {
                                return None;
                            }
                            exclude.insert(item);
                        }
                    }
                }
                let kind = Kind::Universal {
                    initial_forks: Arc::clone(initial_forks),
                    markers: *markers,
                    include: Arc::new(include),
                    exclude: Arc::new(exclude),
                };
                Some(Self { kind })
            }
        }
    }

    /// Create an initial set of forked states based on this resolver
    /// environment configuration.
    ///
    /// In the "clean" universal case, this just returns a singleton `Vec` with
    /// the given fork state. But when the resolver is configured to start
    /// with an initial set of forked resolver states (e.g., those present in
    /// a lock file), then this creates the initial set of forks from that
    /// configuration.
    pub(crate) fn initial_forked_states(
        &self,
        init: ForkState,
    ) -> Result<Vec<ForkState>, ResolveError> {
        let Kind::Universal {
            ref initial_forks,
            markers: ref _markers,
            include: ref _include,
            exclude: ref _exclude,
        } = self.kind
        else {
            return Ok(vec![init]);
        };
        if initial_forks.is_empty() {
            return Ok(vec![init]);
        }
        initial_forks
            .iter()
            .rev()
            .filter_map(|&initial_fork| {
                let combined = UniversalMarker::from_combined(initial_fork);
                let (include, exclude) = match combined.conflict().filter_rules() {
                    Ok(rules) => rules,
                    Err(err) => return Some(Err(err)),
                };
                let mut env = self.filter_by_group(
                    include
                        .into_iter()
                        .map(Ok)
                        .chain(exclude.into_iter().map(Err)),
                )?;
                env = env.narrow_environment(combined.pep508());
                Some(Ok(init.clone().with_env(env)))
            })
            .collect()
    }

    /// Narrow the [`PythonRequirement`] if this resolver environment
    /// corresponds to a more constraining fork.
    ///
    /// For example, if this is a fork where `python_version >= '3.12'` is
    /// always true, and if the given python requirement (perhaps derived from
    /// `Requires-Python`) is `>=3.10`, then this will "narrow" the requirement
    /// to `>=3.12`, corresponding to the marker expression describing this
    /// fork.
    ///
    /// If this environment is not a fork, then this returns `None`.
    pub(crate) fn narrow_python_requirement(
        &self,
        python_requirement: &PythonRequirement,
    ) -> Option<PythonRequirement> {
        python_requirement.narrow(&self.requires_python()?)
    }

    /// Returns a message formatted for end users representing a fork in the
    /// resolver.
    ///
    /// If this resolver environment does not correspond to a particular fork,
    /// then `None` is returned.
    ///
    /// This is useful in contexts where one wants to display a message
    /// relating to a particular fork, but either no message or an entirely
    /// different message when this isn't a fork.
    pub(crate) fn end_user_fork_display(&self) -> Option<String> {
        match &self.kind {
            Kind::Specific { .. } => None,
            Kind::Universal {
                initial_forks: _,
                markers,
                include,
                exclude,
            } => {
                let format_conflict_item = |conflict_item: &ConflictItem| {
                    format!(
                        "{}{}",
                        conflict_item.package(),
                        match conflict_item.kind() {
                            ConflictKind::Extra(extra) => format!("[{extra}]"),
                            ConflictKind::Group(group) => {
                                format!("[group:{group}]")
                            }
                            ConflictKind::Project => String::new(),
                        }
                    )
                };

                if markers.is_true() && include.is_empty() && exclude.is_empty() {
                    return None;
                }

                let mut descriptors = Vec::new();
                if !markers.is_true() {
                    descriptors.push(format!("markers: {markers:?}"));
                }
                if !include.is_empty() {
                    descriptors.push(format!(
                        "included: {}",
                        // Sort to ensure stable error messages
                        include
                            .iter()
                            .map(format_conflict_item)
                            .collect::<BTreeSet<_>>()
                            .into_iter()
                            .join(", "),
                    ));
                }
                if !exclude.is_empty() {
                    descriptors.push(format!(
                        "excluded: {}",
                        // Sort to ensure stable error messages
                        exclude
                            .iter()
                            .map(format_conflict_item)
                            .collect::<BTreeSet<_>>()
                            .into_iter()
                            .join(", "),
                    ));
                }

                Some(format!("split ({})", descriptors.join("; ")))
            }
        }
    }

    /// Creates a universal marker expression corresponding to the fork that is
    /// represented by this resolver environment. A universal marker includes
    /// not just the standard PEP 508 marker, but also a marker based on
    /// conflicting extras/groups.
    ///
    /// This returns `None` when this does not correspond to a fork.
    pub(crate) fn try_universal_markers(&self) -> Option<UniversalMarker> {
        match self.kind {
            Kind::Specific { .. } => None,
            Kind::Universal {
                ref markers,
                ref include,
                ref exclude,
                ..
            } => {
                let mut conflict_marker = ConflictMarker::TRUE;
                for item in exclude.iter() {
                    conflict_marker =
                        conflict_marker.and(ConflictMarker::from_conflict_item(item).negate());
                }
                for item in include.iter() {
                    conflict_marker = conflict_marker.and(ConflictMarker::from_conflict_item(item));
                }
                Some(UniversalMarker::new(*markers, conflict_marker))
            }
        }
    }
}

/// A user visible representation of a resolver environment.
///
/// This is most useful in error and log messages.
impl std::fmt::Display for ResolverEnvironment {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self.kind {
            Kind::Specific { .. } => write!(f, "marker environment"),
            Kind::Universal { ref markers, .. } => {
                if markers.is_true() {
                    write!(f, "all marker environments")
                } else {
                    write!(f, "split `{markers:?}`")
                }
            }
        }
    }
}

/// The different forking possibilities.
///
/// Upon seeing a dependency, when determining whether to fork, three
/// different cases are possible:
///
/// 1. Forking cannot be ruled out.
/// 2. The dependency is excluded by the "parent" fork.
/// 3. The dependency is unconditional and thus cannot provoke new forks.
///
/// This enum encapsulates those possibilities. In the first case, a helper is
/// returned to help management the nuts and bolts of forking.
#[derive(Debug)]
pub(crate) enum ForkingPossibility<'d> {
    Possible(Forker<'d>),
    DependencyAlwaysExcluded,
    NoForkingPossible,
}

impl<'d> ForkingPossibility<'d> {
    pub(crate) fn new(env: &ResolverEnvironment, dep: &'d PubGrubDependency) -> Self {
        let marker = dep.package.marker();
        if !env.included_by_marker(marker) {
            ForkingPossibility::DependencyAlwaysExcluded
        } else if marker.is_true() {
            ForkingPossibility::NoForkingPossible
        } else {
            let forker = Forker {
                package: &dep.package,
                marker,
            };
            ForkingPossibility::Possible(forker)
        }
    }
}

/// An encapsulation of forking based on a single dependency.
#[derive(Debug)]
pub(crate) struct Forker<'d> {
    package: &'d PubGrubPackage,
    marker: MarkerTree,
}

impl Forker<'_> {
    /// Attempt a fork based on the given resolver environment.
    ///
    /// If a fork is possible, then a new forker and at least one new
    /// resolver environment is returned. In some cases, it is possible for
    /// more resolver environments to be returned. (For example, when the
    /// negation of this forker's markers has overlap with the given resolver
    /// environment.)
    pub(crate) fn fork(
        &self,
        env: &ResolverEnvironment,
    ) -> Option<(Self, Vec<ResolverEnvironment>)> {
        if !env.included_by_marker(self.marker) {
            return None;
        }

        let Kind::Universal {
            markers: ref env_marker,
            ..
        } = env.kind
        else {
            panic!("resolver must be in universal mode for forking")
        };

        let mut envs = vec![];
        {
            let not_marker = self.marker.negate();
            if !env_marker.is_disjoint(not_marker) {
                envs.push(env.narrow_environment(not_marker));
            }
        }
        // Note also that we push this one last for historical reasons.
        // Changing the order of forks can change the output in some
        // ways. While it's probably fine, we try to avoid changing the
        // output.
        envs.push(env.narrow_environment(self.marker));

        let mut remaining_marker = self.marker;
        remaining_marker = remaining_marker.and(env_marker.negate());
        let remaining_forker = Forker {
            package: self.package,
            marker: remaining_marker,
        };
        Some((remaining_forker, envs))
    }

    /// Returns true if the dependency represented by this forker may be
    /// included in the given resolver environment.
    pub(crate) fn included(&self, env: &ResolverEnvironment) -> bool {
        let marker = self.package.marker();
        env.included_by_marker(marker)
    }
}

/// Fork the resolver based on a `Requires-Python` specifier.
pub(crate) fn fork_version_by_python_requirement(
    requires_python: &VersionSpecifiers,
    python_requirement: &PythonRequirement,
    env: &ResolverEnvironment,
) -> Vec<ResolverEnvironment> {
    let requires_python = RequiresPython::from_specifiers(requires_python.clone());
    let lower = requires_python.range().lower().clone();

    // Attempt to split the current Python requirement based on the `requires-python` specifier.
    //
    // For example, if the current requirement is `>=3.10`, and the split point is `>=3.11`, then
    // the result will be `>=3.10 and <3.11` and `>=3.11`.
    //
    // However, if the current requirement is `>=3.10`, and the split point is `>=3.9`, then the
    // lower segment will be empty, so we should return an empty list.
    let Some((lower, upper)) = python_requirement.split(lower.into()) else {
        trace!(
            "Unable to split Python requirement `{}` via `Requires-Python` specifier `{}`",
            python_requirement.target(),
            requires_python,
        );
        return vec![];
    };

    let Kind::Universal {
        markers: ref env_marker,
        ..
    } = env.kind
    else {
        panic!("resolver must be in universal mode for forking")
    };

    let mut envs = vec![];
    if !env_marker.is_disjoint(lower.to_marker_tree()) {
        envs.push(env.narrow_environment(lower.to_marker_tree()));
    }
    if !env_marker.is_disjoint(upper.to_marker_tree()) {
        envs.push(env.narrow_environment(upper.to_marker_tree()));
    }
    debug_assert!(!envs.is_empty(), "at least one fork should be produced");
    envs
}

/// Fork the resolver based on a marker.
pub(crate) fn fork_version_by_marker(
    env: &ResolverEnvironment,
    marker: MarkerTree,
) -> Option<(ResolverEnvironment, ResolverEnvironment)> {
    let Kind::Universal {
        markers: ref env_marker,
        ..
    } = env.kind
    else {
        panic!("resolver must be in universal mode for forking")
    };

    // Attempt to split based on the marker.
    //
    // For example, given `python_version >= '3.10'` and the split marker `sys_platform == 'linux'`,
    // the result will be:
    //
    //   `python_version >= '3.10' and sys_platform == 'linux'`
    //   `python_version >= '3.10' and sys_platform != 'linux'`
    //
    // If the marker is disjoint with the current environment, then we should return an empty list.
    // If the marker complement is disjoint with the current environment, then we should also return
    // an empty list.
    //
    // For example, given `python_version >= '3.10' and sys_platform == 'linux'` and the split marker
    // `sys_platform == 'win32'`, return an empty list, since the following isn't satisfiable:
    //
    //   python_version >= '3.10' and sys_platform == 'linux' and sys_platform == 'win32'
    if env_marker.is_disjoint(marker) {
        return None;
    }
    let with_marker = env.narrow_environment(marker);

    let complement = marker.negate();
    if env_marker.is_disjoint(complement) {
        return None;
    }
    let without_marker = env.narrow_environment(complement);

    Some((with_marker, without_marker))
}

/// Find a strict environment partition for a semantic no-solution proof.
///
/// An unavailable-metadata failure is not evidence that dependency markers conflict. Proxy
/// packages can lose the original unavailable reason in a `NoVersions` leaf, so callers also
/// provide the metadata-failure state for every package named in the proof.
pub(crate) fn fork_on_no_solution(
    env: &ResolverEnvironment,
    python_requirement: &PythonRequirement,
    error: &ErrorTree,
    has_metadata_failure: impl Fn(&PackageName) -> bool,
) -> Option<(ResolverEnvironment, ResolverEnvironment)> {
    env.fork_markers()?;
    fork_on_disjoint_markers(
        env,
        python_requirement,
        no_solution_markers(error, has_metadata_failure)?,
    )
}

/// Record a pristine restart when a semantic proof uses a dependency excluded by its environment.
///
/// Dependency-created forks can inherit constraints from a broader parent. The full effective
/// environment, including conflict rules and Python bounds, permits at most one pristine restart.
pub(crate) fn restart_on_no_solution(
    env: &ResolverEnvironment,
    python_requirement: &PythonRequirement,
    error: &ErrorTree,
    restarted_environments: &mut FxHashSet<UniversalMarker>,
    has_metadata_failure: impl Fn(&PackageName) -> bool,
) -> bool {
    if env.fork_markers().is_none() {
        return false;
    }
    let Some(markers) = no_solution_markers(error, has_metadata_failure) else {
        return false;
    };
    restart_on_excluded_markers(env, python_requirement, markers, restarted_environments)
}

/// Collect markers only when the proof describes a semantic dependency failure.
fn no_solution_markers(
    error: &ErrorTree,
    has_metadata_failure: impl Fn(&PackageName) -> bool,
) -> Option<Vec<MarkerTree>> {
    if derivation_tree_has_metadata_failure(error) {
        return None;
    }
    let mut markers = Vec::new();
    for package in derivation_tree_packages(error) {
        if package.name_no_root().is_some_and(&has_metadata_failure) {
            return None;
        }
        markers.push(package.marker());
    }
    Some(markers)
}

/// Record a same-environment restart only if the proof contains an inapplicable marker.
fn restart_on_excluded_markers(
    env: &ResolverEnvironment,
    python_requirement: &PythonRequirement,
    markers: impl IntoIterator<Item = MarkerTree>,
    restarted_environments: &mut FxHashSet<UniversalMarker>,
) -> bool {
    let Some(mut effective) = env.try_universal_markers() else {
        return false;
    };
    effective.and(UniversalMarker::new(
        python_requirement.to_marker_tree(),
        ConflictMarker::TRUE,
    ));
    if effective.is_false() {
        return false;
    }
    let has_excluded_marker = markers
        .into_iter()
        .any(|marker| is_environment_marker(marker) && effective.pep508().is_disjoint(marker));

    // The marker's canonical identity accounts for equivalent syntax and conflict selections.
    // Inserting before the retry also bounds an eager fork that recreates the same environment.
    has_excluded_marker && restarted_environments.insert(effective)
}

/// Find a strict environment partition that separates disjoint markers from a failed resolution.
///
/// Dependency forking is deliberately conservative: requirements for the same package that occur
/// on different edges might be combined even when their markers never hold together. A failure
/// proof containing such markers can be retried in narrower environments. The two proof markers
/// need not cover the parent, so the partition is always a marker and its complement.
fn fork_on_disjoint_markers(
    env: &ResolverEnvironment,
    python_requirement: &PythonRequirement,
    markers: impl IntoIterator<Item = MarkerTree>,
) -> Option<(ResolverEnvironment, ResolverEnvironment)> {
    let effective = env.fork_markers()?.and(python_requirement.to_marker_tree());
    let markers: BTreeSet<_> = markers
        .into_iter()
        .filter(|&marker| {
            is_environment_marker(marker)
                && !effective.is_disjoint(marker)
                && !effective.is_disjoint(marker.negate())
        })
        .collect();

    // Use structural marker order, not proof traversal or BDD allocation order. Requiring both
    // intersections to be nonempty makes each child strictly narrower; this predicate therefore
    // cannot be selected again in either child.
    for &marker in &markers {
        let included = effective.and(marker);
        if markers.iter().any(|&other| included.is_disjoint(other)) {
            return fork_version_by_marker(env, marker);
        }
    }
    None
}

/// Whether a marker depends only on the concrete Python environment.
///
/// Extra, group, and conflict selections have their own resolver state and cannot be split by
/// treating them as independent environment variables.
fn is_environment_marker(marker: MarkerTree) -> bool {
    let mut pending = vec![marker];
    let mut visited = FxHashSet::default();
    while let Some(marker) = pending.pop() {
        if !visited.insert(marker) {
            continue;
        }
        match marker.kind() {
            MarkerTreeKind::True | MarkerTreeKind::False => {}
            MarkerTreeKind::Version(marker) => {
                pending.extend(marker.edges().map(|(_, child)| child));
            }
            MarkerTreeKind::String(marker) => {
                pending.extend(marker.children().map(|(_, child)| child));
            }
            MarkerTreeKind::In(marker) => {
                pending.extend(marker.children().map(|(_, child)| child));
            }
            MarkerTreeKind::Contains(marker) => {
                pending.extend(marker.children().map(|(_, child)| child));
            }
            MarkerTreeKind::List(_) | MarkerTreeKind::Extra(_) => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use std::ops::Bound;
    use std::sync::LazyLock;

    use pubgrub::{Derived, External, Map};
    use reqwest::StatusCode;
    use uv_normalize::{ExtraName, GroupName};
    use uv_pep440::{LowerBound, UpperBound, Version};
    use uv_pep508::{MarkerEnvironment, MarkerEnvironmentBuilder};

    use uv_distribution_types::{RequiresPython, RequiresPythonRange};

    use crate::pubgrub::Range;
    use crate::resolver::{UnavailablePackage, UnavailableReason, UnavailableVersion};

    use super::*;

    /// A dummy marker environment used in tests below.
    ///
    /// It doesn't matter too much what we use here, and indeed, this one was
    /// copied from our uv microbenchmarks.
    static MARKER_ENV: LazyLock<MarkerEnvironment> = LazyLock::new(|| {
        MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
            implementation_name: "cpython",
            implementation_version: "3.11.5",
            os_name: "posix",
            platform_machine: "arm64",
            platform_python_implementation: "CPython",
            platform_release: "21.6.0",
            platform_system: "Darwin",
            platform_version: "Darwin Kernel Version 21.6.0: Mon Aug 22 20:19:52 PDT 2022; root:xnu-8020.140.49~2/RELEASE_ARM64_T6000",
            python_full_version: "3.11.5",
            python_version: "3.11",
            sys_platform: "darwin",
        }).unwrap()
    });

    fn requires_python_lower(lower_version_bound: &str) -> RequiresPython {
        RequiresPython::greater_than_equal_version(&version(lower_version_bound))
    }

    fn requires_python_range_lower(lower_version_bound: &str) -> RequiresPythonRange {
        let lower = LowerBound::new(Bound::Included(version(lower_version_bound)));
        RequiresPythonRange::new(lower, UpperBound::default())
    }

    fn marker(marker: &str) -> MarkerTree {
        marker
            .parse::<MarkerTree>()
            .expect("valid pep508 marker expression")
    }

    fn version(v: &str) -> Version {
        v.parse().expect("valid pep440 version string")
    }

    fn python_requirement(python_version_greater_than_equal: &str) -> PythonRequirement {
        let requires_python = requires_python_lower(python_version_greater_than_equal);
        PythonRequirement::from_marker_environment(&MARKER_ENV, requires_python)
    }

    /// Tests that narrowing a Python requirement when resolving for a
    /// specific marker environment never produces a more constrained Python
    /// requirement.
    #[test]
    fn narrow_python_requirement_specific() {
        let resolver_marker_env = ResolverMarkerEnvironment::from(MARKER_ENV.clone());
        let resolver_env = ResolverEnvironment::specific(resolver_marker_env);

        let pyreq = python_requirement("3.10");
        assert_eq!(resolver_env.narrow_python_requirement(&pyreq), None);

        let pyreq = python_requirement("3.11");
        assert_eq!(resolver_env.narrow_python_requirement(&pyreq), None);

        let pyreq = python_requirement("3.12");
        assert_eq!(resolver_env.narrow_python_requirement(&pyreq), None);
    }

    /// Tests that narrowing a Python requirement during a universal resolution
    /// *without* any forks will never produce a more constrained Python
    /// requirement.
    #[test]
    fn narrow_python_requirement_universal() {
        let resolver_env = ResolverEnvironment::universal(vec![]);

        let pyreq = python_requirement("3.10");
        assert_eq!(resolver_env.narrow_python_requirement(&pyreq), None);

        let pyreq = python_requirement("3.11");
        assert_eq!(resolver_env.narrow_python_requirement(&pyreq), None);

        let pyreq = python_requirement("3.12");
        assert_eq!(resolver_env.narrow_python_requirement(&pyreq), None);
    }

    /// Inside a fork whose marker's Python requirement is equal
    /// to our Requires-Python means that narrowing does not produce
    /// a result.
    #[test]
    fn narrow_python_requirement_forking_no_op() {
        let pyreq = python_requirement("3.10");
        let resolver_env = ResolverEnvironment::universal(vec![])
            .narrow_environment(marker("python_version >= '3.10'"));
        assert_eq!(resolver_env.narrow_python_requirement(&pyreq), None);
    }

    /// In this test, we narrow a more relaxed requirement compared to the
    /// marker for the current fork. This in turn results in a stricter
    /// requirement corresponding to what's specified in the fork.
    #[test]
    fn narrow_python_requirement_forking_stricter() {
        let pyreq = python_requirement("3.10");
        let resolver_env = ResolverEnvironment::universal(vec![])
            .narrow_environment(marker("python_version >= '3.11'"));
        let expected = {
            let range = requires_python_range_lower("3.11");
            let requires_python = requires_python_lower("3.10").narrow(&range).unwrap();
            PythonRequirement::from_marker_environment(&MARKER_ENV, requires_python)
        };
        assert_eq!(
            resolver_env.narrow_python_requirement(&pyreq),
            Some(expected)
        );
    }

    /// In this test, we narrow a stricter requirement compared to the marker
    /// for the current fork. This in turn results in a requirement that
    /// remains unchanged.
    #[test]
    fn narrow_python_requirement_forking_relaxed() {
        let pyreq = python_requirement("3.11");
        let resolver_env = ResolverEnvironment::universal(vec![])
            .narrow_environment(marker("python_version >= '3.10'"));
        assert_eq!(
            resolver_env.narrow_python_requirement(&pyreq),
            Some(python_requirement("3.11")),
        );
    }

    #[test]
    fn disjoint_marker_recovery_partitions_the_whole_environment() {
        let python_requirement = python_requirement("3.12");
        let env = ResolverEnvironment::universal(vec![]).narrow_environment(marker(
            "python_version >= '3.12' and sys_platform != 'win32'",
        ));
        let markers = [
            marker("sys_platform == 'linux'"),
            marker("sys_platform == 'darwin'"),
        ];
        let (left, right) = fork_on_disjoint_markers(&env, &python_requirement, markers)
            .expect("the proof has disjoint markers");
        let parent = env.fork_markers().expect("universal environment");
        let left_marker = left.fork_markers().expect("universal environment");
        let right_marker = right.fork_markers().expect("universal environment");
        assert_eq!(left_marker.or(right_marker), parent);
        assert!(left_marker.is_disjoint(right_marker));
        assert_ne!(left_marker, parent);
        assert_ne!(right_marker, parent);
        assert!(!left_marker.is_disjoint(python_requirement.to_marker_tree()));
        assert!(!right_marker.is_disjoint(python_requirement.to_marker_tree()));

        assert_eq!(
            fork_on_disjoint_markers(&env, &python_requirement, markers.into_iter().rev()),
            Some((left.clone(), right.clone()))
        );
        assert_eq!(
            fork_on_disjoint_markers(&left, &python_requirement, markers),
            None
        );
        assert_eq!(
            fork_on_disjoint_markers(&right, &python_requirement, markers),
            None
        );
    }

    #[test]
    fn disjoint_marker_recovery_respects_requires_python() {
        let python_requirement = PythonRequirement::from_marker_environment(
            &MARKER_ENV,
            RequiresPython::from_specifiers(">=3.12,<3.15".parse().expect("valid Python range")),
        );
        let env = ResolverEnvironment::universal(vec![]);
        for boundary in ["3.12", "3.15"] {
            assert_eq!(
                fork_on_disjoint_markers(
                    &env,
                    &python_requirement,
                    [
                        marker(&format!("python_version < '{boundary}'")),
                        marker(&format!("python_version >= '{boundary}'")),
                    ],
                ),
                None
            );
        }

        // These predicates overlap outside the supported Python range, but not inside it.
        let markers = [
            marker("python_version >= '3.15' or sys_platform == 'linux'"),
            marker("python_version >= '3.15' or sys_platform == 'darwin'"),
        ];
        assert!(!markers[0].is_disjoint(markers[1]));
        let (left, right) = fork_on_disjoint_markers(&env, &python_requirement, markers)
            .expect("the supported domain makes the markers disjoint");
        for child in [left, right] {
            assert!(child.included_by_marker(python_requirement.to_marker_tree()));
        }
    }

    #[test]
    fn disjoint_marker_recovery_preserves_conflict_rules() {
        let package: PackageName = "project".parse().expect("valid package");
        let extra: ExtraName = "feature".parse().expect("valid extra");
        let group: GroupName = "dev".parse().expect("valid group");
        let include = ConflictItem::from((package.clone(), extra));
        let exclude = ConflictItem::from((package, group));
        let env = ResolverEnvironment::universal(vec![])
            .filter_by_group([Ok(include.clone()), Err(exclude.clone())])
            .expect("consistent conflict rules");
        let (left, right) = fork_on_disjoint_markers(
            &env,
            &python_requirement("3.12"),
            [
                marker("sys_platform == 'win32'"),
                marker("sys_platform != 'win32'"),
            ],
        )
        .expect("the proof has disjoint markers");
        for child in [left, right] {
            assert!(child.included_by_group(include.as_ref()));
            assert!(!child.included_by_group(exclude.as_ref()));
            assert_eq!(
                child
                    .try_universal_markers()
                    .expect("universal environment")
                    .conflict(),
                env.try_universal_markers()
                    .expect("universal environment")
                    .conflict()
            );
        }
    }

    #[test]
    fn disjoint_marker_recovery_excludes_selection_markers() {
        let env = ResolverEnvironment::universal(vec![]);
        let python_requirement = python_requirement("3.12");
        for selection in [
            "extra == 'feature'",
            "'feature' in extras",
            "'dev' in dependency_groups",
        ] {
            let selection = marker(selection);
            assert!(!is_environment_marker(selection));
            assert_eq!(
                fork_on_disjoint_markers(
                    &env,
                    &python_requirement,
                    [selection, selection.negate()],
                ),
                None
            );
        }
        assert!(is_environment_marker(marker(
            "python_version >= '3.12' and (sys_platform in 'linux, win32' or 'arm' in platform_machine)"
        )));
    }

    #[test]
    fn disjoint_marker_recovery_requires_universal_disjointness() {
        let python_requirement = python_requirement("3.12");
        let markers = [
            marker("sys_platform == 'win32'"),
            marker("sys_platform != 'win32'"),
        ];
        let specific =
            ResolverEnvironment::specific(ResolverMarkerEnvironment::from(MARKER_ENV.clone()));
        assert_eq!(
            fork_on_disjoint_markers(&specific, &python_requirement, markers),
            None
        );
        assert_eq!(
            fork_on_disjoint_markers(
                &ResolverEnvironment::universal(vec![]),
                &python_requirement,
                [
                    marker("python_version >= '3.13'"),
                    marker("sys_platform == 'win32'"),
                ],
            ),
            None
        );
    }

    #[test]
    fn excluded_marker_restart_uses_effective_domain_once() {
        let python_requirement = PythonRequirement::from_marker_environment(
            &MARKER_ENV,
            RequiresPython::from_specifiers(">=3.12,<3.15".parse().expect("valid Python range")),
        );
        let env = ResolverEnvironment::universal(vec![]).narrow_environment(marker(
            "python_version >= '3.13' and sys_platform != 'win32'",
        ));
        let excluded = marker("sys_platform == 'win32'");
        let mut restarted = FxHashSet::default();
        assert!(restart_on_excluded_markers(
            &env,
            &python_requirement,
            [excluded],
            &mut restarted,
        ));

        // The same domain can be expressed in the fork marker or in `Requires-Python`.
        let equivalent = ResolverEnvironment::universal(vec![])
            .narrow_environment(marker("sys_platform != 'win32'"));
        let narrower_python = PythonRequirement::from_marker_environment(
            &MARKER_ENV,
            RequiresPython::from_specifiers(">=3.13,<3.15".parse().expect("valid Python range")),
        );
        assert!(!restart_on_excluded_markers(
            &equivalent,
            &narrower_python,
            [excluded],
            &mut restarted,
        ));
        assert!(restart_on_excluded_markers(
            &env.narrow_environment(marker("python_version >= '3.14'")),
            &python_requirement,
            [excluded],
            &mut restarted,
        ));
        assert_eq!(restarted.len(), 2);
    }

    #[test]
    fn excluded_marker_restart_distinguishes_conflict_rules() {
        let package: PackageName = "project".parse().expect("valid package");
        let extra: ExtraName = "feature".parse().expect("valid extra");
        let group: GroupName = "dev".parse().expect("valid group");
        let extra = ConflictItem::from((package.clone(), extra));
        let group = ConflictItem::from((package, group));
        let env = ResolverEnvironment::universal(vec![])
            .narrow_environment(marker("sys_platform != 'win32'"));
        let include = env
            .filter_by_group([Ok(extra.clone()), Err(group.clone())])
            .expect("consistent conflict rules");
        let exclude = env
            .filter_by_group([Err(extra), Err(group)])
            .expect("consistent conflict rules");
        let mut restarted = FxHashSet::default();
        for env in [&include, &exclude] {
            assert!(restart_on_excluded_markers(
                env,
                &python_requirement("3.12"),
                [marker("sys_platform == 'win32'")],
                &mut restarted,
            ));
            assert!(!restart_on_excluded_markers(
                env,
                &python_requirement("3.12"),
                [marker("sys_platform == 'win32'")],
                &mut restarted,
            ));
        }
        assert_eq!(restarted.len(), 2);
    }

    #[test]
    fn excluded_marker_restart_requires_an_inapplicable_environment_marker() {
        let python_requirement = PythonRequirement::from_marker_environment(
            &MARKER_ENV,
            RequiresPython::from_specifiers(">=3.12,<3.15".parse().expect("valid Python range")),
        );
        let universal = ResolverEnvironment::universal(vec![]);
        let env = universal.narrow_environment(marker("sys_platform != 'win32'"));
        let mut restarted = FxHashSet::default();
        assert!(!restart_on_excluded_markers(
            &universal,
            &python_requirement,
            [marker("sys_platform == 'win32'")],
            &mut restarted,
        ));
        for selection in [
            "extra == 'feature'",
            "'feature' in extras",
            "'dev' in dependency_groups",
        ] {
            assert!(!restart_on_excluded_markers(
                &env,
                &python_requirement,
                [marker(&format!("{selection} and sys_platform == 'win32'"))],
                &mut restarted,
            ));
        }
        let specific =
            ResolverEnvironment::specific(ResolverMarkerEnvironment::from(MARKER_ENV.clone()));
        for rejected in [specific, universal.narrow_environment(MarkerTree::FALSE)] {
            assert!(!restart_on_excluded_markers(
                &rejected,
                &python_requirement,
                [marker("sys_platform == 'win32'")],
                &mut restarted,
            ));
        }
        assert!(restarted.is_empty());

        // A proof marker can also be excluded by the Python domain rather than the fork marker.
        assert!(restart_on_excluded_markers(
            &universal,
            &python_requirement,
            [marker("python_version >= '3.15'")],
            &mut restarted,
        ));
    }

    #[test]
    fn no_solution_recovery_excludes_metadata_failures() {
        let env = ResolverEnvironment::universal(vec![]);
        let python_requirement = python_requirement("3.12");
        let parent = PubGrubPackage::from_package(
            "a".parse().expect("valid package"),
            None,
            None,
            marker("sys_platform == 'win32'"),
        );
        let missing = PubGrubPackage::from_package(
            "missing".parse().expect("valid package"),
            None,
            None,
            marker("sys_platform != 'win32'"),
        );
        let proof = |reason| {
            ErrorTree::Derived(Derived {
                terms: Map::default(),
                shared_id: None,
                cause1: Arc::new(ErrorTree::External(External::FromDependencyOf(
                    parent.clone(),
                    Range::full(),
                    missing.clone(),
                    Range::full(),
                ))),
                cause2: Arc::new(ErrorTree::External(External::Custom(
                    missing.clone(),
                    Range::full(),
                    reason,
                ))),
            })
        };

        let absent = proof(UnavailableReason::Package(UnavailablePackage::NotFound));
        assert!(fork_on_no_solution(&env, &python_requirement, &absent, |_| false).is_some());
        let narrowed = env.narrow_environment(marker("sys_platform != 'win32'"));
        let mut restarted = FxHashSet::default();
        assert!(restart_on_no_solution(
            &narrowed,
            &python_requirement,
            &absent,
            &mut restarted,
            |_| false,
        ));
        assert!(!restart_on_no_solution(
            &narrowed,
            &python_requirement,
            &absent,
            &mut restarted,
            |_| false,
        ));
        // A proxy's `NoVersions` leaf can hide a package-level retrieval failure.
        let masked = ErrorTree::Derived(Derived {
            terms: Map::default(),
            shared_id: None,
            cause1: Arc::new(ErrorTree::External(External::FromDependencyOf(
                parent.clone(),
                Range::full(),
                missing.clone(),
                Range::full(),
            ))),
            cause2: Arc::new(ErrorTree::External(External::NoVersions(
                missing.clone(),
                Range::full(),
            ))),
        });
        assert_eq!(
            fork_on_no_solution(&env, &python_requirement, &masked, |name| {
                name == missing.name_no_root().expect("named package")
            }),
            None
        );
        assert!(!restart_on_no_solution(
            &narrowed,
            &python_requirement,
            &masked,
            &mut FxHashSet::default(),
            |name| name == missing.name_no_root().expect("named package"),
        ));

        for reason in [
            UnavailableReason::Package(UnavailablePackage::Offline),
            UnavailableReason::Package(UnavailablePackage::Network(StatusCode::FORBIDDEN)),
            UnavailableReason::Version(UnavailableVersion::InvalidMetadata),
            UnavailableReason::Version(UnavailableVersion::InconsistentMetadata),
            UnavailableReason::Version(UnavailableVersion::InvalidStructure),
            UnavailableReason::Version(UnavailableVersion::Offline),
            UnavailableReason::Version(UnavailableVersion::RequiresPython(
                ">=3.13".parse().expect("valid Python range"),
            )),
            UnavailableReason::Version(UnavailableVersion::Network(
                StatusCode::SERVICE_UNAVAILABLE,
            )),
        ] {
            let error = proof(reason);
            assert_eq!(
                fork_on_no_solution(&env, &python_requirement, &error, |_| false),
                None
            );
            assert!(!restart_on_no_solution(
                &narrowed,
                &python_requirement,
                &error,
                &mut FxHashSet::default(),
                |_| false,
            ));
        }
    }
}
