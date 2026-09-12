use std::borrow::Cow;
use std::collections::BTreeSet;

use either::Either;

use uv_configuration::{Constraints, Excludes, Overrides};
use uv_distribution_types::Requirement;
use uv_normalize::PackageName;
use uv_types::RequestedRequirements;

use crate::preferences::Preferences;
use crate::{DependencyMode, Exclusions, ResolverEnvironment};

/// A manifest of requirements, constraints, and preferences.
#[derive(Clone, Debug)]
pub struct Manifest {
    /// The direct requirements for the project.
    pub(super) requirements: Vec<Requirement>,

    /// The constraints for the project.
    pub(super) constraints: Constraints,

    /// The overrides for the project.
    pub(super) overrides: Overrides,

    /// The dependency excludes for the project.
    pub(super) excludes: Excludes,

    /// The preferences for the project.
    ///
    /// These represent "preferred" versions of a given package. For example, they may be the
    /// versions that are already installed in the environment, or already pinned in an existing
    /// lockfile.
    pub(super) preferences: Preferences,

    /// The name of the project.
    pub(super) project: Option<PackageName>,

    /// Members of the project's workspace.
    pub(super) workspace_members: BTreeSet<PackageName>,

    /// The installed packages to exclude from consideration during resolution.
    ///
    /// These typically represent packages that are being upgraded or reinstalled
    /// and should be pulled from a remote source like a package index.
    pub(super) exclusions: Exclusions,

    /// The lookahead requirements for the project.
    ///
    /// These represent transitive dependencies that should be incorporated when making
    /// determinations around "allowed" versions (for example, "allowed" URLs or "allowed"
    /// pre-release versions).
    pub(super) lookaheads: Vec<RequestedRequirements>,
}

impl Manifest {
    pub fn new(
        requirements: Vec<Requirement>,
        constraints: Constraints,
        overrides: Overrides,
        excludes: Excludes,
        preferences: Preferences,
        project: Option<PackageName>,
        workspace_members: BTreeSet<PackageName>,
        exclusions: Exclusions,
        lookaheads: Vec<RequestedRequirements>,
    ) -> Self {
        Self {
            requirements,
            constraints,
            overrides,
            excludes,
            preferences,
            project,
            workspace_members,
            exclusions,
            lookaheads,
        }
    }

    pub fn simple(requirements: Vec<Requirement>) -> Self {
        Self {
            requirements,
            constraints: Constraints::default(),
            overrides: Overrides::default(),
            excludes: Excludes::default(),
            preferences: Preferences::default(),
            project: None,
            exclusions: Exclusions::default(),
            workspace_members: BTreeSet::new(),
            lookaheads: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_constraints(mut self, constraints: Constraints) -> Self {
        self.constraints = constraints;
        self
    }

    #[must_use]
    pub fn with_lookaheads(mut self, lookaheads: Vec<RequestedRequirements>) -> Self {
        self.lookaheads = lookaheads;
        self
    }

    /// Return an iterator over all requirements, constraints, and overrides, in priority order,
    /// such that requirements come first, followed by constraints, followed by overrides.
    ///
    /// At time of writing, this is used for:
    /// - Determining which requirements should allow yanked versions.
    /// - Determining which requirements should allow pre-release versions (e.g., `torch>=2.2.0a1`).
    /// - Determining which requirements should allow direct URLs (e.g., `torch @ https://...`).
    pub(crate) fn requirements<'a>(
        &'a self,
        env: &'a ResolverEnvironment,
        mode: DependencyMode,
    ) -> impl Iterator<Item = Cow<'a, Requirement>> + 'a {
        self.requirements_no_overrides(env, mode)
            .chain(self.overrides(env))
    }

    /// Return all requirements that affect manifest-wide candidate selection policy.
    ///
    /// Scoped overrides are included even when their scope is not selected. Whether a scoped
    /// override applies is only known during resolution, after yanked-version policy has already
    /// been initialized.
    pub(crate) fn candidate_selection_requirements<'a>(
        &'a self,
        env: &'a ResolverEnvironment,
        mode: DependencyMode,
    ) -> impl Iterator<Item = Cow<'a, Requirement>> + 'a {
        self.requirements(env, mode).chain(
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
                .map(|(_, _, requirement)| Cow::Borrowed(requirement))
                .filter(move |requirement| {
                    requirement.evaluate_markers(env.marker_environment(), &[])
                }),
        )
    }

