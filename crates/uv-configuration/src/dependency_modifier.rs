use std::borrow::Cow;

use uv_distribution_types::{Requirement, ResolutionRecorder};
use uv_normalize::PackageName;
use uv_pep440::Version;

use crate::{Excludes, Overrides};

/// Overrides and exclusions to apply to dependency requirements.
#[derive(Debug, Default, Clone)]
pub struct DependencyModifiers {
    overrides: Overrides,
    excludes: Excludes,
}

/// The package scope in which to apply dependency modifiers.
#[derive(Debug, Clone, Copy)]
pub enum DependencyModifierScope<'a> {
    /// Apply global overrides and exclusions.
    Global,
    /// Apply global and package-scoped overrides and exclusions to regular package metadata.
    Package(&'a PackageName, &'a Version),
    /// Apply global modifiers and package-scoped exclusions to a dependency group.
    DependencyGroup(&'a PackageName, &'a Version),
}

impl DependencyModifiers {
    /// Record which settings are consulted while resolving runtime dependencies.
    #[must_use]
    pub fn with_recorder(mut self, recorder: Option<ResolutionRecorder>) -> Self {
        self.overrides = self.overrides.with_recorder(recorder.clone());
        self.excludes = self.excludes.with_recorder(recorder);
        self
    }

    pub fn new(overrides: Overrides, excludes: Excludes) -> Self {
        Self {
            overrides,
            excludes,
        }
    }

    /// Return whether any modifiers are scoped to this package.
    pub fn has_scoped_package(&self, package: &PackageName) -> bool {
        self.overrides.has_scoped_package(package) || self.excludes.has_scoped_package(package)
    }

    /// Return all global override requirements that are not excluded.
    pub fn global_overrides(&self) -> impl Iterator<Item = &Requirement> {
        self.overrides
            .global_requirements()
            .filter(|requirement| !self.excludes.contains(&requirement.name))
    }

    /// Return all scoped override requirements that are not excluded in their scope.
    pub fn scoped_overrides(&self) -> impl Iterator<Item = &Requirement> {
        self.overrides
            .scoped_requirements()
            .filter(|(package, version, requirement)| {
                !self.excludes.contains_for_scope(
                    &self.overrides,
                    package,
                    *version,
                    &requirement.name,
                )
            })
            .map(|(_, _, requirement)| requirement)
    }

    /// Return the scoped override requirements that apply to this package version.
    pub fn scoped_overrides_for(
        &self,
        package: &PackageName,
        version: &Version,
    ) -> impl Iterator<Item = &Requirement> {
        self.overrides
            .scoped_requirements_for(package, version)
            .filter(|requirement| {
                !self
                    .excludes
                    .contains_for(package, version, &requirement.name)
            })
    }

    /// Return whether a dependency is globally excluded.
    pub fn is_excluded(&self, dependency: &PackageName) -> bool {
        self.excludes.contains(dependency)
    }

    /// Return whether a dependency is excluded from a specific package version.
    pub fn is_excluded_for(
        &self,
        package: &PackageName,
        version: &Version,
        dependency: &PackageName,
    ) -> bool {
        self.excludes.contains_for(package, version, dependency)
    }

