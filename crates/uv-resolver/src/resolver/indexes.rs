use crate::fork_indexes::ForkIndexes;
use crate::resolver::ForkMap;
use crate::{DependencyMode, Manifest, ResolveError, ResolverEnvironment};
use uv_distribution_types::{IndexMetadata, RequirementSource, ResolutionRecorder};
use uv_normalize::PackageName;

/// A map of package names to their explicit index.
///
/// For example, given:
/// ```toml
/// [[tool.uv.index]]
/// name = "pytorch"
/// url = "https://download.pytorch.org/whl/cu130"
///
/// [tool.uv.sources]
/// torch = { index = "pytorch" }
/// ```
///
/// [`Indexes`] would contain a single entry mapping `torch` to `https://download.pytorch.org/whl/cu130`.
#[derive(Debug, Default, Clone)]
pub(crate) struct Indexes {
    indexes: ForkMap<IndexMetadata>,
    recorder: Option<ResolutionRecorder>,
}

impl Indexes {
    /// Determine the set of explicit, pinned indexes in the [`Manifest`].
    pub(crate) fn from_manifest(
        manifest: &Manifest,
        env: &ResolverEnvironment,
        dependencies: DependencyMode,
    ) -> Self {
        let mut indexes = ForkMap::default();

        for requirement in manifest.requirements(env, dependencies) {
            let RequirementSource::Registry {
                index: Some(index), ..
            } = &requirement.source
            else {
                continue;
            };
            indexes.add(requirement.as_ref(), index.clone());
        }

        Self {
            indexes,
            recorder: manifest.recorder.clone(),
        }
    }

    /// Returns `true` if the map contains any indexes for a package.
    pub(crate) fn contains_key(&self, name: &PackageName) -> bool {
        if let Some(recorder) = &self.recorder {
            recorder.source_policy(name);
        }
        self.indexes.contains_key(name)
    }

    /// Return the explicit index used for a package in the given fork.
    pub(crate) fn get(&self, name: &PackageName, env: &ResolverEnvironment) -> Vec<&IndexMetadata> {
        if let Some(recorder) = &self.recorder {
            recorder.source_policy(name);
        }
        self.indexes.get(name, env)
    }
}

