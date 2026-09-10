use std::cmp::min;
use std::sync::Arc;

use itertools::Itertools;
use pubgrub::Term;
use rustc_hash::{FxHashMap, FxHashSet};
use tokio::sync::mpsc::Sender;
use tracing::{debug, trace};

use crate::candidate_selector::CandidateSelector;
use crate::pubgrub::{PubGrubPackage, PubGrubPackageInner, Range};
use crate::resolver::Request;
use crate::{
    InMemoryIndex, PythonRequirement, ResolveError, ResolverEnvironment, VersionsResponse,
};
use uv_distribution_types::{CompatibleDist, Identifier, IndexCapabilities, IndexMetadata};
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_pep508::MarkerTree;

enum BatchPrefetchStrategy {
    /// Go through the next versions assuming the existing selection and its constraints
    /// remain.
    Compatible {
        compatible: Range<Version>,
        previous: Version,
    },
    /// We encounter cases (botocore) where the above doesn't work: Say we previously selected
    /// a==x.y.z, which depends on b==x.y.z. a==x.y.z is incompatible, but we don't know that
    /// yet. We just selected b==x.y.z and want to prefetch, since for all versions of a we try,
    /// we have to wait for the matching version of b. The exiting range gives us only one version
    /// of b, so the compatible strategy doesn't prefetch any version. Instead, we try the next
    /// heuristic where the next version of b will be x.y.(z-1) and so forth.
    InOrder { previous: Version },
}

/// Prefetch a large number of versions if we already unsuccessfully tried many versions.
///
/// This is an optimization specifically targeted at cold cache urllib3/boto3/botocore, where we
/// have to fetch the metadata for a lot of versions.
///
/// Note that these all heuristics that could totally prefetch lots of irrelevant versions.
#[derive(Clone)]
pub(crate) struct BatchPrefetcher {
    // Types to determine whether we need to prefetch.
    tried_versions: FxHashMap<PackageName, FxHashSet<Version>>,
    last_prefetch: FxHashMap<PackageName, usize>,
    // Types to execute the prefetch.
    prefetch_runner: BatchPrefetcherRunner,
}

/// The types that are needed for running the batch prefetching after we determined that we need to
/// prefetch.
///
/// These types are shared (e.g., `Arc`) so they can be cheaply cloned and moved between threads.
#[derive(Clone)]
pub(crate) struct BatchPrefetcherRunner {
    capabilities: IndexCapabilities,
    index: InMemoryIndex,
    request_sink: Sender<Request>,
}

impl BatchPrefetcher {
    pub(crate) fn new(
        capabilities: IndexCapabilities,
        index: InMemoryIndex,
        request_sink: Sender<Request>,
    ) -> Self {
        Self {
            tried_versions: FxHashMap::default(),
            last_prefetch: FxHashMap::default(),
            prefetch_runner: BatchPrefetcherRunner {
                capabilities,
                index,
                request_sink,
            },
        }
    }

    /// Prefetch a large number of versions if we already unsuccessfully tried many versions.
    pub(crate) fn prefetch_batches(
        &mut self,
        next: &PubGrubPackage,
        index: Option<&IndexMetadata>,
        version: &Version,
        current_range: &Range<Version>,
        unchangeable_constraints: Option<&Term<Range<Version>>>,
        python_requirement: &PythonRequirement,
        selector: &CandidateSelector,
        env: &ResolverEnvironment,
    ) -> Result<(), ResolveError> {
        let PubGrubPackageInner::Package {
            name,
            extra: None,
            group: None,
            marker: MarkerTree::TRUE,
        } = &**next
        else {
            return Ok(());
        };

        let (num_tried, do_prefetch) = self.should_prefetch(name);
        if !do_prefetch {
            return Ok(());
        }
        let total_prefetch = min(num_tried, 50);

        // This is immediate, we already fetched the version map.
        let versions_response = if let Some(index) = index {
            self.prefetch_runner
                .index
                .explicit()
                .wait_blocking(&(name.clone(), index.url().clone()))
                .map_err(|_| ResolveError::UnregisteredTask(name.to_string()))?
        } else {
            self.prefetch_runner
                .index
                .implicit()
                .wait_blocking(name)
                .map_err(|_| ResolveError::UnregisteredTask(name.to_string()))?
        };

        let phase = BatchPrefetchStrategy::Compatible {
            compatible: current_range.clone(),
            previous: version.clone(),
        };

        self.last_prefetch.insert(name.clone(), num_tried);

        self.prefetch_runner.send_prefetch(
            name,
            unchangeable_constraints,
            total_prefetch,
            &versions_response,
            phase,
            python_requirement,
            selector,
            env,
        )?;

        Ok(())
    }

