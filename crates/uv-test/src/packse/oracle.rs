//! A deliberately small, exhaustive resolver oracle for Packse scenarios.
//!
//! The oracle checks satisfiability and dependency closure in one concrete marker environment.
//! It does not implement uv's version preferences, universal forking, or artifact selection. Inputs
//! outside its supported subset are rejected rather than silently compared with different rules.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;

use anyhow::{Result, bail, ensure};

use uv_normalize::{ExtraName, PackageName};
use uv_pep440::{Version, VersionSpecifiers, release_specifiers_to_ranges};
use uv_pep508::{MarkerEnvironment, MarkerExpression, Requirement, VersionOrUrl};

use super::domain::lock_environment_marker;
use super::scenario::Scenario;

/// One selected version per reachable package.
pub type Selection = BTreeMap<PackageName, Version>;

/// The result of enumerating a finite scenario's possible selections.
#[derive(Debug)]
pub struct SearchResult {
    /// A valid selection, if one exists. This is not necessarily uv's preferred selection.
    pub solution: Option<Selection>,
    /// The number of complete selections examined.
    pub checked: usize,
}

/// An independent, finite-domain checker for a Packse dependency graph.
pub struct ScenarioOracle<'a> {
    scenario: &'a Scenario,
    environment: &'a MarkerEnvironment,
    root_requirements: Vec<Requirement>,
}

impl<'a> ScenarioOracle<'a> {
    /// Validate that the graph only uses policies understood by the oracle.
    ///
    /// Non-yanked versions with platform-independent wheels are supported, including pre-releases
    /// under the default stable-first policy or global pre-release opt-in. Dependencies may have
    /// ordinary environment markers and additive extras, including recursive extras. Dependency
    /// `Requires-Python` is interpreted as a lower bound, as documented by uv.
    pub fn new(scenario: &'a Scenario, environment: &'a MarkerEnvironment) -> Result<Self> {
        ensure!(
            !scenario.root.has_project_dependencies(),
            "the scenario oracle requires an explicit project selection for optional dependencies or groups"
        );
        Self::with_root_requirements(scenario, environment, scenario.root.requires.clone())
    }

    pub(super) fn with_root_requirements(
        scenario: &'a Scenario,
        environment: &'a MarkerEnvironment,
        root_requirements: Vec<Requirement>,
    ) -> Result<Self> {
        validate_scenario(scenario, &root_requirements)?;
        ensure!(
            scenario
                .root
                .requires_python
                .as_ref()
                .is_none_or(|specifier| {
                    specifier.contains(&environment.python_full_version().only_release())
                }),
            "the root does not support Python {}",
            environment.python_full_version()
        );
        ensure!(
            lock_environment_marker(scenario)?.evaluate(environment, &[]),
            "the marker environment is outside the scenario's supported lock environments"
        );

        Ok(Self {
            scenario,
            environment,
            root_requirements,
        })
    }

    /// Check that a selection is exactly the reachable, compatible dependency closure.
    pub fn validate(&self, selection: &Selection) -> Result<()> {
        let active = self.activated_packages(selection)?;
        if let Some(name) = selection.keys().find(|name| !active.contains_key(*name)) {
            bail!("the selection contains unreachable package `{name}`");
        }
        Ok(())
    }

    /// Project a proposed version assignment to its compatible dependency closure.
    ///
    /// This does not choose versions. Every reachable package must already have a compatible
    /// version in `assignment`; unreachable entries are ignored. The result can be passed to
    /// [`Self::validate`] as an independent satisfiability witness.
    pub fn reachable_selection(&self, assignment: &Selection) -> Result<Selection> {
        let active = self.activated_packages(assignment)?;
        Ok(active
            .into_keys()
            .map(|name| {
                let version = assignment[&name].clone();
                (name, version)
            })
            .collect())
    }

    fn activated_packages(
        &self,
        selection: &Selection,
    ) -> Result<BTreeMap<PackageName, BTreeSet<ExtraName>>> {
        let mut active: BTreeMap<PackageName, BTreeSet<ExtraName>> = BTreeMap::new();
        for requirement in &self.root_requirements {
            if requirement.evaluate_markers(self.environment, &[]) {
                self.activate(requirement, selection, &mut active)?;
            }
        }

        loop {
            let mut requirements = Vec::new();
            for (name, extras) in &active {
                let version = &selection[name];
                let metadata = &self.scenario.packages[name].versions[version];
                let extras = extras.iter().cloned().collect::<Vec<_>>();
                requirements.extend(
                    metadata
                        .requires
                        .iter()
                        .chain(
                            extras
                                .iter()
                                .filter_map(|extra| metadata.extras.get(extra))
                                .flatten(),
                        )
                        .filter(|requirement| {
                            requirement.evaluate_markers(self.environment, &extras)
                        }),
                );
            }

            let mut changed = false;
            for requirement in requirements {
                changed |= self.activate(requirement, selection, &mut active)?;
            }
            if !changed {
                break;
            }
        }

        Ok(active)
    }

