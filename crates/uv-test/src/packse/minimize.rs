//! Deletion-based reduction of fixed-environment resolver counterexamples.

use std::fmt;

use anyhow::{Context, Result, bail, ensure};

use super::check::{CheckResult, ScenarioFailureKind, ScenarioTarget};
use super::oracle::ScenarioOracle;
use super::scenario::{Scenario, ScenarioDocument};

/// The parts of a dependency graph that the reducer can delete.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScenarioSize {
    pub packages: usize,
    pub versions: usize,
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
pub struct MinimizedScenario {
    pub document: ScenarioDocument,
    pub failure: ScenarioFailureKind,
    pub original_size: ScenarioSize,
    pub reduced_size: ScenarioSize,
    /// Candidate checks, excluding the initial and final confirmation.
    pub attempts: usize,
    pub accepted: usize,
    /// No supported single deletion preserves the mismatch. This is not a global minimum.
    pub deletion_minimal: bool,
}

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

    let mut current = document.clone();
    let mut attempts = 0;
    let mut accepted = 0;
    let deletion_minimal = 'reduction: loop {
        for deletion in deletions(&current) {
            if attempts == max_attempts {
                break 'reduction false;
            }
            let candidate = deletion.apply(&current)?;
            attempts += 1;
            match check(&candidate.scenario()?) {
                Ok(_) => {}
                Err(error) => match ScenarioFailureKind::from_error(&error) {
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

    let document = fixed_environment_document(&current, target, max_states)?;
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

fn deletions(document: &ScenarioDocument) -> Vec<Deletion> {
    let value = document.value();
    let mut deletions = Vec::new();
    array_deletions(
        &mut deletions,
        value.get("root").and_then(|root| root.get("requires")),
        &[key("root"), key("requires")],
    );
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

fn table_entry<'a>(table: &'a mut toml::Table, key: &str) -> Result<&'a mut toml::Table> {
    table
        .entry(key.to_string())
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .with_context(|| format!("scenario `{key}` must be a TOML table"))
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use uv_python::PythonVersion;

    use super::*;
    use crate::packse::check::ScenarioPlatform;

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
}
