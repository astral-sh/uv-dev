use std::fmt::{Display, Formatter};

use crate::{
    BuiltDist, Dist, DistributionId, DistributionMetadata, Identifier, InstalledDist, Name,
    ResolvedDistRef, ResourceId, SourceDist, VersionId, VersionOrUrlRef,
};
use uv_normalize::PackageName;
use uv_pep440::Version;

/// A distribution that can be requested during resolution.
///
/// Either an already-installed distribution or a distribution that can be installed.
#[derive(Debug, Clone)]
#[expect(clippy::large_enum_variant)]
pub enum RequestedDist {
    Installed(InstalledDist),
    Installable(Dist),
}

impl From<&ResolvedDistRef<'_>> for RequestedDist {
    fn from(dist: &ResolvedDistRef<'_>) -> Self {
        match dist {
            ResolvedDistRef::InstallableRegistrySourceDist { sdist, prioritized } => {
                // This is okay because we're only here if the prioritized dist
                // has an sdist, so this always succeeds.
                let source = prioritized.source_dist().expect("a source distribution");
                assert_eq!(
                    (&sdist.name, &sdist.version),
                    (&source.name, &source.version),
                    "expected chosen sdist to match prioritized sdist"
                );
                Self::Installable(Dist::Source(SourceDist::Registry(source)))
            }
            ResolvedDistRef::InstallableRegistryBuiltDist {
                wheel, prioritized, ..
            } => {
                assert_eq!(
                    Some(&wheel.filename),
                    prioritized.best_wheel().map(|(wheel, _)| &wheel.filename),
                    "expected chosen wheel to match best wheel"
                );
                // This is okay because we're only here if the prioritized dist
                // has at least one wheel, so this always succeeds.
                let built = prioritized.built_dist().expect("at least one wheel");
                Self::Installable(Dist::Built(BuiltDist::Registry(built)))
            }
            ResolvedDistRef::Installed { dist } => Self::Installed((*dist).clone()),
        }
    }
}

impl RequestedDist {
    /// Returns the version of the distribution, if it is known.
    pub fn version(&self) -> Option<&Version> {
        match self {
            Self::Installed(dist) => Some(dist.version()),
            Self::Installable(dist) => dist.version(),
        }
    }
}

impl Name for RequestedDist {
    fn name(&self) -> &PackageName {
        match self {
            Self::Installable(dist) => dist.name(),
            Self::Installed(dist) => dist.name(),
        }
    }
}

impl DistributionMetadata for RequestedDist {
    fn version_or_url(&self) -> VersionOrUrlRef<'_> {
        match self {
            Self::Installed(dist) => dist.version_or_url(),
            Self::Installable(dist) => dist.version_or_url(),
        }
    }

    fn version_id(&self) -> VersionId {
        match self {
            Self::Installed(dist) => dist.version_id(),
            Self::Installable(dist) => dist.version_id(),
        }
    }
}

impl Identifier for RequestedDist {
    fn distribution_id(&self) -> DistributionId {
        match self {
            Self::Installed(dist) => dist.distribution_id(),
            Self::Installable(dist) => dist.distribution_id(),
        }
    }

    fn resource_id(&self) -> ResourceId {
        match self {
            Self::Installed(dist) => dist.resource_id(),
            Self::Installable(dist) => dist.resource_id(),
        }
    }
}

impl Display for RequestedDist {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Installed(dist) => dist.fmt(f),
            Self::Installable(dist) => dist.fmt(f),
        }
    }
}

#[cfg(test)]
mod tests {
    use uv_distribution_filename::SourceDistExtension;
    use uv_pypi_types::{HashDigests, Yanked};

    use crate::{
        File, FileLocation, HashComparison, IndexUrl, PrioritizedDist, RegistryBuiltDist,
        RegistryBuiltWheel, RegistrySourceDist, ResolvedDist, SourceDistCompatibility,
        WheelCompatibility,
    };

    use super::*;

    fn file(filename: &str, size: u64) -> File {
        File {
            dist_info_metadata: Some(HashDigests::empty()),
            filename: filename.into(),
            hashes: HashDigests::empty(),
            requires_python: Some(std::sync::Arc::new(">=3.10".parse().unwrap())),
            size: Some(size),
            upload_time_utc_ms: Some(123),
            url: FileLocation::RelativeUrl("https://files.example.org/".into(), filename.into()),
            yanked: Some(Box::new(Yanked::Bool(false))),
        }
    }

    fn wheel(filename: &str, index: &str, size: u64, authoritative: bool) -> RegistryBuiltWheel {
        RegistryBuiltWheel {
            filename: filename.parse().unwrap(),
            file: Box::new(file(filename, size)),
            index: index.parse::<IndexUrl>().unwrap(),
            size_is_authoritative: authoritative,
        }
    }

    #[test]
    fn registry_conversions_keep_payloads_and_version() {
        let source = RegistrySourceDist {
            name: "demo".parse().unwrap(),
            version: "1.2.3".parse().unwrap(),
            file: Box::new(file("demo-1.2.3.tar.gz", 103)),
            ext: SourceDistExtension::TarGz,
            index: "https://source.example.org/simple".parse().unwrap(),
            wheels: Vec::new(),
            size_is_authoritative: true,
        };
        let other = wheel(
            "demo-1.2.3-1-py3-none-any.whl",
            "https://other.example.org/simple",
            101,
            false,
        );
        let chosen = wheel(
            "demo-1.2.3-2-py3-none-any.whl",
            "https://chosen.example.org/simple",
            102,
            true,
        );
        let mut prioritized = PrioritizedDist::from_built(
            other.clone(),
            Vec::new(),
            WheelCompatibility::Compatible(HashComparison::Missing, None, None),
        );
        prioritized.insert_built(
            chosen.clone(),
            [],
            WheelCompatibility::Compatible(HashComparison::Matched, None, None),
        );
        prioritized.insert_source(
            source.clone(),
            [],
            SourceDistCompatibility::Compatible(HashComparison::Matched),
        );
        let (selected, _) = prioritized.best_wheel().unwrap();

        let mut expected_source = source.clone();
        expected_source.wheels = vec![other.clone(), chosen.clone()];
        let expected_built = RegistryBuiltDist {
            wheels: vec![other, chosen],
            best_wheel_index: 1,
            sdist: Some(source.clone()),
        };
        for (borrowed, expected, selected_version) in [
            (
                ResolvedDistRef::InstallableRegistrySourceDist {
                    sdist: &source,
                    prioritized: &prioritized,
                },
                Dist::Source(SourceDist::Registry(expected_source)),
                &source.version,
            ),
            (
                ResolvedDistRef::InstallableRegistryBuiltDist {
                    wheel: selected,
                    prioritized: &prioritized,
                },
                Dist::Built(BuiltDist::Registry(expected_built)),
                &selected.filename.version,
            ),
        ] {
            let RequestedDist::Installable(requested) = RequestedDist::from(&borrowed) else {
                panic!("expected an installable request");
            };
            let ResolvedDist::Installable { dist, version } = borrowed.to_owned() else {
                panic!("expected an installable resolved distribution");
            };
            assert_eq!(requested, expected);
            assert_eq!(dist.as_ref(), &expected);
            assert_eq!(version.as_ref(), Some(selected_version));
        }
    }
}