    /// Each time we tried a version for a package, we register that here.
    pub(crate) fn version_tried(&mut self, package: &PubGrubPackage, version: &Version) {
        // Only track base packages, no virtual packages from extras.
        let PubGrubPackageInner::Package {
            name,
            extra: None,
            group: None,
            marker: MarkerTree::TRUE,
        } = &**package
        else {
            return;
        };
        self.tried_versions
            .entry(name.clone())
            .or_default()
            .insert(version.clone());
    }

    /// After 5, 10, 20, 40 tried versions, prefetch that many versions to start early but not
    /// too aggressive. Later we schedule the prefetch of 50 versions every 20 versions, this gives
    /// us a good buffer until we see prefetch again and is high enough to saturate the task pool.
    fn should_prefetch(&self, name: &PackageName) -> (usize, bool) {
        let num_tried = self.tried_versions.get(name).map_or(0, FxHashSet::len);
        let previous_prefetch = self.last_prefetch.get(name).copied().unwrap_or_default();
        let do_prefetch = (num_tried >= 5 && previous_prefetch < 5)
            || (num_tried >= 10 && previous_prefetch < 10)
            || (num_tried >= 20 && previous_prefetch < 20)
            || (num_tried >= 20 && num_tried - previous_prefetch >= 20);
        (num_tried, do_prefetch)
    }

    /// Log stats about how many versions we tried.
    pub(crate) fn log_tried_versions(&self) {
        let total_versions: usize = self.tried_versions.values().map(FxHashSet::len).sum();
        let mut tried_versions: Vec<_> = self
            .tried_versions
            .iter()
            .map(|(name, versions)| (name, versions.len()))
            .collect();
        tried_versions.sort_by(|(p1, c1), (p2, c2)| {
            c1.cmp(c2)
                .reverse()
                .then(p1.to_string().cmp(&p2.to_string()))
        });
        let counts = tried_versions
            .iter()
            .map(|(package, count)| format!("{package} {count}"))
            .join(", ");
        debug!("Tried {total_versions} versions: {counts}");
    }
}

impl BatchPrefetcherRunner {
    /// Given that the conditions for prefetching are met, find the versions to prefetch and
    /// send the prefetch requests.
    fn send_prefetch(
        &self,
        name: &PackageName,
        unchangeable_constraints: Option<&Term<Range<Version>>>,
        total_prefetch: usize,
        versions_response: &Arc<VersionsResponse>,
        mut phase: BatchPrefetchStrategy,
        python_requirement: &PythonRequirement,
        selector: &CandidateSelector,
        env: &ResolverEnvironment,
    ) -> Result<(), ResolveError> {
        let VersionsResponse::Found(version_map) = &**versions_response else {
            return Ok(());
        };

        let mut prefetch_count = 0;
        for _ in 0..total_prefetch {
            let candidate = match phase {
                BatchPrefetchStrategy::Compatible {
                    compatible,
                    previous,
                } => {
                    if let Some(candidate) =
                        selector.select_no_preference(name, &compatible, version_map, env)
                    {
                        let compatible =
                            compatible.difference(&Range::singleton(candidate.version().clone()));
                        phase = BatchPrefetchStrategy::Compatible {
                            compatible,
                            previous: candidate.version().clone(),
                        };
                        candidate
                    } else {
                        // We exhausted the compatible version, switch to ignoring the existing
                        // constraints on the package and instead going through versions in order.
                        phase = BatchPrefetchStrategy::InOrder { previous };
                        continue;
                    }
                }
                BatchPrefetchStrategy::InOrder { previous } => {
                    let mut range = if selector.use_highest_version(name, env) {
                        Range::strictly_lower_than(previous)
                    } else {
                        Range::strictly_higher_than(previous)
                    };
                    // If we have constraints from root, don't go beyond those. Example: We are
                    // prefetching for foo 1.60 and have a dependency for `foo>=1.50`, so we should
                    // only prefetch 1.60 to 1.50, knowing 1.49 will always be rejected.
                    if let Some(unchangeable_constraints) = &unchangeable_constraints {
                        range = match unchangeable_constraints {
                            Term::Positive(constraints) => range.intersection(constraints),
                            Term::Negative(negative_constraints) => {
                                range.difference(negative_constraints)
                            }
                        };
                    }
                    if let Some(candidate) =
                        selector.select_no_preference(name, &range, version_map, env)
                    {
                        phase = BatchPrefetchStrategy::InOrder {
                            previous: candidate.version().clone(),
                        };
                        candidate
                    } else {
                        // Both strategies exhausted their candidates.
                        break;
                    }
                }
            };

            let Some(dist) = candidate.compatible() else {
                continue;
            };

            // Avoid prefetching source distributions, which could be expensive.
            let Some(wheel) = dist.wheel() else {
                continue;
            };

            // Avoid prefetching built distributions that don't support _either_ PEP 658 (`.metadata`)
            // or range requests.
            if !(wheel.file.dist_info_metadata
                || self.capabilities.supports_range_requests(&wheel.index))
            {
                debug!("Abandoning prefetch for {wheel} due to missing registry capabilities");
                return Ok(());
            }

            // Avoid prefetching for distributions that don't satisfy the Python requirement.
            if !satisfies_python(dist, python_requirement) {
                continue;
            }

            let dist = dist.for_resolution();

            // Emit a request to fetch the metadata for this version.
            trace!(
                "Prefetching {prefetch_count} ({}) {}",
                match phase {
                    BatchPrefetchStrategy::Compatible { .. } => "compatible",
                    BatchPrefetchStrategy::InOrder { .. } => "in order",
                },
                dist
            );
            prefetch_count += 1;

            if self.index.distributions().register(dist.distribution_id()) {
                let request = Request::from(dist);
                self.request_sink.blocking_send(request)?;
            }
        }

        match prefetch_count {
            0 => debug!("No `{name}` versions to prefetch"),
            1 => debug!("Prefetched 1 `{name}` version"),
            _ => debug!("Prefetched {prefetch_count} `{name}` versions"),
        }

        Ok(())
    }
}

