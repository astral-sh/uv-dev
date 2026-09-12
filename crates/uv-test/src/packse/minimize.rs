//! Deletion-based reduction of resolver and lockfile counterexamples.

use std::fmt;

use anyhow::{Context, Result, bail, ensure};

use super::check::{
    CheckResult, LockCheckResult, LockScenarioFailureKind, ScenarioFailureKind, ScenarioTarget,
};
use super::oracle::ScenarioOracle;
use super::project::ScenarioProject;
use super::scenario::{Scenario, ScenarioDocument};

/// The parts of a dependency graph that the reducer can delete.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScenarioSize {
    pub packages: usize,
    pub versions: usize,
    /// Requirement entries, including dependency-group includes.
    pub requirements: usize,
}

impl ScenarioSize {
    fn of(scenario: &Scenario) -> Self {
        Self {
            packages: scenario.packages.len(),
            versions: scenario
                .packages
                .values()
                .map(|package| package.versions.len())
                .sum(),
            requirements: scenario.root.requires.len()
                + scenario
                    .root
                    .optional_dependencies
                    .values()
                    .map(Vec::len)
                    .sum::<usize>()
                + scenario
                    .root
                    .dependency_groups
                    .iter()
                    .flat_map(IntoIterator::into_iter)
                    .map(|(_, entries)| entries.len())
                    .sum::<usize>()
                + scenario
                    .packages
                    .values()
                    .flat_map(|package| package.versions.values())
                    .map(|metadata| {
                        metadata.requires.len()
                            + metadata.extras.values().map(Vec::len).sum::<usize>()
                    })
                    .sum::<usize>(),
        }
    }
}

impl fmt::Display for ScenarioSize {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} packages, {} versions, {} requirements",
            self.packages, self.versions, self.requirements
        )
    }
}

/// A replayable graph that retains the original kind of semantic mismatch.
#[derive(Debug)]
pub struct MinimizedScenario<Failure = ScenarioFailureKind> {
    pub document: ScenarioDocument,
    pub failure: Failure,
    pub original_size: ScenarioSize,
    pub reduced_size: ScenarioSize,
    /// Candidate checks, excluding the initial and final confirmation.
    pub attempts: usize,
    pub accepted: usize,
    /// No supported single deletion preserves the mismatch. This is not a global minimum.
    pub deletion_minimal: bool,
}

/// A replayable universal lockfile counterexample.
pub type MinimizedLockScenario = MinimizedScenario<LockScenarioFailureKind>;

/// Remove graph elements while preserving the initial semantic failure kind.
///
/// `check` must run the real fixed-environment comparison with fresh state for each candidate.
/// Different semantic failures reject a candidate; unclassified errors stop the reduction rather
/// than turning an infrastructure problem into a purported counterexample.
pub fn minimize_scenario(
    document: &ScenarioDocument,
    target: &ScenarioTarget,
    max_states: usize,
    max_attempts: usize,
    mut check: impl FnMut(&Scenario) -> Result<CheckResult>,
) -> Result<MinimizedScenario> {
    ensure!(max_attempts > 0, "the reduction budget must be positive");
    let scenario = document.scenario()?;
    let original_size = ScenarioSize::of(&scenario);
    let Err(original_error) = check(&scenario) else {
        bail!("the input does not reproduce a resolver mismatch");
    };
    let Some(failure) = ScenarioFailureKind::from_error(&original_error) else {
        return Err(original_error.context("the input failed without a semantic resolver mismatch"));
    };

    let reduction = reduce_document(
        document,
        max_attempts,
        failure,
        ScenarioFailureKind::from_error,
        |_| true,
        |scenario| check(scenario).map(|_| ()),
    )?;
    let document = fixed_environment_document(&reduction.document, target, max_states)?;
    let scenario = document.scenario()?;
    let reduced_size = ScenarioSize::of(&scenario);
    match check(&scenario) {
        Err(error) if ScenarioFailureKind::from_error(&error) == Some(failure) => {}
        Err(error) => {
            return Err(error.context("the final replay changed the resolver failure"));
        }
        Ok(_) => bail!("the final replay no longer reproduces the resolver failure"),
    }
    Ok(MinimizedScenario {
        document,
        failure,
        original_size,
        reduced_size,
        attempts: reduction.attempts,
        accepted: reduction.accepted,
        deletion_minimal: reduction.deletion_minimal,
    })
}

