use std::fmt::{Display, Formatter};
use std::ops::Bound;

use either::Either;
use itertools::Itertools;
use smallvec::SmallVec;
use tracing::{debug, trace};

use uv_configuration::IndexStrategy;
use uv_distribution_types::{CompatibleDist, IncompatibleDist, IncompatibleSource, IndexUrl};
use uv_distribution_types::{DistributionMetadata, IncompatibleWheel, Name, PrioritizedDist};
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_platform_tags::Tags;
use uv_types::InstalledPackagesProvider;

use crate::preferences::{Entry, PreferenceSource, Preferences};
use crate::prerelease::{PrereleaseSelection, PrereleaseStrategy};
use crate::pubgrub::Range;
use crate::resolution_mode::ResolutionStrategy;
use crate::version_map::{VersionMap, VersionMapDistHandle};
use crate::{Exclusions, Manifest, Options, ResolverEnvironment};

#[derive(Debug, Clone)]
#[expect(clippy::struct_field_names)]
pub(crate) struct CandidateSelector {
    resolution_strategy: ResolutionStrategy,
    prerelease_strategy: PrereleaseStrategy,
    index_strategy: IndexStrategy,
}

impl CandidateSelector {
    /// Return a [`CandidateSelector`] for the given [`Manifest`].
    pub(crate) fn for_resolution(
        options: &Options,
        manifest: &Manifest,
        env: &ResolverEnvironment,
    ) -> Self {
        Self {
            resolution_strategy: ResolutionStrategy::from_mode(
                options.resolution_mode,
                manifest,
                env,
                options.dependency_mode,
            ),
            prerelease_strategy: PrereleaseStrategy::from_prerelease(
                &options.prerelease,
                manifest,
                env,
                options.dependency_mode,
            ),
            index_strategy: options.index_strategy,
        }
    }

    #[inline]
    #[allow(dead_code)]
    pub(crate) fn resolution_strategy(&self) -> &ResolutionStrategy {
        &self.resolution_strategy
    }

    #[inline]
    #[allow(dead_code)]
    pub(crate) fn prerelease_strategy(&self) -> &PrereleaseStrategy {
        &self.prerelease_strategy
    }

    #[inline]
    #[allow(dead_code)]
    pub(crate) fn index_strategy(&self) -> &IndexStrategy {
        &self.index_strategy
    }