fn satisfies_python(dist: &CompatibleDist, python_requirement: &PythonRequirement) -> bool {
    match dist {
        CompatibleDist::InstalledDist(_) => {}
        CompatibleDist::SourceDist { sdist, .. }
        | CompatibleDist::IncompatibleWheel { sdist, .. } => {
            // Source distributions must meet both the _target_ Python version and the
            // _installed_ Python version (to build successfully).
            if let Some(requires_python) = sdist.file.requires_python.as_ref() {
                if !python_requirement
                    .installed()
                    .is_contained_by(requires_python)
                {
                    return false;
                }
                if !python_requirement.target().is_contained_by(requires_python) {
                    return false;
                }
            }
        }
        CompatibleDist::CompatibleWheel { wheel, .. } => {
            // Wheels must meet the _target_ Python version.
            if let Some(requires_python) = wheel.file.requires_python.as_ref() {
                if !python_requirement.target().is_contained_by(requires_python) {
                    return false;
                }
            }
        }
    }

    true
}

#[cfg(test)]
mod scheduling_tests {
    use uv_normalize::{ExtraName, GroupName};

    use super::*;
    use crate::pubgrub::PubGrubPython;

    fn prefetcher() -> BatchPrefetcher {
        let (request_sink, _) = tokio::sync::mpsc::channel(1);
        BatchPrefetcher::new(
            IndexCapabilities::default(),
            InMemoryIndex::default(),
            request_sink,
        )
    }

    fn package_name(name: &str) -> PackageName {
        name.parse().expect("valid package name")
    }

    fn version(minor: usize) -> Version {
        Version::new([1, u64::try_from(minor).expect("small test version")])
    }

    fn record_versions(prefetcher: &mut BatchPrefetcher, package: &PubGrubPackage, count: usize) {
        for minor in 0..count {
            prefetcher.version_tried(package, &version(minor));
        }
    }

    fn scheduling_state(prefetcher: &BatchPrefetcher, name: &PackageName) -> (usize, bool) {
        prefetcher.should_prefetch(name)
    }

