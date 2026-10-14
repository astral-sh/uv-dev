use std::borrow::Cow;

use uv_configuration::{Prerelease, PrereleaseMode, specifier_opts_into_prereleases};
use uv_distribution_types::{Requirement, RequirementSource};
use uv_normalize::PackageName;

use crate::resolver::ForkSet;
use crate::{DependencyMode, Manifest, ResolverEnvironment};

/// The configured pre-release modes and the requirements eligible for explicit opt-in.
#[derive(Debug, Clone)]
pub(crate) struct PrereleaseStrategy {
    prerelease: Prerelease,
    explicit_packages: ForkSet,
}

impl PrereleaseStrategy {
    pub(crate) fn from_prerelease(
        prerelease: &Prerelease,
        manifest: &Manifest,
        env: &ResolverEnvironment,
        dependencies: DependencyMode,
    ) -> Self {
        let explicit_packages = if std::iter::once(&prerelease.global)
            .chain(prerelease.package.values())
            .any(|mode| *mode == PrereleaseMode::Explicit)
        {
            Self::explicit_packages(manifest.candidate_selection_requirements(env, dependencies))
        } else {
            ForkSet::default()
        };
        Self {
            prerelease: prerelease.clone(),
            explicit_packages,
        }
    }

    fn explicit_packages<'a>(requirements: impl Iterator<Item = Cow<'a, Requirement>>) -> ForkSet {
        let mut packages = ForkSet::default();
        for requirement in requirements {
            let RequirementSource::Registry { specifier, .. } = &requirement.source else {
                continue;
            };

            if specifier.iter().any(specifier_opts_into_prereleases) {
                packages.add(&requirement, ());
            }
        }
        packages
    }

    /// Returns the pre-release candidate selection policy for a package.
    ///
    /// Pre-releases remain in the candidate universe but, unless they are globally allowed, are
    /// considered only after stable candidates. Keeping the candidate universe fixed is required
    /// for PubGrub's learned incompatibilities to remain valid.
    #[allow(deprecated)]
    pub(crate) fn selection(
        &self,
        package_name: &PackageName,
        env: &ResolverEnvironment,
    ) -> PrereleaseSelection {
        match self.prerelease.mode(package_name) {
            PrereleaseMode::Disallow => PrereleaseSelection::Disallow,
            PrereleaseMode::Allow => PrereleaseSelection::Allow,
            PrereleaseMode::IfNecessary | PrereleaseMode::IfNecessaryOrExplicit => {
                PrereleaseSelection::PreferStable
            }
            PrereleaseMode::Explicit => {
                if self.explicit_packages.contains(package_name, env) {
                    PrereleaseSelection::PreferStable
                } else {
                    PrereleaseSelection::Disallow
                }
            }
        }
    }
}

/// How pre-release candidates participate in version selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrereleaseSelection {
    /// Do not consider pre-release candidates.
    Disallow,
    /// Consider stable and pre-release candidates in normal version order.
    Allow,
    /// Prefer stable candidates, falling back to pre-releases only after stable candidates are
    /// exhausted.
    PreferStable,
}