    /// Select a [`Candidate`] from a set of candidate versions and files.
    ///
    /// Unless present in the provided [`Exclusions`], local distributions from the
    /// [`InstalledPackagesProvider`] are preferred over remote distributions in
    /// the [`VersionMap`].
    pub(crate) fn select<'a, InstalledPackages: InstalledPackagesProvider>(
        &'a self,
        package_name: &'a PackageName,
        range: &Range<Version>,
        version_maps: &'a [VersionMap],
        preferences: &'a Preferences,
        installed_packages: &'a InstalledPackages,
        exclusions: &'a Exclusions,
        index: Option<&'a IndexUrl>,
        env: &ResolverEnvironment,
        tags: Option<&'a Tags>,
    ) -> Option<Candidate<'a>> {
        let reinstall = exclusions.reinstall(package_name);
        let upgrade = exclusions.upgrade(package_name);
        let prerelease_selection = self.prerelease_strategy.selection(package_name, env);

        // If we have a preference (e.g., from a lockfile), search for a version matching that
        // preference.
        //
        // If `--reinstall` is provided, we should omit any already-installed packages from here,
        // since we can't reinstall already-installed packages.
        //
        // The caller removes the current lockfile's preferences for packages selected by
        // `--upgrade`. Preferences inherited from another workspace remain applicable.
        if let Some(preferred) = self.get_preferred(
            package_name,
            range,
            version_maps,
            preferences,
            installed_packages,
            reinstall,
            index,
            prerelease_selection,
            env,
            tags,
        ) {
            trace!("Using preference {} {}", preferred.name, preferred.version);
            return Some(preferred);
        }

        // If we don't have a preference, find an already-installed distribution that satisfies the
        // range.
        let installed = if reinstall {
            None
        } else {
            Self::get_installed(package_name, range, installed_packages, tags)
        };

        // If we're not upgrading, we should prefer the already-installed distribution.
        if !upgrade && let Some(installed) = installed {
            trace!(
                "Using installed {} {} that satisfies {range}",
                installed.name, installed.version
            );
            return Some(installed);
        }

        // Otherwise, find the best candidate from the version maps.
        let compatible = self.select_no_preference_with(
            package_name,
            range,
            version_maps,
            prerelease_selection,
            env,
        );

        // Cross-reference against the already-installed distribution.
        //
        // If the already-installed version is _more_ compatible than the best candidate
        // from the version maps, use the installed version.
        if let Some(installed) = installed
            && compatible.as_ref().is_none_or(|compatible| {
                let highest = self.use_highest_version(package_name, env);
                if highest {
                    installed.version() >= compatible.version()
                } else {
                    installed.version() <= compatible.version()
                }
            })
        {
            trace!(
                "Using installed {} {} that satisfies {range}",
                installed.name, installed.version
            );
            return Some(installed);
        }

        compatible
    }

    /// If the package has an applicable preference that satisfies the current range, use it.
    ///
    /// We try to find a resolution that, depending on the input, does not diverge from the
    /// lockfile or matches a sibling fork. We try an exact match for the current markers (fork
    /// or specific) first, to ensure stability with repeated locking. If that doesn't work, we
    /// fall back to current-lock and sibling-fork preferences that don't match in hopes of still
    /// resolving different forks into the same version. Inherited preferences apply only to the
    /// environments and registry sources recorded by the other workspace.
    fn get_preferred<'a, InstalledPackages: InstalledPackagesProvider>(
        &'a self,
        package_name: &'a PackageName,
        range: &Range<Version>,
        version_maps: &'a [VersionMap],
        preferences: &'a Preferences,
        installed_packages: &'a InstalledPackages,
        reinstall: bool,
        index: Option<&'a IndexUrl>,
        prerelease_selection: PrereleaseSelection,
        env: &ResolverEnvironment,
        tags: Option<&'a Tags>,
    ) -> Option<Candidate<'a>> {
        let preferences = preferences.get(package_name);
        let applicable = |entry: &Entry| {
            index.is_none_or(|index| entry.index().matches(index))
                && (!entry.source().is_inherited()
                    || (range.contains(entry.pin().version())
                        && env.included_by_marker(entry.marker().pep508())))
        };

        // If there are multiple preferences for the same package, we need to sort them by priority.
        let preferences = match preferences {
            [] => return None,
            [entry] => {
                if !applicable(entry) {
                    return None;
                }
                Either::Left(std::iter::once(entry))
            }
            [..] => {
                type Entries<'a> = SmallVec<[&'a Entry; 3]>;

                let mut preferences = preferences.iter().collect::<Entries>();

                preferences.retain(|entry| applicable(entry));

                // Sort the preferences by priority.
                let highest = self.use_highest_version(package_name, env);
                // Existing lockfiles and sibling forks use marker/version ordering when there
                // is no inherited baseline for this package.
                let prioritize_sources = preferences
                    .iter()
                    .any(|entry| entry.source().is_inherited());
                preferences.sort_by_key(|entry| {
                    let marker = entry.marker();

                    // Prefer preferences that match the current environment.
                    let matches_env = env.included_by_marker(marker.pep508());

                    // Prefer the latest (or earliest) version.
                    let version = if highest {
                        Either::Left(entry.pin().version())
                    } else {
                        Either::Right(std::cmp::Reverse(entry.pin().version()))
                    };

                    let source = prioritize_sources.then(|| entry.source().priority());
                    std::cmp::Reverse((matches_env, source, version))
                });

                Either::Right(preferences.into_iter())
            }
        };

        Self::get_preferred_from_iter(
            preferences,
            package_name,
            range,
            version_maps,
            installed_packages,
            reinstall,
            prerelease_selection,
            tags,
        )
    }

    /// Return the first preference that satisfies the current range and is allowed.
    fn get_preferred_from_iter<'a, InstalledPackages: InstalledPackagesProvider>(
        preferences: impl Iterator<Item = &'a Entry>,
        package_name: &'a PackageName,
        range: &Range<Version>,
        version_maps: &'a [VersionMap],
        installed_packages: &'a InstalledPackages,
        reinstall: bool,
        prerelease_selection: PrereleaseSelection,
        tags: Option<&Tags>,
    ) -> Option<Candidate<'a>> {
        for preference in preferences {
            let version = preference.pin().version();
            let source = preference.source();

            // Respect the version range for this requirement.
            if !range.contains(version) {
                continue;
            }

            // Check for a locally installed distribution that matches the preferred version, unless
            // we have to reinstall, in which case we can't reuse an already-installed distribution.
            // Installed registry distributions do not retain their index identity, so they
            // cannot establish that an inherited registry preference matches the source.
            if !reinstall && !source.is_inherited() {
                let installed_dists = installed_packages.get_packages(package_name);
                match installed_dists.as_slice() {
                    [] => {}
                    [dist] => {
                        if dist.version() == version {
                            debug!(
                                "Found installed version of {dist} that satisfies preference in {range}"
                            );

                            // Verify that the installed distribution is compatible with the environment.
                            if tags.is_some_and(|tags| {
                                let Ok(Some(wheel_tags)) = dist.read_tags() else {
                                    return false;
                                };
                                !wheel_tags.is_compatible(tags)
                            }) {
                                debug!("Platform tags mismatch for installed {dist}");
                                continue;
                            }

                            return Some(Candidate {
                                name: package_name,
                                version,
                                dist: CandidateDist::Compatible(CompatibleDist::InstalledDist(
                                    dist,
                                )),
                                choice_kind: VersionChoiceKind::Preference,
                            });
                        }
                    }
                    // We do not consider installed distributions with multiple versions because
                    // during installation these must be reinstalled from the remote
                    _ => {
                        debug!(
                            "Ignoring installed versions of {package_name}: multiple distributions found"
                        );
                    }
                }
            }

            // Respect the pre-release strategy for this fork.
            if version.any_prerelease() {
                let allow = match prerelease_selection {
                    PrereleaseSelection::Allow => true,
                    PrereleaseSelection::Disallow => false,
                    // If the pre-release was provided via an existing file, rather than from the
                    // current solve, accept it unless pre-releases are completely banned.
                    PrereleaseSelection::PreferStable => match source {
                        PreferenceSource::Resolver => false,
                        PreferenceSource::Lock
                        | PreferenceSource::InheritedLock
                        | PreferenceSource::Environment
                        | PreferenceSource::RequirementsTxt => true,
                    },
                };
                if !allow {
                    continue;
                }
            }

            // Check for a remote distribution that matches the preferred version
            if let Some((version_map, file)) = version_maps.iter().find_map(|version_map| {
                let dist = version_map.get(version)?;
                if source.is_inherited() && !Self::matches_inherited_index(preference, dist) {
                    return None;
                }
                Some((version_map, dist))
            }) {
                // An inherited lock records a literal version. A local variant remains a
                // candidate through ordinary selection, but is not the inherited pin itself.
                if version_map.local() && !source.is_inherited() {
                    for local in version_map
                        .versions()
                        .rev()
                        .take_while(|local| *local > version)
                    {
                        if !local.is_local() {
                            continue;
                        }
                        if local.clone().without_local() != *version {
                            continue;
                        }
                        if !range.contains(local) {
                            continue;
                        }
                        if let Some(dist) = version_map.get(local) {
                            debug!("Preferring local version `{package_name}` (v{local})");
                            return Some(Candidate::new(
                                package_name,
                                local,
                                dist,
                                VersionChoiceKind::Preference,
                            ));
                        }
                    }
                }

                return Some(Candidate::new(
                    package_name,
                    version,
                    file,
                    VersionChoiceKind::Preference,
                ));
            }
        }
        None
    }

    /// Check both artifact and metadata sources for an inherited registry preference.
    fn matches_inherited_index(preference: &Entry, dist: &PrioritizedDist) -> bool {
        let Some(dist) = dist.get() else {
            return false;
        };

        dist.for_resolution()
            .index()
            .is_some_and(|index| preference.index().matches(index))
            && dist
                .for_installation()
                .index()
                .is_some_and(|index| preference.index().matches(index))
    }

    /// Check for an installed distribution that satisfies the current range and is allowed.
    fn get_installed<'a, InstalledPackages: InstalledPackagesProvider>(
        package_name: &'a PackageName,
        range: &Range<Version>,
        installed_packages: &'a InstalledPackages,
        tags: Option<&'a Tags>,
    ) -> Option<Candidate<'a>> {
        let installed_dists = installed_packages.get_packages(package_name);
        match installed_dists.as_slice() {
            [] => {}
            [dist] => {
                let version = dist.version();

                // Respect the version range for this requirement.
                if !range.contains(version) {
                    return None;
                }

                // Verify that the installed distribution is compatible with the environment.
                if tags.is_some_and(|tags| {
                    let Ok(Some(wheel_tags)) = dist.read_tags() else {
                        return false;
                    };
                    !wheel_tags.is_compatible(tags)
                }) {
                    debug!("Platform tags mismatch for installed {dist}");
                    return None;
                }

                return Some(Candidate {
                    name: package_name,
                    version,
                    dist: CandidateDist::Compatible(CompatibleDist::InstalledDist(dist)),
                    choice_kind: VersionChoiceKind::Installed,
                });
            }
            // We do not consider installed distributions with multiple versions because
            // during installation these must be reinstalled from the remote
            _ => {
                debug!(
                    "Ignoring installed versions of {package_name}: multiple distributions found"
                );
            }
        }
        None
    }

    /// Select a [`Candidate`] without checking for version preference such as an existing
    /// lockfile.
    pub(crate) fn select_no_preference<'a>(
        &'a self,
        package_name: &'a PackageName,
        range: &Range<Version>,
        version_maps: &'a [VersionMap],
        env: &ResolverEnvironment,
    ) -> Option<Candidate<'a>> {
        self.select_no_preference_with(
            package_name,
            range,
            version_maps,
            self.prerelease_strategy.selection(package_name, env),
            env,
        )
    }

    fn select_no_preference_with<'a>(
        &'a self,
        package_name: &'a PackageName,
        range: &Range<Version>,
        version_maps: &'a [VersionMap],
        prerelease_selection: PrereleaseSelection,
        env: &ResolverEnvironment,
    ) -> Option<Candidate<'a>> {
        match prerelease_selection {
            PrereleaseSelection::Allow => self.select_no_preference_from(
                package_name,
                range,
                version_maps,
                PrereleaseCandidates::All,
                env,
            ),
            PrereleaseSelection::Disallow => self.select_no_preference_from(
                package_name,
                range,
                version_maps,
                PrereleaseCandidates::Stable,
                env,
            ),
            PrereleaseSelection::PreferStable
                if self.index_strategy == IndexStrategy::UnsafeFirstMatch =>
            {
                version_maps.iter().find_map(|version_map| {
                    let version_maps = std::slice::from_ref(version_map);
                    self.select_no_preference_from(
                        package_name,
                        range,
                        version_maps,
                        PrereleaseCandidates::Stable,
                        env,
                    )
                    .or_else(|| {
                        self.select_no_preference_from(
                            package_name,
                            range,
                            version_maps,
                            PrereleaseCandidates::Prerelease,
                            env,
                        )
                    })
                })
            }
            PrereleaseSelection::PreferStable => self
                .select_no_preference_from(
                    package_name,
                    range,
                    version_maps,
                    PrereleaseCandidates::Stable,
                    env,
                )
                .or_else(|| {
                    self.select_no_preference_from(
                        package_name,
                        range,
                        version_maps,
                        PrereleaseCandidates::Prerelease,
                        env,
                    )
                }),
        }
    }

    fn select_no_preference_from<'a>(
        &'a self,
        package_name: &'a PackageName,
        range: &Range<Version>,
        version_maps: &'a [VersionMap],
        prerelease_candidates: PrereleaseCandidates,
        env: &ResolverEnvironment,
    ) -> Option<Candidate<'a>> {
        trace!(
            "Selecting candidate for {package_name} with range {range} with {} remote versions",
            version_maps.iter().map(VersionMap::len).sum::<usize>(),
        );
        let highest = self.use_highest_version(package_name, env);

        if self.index_strategy == IndexStrategy::UnsafeBestMatch {
            if highest {
                Self::select_candidate(
                    version_maps
                        .iter()
                        .enumerate()
                        .map(|(map_index, version_map)| {
                            version_map
                                .iter_included(range)
                                .rev()
                                .map(move |item| (map_index, item))
                        })
                        .kmerge_by(
                            |(index1, (version1, _)), (index2, (version2, _))| match version1
                                .cmp(version2)
                            {
                                std::cmp::Ordering::Equal => index1 < index2,
                                std::cmp::Ordering::Less => false,
                                std::cmp::Ordering::Greater => true,
                            },
                        )
                        .map(|(_, item)| item),
                    package_name,
                    range,
                    prerelease_candidates,
                    highest,
                )
            } else {
                Self::select_candidate(
                    version_maps
                        .iter()
                        .enumerate()
                        .map(|(map_index, version_map)| {
                            version_map
                                .iter_included(range)
                                .map(move |item| (map_index, item))
                        })
                        .kmerge_by(
                            |(index1, (version1, _)), (index2, (version2, _))| match version1
                                .cmp(version2)
                            {
                                std::cmp::Ordering::Equal => index1 < index2,
                                std::cmp::Ordering::Less => true,
                                std::cmp::Ordering::Greater => false,
                            },
                        )
                        .map(|(_, item)| item),
                    package_name,
                    range,
                    prerelease_candidates,
                    highest,
                )
            }
        } else {
            if highest {
                version_maps.iter().find_map(|version_map| {
                    Self::select_candidate(
                        version_map.iter_included(range).rev(),
                        package_name,
                        range,
                        prerelease_candidates,
                        highest,
                    )
                })
            } else {
                version_maps.iter().find_map(|version_map| {
                    Self::select_candidate(
                        version_map.iter_included(range),
                        package_name,
                        range,
                        prerelease_candidates,
                        highest,
                    )
                })
            }
        }
    }

    /// By default, we select the latest version, but we also allow using the lowest version instead
    /// to check the lower bounds.
    pub(crate) fn use_highest_version(
        &self,
        package_name: &PackageName,
        env: &ResolverEnvironment,
    ) -> bool {
        match &self.resolution_strategy {
            ResolutionStrategy::Highest => true,
            ResolutionStrategy::Lowest => false,
            ResolutionStrategy::LowestDirect(direct_dependencies) => {
                !direct_dependencies.contains(package_name, env)
            }
        }
    }

    /// Select the first-matching [`Candidate`] from a set of candidate versions and files,
    /// preferring wheels to source distributions.
    ///
    /// The returned [`Candidate`] _may not_ be compatible with the current platform; in such
    /// cases, the resolver is responsible for tracking the incompatibility and re-running the
    /// selection process with additional constraints.
    ///
    /// `versions` must be ordered from highest to lowest when `highest` is `true`, and from lowest
    /// to highest otherwise.
    fn select_candidate<'a>(
        versions: impl Iterator<Item = (&'a Version, VersionMapDistHandle<'a>)>,
        package_name: &'a PackageName,
        range: &Range<Version>,
        prerelease_candidates: PrereleaseCandidates,
        highest: bool,
    ) -> Option<Candidate<'a>> {
        let segments = range.iter();
        let segments = if highest {
            Either::Left(segments.rev())
        } else {
            Either::Right(segments)
        };
        let Some(mut cursor) = RangeCursor::new(segments, highest) else {
            trace!("Exhausted all candidates for package {package_name} with empty range");
            return None;
        };
        let mut steps = 0usize;
        let mut incompatible: Option<Candidate> = None;
        for (version, maybe_dist) in versions {
            steps += 1;

            // If we have an incompatible candidate, and we've progressed past it, return it.
            if incompatible
                .as_ref()
                .is_some_and(|incompatible| version != incompatible.version)
            {
                trace!(
                    "Returning incompatible candidate for package {package_name} with range {range} after {steps} steps",
                );
                return incompatible;
            }

            let candidate = {
                if match prerelease_candidates {
                    PrereleaseCandidates::All => false,
                    PrereleaseCandidates::Stable => version.any_prerelease(),
                    PrereleaseCandidates::Prerelease => !version.any_prerelease(),
                } {
                    continue;
                }
                if !cursor.contains(version) {
                    continue;
                }
                let Some(dist) = maybe_dist.prioritized_dist() else {
                    continue;
                };
                trace!(
                    "Found candidate for package {package_name} with range {range} after {steps} steps: {version} version"
                );
                Candidate::new(package_name, version, dist, VersionChoiceKind::Compatible)
            };

            // If candidate is not compatible due to exclude newer, continue searching.
            // This is a special case — we pretend versions with exclude newer incompatibilities
            // do not exist so that they are not present in error messages in our test suite.
            // TODO(zanieb): Now that `--exclude-newer` is user facing we may want to consider
            // flagging this behavior such that we _will_ report filtered distributions due to
            // exclude-newer in our error messages.
            if matches!(
                candidate.dist(),
                CandidateDist::Incompatible {
                    incompatible_dist: IncompatibleDist::Source(IncompatibleSource::ExcludeNewer(
                        _
                    )) | IncompatibleDist::Wheel(
                        IncompatibleWheel::ExcludeNewer(_)
                    ),
                    ..
                }
            ) {
                continue;
            }

            // If the candidate isn't compatible, we store it as incompatible and continue
            // searching. Typically, we want to return incompatible candidates so that PubGrub can
            // track them (then continue searching, with additional constraints). However, we may
            // see multiple entries for the same version (e.g., if the same version exists on
            // multiple indexes and `--index-strategy unsafe-best-match` is enabled), and it's
            // possible that one of them is compatible while the other is not.
            //
            // See, e.g., <https://github.com/astral-sh/uv/issues/8922>. At time of writing,
            // markupsafe==3.0.2 exists on the PyTorch index, but there's only a single wheel:
            //
            //   MarkupSafe-3.0.2-cp313-cp313-manylinux_2_17_x86_64.manylinux2014_x86_64.whl
            //
            // Meanwhile, there are a large number of wheels on PyPI for the same version. If the
            // user is on Python 3.12, and we return the incompatible PyTorch wheel without
            // considering the PyPI wheels, PubGrub will mark 3.0.2 as an incompatible version,
            // even though there are compatible wheels on PyPI. Thus, we need to ensure that we
            // return the first _compatible_ candidate across all indexes, if such a candidate
            // exists.
            if matches!(candidate.dist(), CandidateDist::Incompatible { .. }) {
                if incompatible.is_none() {
                    incompatible = Some(candidate);
                }
                continue;
            }

            trace!(
                "Returning candidate for package {package_name} with range {range} after {steps} steps",
            );
            return Some(candidate);
        }

        if incompatible.is_some() {
            trace!(
                "Returning incompatible candidate for package {package_name} with range {range} after {steps} steps",
            );
            return incompatible;
        }

        trace!(
            "Exhausted all candidates for package {package_name} with range {range} after {steps} steps"
        );
        None
    }
}