    #[test]
    fn counts_distinct_versions_per_package() {
        let mut prefetcher = prefetcher();
        let alpha = package_name("alpha");
        let beta = package_name("beta");
        let alpha_package = PubGrubPackage::base(alpha.clone());
        let beta_package = PubGrubPackage::base(beta.clone());

        assert_eq!(scheduling_state(&prefetcher, &alpha), (0, false));
        prefetcher.version_tried(&alpha_package, &version(0));
        prefetcher.version_tried(&alpha_package, &version(0));
        assert_eq!(scheduling_state(&prefetcher, &alpha), (1, false));

        record_versions(&mut prefetcher, &alpha_package, 5);
        assert_eq!(scheduling_state(&prefetcher, &alpha), (5, true));
        assert_eq!(scheduling_state(&prefetcher, &beta), (0, false));

        // Emulate a completed scheduling decision without invoking the runner.
        prefetcher.last_prefetch.insert(alpha.clone(), 5);
        record_versions(&mut prefetcher, &beta_package, 5);
        assert_eq!(scheduling_state(&prefetcher, &alpha), (5, false));
        assert_eq!(scheduling_state(&prefetcher, &beta), (5, true));
        assert_eq!(prefetcher.last_prefetch.get(&beta), None);
    }

    #[test]
    fn ignores_non_base_packages() {
        let mut prefetcher = prefetcher();
        let name = package_name("example");
        let extra: ExtraName = "feature".parse().expect("valid extra");
        let group: GroupName = "development".parse().expect("valid group");
        let marker: MarkerTree = "sys_platform == 'linux'".parse().expect("valid marker");
        let packages = [
            PubGrubPackageInner::Root(None),
            PubGrubPackageInner::Root(Some(name.clone())),
            PubGrubPackageInner::Python(PubGrubPython::Installed),
            PubGrubPackageInner::Python(PubGrubPython::Target),
            PubGrubPackageInner::System(name.clone()),
            PubGrubPackageInner::Package {
                name: name.clone(),
                extra: Some(extra.clone()),
                group: None,
                marker: MarkerTree::TRUE,
            },
            PubGrubPackageInner::Package {
                name: name.clone(),
                extra: None,
                group: Some(group.clone()),
                marker: MarkerTree::TRUE,
            },
            PubGrubPackageInner::Package {
                name: name.clone(),
                extra: None,
                group: None,
                marker,
            },
            PubGrubPackageInner::Package {
                name: name.clone(),
                extra: None,
                group: None,
                marker: MarkerTree::FALSE,
            },
            PubGrubPackageInner::Extra {
                name: name.clone(),
                extra,
                marker: MarkerTree::TRUE,
            },
            PubGrubPackageInner::Group {
                name: name.clone(),
                group,
                marker: MarkerTree::TRUE,
            },
            PubGrubPackageInner::Marker {
                name: name.clone(),
                marker,
            },
        ];

        for package in packages {
            let package = PubGrubPackage::from(package);
            record_versions(&mut prefetcher, &package, 60);
            assert_eq!(
                scheduling_state(&prefetcher, &name),
                (0, false),
                "{package:?}"
            );
        }
        assert!(prefetcher.tried_versions.is_empty());
        assert!(prefetcher.last_prefetch.is_empty());
    }

    #[test]
    fn prefetches_at_distinct_version_thresholds() {
        let mut prefetcher = prefetcher();
        let name = package_name("example");
        let package = PubGrubPackage::base(name.clone());
        let mut scheduled = Vec::new();

        for num_tried in 1..=100 {
            prefetcher.version_tried(&package, &version(num_tried - 1));
            let (count, do_prefetch) = scheduling_state(&prefetcher, &name);
            assert_eq!(count, num_tried);
            if do_prefetch {
                scheduled.push(count);
                prefetcher.last_prefetch.insert(name.clone(), count);
                assert_eq!(scheduling_state(&prefetcher, &name), (count, false));
            }
        }

        assert_eq!(scheduled, [5, 10, 20, 40, 60, 80, 100]);
    }

    #[test]
    fn respects_previous_prefetch_count() {
        for (num_tried, previous_prefetch, expected) in [
            (0, 0, false),
            (4, 0, false),
            (5, 0, true),
            (5, 5, false),
            (6, 4, true),
            (9, 5, false),
            (10, 5, true),
            (11, 9, true),
            (19, 10, false),
            (20, 10, true),
            (21, 19, true),
            (39, 20, false),
            (40, 20, true),
            (40, 40, false),
            (59, 40, false),
            (60, 40, true),
            (74, 55, false),
            (75, 55, true),
            (100, 100, false),
        ] {
            let mut prefetcher = prefetcher();
            let name = package_name("example");
            let package = PubGrubPackage::base(name.clone());
            record_versions(&mut prefetcher, &package, num_tried);
            prefetcher
                .last_prefetch
                .insert(name.clone(), previous_prefetch);

            assert_eq!(
                scheduling_state(&prefetcher, &name),
                (num_tried, expected),
                "tried {num_tried}, previous prefetch {previous_prefetch}"
            );
        }
    }
}
