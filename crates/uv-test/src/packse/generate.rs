//! Deterministic small dependency graphs for the exhaustive scenario checker.

use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value, json};

use super::check::ScenarioTarget;
use super::oracle::ScenarioOracle;
use super::project::ScenarioProject;
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
    expected_document(
        small_graph_value(seed, options, target, max_states)?,
        target,
        max_states,
    )
}

fn small_graph_value(
    seed: u64,
    options: SmallGraphOptions,
    target: &ScenarioTarget,
    max_states: usize,
) -> Result<Value> {
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

    Ok(json!({
        "name": name,
        "description": format!("Finite dependency graph generated from seed {seed}."),
        "root": { "requires_python": python_range, "requires": root },
        "expected": { "satisfiable": false },
        "resolver_options": { "universal": true },
        "packages": packages,
    }))
}

/// Generate a graph with Python and platform forks over three Python minor lines.
///
/// The recorded expectation belongs to `target`; other projections must be checked separately.
/// Marker expressions only use ordinary environments and additive extras. This does not model
/// extra- or group-scoped source selection or non-additive extra markers.
pub fn generate_marker_graph(
    seed: u64,
    options: SmallGraphOptions,
    target: &ScenarioTarget,
    max_states: usize,
) -> Result<ScenarioDocument> {
    expected_document(
        marker_graph_value(seed, options, target, max_states)?,
        target,
        max_states,
    )
}

fn marker_graph_value(
    seed: u64,
    options: SmallGraphOptions,
    target: &ScenarioTarget,
    max_states: usize,
) -> Result<Value> {
    let mut value = small_graph_value(seed, options, target, max_states)?;
    let major = target.python.major();
    let minor = u16::from(target.python.minor());
    ensure!(
        u8::try_from(minor + 2).is_ok(),
        "marker graphs require three representable Python minor versions"
    );
    let lower = format!("{major}.{minor}");
    let middle = format!("{major}.{}", minor + 1);
    let upper = format!("{major}.{}", minor + 2);
    let end = format!("{major}.{}", minor + 3);
    let python_ranges = [
        format!(">={lower},<{end}"),
        format!(">={middle},<{end}"),
        format!(">={lower},<{upper}"),
        format!(">={upper},<{end}"),
        format!(">={lower},<{middle}"),
    ];
    // Keep the fixed-graph stream unchanged, so existing saved seeds remain replayable.
    let mut random = fastrand::Rng::with_seed(seed ^ 0xb2f1_7c93_6a40_d85e);
    value["name"] = json!(format!(
        "marker-graph-{seed:016x}-{}p-{}v-py{}-through{major}.{}-{}",
        options.packages,
        options.versions,
        target.python,
        minor + 2,
        target.platform,
    ));
    value["description"] = json!(format!(
        "Finite marker graph generated from seed {seed}. Its expected outcome is for {target}."
    ));
    value["root"]["requires_python"] = json!(&python_ranges[0]);
    mark_requirements(
        &mut value["root"]["requires"],
        &mut random,
        &middle,
        &upper,
        false,
    )?;
    let packages = value["packages"]
        .as_object_mut()
        .context("generated packages must be a table")?;
    for package in packages.values_mut() {
        let versions = package["versions"]
            .as_object_mut()
            .context("generated versions must be a table")?;
        for metadata in versions.values_mut() {
            metadata["requires_python"] =
                json!(&python_ranges[random.usize(0..python_ranges.len())]);
            mark_requirements(
                &mut metadata["requires"],
                &mut random,
                &middle,
                &upper,
                true,
            )?;
            let extras = metadata["extras"]
                .as_object_mut()
                .context("generated extras must be a table")?;
            for requirements in extras.values_mut() {
                mark_requirements(requirements, &mut random, &middle, &upper, false)?;
            }
        }
    }
    value["environment"] = json!({ "python": target.python.to_string() });
    value["resolver_options"] = json!({
        "universal": false,
        "python": target.python.to_string(),
        "python_platform": target.platform.as_str(),
    });
    value["testgen"] = json!({ "kind": "compile" });
    Ok(value)
}