/// Tracks membership in a range while visiting versions monotonically.
///
/// Unlike [`Range::contains`], which searches the segments for every version, the cursor visits
/// each segment at most once.
struct RangeCursor<'a, Segments> {
    current: (Bound<&'a Version>, Bound<&'a Version>),
    segments: Segments,
    highest: bool,
}

impl<'a, Segments> RangeCursor<'a, Segments>
where
    Segments: Iterator<Item = (Bound<&'a Version>, Bound<&'a Version>)>,
{
    /// Create a cursor over segments ordered in the same direction as the visited versions.
    fn new(mut segments: Segments, highest: bool) -> Option<Self> {
        Some(Self {
            current: segments.next()?,
            segments,
            highest,
        })
    }

    /// Return whether `version` is in the range, advancing past segments that cannot contain any
    /// subsequently visited versions.
    fn contains(&mut self, version: &Version) -> bool {
        if self.highest {
            loop {
                let (start, end) = self.current;
                if is_before(version, start) {
                    let Some(current) = self.segments.next() else {
                        return false;
                    };
                    self.current = current;
                } else {
                    return !is_after(version, end);
                }
            }
        } else {
            loop {
                let (start, end) = self.current;
                if is_after(version, end) {
                    let Some(current) = self.segments.next() else {
                        return false;
                    };
                    self.current = current;
                } else {
                    return !is_before(version, start);
                }
            }
        }
    }
}

