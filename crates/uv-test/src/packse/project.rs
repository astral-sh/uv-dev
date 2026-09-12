//! Explicit project-extra and dependency-group projections for scenario checks.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use anyhow::{Context, Result, bail, ensure};

use uv_normalize::{ExtraName, GroupName, PackageName};
use uv_pep508::{MarkerEnvironment, Requirement};
use uv_pypi_types::{DependencyGroupSpecifier, DependencyGroups};

use super::oracle::{ScenarioOracle, validate_requirement};
use super::scenario::Scenario;

const MAX_EXPANDED_GROUP_REQUIREMENTS: usize = 100_000;
const MAX_GROUP_DEPTH: usize = 128;

/// The root requirements included in one explicit project export.
///
/// Default groups are not implicit: callers select every group they want to include.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectSelection {
    pub include_project: bool,
    pub extras: BTreeSet<ExtraName>,
    pub groups: BTreeSet<GroupName>,
}

impl Default for ProjectSelection {
    fn default() -> Self {
        Self {
            include_project: true,
            extras: BTreeSet::new(),
            groups: BTreeSet::new(),
        }
    }
}

impl fmt::Display for ProjectSelection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(if self.include_project {
            "project"
        } else {
            "groups only"
        })?;
        if !self.extras.is_empty() {
            write!(
                formatter,
                "; extras: {}",
                self.extras
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )?;
        }
        if !self.groups.is_empty() {
            write!(
                formatter,
                "; groups: {}",
                self.groups
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )?;
        }
        Ok(())
    }
}

/// A project root with independently expanded PEP 735 dependency groups.
pub struct ScenarioProject<'a> {
    scenario: &'a Scenario,
    name: PackageName,
    groups: BTreeMap<GroupName, Vec<Requirement>>,
}

impl<'a> ScenarioProject<'a> {
    /// Validate every project root, including unselected extras and groups.
    ///
    /// Project self-references, root `extra` markers, and conflict declarations are outside this
    /// checker's contract. Ordinary environment markers and package extras remain supported.
    pub fn new(scenario: &'a Scenario) -> Result<Self> {
        let name = project_name(scenario)?;
        let groups = match &scenario.root.dependency_groups {
            Some(groups) => flatten_groups(groups, MAX_EXPANDED_GROUP_REQUIREMENTS)?,
            None => BTreeMap::new(),
        };
        for requirement in scenario
            .root
            .requires
            .iter()
            .chain(scenario.root.optional_dependencies.values().flatten())
            .chain(groups.values().flatten())
        {
            validate_project_requirement(requirement, &name)?;
        }
        Ok(Self {
            scenario,
            name,
            groups,
        })
    }

    /// The deterministic name used for the temporary project.
    pub fn name(&self) -> &PackageName {
        &self.name
    }

    /// Select every optional dependency and group, as required by a conflict-free universal lock.
    pub fn all_selection(&self) -> ProjectSelection {
        ProjectSelection {
            include_project: true,
            extras: self
                .scenario
                .root
                .optional_dependencies
                .keys()
                .cloned()
                .collect(),
            groups: self.groups.keys().cloned().collect(),
        }
    }

    /// Expand an explicit selection without dropping duplicate group requirements.
    pub fn requirements(&self, selection: &ProjectSelection) -> Result<Vec<Requirement>> {
        ensure!(
            selection.include_project || selection.extras.is_empty(),
            "project extras cannot be selected in a groups-only projection"
        );
        ensure!(
            selection.include_project || !selection.groups.is_empty(),
            "a groups-only projection requires at least one dependency group"
        );
        let mut requirements = Vec::new();
        if selection.include_project {
            requirements.extend(self.scenario.root.requires.iter().cloned());
            for extra in &selection.extras {
                let extra_requirements = self
                    .scenario
                    .root
                    .optional_dependencies
                    .get(extra)
                    .with_context(|| format!("project extra `{extra}` is not defined"))?;
                requirements.extend(extra_requirements.iter().cloned());
            }
        }
        for group in &selection.groups {
            let group_requirements = self
                .groups
                .get(group)
                .with_context(|| format!("dependency group `{group}` is not defined"))?;
            requirements.extend(group_requirements.iter().cloned());
        }
        Ok(requirements)
    }

    /// Construct an exhaustive oracle for exactly the selected project roots.
    pub fn oracle<'project>(
        &'project self,
        environment: &'project MarkerEnvironment,
        selection: &ProjectSelection,
    ) -> Result<ScenarioOracle<'project>> {
        ScenarioOracle::with_root_requirements(
            self.scenario,
            environment,
            self.requirements(selection)?,
        )
    }
}

pub(super) fn project_name(scenario: &Scenario) -> Result<PackageName> {
    let mut name = PackageName::from_str("uv-scenario-root")?;
    while scenario.packages.contains_key(&name) {
        name = PackageName::from_str(&format!("{name}-root"))?;
    }
    Ok(name)
}