/// Generate a marker graph with independently selectable project extras and dependency groups.
///
/// The recorded expectation belongs to all project roots in `target`. The two extras and two
/// groups bound the export matrix; dependency-group includes may repeat, but cannot form cycles.
pub fn generate_project_graph(
    seed: u64,
    options: SmallGraphOptions,
    target: &ScenarioTarget,
    max_states: usize,
) -> Result<ScenarioDocument> {
    let mut value = marker_graph_value(seed, options, target, max_states)?;
    let major = target.python.major();
    let minor = u16::from(target.python.minor());
    let middle = format!("{major}.{}", minor + 1);
    let upper = format!("{major}.{}", minor + 2);
    let names = value["packages"]
        .as_object()
        .context("generated packages must be a table")?
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let mut random = fastrand::Rng::with_seed(seed ^ 0x48de_a709_c63b_215f);
    let mut project_requirements = || -> Result<Value> {
        let mut requirements = json!(
            names
                .iter()
                .filter_map(|name| {
                    if random.usize(0..3) == 0 {
                        Some(requirement(&mut random, name, options.versions))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        );
        mark_requirements(&mut requirements, &mut random, &middle, &upper, false)?;
        Ok(requirements)
    };
    let one = project_requirements()?;
    let two = project_requirements()?;
    let shared = project_requirements()?;
    let mut dev = project_requirements()?
        .as_array()
        .context("generated project requirements must be an array")?
        .clone();
    dev.insert(0, json!({ "include-group": "SHARED" }));
    if random.bool() {
        dev.push(json!({ "include-group": "shared" }));
    }
    value["name"] = json!(format!(
        "project-graph-{seed:016x}-{}p-{}v-py{}-through{major}.{}-{}",
        options.packages,
        options.versions,
        target.python,
        minor + 2,
        target.platform,
    ));
    value["description"] = json!(format!(
        "Finite project graph generated from seed {seed}. Its expected outcome includes all project roots for {target}."
    ));
    value["root"]["optional_dependencies"] = json!({ "one": one, "two": two });
    value["root"]["dependency_groups"] = json!({ "shared": shared, "dev": dev });
    value["resolver_options"] = json!({ "universal": true });
    value["testgen"] = json!({ "disable": true });

    let document = ScenarioDocument::from_value(toml::Value::try_from(&value)?)?;
    let scenario = document.scenario()?;
    let environment = target.markers()?;
    let project = ScenarioProject::new(&scenario)?;
    let search = project
        .oracle(&environment, &project.all_selection())?
        .find_solution(max_states)?;
    value["expected"]["satisfiable"] = Value::Bool(search.solution.is_some());
    ScenarioDocument::from_value(toml::Value::try_from(value)?)
}

fn mark_requirements(
    requirements: &mut Value,
    random: &mut fastrand::Rng,
    middle: &str,
    upper: &str,
    allow_extra: bool,
) -> Result<()> {
    let mut markers = vec![
        String::new(),
        "sys_platform == 'linux'".to_string(),
        "sys_platform == 'darwin'".to_string(),
        "sys_platform == 'win32'".to_string(),
        "sys_platform != 'win32'".to_string(),
        format!("python_version < '{middle}'"),
        format!("python_version >= '{middle}'"),
        format!("python_version < '{upper}'"),
        format!("python_version >= '{upper}'"),
        format!("sys_platform == 'win32' and python_version >= '{middle}'"),
        format!("sys_platform == 'darwin' or python_version < '{upper}'"),
    ];
    if allow_extra {
        markers.extend([
            "extra == 'feature'".to_string(),
            format!("extra == 'feature' and python_version >= '{middle}'"),
            "extra == 'feature' or sys_platform == 'win32'".to_string(),
        ]);
    }
    for requirement in requirements
        .as_array_mut()
        .context("generated requirements must be an array")?
    {
        let marker = &markers[random.usize(0..markers.len())];
        if !marker.is_empty() {
            let requirement_str = requirement
                .as_str()
                .context("generated requirements must be strings")?;
            *requirement = json!(format!("{requirement_str}; {marker}"));
        }
    }
    Ok(())
}

fn expected_document(
    mut value: Value,
    target: &ScenarioTarget,
    max_states: usize,
) -> Result<ScenarioDocument> {
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

    use sha2::{Digest, Sha256};
    use uv_python::PythonVersion;

    use super::*;
    use crate::packse::check::ScenarioPlatform;

    #[test]
    fn seed_formats_are_stable() -> Result<()> {
        let target = ScenarioTarget {
            python: PythonVersion::from_str("3.12").expect("valid Python version"),
            platform: ScenarioPlatform::Linux,
        };
        let options = SmallGraphOptions {
            packages: 3,
            versions: 2,
        };
        for (document, expected) in [
            (
                generate_small_graph(0, options, &target, 27)?,
                "3963b13f39b08c6dc572eecae302b6023067a64c8bf9ed1f4e1e6aa4081dfe95",
            ),
            (
                generate_marker_graph(0, options, &target, 27)?,
                "7442588496c1fbdec57c1091d14b8e1435da24175347b0618f245ffd8e87ea73",
            ),
            (
                generate_project_graph(39, options, &target, 27)?,
                "ee0013ebfca9336bbd78bc0dcef0ec335fe1f1e2a6e64a074a509e217287aa41",
            ),
        ] {
            assert_eq!(
                hex::encode(Sha256::digest(document.to_toml()?.as_bytes())),
                expected
            );
        }
        Ok(())
    }

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

    #[test]
    fn marker_graphs_have_independent_replayable_projections() -> Result<()> {
        let versions = ["3.12", "3.13", "3.14"]
            .map(|version| PythonVersion::from_str(version).expect("valid Python version"));
        let targets = ScenarioTarget::matrix(
            &versions,
            &[
                ScenarioPlatform::Linux,
                ScenarioPlatform::Macos,
                ScenarioPlatform::Windows,
            ],
        );
        let anchor = targets.first().expect("nonempty target matrix");
        let options = SmallGraphOptions {
            packages: 3,
            versions: 2,
        };
        let mut satisfiable = 0;
        let mut unsatisfiable = 0;
        let mut varying = false;
        for seed in 0..32 {
            let document = generate_marker_graph(seed, options, anchor, 27)?;
            assert_eq!(
                document.to_toml()?,
                generate_marker_graph(seed, options, anchor, 27)?.to_toml()?
            );
            let replay: ScenarioDocument = document.to_toml()?.parse()?;
            let scenario = replay.scenario()?;
            assert!(!scenario.resolver_options.universal);
            assert_eq!(
                scenario.resolver_options.python.as_ref(),
                Some(&anchor.python)
            );
            let mut outcomes = Vec::new();
            for target in &targets {
                let environment = target.markers()?;
                let search = ScenarioOracle::new(&scenario, &environment)?.find_solution(27)?;
                assert!(search.checked <= 27);
                outcomes.push(search.solution.is_some());
                if search.solution.is_some() {
                    satisfiable += 1;
                } else {
                    unsatisfiable += 1;
                }
            }
            assert_eq!(scenario.expected.satisfiable, outcomes[0]);
            varying |= outcomes.iter().any(|outcome| *outcome != outcomes[0]);
        }
        assert!(satisfiable > 0);
        assert!(unsatisfiable > 0);
        assert!(varying);

        let windows = ScenarioTarget {
            platform: ScenarioPlatform::Windows,
            ..anchor.clone()
        };
        assert_ne!(
            generate_marker_graph(0, options, anchor, 27)?
                .scenario()?
                .name,
            generate_marker_graph(0, options, &windows, 27)?
                .scenario()?
                .name
        );
        Ok(())
    }

    #[test]
    fn project_graphs_have_independent_replayable_roots() -> Result<()> {
        let versions = ["3.12", "3.13", "3.14"]
            .map(|version| PythonVersion::from_str(version).expect("valid Python version"));
        let targets = ScenarioTarget::matrix(
            &versions,
            &[
                ScenarioPlatform::Linux,
                ScenarioPlatform::Macos,
                ScenarioPlatform::Windows,
            ],
        );
        let anchor = targets.first().expect("nonempty target matrix");
        let options = SmallGraphOptions {
            packages: 3,
            versions: 2,
        };
        let mut satisfiable = 0;
        let mut unsatisfiable = 0;
        let mut varying_roots = false;
        let mut all_satisfiable_seeds = Vec::new();
        for seed in 0..64 {
            let document = generate_project_graph(seed, options, anchor, 27)?;
            assert_eq!(
                document.to_toml()?,
                generate_project_graph(seed, options, anchor, 27)?.to_toml()?
            );
            let replay: ScenarioDocument = document.to_toml()?.parse()?;
            let scenario = replay.scenario()?;
            assert!(scenario.testgen.disable);
            assert!(scenario.resolver_options.universal);
            let project = ScenarioProject::new(&scenario)?;
            let selections = project.selection_matrix();
            assert_eq!(selections.len(), 11);
            let all = project.all_selection();
            let mut all_targets_satisfiable = true;
            for (index, target) in targets.iter().enumerate() {
                let environment = target.markers()?;
                let search = project.oracle(&environment, &all)?.find_solution(27)?;
                let all_satisfiable = search.solution.is_some();
                assert!(search.checked <= 27);
                if index == 0 {
                    assert_eq!(scenario.expected.satisfiable, all_satisfiable);
                }
                all_targets_satisfiable &= all_satisfiable;
                if all_satisfiable {
                    satisfiable += 1;
                } else {
                    unsatisfiable += 1;
                }
                for selection in &selections {
                    let search = project.oracle(&environment, selection)?.find_solution(27)?;
                    assert!(search.checked <= 27);
                    let selected_satisfiable = search.solution.is_some();
                    assert!(!all_satisfiable || selected_satisfiable);
                    varying_roots |= selected_satisfiable != all_satisfiable;
                }
            }
            if all_targets_satisfiable {
                all_satisfiable_seeds.push(seed);
            }
        }
        assert!(satisfiable > 0);
        assert!(unsatisfiable > 0);
        assert!(varying_roots);
        assert_eq!(all_satisfiable_seeds, [39, 40, 46, 54]);
        Ok(())
    }
}