fn is_before(version: &Version, bound: Bound<&Version>) -> bool {
    match bound {
        Bound::Included(start) => version < start,
        Bound::Excluded(start) => version <= start,
        Bound::Unbounded => false,
    }
}

fn is_after(version: &Version, bound: Bound<&Version>) -> bool {
    match bound {
        Bound::Included(end) => version > end,
        Bound::Excluded(end) => version >= end,
        Bound::Unbounded => false,
    }
}

#[cfg(test)]
mod tests {
    use uv_configuration::{Prerelease, PrereleaseMode};
    use uv_distribution_filename::SourceDistExtension;
    use uv_distribution_types::{
        File, FileLocation, HashComparison, RegistrySourceDist, SourceDistCompatibility,
    };
    use uv_pep508::{MarkerEnvironment, MarkerEnvironmentBuilder, MarkerTree};
    use uv_pypi_types::{HashDigests, ResolverMarkerEnvironment};
    use uv_types::EmptyInstalledPackages;

    use crate::{Preference, UniversalMarker};

    use super::*;

    fn version(value: &str) -> Version {
        value.parse().expect("valid test version")
    }

    fn package_name() -> PackageName {
        "example".parse().expect("valid test package name")
    }

    fn index(value: &str) -> IndexUrl {
        value.parse().expect("valid test index")
    }