fn validate_project_requirement(requirement: &Requirement, name: &PackageName) -> Result<()> {
    validate_requirement(requirement)?;
    ensure!(
        requirement.name != *name,
        "the scenario project does not model root self-references: {requirement}"
    );
    let mut has_extra_marker = false;
    requirement
        .marker
        .visit_extras(|_, _| has_extra_marker = true);
    ensure!(
        !has_extra_marker,
        "the scenario project does not model root extra markers: {requirement}"
    );
    Ok(())
}

fn flatten_groups(
    groups: &DependencyGroups,
    max_requirements: usize,
) -> Result<BTreeMap<GroupName, Vec<Requirement>>> {
    fn expand(
        name: &GroupName,
        groups: &DependencyGroups,
        path: &mut Vec<GroupName>,
        flattened: &mut BTreeMap<GroupName, Vec<Requirement>>,
        remaining: &mut usize,
        max_requirements: usize,
    ) -> Result<()> {
        if let Some(start) = path.iter().position(|parent| parent == name) {
            let cycle = path[start..]
                .iter()
                .chain(std::iter::once(name))
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" -> ");
            bail!("dependency group include cycle: {cycle}");
        }
        if flattened.contains_key(name) {
            return Ok(());
        }
        let entries = groups
            .get(name)
            .with_context(|| format!("included dependency group `{name}` is not defined"))?;
        ensure!(
            path.len() < MAX_GROUP_DEPTH,
            "dependency group nesting exceeds {MAX_GROUP_DEPTH} levels"
        );
        path.push(name.clone());
        let mut requirements = Vec::new();
        for entry in entries {
            match entry {
                DependencyGroupSpecifier::Requirement(requirement) => {
                    requirements.push(Requirement::from_str(requirement).with_context(|| {
                        format!("invalid requirement in dependency group `{name}`")
                    })?);
                }
                DependencyGroupSpecifier::IncludeGroup { include_group } => {
                    expand(
                        include_group,
                        groups,
                        path,
                        flattened,
                        remaining,
                        max_requirements,
                    )?;
                    requirements.extend(flattened[include_group].iter().cloned());
                }
                DependencyGroupSpecifier::Object(_) => {
                    bail!("unsupported dependency object in group `{name}`");
                }
            }
            ensure!(
                requirements.len() <= *remaining,
                "dependency group expansion exceeds {max_requirements} requirements"
            );
        }
        path.pop();
        *remaining -= requirements.len();
        flattened.insert(name.clone(), requirements);
        Ok(())
    }

    let mut flattened = BTreeMap::new();
    let mut remaining = max_requirements;
    for name in groups.keys() {
        expand(
            name,
            groups,
            &mut Vec::new(),
            &mut flattened,
            &mut remaining,
            max_requirements,
        )?;
    }
    Ok(flattened)
}

#[cfg(test)]
mod tests {
    use uv_python::PythonVersion;

    use super::*;
    use crate::packse::check::{ScenarioPlatform, ScenarioTarget};

    fn scenario(contents: &str) -> Scenario {
        toml::from_str(contents).expect("valid scenario")
    }

    fn selection(extras: &[&str], groups: &[&str], include_project: bool) -> ProjectSelection {
        ProjectSelection {
            include_project,
            extras: extras
                .iter()
                .map(|extra| ExtraName::from_str(extra).expect("valid extra"))
                .collect(),
            groups: groups
                .iter()
                .map(|group| GroupName::from_str(group).expect("valid group"))
                .collect(),
        }
    }

    fn requirement_strings(
        project: &ScenarioProject<'_>,
        selection: &ProjectSelection,
    ) -> Result<Vec<String>> {
        Ok(project
            .requirements(selection)?
            .iter()
            .map(ToString::to_string)
            .collect())
    }

    #[test]
    fn expands_groups_in_order_without_deduplication() -> Result<()> {
        let scenario = scenario(
            r#"
name = "project-groups"
[root]
requires = ["base"]
[root.optional_dependencies]
docs = ["docs-lib; sys_platform == 'linux'"]
[root.dependency_groups]
shared = ["dep==1", "dep>=1"]
dev = [{include-group = "SHARED"}, "other", {include-group = "shared"}]
[expected]
satisfiable = true
"#,
        );
        let project = ScenarioProject::new(&scenario)?;
        assert_eq!(
            requirement_strings(&project, &selection(&[], &["DEV"], true))?,
            ["base", "dep==1", "dep>=1", "other", "dep==1", "dep>=1"]
        );
        assert_eq!(
            requirement_strings(&project, &selection(&[], &["shared"], false))?,
            ["dep==1", "dep>=1"]
        );
        assert_eq!(
            requirement_strings(&project, &selection(&["docs"], &[], true))?,
            ["base", "docs-lib ; sys_platform == 'linux'"]
        );
        Ok(())
    }