impl Manifest {
    /// Return the package index selected by active requirements, constraints, and overrides.
    ///
    /// Conflicting active pins are rejected with the same diagnostic as dependency resolution.
    pub fn index_for(
        &self,
        package: &PackageName,
        environment: &ResolverEnvironment,
    ) -> Result<Option<IndexMetadata>, ResolveError> {
        let indexes = Indexes::from_manifest(self, environment, DependencyMode::Transitive);
        let mut selected = ForkIndexes::default();
        for index in indexes.get(package, environment) {
            selected.insert(package, index, environment)?;
        }
        Ok(selected.get(package).cloned())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::error::Error;

    use uv_configuration::{
        Constraints, DependencyModifiers, ExcludeDependency, Excludes, Overrides,
    };
    use uv_distribution_types::{
        IndexFormat, IndexMetadata, Requirement, RequirementScope, RequirementSource,
    };
    use uv_pep508::{MarkerEnvironment, MarkerEnvironmentBuilder, MarkerTree};

    use crate::{Exclusions, Manifest, Preferences, ResolverEnvironment};

    fn environment() -> Result<ResolverEnvironment, Box<dyn Error>> {
        Ok(ResolverEnvironment::specific(
            MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
                implementation_name: "cpython",
                implementation_version: "3.12.9",
                os_name: "posix",
                platform_machine: "x86_64",
                platform_python_implementation: "CPython",
                platform_release: "6.8.0",
                platform_system: "Linux",
                platform_version: "6.8.0",
                python_full_version: "3.12.9",
                python_version: "3.12",
                sys_platform: "linux",
            })?
            .into(),
        ))
    }

    fn index(url: &str, format: IndexFormat) -> Result<IndexMetadata, Box<dyn Error>> {
        Ok(IndexMetadata {
            url: url.parse()?,
            format,
        })
    }

    fn requirement(
        index: Option<IndexMetadata>,
        marker: MarkerTree,
    ) -> Result<Requirement, Box<dyn Error>> {
        Ok(Requirement {
            name: "a".parse()?,
            extras: Box::new([]),
            groups: Box::new([]),
            marker,
            source: RequirementSource::Registry {
                specifier: ">=1".parse()?,
                index,
                conflict: None,
            },
            scope: RequirementScope::Global,
            origin: None,
        })
    }

    #[test]
    fn manifest_index_uses_active_pins() -> Result<(), Box<dyn Error>> {
        let selected = index("https://selected.example/simple", IndexFormat::Simple)?;
        let inactive = index("https://inactive.example/simple", IndexFormat::Simple)?;
        let manifest = Manifest::simple(vec![
            requirement(Some(inactive), "python_version < '3.0'".parse()?)?,
            requirement(Some(selected.clone()), "python_version >= '3.12'".parse()?)?,
            requirement(Some(selected.clone()), MarkerTree::TRUE)?,
        ]);

        assert_eq!(
            manifest.index_for(&"a".parse()?, &environment()?)?,
            Some(selected)
        );
        Ok(())
    }

    #[test]
    fn manifest_index_rejects_conflicting_active_pins() -> Result<(), Box<dyn Error>> {
        let first = requirement(
            Some(index("https://z.example/simple", IndexFormat::Simple)?),
            MarkerTree::TRUE,
        )?;
        let second = requirement(
            Some(index("https://a.example/simple", IndexFormat::Simple)?),
            MarkerTree::TRUE,
        )?;
        let environment = environment()?;
        let package = "a".parse()?;
        let forward = Manifest::simple(vec![first.clone(), second.clone()]);
        let reverse = Manifest::simple(vec![second, first]);
        let expected = Some("Requirements contain conflicting indexes for package `a`:\n- https://a.example/simple\n- https://z.example/simple".to_owned());

        assert_eq!(
            forward
                .index_for(&package, &environment)
                .err()
                .map(|error| error.to_string()),
            expected
        );
        assert_eq!(
            reverse
                .index_for(&package, &environment)
                .err()
                .map(|error| error.to_string()),
            expected
        );
        Ok(())
    }

    #[test]
    fn manifest_index_uses_constraint_pin() -> Result<(), Box<dyn Error>> {
        let selected = index("https://selected.example/simple", IndexFormat::Simple)?;
        let manifest = Manifest::simple(vec![requirement(None, MarkerTree::TRUE)?])
            .with_constraints(Constraints::from_requirements(
                [
                    requirement(Some(selected.clone()), MarkerTree::TRUE)?,
                    requirement(
                        Some(index(
                            "https://inactive.example/simple",
                            IndexFormat::Simple,
                        )?),
                        "python_version < '3.0'".parse()?,
                    )?,
                ]
                .into_iter(),
            ));

        assert_eq!(
            manifest.index_for(&"a".parse()?, &environment()?)?,
            Some(selected)
        );
        Ok(())
    }

    #[test]
    fn manifest_index_applies_override_pin() -> Result<(), Box<dyn Error>> {
        let selected = index("https://override.example/simple", IndexFormat::Simple)?;
        let manifest = Manifest::new(
            vec![requirement(
                Some(index(
                    "https://original.example/simple",
                    IndexFormat::Simple,
                )?),
                MarkerTree::TRUE,
            )?],
            Constraints::default(),
            DependencyModifiers::new(
                Overrides::from_requirements(vec![requirement(
                    Some(selected.clone()),
                    MarkerTree::TRUE,
                )?]),
                Excludes::default(),
            ),
            Preferences::default(),
            None,
            BTreeMap::new(),
            Exclusions::default(),
            Vec::new(),
        );

        assert_eq!(
            manifest.index_for(&"a".parse()?, &environment()?)?,
            Some(selected)
        );
        Ok(())
    }

    #[test]
    fn manifest_index_ignores_excluded_pins() -> Result<(), Box<dyn Error>> {
        let manifest = Manifest::new(
            vec![requirement(
                Some(index(
                    "https://requirement.example/simple",
                    IndexFormat::Simple,
                )?),
                MarkerTree::TRUE,
            )?],
            Constraints::from_requirements(
                [requirement(
                    Some(index(
                        "https://constraint.example/simple",
                        IndexFormat::Simple,
                    )?),
                    MarkerTree::TRUE,
                )?]
                .into_iter(),
            ),
            DependencyModifiers::new(
                Overrides::from_requirements(vec![requirement(
                    Some(index(
                        "https://override.example/simple",
                        IndexFormat::Simple,
                    )?),
                    MarkerTree::TRUE,
                )?]),
                Excludes::from_entries([ExcludeDependency::Dependency("a".parse()?)]),
            ),
            Preferences::default(),
            None,
            BTreeMap::new(),
            Exclusions::default(),
            Vec::new(),
        );

        assert_eq!(manifest.index_for(&"a".parse()?, &environment()?)?, None);
        Ok(())
    }

    #[test]
    fn manifest_index_preserves_flat_format() -> Result<(), Box<dyn Error>> {
        let selected = index("https://selected.example/wheels", IndexFormat::Flat)?;
        let manifest =
            Manifest::simple(vec![requirement(Some(selected.clone()), MarkerTree::TRUE)?]);

        assert_eq!(
            manifest.index_for(&"a".parse()?, &environment()?)?,
            Some(selected)
        );
        Ok(())
    }

    #[test]
    fn manifest_index_ignores_other_packages() -> Result<(), Box<dyn Error>> {
        let manifest = Manifest::simple(vec![requirement(
            Some(index(
                "https://selected.example/simple",
                IndexFormat::Simple,
            )?),
            MarkerTree::TRUE,
        )?]);

        assert_eq!(manifest.index_for(&"b".parse()?, &environment()?)?, None);
        Ok(())
    }
}