    fn version_map(index: &IndexUrl, values: &[&str]) -> VersionMap {
        VersionMap::from_test_distributions(values.iter().map(|value| {
            let version = version(value);
            let filename = format!("example-{version}.tar.gz");
            let mut dist = PrioritizedDist::default();
            dist.insert_source(
                RegistrySourceDist {
                    name: package_name(),
                    version: version.clone(),
                    file: Box::new(File {
                        dist_info_metadata: None,
                        filename: filename.clone().into(),
                        hashes: HashDigests::empty(),
                        requires_python: None,
                        size: None,
                        upload_time_utc_ms: None,
                        url: FileLocation::new(
                            filename.into(),
                            &"https://example.org/files/".into(),
                        ),
                        yanked: None,
                    }),
                    ext: SourceDistExtension::TarGz,
                    index: index.clone(),
                    wheels: Vec::new(),
                    size_is_authoritative: false,
                },
                [],
                SourceDistCompatibility::Compatible(HashComparison::Matched),
            );
            (version, dist)
        }))
    }

    fn inherited(index: &IndexUrl, value: &str) -> Preference {
        Preference::from_inherited_locked(package_name(), version(value), index.clone(), Vec::new())
    }

    fn selected_version(
        preferences: &Preferences,
        version_maps: &[VersionMap],
        range: &Range<Version>,
        index: Option<&IndexUrl>,
        env: &ResolverEnvironment,
        prerelease: PrereleaseMode,
    ) -> Version {
        let options = Options {
            prerelease: Prerelease {
                global: prerelease,
                ..Prerelease::default()
            },
            ..Options::default()
        };
        let selector =
            CandidateSelector::for_resolution(&options, &Manifest::simple(Vec::new()), env);
        selector
            .select(
                &package_name(),
                range,
                version_maps,
                preferences,
                &EmptyInstalledPackages,
                &Exclusions::default(),
                index,
                env,
                None,
            )
            .expect("test has a selectable version")
            .version()
            .clone()
    }