/// Reduce a universal lockfile counterexample, retaining the original demonstrated failure kind.
///
/// `check` must use fresh state and the same target matrix for every candidate. The saved
/// expectation describes the first target, not the entire marker universe.
pub fn minimize_lock_scenario(
    document: &ScenarioDocument,
    targets: &[ScenarioTarget],
    max_states: usize,
    max_attempts: usize,
    check: impl FnMut(&Scenario) -> Result<LockCheckResult>,
) -> Result<MinimizedLockScenario> {
    minimize_lock_scenario_inner(document, targets, max_states, max_attempts, false, check)
}

/// Reduce a universal project lock, including optional dependencies and dependency-group entries.
///
/// `check` must recompute the explicit selection matrix for each candidate and use fresh state.
/// Deletions that leave invalid group includes are skipped before invoking it.
pub fn minimize_project_lock_scenario(
    document: &ScenarioDocument,
    targets: &[ScenarioTarget],
    max_states: usize,
    max_attempts: usize,
    check: impl FnMut(&Scenario) -> Result<LockCheckResult>,
) -> Result<MinimizedLockScenario> {
    minimize_lock_scenario_inner(document, targets, max_states, max_attempts, true, check)
}

fn minimize_lock_scenario_inner(
    document: &ScenarioDocument,
    targets: &[ScenarioTarget],
    max_states: usize,
    max_attempts: usize,
    project_selections: bool,
    mut check: impl FnMut(&Scenario) -> Result<LockCheckResult>,
) -> Result<MinimizedLockScenario> {
    ensure!(max_attempts > 0, "the reduction budget must be positive");
    let target = targets.first().context("at least one target is required")?;
    let scenario = document.scenario()?;
    let original_size = ScenarioSize::of(&scenario);
    let Err(original_error) = check(&scenario) else {
        bail!("the input does not reproduce a lockfile mismatch");
    };
    let Some(failure) = LockScenarioFailureKind::from_error(&original_error) else {
        return Err(
            original_error.context("the input failed without a demonstrated lockfile mismatch")
        );
    };

    let reduction = reduce_document(
        document,
        max_attempts,
        failure,
        LockScenarioFailureKind::from_error,
        |scenario| !project_selections || ScenarioProject::new(scenario).is_ok(),
        |scenario| check(scenario).map(|_| ()),
    )?;
    let document = universal_document(&reduction.document, target, project_selections, max_states)?;
    let scenario = document.scenario()?;
    let reduced_size = ScenarioSize::of(&scenario);
    match check(&scenario) {
        Err(error) if LockScenarioFailureKind::from_error(&error) == Some(failure) => {}
        Err(error) => return Err(error.context("the final replay changed the lockfile failure")),
        Ok(_) => bail!("the final replay no longer reproduces the lockfile failure"),
    }
    Ok(MinimizedScenario {
        document,
        failure,
        original_size,
        reduced_size,
        attempts: reduction.attempts,
        accepted: reduction.accepted,
        deletion_minimal: reduction.deletion_minimal,
    })
}

struct Reduction {
    document: ScenarioDocument,
    attempts: usize,
    accepted: usize,
    deletion_minimal: bool,
}

fn reduce_document<Failure: Copy + Eq>(
    document: &ScenarioDocument,
    max_attempts: usize,
    failure: Failure,
    classify: impl Fn(&anyhow::Error) -> Option<Failure>,
    supported: impl Fn(&Scenario) -> bool,
    mut check: impl FnMut(&Scenario) -> Result<()>,
) -> Result<Reduction> {
    let mut current = document.clone();
    let mut attempts = 0;
    let mut accepted = 0;
    let deletion_minimal = 'reduction: loop {
        for deletion in deletions(&current) {
            let candidate = deletion.apply(&current)?;
            let scenario = candidate.scenario()?;
            if !supported(&scenario) {
                continue;
            }
            if attempts == max_attempts {
                break 'reduction false;
            }
            attempts += 1;
            match check(&scenario) {
                Ok(()) => {}
                Err(error) => match classify(&error) {
                    Some(kind) if kind == failure => {
                        current = candidate;
                        accepted += 1;
                        continue 'reduction;
                    }
                    Some(_) => {}
                    None => {
                        return Err(error.context(format!(
                            "reduction stopped after {attempts} candidate checks"
                        )));
                    }
                },
            }
        }
        break true;
    };

    Ok(Reduction {
        document: current,
        attempts,
        accepted,
        deletion_minimal,
    })
}

