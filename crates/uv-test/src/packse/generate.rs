//! Deterministic small dependency graphs for the exhaustive scenario checker.

use anyhow::{Result, ensure};
use serde_json::{Map, Value, json};

use super::check::ScenarioTarget;
use super::oracle::ScenarioOracle;
use super::scenario::ScenarioDocument;

/// The bounded shape of a generated dependency graph.
#[derive(Clone, Copy, Debug)]
pub struct SmallGraphOptions {
    pub packages: usize,
    pub versions: usize,
}

/// Generate a complete replayable fixture and compute its satisfiability independently.
///
/// These graphs use stable versions, universal wheels, additive extras, and one Python minor
/// line. They intentionally do not encode candidate preferences or non-additive extras.
pub fn generate_small_graph(
    seed: u64,
    options: SmallGraphOptions,
    target: &ScenarioTarget,
    max_states: usize,
) -> Result<ScenarioDocument> {
    ensure!(
        (1..=8).contains(&options.packages),
        "small graphs require between 1 and 8 packages"
    );
    ensure!(
        (1..=4).contains(&options.versions),
        "small graphs require between 1 and 4 versions per package"
    );
    ensure!(
        !target.python.version().any_prerelease(),
        "small graphs require a stable Python marker version"
    );
    let mut states = 1_usize;
    for _ in 0..options.packages {
        states = states
            .checked_mul(options.versions + 1)
            .filter(|states| *states <= max_states)
            .ok_or_else(|| {
                anyhow::anyhow!("scenario search space exceeds {max_states} selections")
            })?;
    }

    let python_range = format!(
        ">={}.{},<{}.{}",
        target.python.major(),
        target.python.minor(),
        target.python.major(),
        u16::from(target.python.minor()) + 1,
    );
    let name = format!(
        "small-graph-{seed:016x}-{}p-{}v-py{}{}",
        options.packages,
        options.versions,
        target.python.major(),
        target.python.minor(),
    );
    let names = (0..options.packages)
        .map(|index| format!("node-{index}"))
        .collect::<Vec<_>>();
    let mut random = fastrand::Rng::with_seed(seed);
    let root = names
        .iter()
        .enumerate()
        .filter_map(|(index, name)| {
            if index == 0 || random.usize(0..3) == 0 {
                Some(requirement(&mut random, name, options.versions))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let mut packages = Map::new();
    for name in &names {
        let mut versions = Map::new();
        for version in 1..=options.versions {
            let requires = names
                .iter()
                .filter_map(|dependency| {
                    if random.usize(0..4) == 0 {
                        Some(requirement(&mut random, dependency, options.versions))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            let feature = names
                .iter()
                .filter_map(|dependency| {
                    if random.usize(0..5) == 0 {
                        Some(requirement(&mut random, dependency, options.versions))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            versions.insert(
                format!("{version}.0.0"),
                json!({
                    "requires_python": python_range,
                    "requires": requires,
                    "extras": { "feature": feature },
                    "sdist": false,
                    "wheel": true,
                }),
            );
        }
        packages.insert(name.clone(), json!({ "versions": versions }));
    }

    let mut value = json!({
        "name": name,
        "description": format!("Finite dependency graph generated from seed {seed}."),
        "root": { "requires_python": python_range, "requires": root },
        "expected": { "satisfiable": false },
        "resolver_options": { "universal": true },
        "packages": packages,
    });
    let document = ScenarioDocument::from_value(toml::Value::try_from(&value)?)?;
    let scenario = document.scenario()?;
    let environment = target.markers()?;
    let search = ScenarioOracle::new(&scenario, &environment)?.find_solution(max_states)?;
    value["expected"]["satisfiable"] = Value::Bool(search.solution.is_some());
    ScenarioDocument::from_value(toml::Value::try_from(value)?)
}

fn requirement(random: &mut fastrand::Rng, name: &str, versions: usize) -> String {
    let extra = if random.usize(0..4) == 0 {
        "[feature]"
    } else {
        ""
    };
    let version = random.usize(1..=versions);
    let specifiers = [
        String::new(),
        format!("=={version}"),
        format!(">={version}"),
        format!("<={version}"),
        format!("!={version}"),
        format!("=={}", versions + 1),
    ];
    let specifier = &specifiers[random.usize(0..specifiers.len())];
    format!("{name}{extra}{specifier}")
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use uv_python::PythonVersion;

    use super::*;
    use crate::packse::check::ScenarioPlatform;

    #[test]
    fn generated_graphs_are_replayable_and_bounded() -> Result<()> {
        let target = ScenarioTarget {
            python: PythonVersion::from_str("3.12").expect("valid Python version"),
            platform: ScenarioPlatform::Linux,
        };
        let options = SmallGraphOptions {
            packages: 3,
            versions: 2,
        };
        let environment = target.markers()?;
        let mut satisfiable = 0;
        let mut unsatisfiable = 0;
        for seed in 0..32 {
            let document = generate_small_graph(seed, options, &target, 27)?;
            assert_eq!(
                document.to_toml()?,
                generate_small_graph(seed, options, &target, 27)?.to_toml()?
            );
            let replay: ScenarioDocument = document.to_toml()?.parse()?;
            let scenario = replay.scenario()?;
            assert_eq!(scenario.packages.len(), options.packages);
            assert!(
                scenario
                    .packages
                    .values()
                    .all(|package| package.versions.len() == options.versions)
            );
            let search = ScenarioOracle::new(&scenario, &environment)?.find_solution(27)?;
            assert_eq!(scenario.expected.satisfiable, search.solution.is_some());
            if search.solution.is_some() {
                satisfiable += 1;
            } else {
                unsatisfiable += 1;
            }
        }
        assert!(satisfiable > 0);
        assert!(unsatisfiable > 0);
        insta::assert_snapshot!(
            generate_small_graph(0, options, &target, 26).expect_err("search should be bounded"),
            @"scenario search space exceeds 26 selections"
        );
        Ok(())
    }
}