    #[test]
    fn inherited_lock_preferences_use_source_tiers() {
        let index = index("https://pypi.org/simple");
        let env = ResolverEnvironment::universal(Vec::new());
        let version_maps = [version_map(&index, &["1", "2", "3"])];

        let mut preferences = Preferences::from_iter(
            [
                inherited(&index, "1"),
                Preference::from_locked(
                    package_name(),
                    version("2"),
                    Some(index.clone()),
                    Vec::new(),
                ),
            ],
            &env,
        );
        preferences.insert(
            package_name(),
            Some(index.clone()),
            UniversalMarker::TRUE,
            version("3"),
            PreferenceSource::Resolver,
        );

        assert_eq!(
            selected_version(
                &preferences,
                &version_maps,
                &Range::full(),
                None,
                &env,
                PrereleaseMode::IfNecessary,
            ),
            version("2")
        );
        assert_eq!(
            selected_version(
                &preferences,
                &version_maps,
                &Range::full().difference(&Range::singleton(version("2"))),
                None,
                &env,
                PrereleaseMode::IfNecessary,
            ),
            version("1")
        );
    }

    #[test]
    fn non_inherited_preferences_use_version_order() {
        let index = index("https://pypi.org/simple");
        let env = ResolverEnvironment::universal(Vec::new());
        let mut preferences = Preferences::from_iter(
            [Preference::from_locked(
                package_name(),
                version("1"),
                Some(index.clone()),
                Vec::new(),
            )],
            &env,
        );
        preferences.insert(
            package_name(),
            Some(index.clone()),
            UniversalMarker::TRUE,
            version("2"),
            PreferenceSource::Resolver,
        );
        let version_maps = [version_map(&index, &["1", "2"])];

        assert_eq!(
            selected_version(
                &preferences,
                &version_maps,
                &Range::full(),
                None,
                &env,
                PrereleaseMode::IfNecessary,
            ),
            version("2")
        );
    }