#[derive(Clone, Debug)]
enum PathPart {
    Key(String),
    Index(usize),
}

#[derive(Debug)]
struct Deletion(Vec<PathPart>);

impl Deletion {
    fn apply(&self, document: &ScenarioDocument) -> Result<ScenarioDocument> {
        let Some((last, parent)) = self.0.split_last() else {
            bail!("a reduction cannot remove the document root");
        };
        let mut value = document.value().clone();
        let mut cursor = &mut value;
        for part in parent {
            cursor = match part {
                PathPart::Key(key) => cursor.get_mut(key),
                PathPart::Index(index) => cursor.get_mut(*index),
            }
            .context("a reduction path is missing from the document")?;
        }
        match last {
            PathPart::Key(key) => {
                cursor
                    .as_table_mut()
                    .context("a reduction expected a TOML table")?
                    .remove(key)
                    .context("a reduction key is missing from the document")?;
            }
            PathPart::Index(index) => {
                let array = cursor
                    .as_array_mut()
                    .context("a reduction expected a TOML array")?;
                ensure!(*index < array.len(), "a reduction array index is missing");
                array.remove(*index);
            }
        }
        ScenarioDocument::from_value(value)
    }
}

fn key(value: &str) -> PathPart {
    PathPart::Key(value.to_string())
}

fn array_deletions(deletions: &mut Vec<Deletion>, value: Option<&toml::Value>, path: &[PathPart]) {
    if let Some(array) = value.and_then(toml::Value::as_array) {
        for index in 0..array.len() {
            let mut path = path.to_vec();
            path.push(PathPart::Index(index));
            deletions.push(Deletion(path));
        }
    }
}

fn root_table_deletions(deletions: &mut Vec<Deletion>, root: Option<&toml::Value>, name: &str) {
    if let Some(entries) = root
        .and_then(|root| root.get(name))
        .and_then(toml::Value::as_table)
    {
        let path = vec![key("root"), key(name)];
        deletions.push(Deletion(path.clone()));
        for (name, requirements) in entries {
            let mut entry_path = path.clone();
            entry_path.push(key(name));
            deletions.push(Deletion(entry_path.clone()));
            array_deletions(deletions, Some(requirements), &entry_path);
        }
    }
}

fn deletions(document: &ScenarioDocument) -> Vec<Deletion> {
    let value = document.value();
    let mut deletions = Vec::new();
    array_deletions(
        &mut deletions,
        value.get("root").and_then(|root| root.get("requires")),
        &[key("root"), key("requires")],
    );
    root_table_deletions(&mut deletions, value.get("root"), "optional_dependencies");
    root_table_deletions(&mut deletions, value.get("root"), "dependency_groups");
    if let Some(packages) = value.get("packages").and_then(toml::Value::as_table) {
        for name in packages.keys() {
            deletions.push(Deletion(vec![key("packages"), key(name)]));
        }
        for (name, package) in packages {
            if let Some(versions) = package.get("versions").and_then(toml::Value::as_table) {
                for version in versions.keys() {
                    deletions.push(Deletion(vec![
                        key("packages"),
                        key(name),
                        key("versions"),
                        key(version),
                    ]));
                }
                for (version, metadata) in versions {
                    let path = vec![key("packages"), key(name), key("versions"), key(version)];
                    let mut requires_path = path.clone();
                    requires_path.push(key("requires"));
                    array_deletions(&mut deletions, metadata.get("requires"), &requires_path);
                    if let Some(extras) = metadata.get("extras").and_then(toml::Value::as_table) {
                        for (extra, requirements) in extras {
                            let mut extra_path = path.clone();
                            extra_path.push(key("extras"));
                            extra_path.push(key(extra));
                            deletions.push(Deletion(extra_path.clone()));
                            array_deletions(&mut deletions, Some(requirements), &extra_path);
                        }
                    }
                }
            }
        }
    }
    deletions
}