    /// Exhaustively search every possible selection, subject to a strict search-space bound.
    ///
    /// The bound includes the absent choice for each package. Exceeding it is an error, not a
    /// claim that the scenario is unsatisfiable.
    pub fn find_solution(&self, max_states: usize) -> Result<SearchResult> {
        let mut candidates = Vec::new();
        let mut states = 1_usize;
        for (name, package) in &self.scenario.packages {
            let versions = package
                .versions
                .iter()
                .filter(|(_, metadata)| {
                    dependency_supports_python(
                        metadata.requires_python.as_ref(),
                        self.environment.python_full_version(),
                    )
                })
                .map(|(version, _)| version)
                .collect::<Vec<_>>();
            states = states
                .checked_mul(versions.len() + 1)
                .filter(|states| *states <= max_states)
                .ok_or_else(|| {
                    anyhow::anyhow!("scenario search space exceeds {max_states} selections")
                })?;
            candidates.push((name, versions));
        }
        ensure!(
            states <= max_states,
            "scenario search space exceeds {max_states} selections"
        );

        let mut checked = 0;
        let solution = self.search(&candidates, &mut Selection::new(), &mut checked);
        Ok(SearchResult { solution, checked })
    }

    fn search(
        &self,
        candidates: &[(&PackageName, Vec<&Version>)],
        selection: &mut Selection,
        checked: &mut usize,
    ) -> Option<Selection> {
        let Some(((name, versions), remaining)) = candidates.split_first() else {
            *checked += 1;
            return self.validate(selection).is_ok().then(|| selection.clone());
        };

        if let Some(solution) = self.search(remaining, selection, checked) {
            return Some(solution);
        }
        for version in versions.iter().rev() {
            selection.insert((*name).clone(), (*version).clone());
            if let Some(solution) = self.search(remaining, selection, checked) {
                return Some(solution);
            }
        }
        selection.remove(*name);
        None
    }

    fn activate(
        &self,
        requirement: &Requirement,
        selection: &Selection,
        active: &mut BTreeMap<PackageName, BTreeSet<ExtraName>>,
    ) -> Result<bool> {
        let Some(version) = selection.get(&requirement.name) else {
            bail!("the selection is missing `{requirement}`");
        };
        let Some(metadata) = self
            .scenario
            .packages
            .get(&requirement.name)
            .and_then(|package| package.versions.get(version))
        else {
            bail!("the scenario has no {}=={version}", requirement.name);
        };
        if let Some(specifier) = &requirement.version_or_url {
            match specifier {
                VersionOrUrl::VersionSpecifier(specifier) => ensure!(
                    specifier.contains(version),
                    "{}=={version} does not satisfy `{requirement}`",
                    requirement.name
                ),
                VersionOrUrl::Url(_) => {
                    bail!("the scenario oracle does not model direct URLs: {requirement}");
                }
            }
        }
        ensure!(
            dependency_supports_python(
                metadata.requires_python.as_ref(),
                self.environment.python_full_version(),
            ),
            "{}=={version} does not support Python {}",
            requirement.name,
            self.environment.python_full_version()
        );

        let mut changed = !active.contains_key(&requirement.name);
        let extras = active.entry(requirement.name.clone()).or_default();
        for extra in &requirement.extras {
            changed |= extras.insert(extra.clone());
        }
        Ok(changed)
    }
}