    #[test]
    fn inherited_lock_preferences_fall_back_to_ordinary_candidates() {
        let index = index("https://pypi.org/simple");
        let env = ResolverEnvironment::universal(Vec::new());
        let preferences = Preferences::from_iter([inherited(&index, "1")], &env);
        let version_maps = [version_map(&index, &["1", "2"])];

        assert_eq!(
            selected_version(
                &preferences,
                &version_maps,
                &Range::singleton(version("2")),
                None,
                &env,
                PrereleaseMode::IfNecessary,
            ),
            version("2")
        );
    }

    #[test]
    fn inherited_lock_preferences_respect_registry_identity() {
        let parent_index = index("https://parent.example.org/simple");
        let child_index = index("https://child.example.org/simple");
        let env = ResolverEnvironment::universal(Vec::new());
        let preferences = Preferences::from_iter([inherited(&parent_index, "1")], &env);
        let version_maps = [version_map(&child_index, &["1", "2"])];

        for explicit_index in [None, Some(&child_index)] {
            assert_eq!(
                selected_version(
                    &preferences,
                    &version_maps,
                    &Range::full(),
                    explicit_index,
                    &env,
                    PrereleaseMode::IfNecessary,
                ),
                version("2")
            );
        }
    }

    #[test]
    fn inherited_lock_preferences_are_literal_versions() {
        let index = index("https://pypi.org/simple");
        let env = ResolverEnvironment::universal(Vec::new());
        let preferences = Preferences::from_iter([inherited(&index, "1")], &env);
        let version_maps = [version_map(&index, &["1", "1+local"])];

        assert_eq!(
            selected_version(
                &preferences,
                &version_maps,
                &Range::full(),
                None,
                &env,
                PrereleaseMode::IfNecessary,
            ),
            version("1")
        );
    }

    #[test]
    fn inherited_lock_preferences_respect_prerelease_policy() {
        let index = index("https://pypi.org/simple");
        let env = ResolverEnvironment::universal(Vec::new());
        let preferences = Preferences::from_iter([inherited(&index, "2rc1")], &env);
        let version_maps = [version_map(&index, &["1", "2rc1"])];

        for (prerelease, expected) in [
            (PrereleaseMode::IfNecessary, "2rc1"),
            (PrereleaseMode::Disallow, "1"),
        ] {
            assert_eq!(
                selected_version(
                    &preferences,
                    &version_maps,
                    &Range::full(),
                    None,
                    &env,
                    prerelease,
                ),
                version(expected)
            );
        }
    }

    #[test]
    fn inherited_lock_preferences_respect_markers() {
        let index = index("https://pypi.org/simple");
        let env = ResolverEnvironment::specific(ResolverMarkerEnvironment::from(
            MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
                implementation_name: "cpython",
                implementation_version: "3.12.0",
                os_name: "posix",
                platform_machine: "x86_64",
                platform_python_implementation: "CPython",
                platform_release: "test",
                platform_system: "Linux",
                platform_version: "test",
                python_full_version: "3.12.0",
                python_version: "3.12",
                sys_platform: "linux",
            })
            .expect("valid test environment"),
        ));
        let marker = "sys_platform == 'win32'"
            .parse::<MarkerTree>()
            .expect("valid test marker");
        let preferences = Preferences::from_iter(
            [Preference::from_inherited_locked(
                package_name(),
                version("1"),
                index.clone(),
                vec![UniversalMarker::from_combined(marker)],
            )],
            &env,
        );
        let version_maps = [version_map(&index, &["1", "2"])];

        assert_eq!(
            selected_version(
                &preferences,
                &version_maps,
                &Range::full(),
                None,
                &env,
                PrereleaseMode::IfNecessary,
            ),
            version("2")
        );
    }

    fn assert_range_cursor(highest: bool, values: &[&str]) {
        let range = [
            (Bound::Unbounded, Bound::Excluded(version("2"))),
            (Bound::Included(version("3")), Bound::Included(version("4"))),
            (Bound::Excluded(version("5")), Bound::Unbounded),
        ]
        .into_iter()
        .collect::<Range<_>>();
        let segments = if highest {
            Either::Left(range.iter().rev())
        } else {
            Either::Right(range.iter())
        };
        let mut cursor = RangeCursor::new(segments, highest).expect("test range is not empty");

        for value in values {
            let version = version(value);
            assert_eq!(
                cursor.contains(&version),
                range.contains(&version),
                "{value}"
            );
        }
    }

    #[test]
    fn range_cursor_ascending() {
        assert_range_cursor(false, &["1", "2", "2.5", "3", "4", "5", "6"]);
    }

    #[test]
    fn range_cursor_descending() {
        assert_range_cursor(true, &["6", "5", "4", "3", "2.5", "2", "1"]);
    }
}