/// Record a fixed-environment expectation without claiming an arbitrary preferred solution or
/// universal satisfiability from one concrete environment.
fn fixed_environment_document(
    document: &ScenarioDocument,
    target: &ScenarioTarget,
    max_states: usize,
) -> Result<ScenarioDocument> {
    let scenario = document.scenario()?;
    let environment = target.markers()?;
    let search = ScenarioOracle::new(&scenario, &environment)?.find_solution(max_states)?;
    let mut value = document.value().clone();
    let root = value
        .as_table_mut()
        .context("a scenario document must be a TOML table")?;
    root.insert(
        "name".to_string(),
        toml::Value::String(format!("{}-reduced", scenario.name)),
    );
    root.insert(
        "description".to_string(),
        toml::Value::String(format!(
            "Fixed-environment resolver counterexample for {target}."
        )),
    );
    let expected = table_entry(root, "expected")?;
    expected.insert(
        "satisfiable".to_string(),
        toml::Value::Boolean(search.solution.is_some()),
    );
    expected.remove("packages");
    expected.remove("explanation");
    let resolver = table_entry(root, "resolver_options")?;
    resolver.insert("universal".to_string(), toml::Value::Boolean(false));
    resolver.insert(
        "python".to_string(),
        toml::Value::String(target.python.to_string()),
    );
    resolver.insert(
        "python_platform".to_string(),
        toml::Value::String(target.platform.as_str().to_string()),
    );
    table_entry(root, "environment")?.insert(
        "python".to_string(),
        toml::Value::String(target.python.to_string()),
    );
    table_entry(root, "testgen")?.insert(
        "kind".to_string(),
        toml::Value::String("compile".to_string()),
    );
    ScenarioDocument::from_value(value)
}

/// Preserve the universal Python range while recording the all-root outcome at one target.
fn universal_document(
    document: &ScenarioDocument,
    target: &ScenarioTarget,
    project_selections: bool,
    max_states: usize,
) -> Result<ScenarioDocument> {
    let scenario = document.scenario()?;
    let environment = target.markers()?;
    let search = if project_selections {
        let project = ScenarioProject::new(&scenario)?;
        project
            .oracle(&environment, &project.all_selection())?
            .find_solution(max_states)?
    } else {
        ScenarioOracle::new(&scenario, &environment)?.find_solution(max_states)?
    };
    let mut value = document.value().clone();
    let root = value
        .as_table_mut()
        .context("a scenario document must be a TOML table")?;
    root.insert(
        "name".to_string(),
        toml::Value::String(format!("{}-reduced", scenario.name)),
    );
    root.insert(
        "description".to_string(),
        toml::Value::String(format!(
            "Universal lockfile counterexample. Its expected outcome includes all project roots for {target}."
        )),
    );
    let expected = table_entry(root, "expected")?;
    expected.insert(
        "satisfiable".to_string(),
        toml::Value::Boolean(search.solution.is_some()),
    );
    expected.remove("packages");
    expected.remove("explanation");
    let resolver = table_entry(root, "resolver_options")?;
    resolver.insert("universal".to_string(), toml::Value::Boolean(true));
    resolver.insert(
        "python".to_string(),
        toml::Value::String(target.python.to_string()),
    );
    resolver.insert(
        "python_platform".to_string(),
        toml::Value::String(target.platform.as_str().to_string()),
    );
    table_entry(root, "environment")?.insert(
        "python".to_string(),
        toml::Value::String(target.python.to_string()),
    );
    let testgen = table_entry(root, "testgen")?;
    testgen.insert("disable".to_string(), toml::Value::Boolean(true));
    testgen.insert("kind".to_string(), toml::Value::String("lock".to_string()));
    ScenarioDocument::from_value(value)
}