/// Validate the policy subset without choosing a marker environment.
pub(super) fn validate_scenario(
    scenario: &Scenario,
    root_requirements: &[Requirement],
) -> Result<()> {
    ensure!(
        scenario.resolver_options.no_binary.is_empty(),
        "the scenario oracle does not model source-build requirements"
    );
    ensure!(
        scenario.resolver_options.required_environments.is_empty(),
        "the scenario oracle checks one environment at a time"
    );
    lock_environment_marker(scenario)?;

    for requirement in root_requirements {
        validate_requirement(requirement)?;
    }
    for (name, package) in &scenario.packages {
        for (version, metadata) in &package.versions {
            ensure!(
                !metadata.yanked,
                "the scenario oracle does not model yanked releases: {name}=={version}"
            );
            ensure!(
                metadata.wheel.is_some()
                    && (metadata.wheel_tags.is_empty()
                        || metadata
                            .wheel_tags
                            .iter()
                            .all(|tag| tag.as_str() == "py3-none-any")),
                "the scenario oracle requires universal wheels: {name}=={version}"
            );
            ensure!(
                metadata.requires_python.as_ref().is_none_or(|specifier| {
                    !release_specifiers_to_ranges(specifier.clone()).is_empty()
                }),
                "the scenario oracle does not model empty Requires-Python ranges: {name}=={version}"
            );
            for requirement in metadata
                .requires
                .iter()
                .chain(metadata.extras.values().flatten())
            {
                validate_requirement(requirement)?;
            }
        }
    }
    Ok(())
}

/// Apply uv's documented lower-bound-only policy to dependency metadata.
///
/// The root's Python range remains a full constraint. Taking the first interval boundary also
/// handles exclusions that raise the minimum version, such as `>=3.12,!=3.12.*`.
fn dependency_supports_python(specifiers: Option<&VersionSpecifiers>, python: &Version) -> bool {
    let Some(specifiers) = specifiers else {
        return true;
    };
    let range = release_specifiers_to_ranges(specifiers.clone());
    let Some((lower, _)) = range.bounding_range() else {
        // Empty ranges are rejected when the oracle is constructed.
        return false;
    };
    let python = python.only_release();
    match lower {
        Bound::Included(lower) => python >= *lower,
        Bound::Excluded(lower) => python > *lower,
        Bound::Unbounded => true,
    }
}