    #[test]
    fn combined_roots_can_be_unsatisfiable() -> Result<()> {
        let scenario = scenario(
            r#"
name = "combined-project-roots"
[root]
requires = ["base"]
[root.optional_dependencies]
legacy = ["dep==1"]
[root.dependency_groups]
dev = ["dep==2"]
[expected]
satisfiable = false
[packages.base.versions."1"]
[packages.dep.versions."1"]
[packages.dep.versions."2"]
"#,
        );
        let environment = ScenarioTarget {
            python: PythonVersion::from_str("3.12").expect("valid Python version"),
            platform: ScenarioPlatform::Linux,
        }
        .markers()?;
        let project = ScenarioProject::new(&scenario)?;
        assert!(
            project
                .oracle(&environment, &selection(&["legacy"], &[], true))?
                .find_solution(6)?
                .solution
                .is_some()
        );
        assert!(
            project
                .oracle(&environment, &selection(&[], &["dev"], true))?
                .find_solution(6)?
                .solution
                .is_some()
        );
        assert!(
            project
                .oracle(&environment, &project.all_selection())?
                .find_solution(6)?
                .solution
                .is_none()
        );
        let error = ScenarioOracle::new(&scenario, &environment)
            .err()
            .expect("a project requires an explicit selection");
        insta::assert_snapshot!(error, @"the scenario oracle requires an explicit project selection for optional dependencies or groups");
        Ok(())
    }

    #[test]
    fn rejects_invalid_group_graphs() {
        let cases = [
            (
                "cycle",
                "a = [{include-group = 'b'}]\nb = [{include-group = 'a'}]",
                "dependency group include cycle: a -> b -> a",
            ),
            (
                "missing",
                "a = [{include-group = 'missing'}]",
                "included dependency group `missing` is not defined",
            ),
            (
                "object",
                "a = [{unknown = 'value'}]",
                "unsupported dependency object in group `a`",
            ),
        ];
        for (name, groups, expected) in cases {
            let scenario = scenario(&format!(
                "name = '{name}'\n[root]\n[expected]\nsatisfiable = true\n[root.dependency_groups]\n{groups}"
            ));
            let error = ScenarioProject::new(&scenario)
                .err()
                .expect("invalid group graph");
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn rejects_normalized_duplicate_names() {
        for field in ["optional_dependencies", "dependency_groups"] {
            let contents = format!(
                "name = 'duplicates'\n[root]\n[expected]\nsatisfiable = true\n[root.{field}]\nfoo_bar = []\nfoo-bar = []"
            );
            let error = toml::from_str::<Scenario>(&contents).expect_err("normalized duplicate");
            assert!(error.to_string().contains("duplicate"));
            assert!(error.to_string().contains("foo-bar"));
        }
    }

    #[test]
    fn bounds_group_expansion() -> Result<()> {
        let scenario = scenario(
            r#"
name = "group-expansion"
[root.dependency_groups]
base = ["a", "b"]
dev = [{include-group = "base"}, {include-group = "base"}]
[expected]
satisfiable = true
"#,
        );
        let groups = scenario.root.dependency_groups.as_ref().expect("groups");
        insta::assert_snapshot!(
            flatten_groups(groups, 5).expect_err("expanded requirements are bounded"),
            @"dependency group expansion exceeds 5 requirements"
        );
        assert_eq!(flatten_groups(groups, 6)?.len(), 2);
        Ok(())
    }

    #[test]
    fn rejects_unsupported_project_roots() {
        for (requirement, expected) in [
            (
                "uv-scenario-root[other]",
                "the scenario project does not model root self-references",
            ),
            (
                "a; extra == 'feature'",
                "the scenario project does not model root extra markers",
            ),
        ] {
            let scenario = scenario(&format!(
                "name = 'unsupported'\n[root]\nrequires = [\"{requirement}\"]\n[expected]\nsatisfiable = true"
            ));
            let error = ScenarioProject::new(&scenario)
                .err()
                .expect("unsupported project root");
            assert!(error.to_string().starts_with(expected));
        }
        let error = toml::from_str::<Scenario>(
            "name = 'conflicts'\n[root]\nconflicts = []\n[expected]\nsatisfiable = true",
        )
        .expect_err("conflict declarations are not supported");
        assert!(error.to_string().contains("unknown field `conflicts`"));
    }

    #[test]
    fn rejects_unknown_or_invalid_selections() -> Result<()> {
        let scenario = scenario("name = 'selection'\n[root]\n[expected]\nsatisfiable = true");
        let project = ScenarioProject::new(&scenario)?;
        for choice in [
            selection(&["missing"], &[], true),
            selection(&[], &["missing"], true),
            selection(&["missing"], &["missing"], false),
            selection(&[], &[], false),
        ] {
            assert!(project.requirements(&choice).is_err());
        }
        Ok(())
    }
}