    /// Like [`Self::requirements`], but without the overrides.
    pub(crate) fn requirements_no_overrides<'a>(
        &'a self,
        env: &'a ResolverEnvironment,
        mode: DependencyMode,
    ) -> impl Iterator<Item = Cow<'a, Requirement>> + 'a {
        match mode {
            // Include all direct and transitive requirements, with constraints and overrides applied.
            DependencyMode::Transitive => Either::Left(
                self.lookaheads
                    .iter()
                    .flat_map(move |lookahead| {
                        self.overrides
                            .apply_for(
                                lookahead.package(),
                                lookahead.version(),
                                lookahead.requirements(),
                            )
                            .filter(|requirement| {
                                !self.excludes.contains_for(
                                    lookahead.package(),
                                    lookahead.version(),
                                    &requirement.name,
                                )
                            })
                            .filter(move |requirement| {
                                requirement
                                    .evaluate_markers(env.marker_environment(), lookahead.extras())
                            })
                    })
                    .chain(
                        self.overrides
                            .apply(&self.requirements)
                            .filter(|requirement| !self.excludes.contains(&requirement.name))
                            .filter(move |requirement| {
                                requirement.evaluate_markers(env.marker_environment(), &[])
                            }),
                    )
                    .chain(
                        self.constraints
                            .requirements()
                            .filter(|requirement| !self.excludes.contains(&requirement.name))
                            .filter(move |requirement| {
                                requirement.evaluate_markers(env.marker_environment(), &[])
                            })
                            .map(Cow::Borrowed),
                    ),
            ),
            // Include direct requirements, with constraints and overrides applied.
            DependencyMode::Direct => Either::Right(
                self.overrides
                    .apply(&self.requirements)
                    .chain(self.constraints.requirements().map(Cow::Borrowed))
                    .filter(|requirement| !self.excludes.contains(&requirement.name))
                    .filter(move |requirement| {
                        requirement.evaluate_markers(env.marker_environment(), &[])
                    }),
            ),
        }
    }

    /// Only the overrides from [`Self::requirements`].
    pub(crate) fn overrides<'a>(
        &'a self,
        env: &'a ResolverEnvironment,
    ) -> impl Iterator<Item = Cow<'a, Requirement>> + 'a {
        self.overrides
            .global_requirements()
            .filter(|requirement| !self.excludes.contains(&requirement.name))
            .filter(move |requirement| requirement.evaluate_markers(env.marker_environment(), &[]))
            .map(Cow::Borrowed)
    }

    /// Return an iterator over the names of all user-provided requirements.
    ///
    /// This includes:
    /// - Direct requirements
    /// - Dependencies of editable requirements
    /// - Transitive dependencies of local package requirements
    ///
    /// At time of writing, this is used for:
    /// - Determining which packages should use the "lowest-compatible version" of a package, when
    ///   the `lowest-direct` strategy is in use.
    pub(crate) fn user_requirements<'a>(
        &'a self,
        env: &'a ResolverEnvironment,
        mode: DependencyMode,
    ) -> impl Iterator<Item = Cow<'a, Requirement>> + 'a {
        match mode {
            // Include direct requirements, dependencies of editables, and transitive dependencies
            // of local packages.
            DependencyMode::Transitive => Either::Left(
                self.lookaheads
                    .iter()
                    .filter(|lookahead| lookahead.direct())
                    .flat_map(move |lookahead| {
                        self.overrides
                            .apply_for(
                                lookahead.package(),
                                lookahead.version(),
                                lookahead.requirements(),
                            )
                            .filter(|requirement| {
                                !self.excludes.contains_for(
                                    lookahead.package(),
                                    lookahead.version(),
                                    &requirement.name,
                                )
                            })
                            .filter(move |requirement| {
                                requirement
                                    .evaluate_markers(env.marker_environment(), lookahead.extras())
                            })
                    })
                    .chain(
                        self.overrides
                            .apply(&self.requirements)
                            .filter(move |requirement| {
                                requirement.evaluate_markers(env.marker_environment(), &[])
                            }),
                    ),
            ),

            // Restrict to the direct requirements.
            DependencyMode::Direct => {
                Either::Right(self.overrides.apply(self.requirements.iter()).filter(
                    move |requirement| requirement.evaluate_markers(env.marker_environment(), &[]),
                ))
            }
        }
    }

    /// Returns the number of input requirements.
    pub fn num_requirements(&self) -> usize {
        self.requirements.len()
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::error::Error;

    use uv_configuration::{Constraints, Excludes, Override, Overrides, PackageOverride};
    use uv_distribution_types::Requirement;
    use uv_git::GitResolver;
    use uv_pep508::{MarkerEnvironment, MarkerEnvironmentBuilder};
    use uv_pypi_types::VerbatimParsedUrl;
    use uv_types::RequestedRequirements;

    use crate::resolver::Urls;
    use crate::{DependencyMode, Manifest, ResolverEnvironment};

    type TestResult<T = ()> = Result<T, Box<dyn Error>>;

    fn requirement(value: &str) -> TestResult<Requirement> {
        Ok(value
            .parse::<uv_pep508::Requirement<VerbatimParsedUrl>>()?
            .into())
    }

    fn linux() -> TestResult<ResolverEnvironment> {
        let markers = MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
            implementation_name: "cpython",
            implementation_version: "3.12.0",
            os_name: "posix",
            platform_machine: "x86_64",
            platform_python_implementation: "CPython",
            platform_release: "6.0",
            platform_system: "Linux",
            platform_version: "fixture",
            python_full_version: "3.12.0",
            python_version: "3.12",
            sys_platform: "linux",
        })?;
        Ok(ResolverEnvironment::specific(markers.into()))
    }

    #[test]
    fn global_overrides_preserve_order_in_both_dependency_modes() -> TestResult {
        let direct = requirement("direct==1")?;
        let constraint = requirement("constrained<4")?;
        let lookahead = requirement("lookahead==2")?;
        let first = requirement("demo>=1")?;
        let second = requirement("demo<4 ; sys_platform == 'linux'")?;
        let third = requirement("demo!=2 ; sys_platform == 'win32'")?;
        let scoped = requirement("scoped-only==9")?;
        let mut manifest = Manifest::simple(vec![direct.clone()])
            .with_constraints(Constraints::from_requirements(std::iter::once(
                constraint.clone(),
            )))
            .with_lookaheads(vec![RequestedRequirements::new(
                "parent".parse()?,
                "1.0".parse()?,
                Box::new([]),
                vec![lookahead.clone()].into_boxed_slice(),
                true,
            )]);
        manifest.overrides = Overrides::from_entries(vec![
            Override::Requirement(first.clone()),
            Override::Requirement(requirement("blocked==7")?),
            Override::Requirement(second.clone()),
            Override::Package(PackageOverride {
                package: serde_json::from_str(r#"{"name":"parent","version":"1.0"}"#)?,
                dependencies: vec![scoped.clone()].into_boxed_slice(),
            }),
            Override::Requirement(third.clone()),
        ])?;
        manifest.excludes = Excludes::from_iter(["blocked".parse()?]);

        for (env, overrides) in [
            (linux()?, vec![first.clone(), second.clone()]),
            (
                ResolverEnvironment::universal(Vec::new()),
                vec![first, second, third],
            ),
        ] {
            for mode in [DependencyMode::Direct, DependencyMode::Transitive] {
                let mut expected = match mode {
                    DependencyMode::Direct => Vec::new(),
                    DependencyMode::Transitive => vec![lookahead.clone(), scoped.clone()],
                };
                expected.extend([direct.clone(), constraint.clone()]);
                expected.extend(overrides.iter().cloned());
                assert_eq!(
                    manifest
                        .requirements(&env, mode)
                        .map(Cow::into_owned)
                        .collect::<Vec<_>>(),
                    expected,
                );

                // Scoped overrides are also included in candidate-selection policy, but not
                // in the global overrides appended to the ordinary requirement iterator.
                expected.push(scoped.clone());
                assert_eq!(
                    manifest
                        .candidate_selection_requirements(&env, mode)
                        .map(Cow::into_owned)
                        .collect::<Vec<_>>(),
                    expected,
                );
            }
        }
        Ok(())
    }

    #[test]
    fn global_url_overrides_apply_in_both_dependency_modes() -> TestResult {
        let original = requirement("demo @ https://example.invalid/demo-1.0-py3-none-any.whl")?;
        let constraint = requirement("demo @ https://example.invalid/demo-2.0-py3-none-any.whl")?;
        let replacement = requirement("demo @ https://example.invalid/demo-3.0-py3-none-any.whl")?;
        let blocked =
            requirement("blocked @ https://example.invalid/blocked-1.0-py3-none-any.whl")?;
        let mut manifest = Manifest::simple(vec![original.clone(), blocked.clone()])
            .with_constraints(Constraints::from_requirements(std::iter::once(constraint)));
        manifest.overrides = Overrides::from_requirements(vec![replacement.clone(), blocked]);
        manifest.excludes = Excludes::from_iter(["blocked".parse()?]);
        let original_url = original
            .source
            .to_verbatim_parsed_url()
            .ok_or("fixture must be a URL")?;
        let replacement_url = replacement
            .source
            .to_verbatim_parsed_url()
            .ok_or("fixture must be a URL")?;
        let git = GitResolver::default();

        for env in [linux()?, ResolverEnvironment::universal(Vec::new())] {
            for mode in [DependencyMode::Direct, DependencyMode::Transitive] {
                let urls = Urls::from_manifest(&manifest, &env, &git, mode);
                assert!(urls.any_url(&original.name));
                assert!(!urls.any_url(&"blocked".parse()?));
                for requested_url in [None, Some(&original_url)] {
                    assert_eq!(
                        urls.get_url(&env, &original.name, requested_url, &git)?
                            .map(|url| url.verbatim.to_string())
                            .collect::<Vec<_>>(),
                        [replacement_url.verbatim.to_string()],
                    );
                }
            }
        }
        Ok(())
    }
}