pub(super) fn validate_requirement(requirement: &Requirement) -> Result<()> {
    ensure!(
        !matches!(requirement.version_or_url, Some(VersionOrUrl::Url(_))),
        "the scenario oracle does not model direct URLs: {requirement}"
    );
    ensure!(
        !requirement
            .marker
            .to_dnf()
            .iter()
            .flatten()
            .any(|expression| matches!(expression, MarkerExpression::List { .. })),
        "the scenario oracle does not model PEP 751 list markers: {requirement}"
    );

    // Resolving additional extras must only add requirements. Check the Boolean function instead
    // of inspecting DNF operators, which can contain negated edges for a monotone disjunction.
    let mut extras = BTreeSet::new();
    requirement
        .marker
        .visit_extras(|_, name| _ = extras.insert(name.clone()));
    for extra in extras {
        let without_extra = requirement
            .marker
            .simplify_not_extras_with(|name| *name == extra);
        let with_extra = requirement
            .marker
            .simplify_extras_with(|name| *name == extra);
        ensure!(
            without_extra.implies(with_extra).is_true(),
            "the scenario oracle does not model non-additive extras: {requirement}"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use uv_pep508::MarkerEnvironmentBuilder;

    use super::*;

    fn environment() -> MarkerEnvironment {
        environment_for_python("3.12.0", "3.12")
    }

    fn environment_for_python(full_version: &str, version: &str) -> MarkerEnvironment {
        MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
            implementation_name: "cpython",
            implementation_version: full_version,
            os_name: "posix",
            platform_machine: "x86_64",
            platform_python_implementation: "CPython",
            platform_release: "",
            platform_system: "Linux",
            platform_version: "",
            python_full_version: full_version,
            python_version: version,
            sys_platform: "linux",
        })
        .expect("valid marker environment")
    }

    fn selection(packages: &[(&str, &str)]) -> Selection {
        packages
            .iter()
            .map(|(name, version)| {
                (
                    PackageName::from_str(name).expect("valid package name"),
                    Version::from_str(version).expect("valid version"),
                )
            })
            .collect()
    }

    fn scenario(contents: &str) -> Scenario {
        toml::from_str(contents).expect("valid scenario")
    }

    #[test]
    fn accepts_prereleases_without_imposing_stable_preference() -> Result<()> {
        let mut scenario = scenario(
            r#"
name = "prerelease-candidates"
[root]
requires = ["a>=1"]
[expected]
satisfiable = true
[packages.a.versions."1"]
[packages.a.versions."2rc1"]
"#,
        );
        let environment = environment();
        for prereleases in [false, true] {
            scenario.resolver_options.prereleases = prereleases;
            let oracle = ScenarioOracle::new(&scenario, &environment)?;
            oracle.validate(&selection(&[("a", "1")]))?;
            oracle.validate(&selection(&[("a", "2rc1")]))?;
            assert_eq!(
                oracle.find_solution(3)?.solution,
                Some(selection(&[("a", "2rc1")]))
            );
        }
        Ok(())
    }

    #[test]
    fn checks_prerelease_dependency_closure() -> Result<()> {
        let scenario = scenario(
            r#"
name = "prerelease-closure"
[root]
requires = ["a"]
[expected]
satisfiable = true
[packages.a.versions."1rc1"]
requires = ["b==2rc1"]
[packages.a.versions."2"]
requires = ["missing"]
[packages.b.versions."1"]
[packages.b.versions."2rc1"]
"#,
        );
        let environment = environment();
        let oracle = ScenarioOracle::new(&scenario, &environment)?;
        let valid = selection(&[("a", "1rc1"), ("b", "2rc1")]);
        oracle.validate(&valid)?;
        assert!(oracle.validate(&selection(&[("a", "1rc1")])).is_err());
        assert!(
            oracle
                .validate(&selection(&[("a", "1rc1"), ("b", "1")]))
                .is_err()
        );
        assert_eq!(oracle.find_solution(9)?.solution, Some(valid));
        Ok(())
    }

    #[test]
    fn searches_cycles_and_backtracks() -> Result<()> {
        let scenario = scenario(
            r#"
name = "cycle"
[root]
requires = ["a", "b"]
[expected]
satisfiable = true
[packages.a.versions."1"]
requires = ["b==2"]
[packages.a.versions."2"]
requires = ["b==1"]
[packages.b.versions."1"]
requires = ["a==1"]
[packages.b.versions."2"]
requires = ["a==1"]
"#,
        );
        let environment = environment();
        let oracle = ScenarioOracle::new(&scenario, &environment)?;
        let result = oracle.find_solution(9)?;
        assert_eq!(result.solution, Some(selection(&[("a", "1"), ("b", "2")])));
        assert!(result.checked <= 9);
        insta::assert_snapshot!(
            oracle.find_solution(8).expect_err("search should be bounded"),
            @"scenario search space exceeds 8 selections"
        );
        Ok(())
    }

    #[test]
    fn requires_the_exact_dependency_closure() -> Result<()> {
        let scenario = scenario(
            r#"
name = "closure"
[root]
requires = ["a==1"]
[expected]
satisfiable = true
[packages.a.versions."1"]
requires = ["b>=2"]
[packages.b.versions."1"]
[packages.b.versions."2"]
[packages.unused.versions."1"]
"#,
        );
        let environment = environment();
        let oracle = ScenarioOracle::new(&scenario, &environment)?;
        insta::assert_snapshot!(
            oracle.validate(&selection(&[("a", "1")])).expect_err("missing dependency"),
            @"the selection is missing `b>=2`"
        );
        insta::assert_snapshot!(
            oracle.validate(&selection(&[("a", "1"), ("b", "1")])).expect_err("incompatible dependency"),
            @"b==1 does not satisfy `b>=2`"
        );
        insta::assert_snapshot!(
            oracle.validate(&selection(&[("a", "1"), ("b", "2"), ("unused", "1")])).expect_err("unreachable dependency"),
            @"the selection contains unreachable package `unused`"
        );
        oracle.validate(&selection(&[("a", "1"), ("b", "2")]))?;
        Ok(())
    }

    #[test]
    fn projects_witnesses_to_reachable_packages() -> Result<()> {
        let scenario = scenario(
            r#"
name = "witness-closure"
[root]
requires = ["a[all]"]
[expected]
satisfiable = true
[packages.a.versions."1"]
requires = ["b>=2"]
[packages.a.versions."1".extras]
all = ["a[feature]"]
feature = ["c; python_version >= '3.13'"]
[packages.b.versions."1"]
[packages.b.versions."2"]
[packages.c.versions."1"]
[packages.unused.versions."1"]
"#,
        );
        let assignment = selection(&[("a", "1"), ("b", "2"), ("c", "1"), ("unused", "1")]);
        let environment = environment();
        let oracle = ScenarioOracle::new(&scenario, &environment)?;
        let projected = oracle.reachable_selection(&assignment)?;
        assert_eq!(projected, selection(&[("a", "1"), ("b", "2")]));
        oracle.validate(&projected)?;
        insta::assert_snapshot!(
            oracle.validate(&assignment).expect_err("the exact selection rejects unused pins"),
            @"the selection contains unreachable package `c`"
        );

        let environment = environment_for_python("3.13.0", "3.13");
        let oracle = ScenarioOracle::new(&scenario, &environment)?;
        let projected = oracle.reachable_selection(&assignment)?;
        assert_eq!(projected, selection(&[("a", "1"), ("b", "2"), ("c", "1")]));
        oracle.validate(&projected)?;
        insta::assert_snapshot!(
            oracle.reachable_selection(&selection(&[("a", "1"), ("b", "2")]))
                .expect_err("the witness must include activated dependencies"),
            @"the selection is missing `c ; python_full_version >= '3.13'`"
        );
        insta::assert_snapshot!(
            oracle.reachable_selection(&selection(&[("a", "1"), ("b", "1"), ("c", "1")]))
                .expect_err("the witness must satisfy dependency constraints"),
            @"b==1 does not satisfy `b>=2`"
        );
        Ok(())
    }

    #[test]
    fn follows_additive_and_recursive_extras() -> Result<()> {
        let scenario = scenario(
            r#"
name = "extras"
[root]
requires = ["a[all]"]
[expected]
satisfiable = true
[packages.a.versions."1"]
requires = ["b; extra == 'left' or extra == 'right'"]
[packages.a.versions."1".extras]
all = ["a[left]", "a[right]"]
left = ["c; sys_platform == 'win32'"]
right = ["c; python_version >= '3.13'"]
[packages.b.versions."1"]
[packages.c.versions."1"]
"#,
        );
        let environment = environment();
        let oracle = ScenarioOracle::new(&scenario, &environment)?;
        assert_eq!(
            oracle.find_solution(8)?.solution,
            Some(selection(&[("a", "1"), ("b", "1")]))
        );
        Ok(())
    }

    #[test]
    fn proves_unsatisfiability() -> Result<()> {
        let scenario = scenario(
            r#"
name = "unsatisfiable"
[root]
requires = ["a"]
[expected]
satisfiable = false
[packages.a.versions."1"]
requires = ["b==2"]
[packages.b.versions."1"]
[packages.b.versions."2"]
requires_python = ">=3.13"
"#,
        );
        let environment = environment();
        let oracle = ScenarioOracle::new(&scenario, &environment)?;
        let result = oracle.find_solution(4)?;
        assert_eq!(result.solution, None);
        assert_eq!(result.checked, 4);
        Ok(())
    }

    #[test]
    fn ignores_dependency_python_upper_bounds() -> Result<()> {
        let mut scenario = scenario(
            r#"
name = "python-upper-bound"
[root]
requires_python = ">=3.12,<3.15"
requires = ["node-0==2"]
[expected]
satisfiable = true
[packages.node-0.versions."2"]
requires = ["node-2==2"]
[packages.node-2.versions."2"]
requires_python = ">=3.12,<3.13"
"#,
        );
        let environment = environment_for_python("3.14.0", "3.14");
        let expected = selection(&[("node-0", "2"), ("node-2", "2")]);
        let oracle = ScenarioOracle::new(&scenario, &environment)?;
        assert_eq!(oracle.find_solution(4)?.solution, Some(expected.clone()));
        oracle.validate(&expected)?;

        scenario.root.requires_python = Some(">=3.12,<3.14".parse()?);
        insta::assert_snapshot!(
            ScenarioOracle::new(&scenario, &environment)
                .err()
                .expect("the root upper bound is enforced before searching"),
            @"the root does not support Python 3.14.0"
        );
        Ok(())
    }

    #[test]
    fn compares_dependency_python_lower_bounds() -> Result<()> {
        let cases = [
            ("<=3.8", "3.12", true),
            (">=3.12,<3.13", "3.14", true),
            ("==3.12.*", "3.14", true),
            ("~=3.12", "3.14", true),
            (">=3.13", "3.12", false),
            (">3.12", "3.12", false),
            (">3.12", "3.12.1", true),
            (">=3.12,!=3.12.*", "3.12.1", false),
            (">=3.12,!=3.12.*", "3.13", true),
            (">=3.12,!=3.13.*", "3.13", true),
            (">=3.13", "3.13.0rc1", true),
        ];
        for (specifier, python, expected) in cases {
            assert_eq!(
                dependency_supports_python(Some(&specifier.parse()?), &python.parse()?),
                expected,
                "{specifier} on Python {python}"
            );
        }
        Ok(())
    }

    #[test]
    fn compares_root_python_release_versions() -> Result<()> {
        let scenario = scenario(
            r#"
name = "python-prerelease"
[root]
requires_python = ">=3.13,<3.14"
requires = ["a"]
[expected]
satisfiable = true
[packages.a.versions."1"]
requires_python = ">=3.13"
"#,
        );
        let environment = environment_for_python("3.13.0rc1", "3.13");
        let oracle = ScenarioOracle::new(&scenario, &environment)?;
        assert_eq!(
            oracle.find_solution(2)?.solution,
            Some(selection(&[("a", "1")]))
        );
        Ok(())
    }

    #[test]
    fn rejects_empty_dependency_python_ranges() {
        let scenario = scenario(
            r#"
name = "empty-python-range"
[root]
requires = ["a"]
[expected]
satisfiable = false
[packages.a.versions."1"]
requires_python = ">=3.13,<3.12"
"#,
        );
        let environment = environment();
        let error = ScenarioOracle::new(&scenario, &environment)
            .err()
            .expect("empty Python ranges are not modeled");
        insta::assert_snapshot!(error, @"the scenario oracle does not model empty Requires-Python ranges: a==1");
    }

    #[test]
    fn rejects_non_additive_extras() {
        let scenario = scenario(
            r#"
name = "negative-extra"
[root]
requires = ["a"]
[expected]
satisfiable = true
[packages.a.versions."1"]
requires = ["b; extra != 'gpu'"]
"#,
        );
        let environment = environment();
        let error = ScenarioOracle::new(&scenario, &environment)
            .err()
            .expect("negative extras are not modeled");
        insta::assert_snapshot!(error, @"the scenario oracle does not model non-additive extras: b ; extra != 'gpu'");
    }

    #[test]
    fn rejects_unmodeled_candidate_policies() {
        let environment = environment();
        let cases = [
            ("yanked", "[packages.a.versions.\"1\"]\nyanked = true"),
            (
                "platform wheel",
                "[packages.a.versions.\"1\"]\nwheel_tags = [\"cp312-abi3-win_amd64\"]",
            ),
            ("source build", "[packages.a.versions.\"1\"]\nwheel = false"),
            (
                "direct URL",
                "[packages.a.versions.\"1\"]\nrequires = [\"b @ https://example.org/b.whl\"]",
            ),
            (
                "PEP 751 marker",
                "[packages.a.versions.\"1\"]\nrequires = [\"b; 'dev' in dependency_groups\"]",
            ),
        ];
        let errors = cases
            .into_iter()
            .map(|(name, package)| {
                let scenario = scenario(&format!(
                    "name = \"unsupported\"\n[root]\nrequires = [\"a\"]\n[expected]\nsatisfiable = true\n{package}"
                ));
                let error = ScenarioOracle::new(&scenario, &environment)
                    .err()
                    .expect("policy should be unsupported");
                format!("{name}: {error}")
            })
            .collect::<Vec<_>>()
            .join("\n");
        insta::assert_snapshot!(errors, @"
        yanked: the scenario oracle does not model yanked releases: a==1
        platform wheel: the scenario oracle requires universal wheels: a==1
        source build: the scenario oracle requires universal wheels: a==1
        direct URL: the scenario oracle does not model direct URLs: b @ https://example.org/b.whl
        PEP 751 marker: the scenario oracle does not model PEP 751 list markers: b ; 'dev' in dependency_groups
        ");
    }

    #[test]
    fn rejects_projections_outside_restricted_lock_environments() {
        let mut scenario = scenario(
            r#"
name = "restricted"
[root]
requires = []
[expected]
satisfiable = true
[resolver_options]
environments = ["sys_platform == 'win32'"]
"#,
        );
        let environment = environment();
        let error = ScenarioOracle::new(&scenario, &environment)
            .err()
            .expect("the Linux projection is outside the supported domain");
        insta::assert_snapshot!(error, @"the marker environment is outside the scenario's supported lock environments");
        scenario.resolver_options.environments =
            vec!["sys_platform == 'linux'".parse().expect("valid marker")];
        ScenarioOracle::new(&scenario, &environment)
            .expect("the Linux projection is in the supported domain");
        let environment = environment_for_python("3.11.0", "3.11");
        let error = ScenarioOracle::new(&scenario, &environment)
            .err()
            .expect("the root Python range is checked before searching");
        insta::assert_snapshot!(error, @"the root does not support Python 3.11.0");
    }
}