    /// Return whether scoped modifiers have the same effect for two package versions.
    pub fn has_same_scoped_effects<'a>(
        &self,
        package: &PackageName,
        version: &Version,
        candidate_version: &Version,
        dependencies: impl IntoIterator<Item = &'a Requirement>,
    ) -> bool {
        if !self
            .scoped_overrides_for(package, version)
            .eq(self.scoped_overrides_for(package, candidate_version))
        {
            return false;
        }

        let mut has_same_exclusion = |dependency: &Requirement| {
            self.is_excluded_for(package, version, &dependency.name)
                == self.is_excluded_for(package, candidate_version, &dependency.name)
        };
        dependencies.into_iter().all(&mut has_same_exclusion)
            && self
                .scoped_overrides_for(package, version)
                .all(has_same_exclusion)
    }

    /// Apply dependency overrides and exclusions in the given scope.
    pub fn apply<'a, 'scope, I>(
        &'a self,
        scope: DependencyModifierScope<'scope>,
        requirements: I,
    ) -> impl Iterator<Item = Cow<'a, Requirement>> + use<'a, 'scope, I>
    where
        I: IntoIterator<Item = &'a Requirement>,
    {
        let (overrides, excludes) = match scope {
            DependencyModifierScope::Global => (None, None),
            DependencyModifierScope::Package(package, version) => {
                (Some((package, version)), Some((package, version)))
            }
            DependencyModifierScope::DependencyGroup(package, version) => {
                (None, Some((package, version)))
            }
        };
        self.overrides
            .apply_for_package(overrides, requirements)
            .filter(move |requirement| {
                !self
                    .excludes
                    .contains_for_package(excludes, &requirement.name)
            })
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use anyhow::Result;
    use uv_distribution_types::Requirement;
    use uv_pep508::Requirement as Pep508Requirement;
    use uv_pypi_types::VerbatimParsedUrl;

    use crate::{DependencyModifiers, ExcludeDependency, Excludes, Override, Overrides};

    fn requirements(inputs: &[&str]) -> Result<Vec<Requirement>> {
        inputs
            .iter()
            .map(|input| {
                Ok(Requirement::from(
                    Pep508Requirement::<VerbatimParsedUrl>::from_str(input)?,
                ))
            })
            .collect()
    }

    #[test]
    fn scoped_effects_prevent_coalescing() -> Result<()> {
        #[derive(serde::Deserialize)]
        struct OverrideInput {
            overrides: Vec<Override<Requirement>>,
        }

        #[derive(serde::Deserialize)]
        struct ExcludeInput {
            excludes: Vec<ExcludeDependency>,
        }

        let name = "parent".parse()?;
        let version = "1".parse()?;
        let candidate_version = "2".parse()?;
        let dependencies = requirements(&["leaf>=1", "group-leaf>=1"])?;
        let has_same_scoped_effects = |modifiers: &DependencyModifiers| {
            modifiers.has_same_scoped_effects(&name, &version, &candidate_version, &dependencies)
        };

        assert!(has_same_scoped_effects(&DependencyModifiers::default()));

        let scoped_override = toml::from_str::<OverrideInput>(
            r#"
            overrides = [{
                package = { name = "parent", version = "1" },
                dependencies = [{ name = "leaf", specifier = "==2" }],
            }]
            "#,
        )?;
        let overrides = Overrides::from_entries(scoped_override.overrides)?;
        assert!(!has_same_scoped_effects(&DependencyModifiers::new(
            overrides,
            Excludes::default(),
        )));

        for dependency in ["leaf", "group-leaf"] {
            let exclusion = toml::from_str::<ExcludeInput>(&format!(
                r#"
                excludes = [{{
                    package = {{ name = "parent", version = "1" }},
                    dependencies = ["{dependency}"],
                }}]
                "#,
            ))?;
            assert!(!has_same_scoped_effects(&DependencyModifiers::new(
                Overrides::default(),
                Excludes::from_entries(exclusion.excludes),
            )));
        }

        let scoped_addition = toml::from_str::<OverrideInput>(
            r#"
            overrides = [{
                package = { name = "parent" },
                dependencies = [{ name = "added", specifier = ">=1" }],
            }]
            "#,
        )?;
        let overrides = Overrides::from_entries(scoped_addition.overrides)?;
        let exclusion = toml::from_str::<ExcludeInput>(
            r#"
            excludes = [{
                package = { name = "parent", version = "1" },
                dependencies = ["added"],
            }]
            "#,
        )?;
        assert!(!has_same_scoped_effects(&DependencyModifiers::new(
            overrides,
            Excludes::from_entries(exclusion.excludes),
        )));

        Ok(())
    }
}