/// Controls which release classes are visible during one candidate-selection pass.
///
/// Stable-first selection uses separate [`Self::Stable`] and [`Self::Prerelease`] passes so that
/// an incompatible stable candidate does not prevent falling back to a pre-release. For
/// [`IndexStrategy::UnsafeFirstMatch`], these passes are applied to each index in turn.
#[derive(Debug, Clone, Copy)]
enum PrereleaseCandidates {
    /// Consider versions with or without a pre-release component.
    All,
    /// Consider only versions without a pre-release component.
    Stable,
    /// Consider only versions with a pre-release component.
    Prerelease,
}

#[derive(Debug, Clone)]
pub(crate) enum CandidateDist<'a> {
    Compatible(CompatibleDist<'a>),
    Incompatible {
        /// The reason the prioritized distribution is incompatible.
        incompatible_dist: IncompatibleDist,
        /// The prioritized distribution that had no compatible wheelr or sdist.
        prioritized_dist: &'a PrioritizedDist,
    },
}

impl CandidateDist<'_> {
    /// For an installable dist, return the prioritized distribution.
    fn prioritized(&self) -> Option<&PrioritizedDist> {
        match self {
            Self::Compatible(dist) => dist.prioritized(),
            Self::Incompatible {
                incompatible_dist: _,
                prioritized_dist: prioritized,
            } => Some(prioritized),
        }
    }
}

impl<'a> From<&'a PrioritizedDist> for CandidateDist<'a> {
    fn from(value: &'a PrioritizedDist) -> Self {
        if let Some(dist) = value.get() {
            CandidateDist::Compatible(dist)
        } else {
            // TODO(zanieb)
            // We always return the source distribution (if one exists) instead of the wheel
            // but in the future we may want to return both so the resolver can explain
            // why neither distribution kind can be used.
            let dist = if let Some(incompatibility) = value.incompatible_source() {
                IncompatibleDist::Source(incompatibility.clone())
            } else if let Some(incompatibility) = value.incompatible_wheel() {
                IncompatibleDist::Wheel(incompatibility.clone())
            } else {
                IncompatibleDist::Unavailable
            };
            CandidateDist::Incompatible {
                incompatible_dist: dist,
                prioritized_dist: value,
            }
        }
    }
}

/// The reason why we selected the version of the candidate version, either a preference or being
/// compatible.
#[derive(Debug, Clone, Copy)]
pub(crate) enum VersionChoiceKind {
    /// A preference from an output file such as `-o requirements.txt` or `uv.lock`.
    Preference,
    /// A preference from an installed version.
    Installed,
    /// The next compatible version in a version map
    Compatible,
}

impl Display for VersionChoiceKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Preference => f.write_str("preference"),
            Self::Installed => f.write_str("installed"),
            Self::Compatible => f.write_str("compatible"),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Candidate<'a> {
    /// The name of the package.
    name: &'a PackageName,
    /// The version of the package.
    version: &'a Version,
    /// The distributions to use for resolving and installing the package.
    dist: CandidateDist<'a>,
    /// Whether this candidate was selected from a preference.
    choice_kind: VersionChoiceKind,
}

impl<'a> Candidate<'a> {
    fn new(
        name: &'a PackageName,
        version: &'a Version,
        dist: &'a PrioritizedDist,
        choice_kind: VersionChoiceKind,
    ) -> Self {
        Self {
            name,
            version,
            dist: CandidateDist::from(dist),
            choice_kind,
        }
    }

    /// Return the name of the package.
    pub(crate) fn name(&self) -> &PackageName {
        self.name
    }

    /// Return the version of the package.
    pub(crate) fn version(&self) -> &Version {
        self.version
    }

    /// Return the distribution for the package, if compatible.
    pub(crate) fn compatible(&self) -> Option<&CompatibleDist<'a>> {
        if let CandidateDist::Compatible(ref dist) = self.dist {
            Some(dist)
        } else {
            None
        }
    }

    /// Return this candidate was selected from a preference.
    pub(crate) fn choice_kind(&self) -> VersionChoiceKind {
        self.choice_kind
    }

    /// Return the distribution for the candidate.
    pub(crate) fn dist(&self) -> &CandidateDist<'a> {
        &self.dist
    }

    /// Return the prioritized distribution for the candidate.
    pub(crate) fn prioritized(&self) -> Option<&PrioritizedDist> {
        self.dist.prioritized()
    }
}

impl Name for Candidate<'_> {
    fn name(&self) -> &PackageName {
        self.name
    }
}

impl DistributionMetadata for Candidate<'_> {
    fn version_or_url(&self) -> uv_distribution_types::VersionOrUrlRef<'_> {
        uv_distribution_types::VersionOrUrlRef::Version(self.version)
    }
}