fn table_entry<'a>(table: &'a mut toml::Table, key: &str) -> Result<&'a mut toml::Table> {
    table
        .entry(key.to_string())
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .with_context(|| format!("scenario `{key}` must be a TOML table"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::str::FromStr;

    use uv_normalize::GroupName;
    use uv_python::PythonVersion;

    use super::*;
    use crate::packse::check::ScenarioPlatform;
    use crate::packse::project::ProjectSelection;
    use crate::packse::scenario::ScenarioTest;

    fn document() -> Result<ScenarioDocument> {
        r#"
name = "reduction"
[root]
requires = ["a", "unused"]
[expected]
satisfiable = true
packages = { a = "2" }
[resolver_options]
universal = true
[packages.a.versions."1"]
requires = ["b"]
[packages.a.versions."2"]
requires = ["b", "unused"]
[packages.b.versions."1"]
[packages.unused.versions."1"]
"#
        .parse()
    }

    fn target() -> ScenarioTarget {
        ScenarioTarget {
            python: PythonVersion::from_str("3.12").expect("valid Python version"),
            platform: ScenarioPlatform::Linux,
        }
    }

    fn mismatch(scenario: &Scenario) -> Result<CheckResult> {
        let reachable = scenario.root.requires.iter().any(|requirement| {
            requirement.name.as_ref() == "a"
                && scenario
                    .packages
                    .get(&requirement.name)
                    .is_some_and(|package| {
                        package.versions.values().any(|metadata| {
                            metadata
                                .requires
                                .iter()
                                .any(|requirement| requirement.name.as_ref() == "b")
                        })
                    })
        });
        if reachable {
            if scenario
                .packages
                .values()
                .any(|package| package.versions.is_empty())
                || !scenario.packages.keys().any(|name| name.as_ref() == "b")
            {
                return Err(anyhow::anyhow!("different semantic failure")
                    .context(ScenarioFailureKind::FalseUnsatisfiable));
            }
            return Err(
                anyhow::anyhow!("missing dependency").context(ScenarioFailureKind::InvalidClosure)
            );
        }
        Ok(CheckResult {
            selection: None,
            checked: 0,
        })
    }

    fn project_document() -> Result<ScenarioDocument> {
        r#"
name = "project-reduction"
[root]
requires = ["unused"]
requires_python = ">=3.12,<3.15"
[root.optional_dependencies]
one = ["a", "unused"]
unused = ["unused"]
[root.dependency_groups]
dev = [{ include-group = "SHARED" }, "unused"]
shared = ["b", "unused"]
unused = ["unused"]
[expected]
satisfiable = true
packages = { a = "2" }
[packages.a.versions."1"]
[packages.a.versions."2"]
[packages.b.versions."1"]
[packages.unused.versions."1"]
"#
        .parse()
    }

    fn project_mismatch(scenario: &Scenario) -> Result<LockCheckResult> {
        let project = ScenarioProject::new(scenario)
            .expect("invalid group includes must not reach the checker");
        let has_extra = scenario
            .root
            .optional_dependencies
            .iter()
            .any(|(extra, requirements)| {
                extra.as_ref() == "one"
                    && requirements
                        .iter()
                        .any(|requirement| requirement.name.as_ref() == "a")
            });
        let has_group = project
            .all_selection()
            .groups
            .iter()
            .any(|group| group.as_ref() == "dev");
        if has_extra && has_group {
            let selection = ProjectSelection {
                include_project: false,
                extras: BTreeSet::new(),
                groups: BTreeSet::from([GroupName::from_str("dev")?]),
            };
            if project
                .requirements(&selection)?
                .iter()
                .any(|requirement| requirement.name.as_ref() == "b")
            {
                if ["a", "b"].into_iter().any(|name| {
                    !scenario.packages.iter().any(|(package_name, package)| {
                        package_name.as_ref() == name && !package.versions.is_empty()
                    })
                }) {
                    return Err(anyhow::anyhow!("different lockfile failure")
                        .context(LockScenarioFailureKind::FalseSatisfiable));
                }
                return Err(anyhow::anyhow!("fresh lock was rejected")
                    .context(LockScenarioFailureKind::NonCanonicalLockfile));
            }
        }
        Ok(LockCheckResult::Satisfiable {
            projections: 1,
            checked: 0,
        })
    }

    #[test]
    fn reduces_only_the_original_failure_kind() -> Result<()> {
        let result = minimize_scenario(&document()?, &target(), 100, 100, mismatch)?;
        assert!(result.deletion_minimal);
        assert_eq!(result.failure, ScenarioFailureKind::InvalidClosure);
        assert_eq!(
            result.reduced_size,
            ScenarioSize {
                packages: 2,
                versions: 2,
                requirements: 2,
            }
        );
        let scenario = result.document.scenario()?;
        assert!(scenario.expected.packages.is_empty());
        assert!(!scenario.resolver_options.universal);
        assert_eq!(scenario.resolver_options.python, Some(target().python));
        assert!(scenario.resolver_options.python_platform.is_some());
        assert!(result.attempts <= 100);
        Ok(())
    }

    #[test]
    fn reports_an_exhausted_budget() -> Result<()> {
        let result = minimize_scenario(&document()?, &target(), 100, 1, mismatch)?;
        assert!(!result.deletion_minimal);
        assert_eq!(result.attempts, 1);
        Ok(())
    }

    #[test]
    fn does_not_minimize_infrastructure_failures() -> Result<()> {
        let error = minimize_scenario(&document()?, &target(), 100, 100, |_| {
            Err(anyhow::anyhow!("failed to start uv"))
        })
        .expect_err("an infrastructure failure is not a counterexample");
        assert_eq!(ScenarioFailureKind::from_error(&error), None);
        insta::assert_snapshot!(error, @"the input failed without a semantic resolver mismatch");
        Ok(())
    }

    #[test]
    fn reduces_project_roots_without_orphaning_group_includes() -> Result<()> {
        let result = minimize_project_lock_scenario(
            &project_document()?,
            &[target()],
            100,
            200,
            project_mismatch,
        )?;
        assert!(result.deletion_minimal);
        assert_eq!(
            result.failure,
            LockScenarioFailureKind::NonCanonicalLockfile
        );
        assert_eq!(
            result.reduced_size,
            ScenarioSize {
                packages: 2,
                versions: 2,
                requirements: 3,
            }
        );
        let scenario = result.document.scenario()?;
        assert!(scenario.root.requires.is_empty());
        assert_eq!(scenario.root.optional_dependencies.len(), 1);
        assert!(scenario.expected.packages.is_empty());
        assert!(scenario.expected.satisfiable);
        assert!(scenario.resolver_options.universal);
        assert!(scenario.testgen.disable);
        assert_eq!(scenario.testgen.kind, Some(ScenarioTest::Lock));
        assert_eq!(
            scenario.root.requires_python.unwrap().to_string(),
            ">=3.12, <3.15"
        );
        Ok(())
    }

    #[test]
    fn universal_expectations_include_unselected_project_roots() -> Result<()> {
        let document = r#"
name = "project-expectation"
[root]
requires_python = ">=3.12,<3.15"
[root.optional_dependencies]
feature = ["a>=2"]
[expected]
satisfiable = true
packages = { a = "1" }
[packages.a.versions."1"]
"#
        .parse()?;
        let document = universal_document(&document, &target(), true, 100)?;
        let scenario = document.scenario()?;
        assert!(!scenario.expected.satisfiable);
        assert!(scenario.expected.packages.is_empty());
        assert!(scenario.resolver_options.universal);
        assert!(scenario.testgen.disable);
        assert_eq!(
            scenario.root.requires_python.unwrap().to_string(),
            ">=3.12, <3.15"
        );
        Ok(())
    }

    #[test]
    fn lock_reduction_stops_on_unclassified_candidate_errors() -> Result<()> {
        let mut checks = 0;
        let error = minimize_lock_scenario(&document()?, &[target()], 100, 100, |_| {
            checks += 1;
            if checks == 1 {
                Err(anyhow::anyhow!("fresh lock was rejected")
                    .context(LockScenarioFailureKind::NonCanonicalLockfile))
            } else {
                Err(anyhow::anyhow!("failed to start uv"))
            }
        })
        .expect_err("an unclassified error must stop reduction");
        assert_eq!(LockScenarioFailureKind::from_error(&error), None);
        assert_eq!(checks, 2);
        insta::assert_snapshot!(error, @"reduction stopped after 1 candidate checks");
        Ok(())
    }
}
