//! Deterministic small dependency graphs for the exhaustive scenario checker.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use serde_json::{Map, Value, json};

use uv_normalize::PackageName;
use uv_pep440::{VersionSpecifiers, release_specifiers_to_ranges};
use uv_pep508::{Requirement, VersionOrUrl};

use super::check::ScenarioTarget;
use super::oracle::{ScenarioOracle, Selection, validate_scenario};
use super::project::ScenarioProject;
use super::scenario::ScenarioDocument;

/// The bounded shape of a generated dependency graph.
#[derive(Clone, Copy, Debug)]
pub struct SmallGraphOptions {
    pub packages: usize,
    pub versions: usize,
}

/// A project graph with a proposed compatible version for every package.
///
/// The assignment is a satisfiability witness, not a preferred resolver outcome. Some assigned
/// packages can be unreachable from a particular project selection or marker environment.
pub struct WitnessedProjectGraph {
    pub document: ScenarioDocument,
    pub assignment: Selection,
}

/// A conservative certificate for a fixed assignment over the project's entire Python domain.
#[derive(Debug, Serialize)]
pub struct ProjectWitnessCertificate {
    pub requires_python: VersionSpecifiers,
    pub assigned_packages: usize,
    pub checked_requirements: usize,
}

impl WitnessedProjectGraph {
    /// Certify a sufficient condition for universal satisfiability without sampling markers.
    ///
    /// Every possible root and every dependency of an assigned version must accept the fixed
    /// assignment, even when its marker is inactive. Assigned versions must provide universal
    /// Python 3 wheels and declare Python ranges covering the whole project range. This is
    /// deliberately stricter than uv's lower-bound-only dependency policy: failure to certify is
    /// not evidence of unsatisfiability.
    pub fn certify_universal_witness(&self) -> Result<ProjectWitnessCertificate> {
        let scenario = self.document.scenario()?;
        let project = ScenarioProject::new(&scenario)?;
        let requirements = project.requirements(&project.all_selection())?;
        validate_scenario(&scenario, &requirements)?;

        let requires_python = scenario
            .root
            .requires_python
            .as_ref()
            .context("a universal project witness requires an explicit root Python range")?;
        let python_range = release_specifiers_to_ranges(requires_python.clone());
        ensure!(
            !python_range.is_empty(),
            "a universal project witness requires a nonempty root Python range"
        );
        let python3 = release_specifiers_to_ranges(">=3,<4".parse()?);
        ensure!(
            python_range.intersection(&python3.complement()).is_empty(),
            "a universal project witness requires a Python 3 root range"
        );
        ensure!(
            self.assignment.len() == scenario.packages.len()
                && scenario
                    .packages
                    .keys()
                    .all(|name| self.assignment.contains_key(name)),
            "a universal project witness must assign every scenario package"
        );

        for requirement in &requirements {
            validate_witness_requirement(requirement, &self.assignment)?;
        }
        let mut checked_requirements = requirements.len();
        for (name, version) in &self.assignment {
            let metadata = scenario
                .packages
                .get(name)
                .and_then(|package| package.versions.get(version))
                .with_context(|| format!("the scenario has no {name}=={version}"))?;
            if let Some(dependency_python) = &metadata.requires_python {
                let dependency_range = release_specifiers_to_ranges(dependency_python.clone());
                ensure!(
                    python_range
                        .intersection(&dependency_range.complement())
                        .is_empty(),
                    "{name}=={version} does not declare support for the entire project Python range `{requires_python}`"
                );
            }
            for requirement in metadata
                .requires
                .iter()
                .chain(metadata.extras.values().flatten())
            {
                validate_witness_requirement(requirement, &self.assignment)?;
                checked_requirements += 1;
            }
        }

        Ok(ProjectWitnessCertificate {
            requires_python: requires_python.clone(),
            assigned_packages: self.assignment.len(),
            checked_requirements,
        })
    }

    /// Check the witness independently for every explicit project selection and target.
    ///
    /// Returns the number of checked projections. Successful samples do not prove the entire
    /// marker universe is satisfiable.
    pub fn check_witness(&self, targets: &[ScenarioTarget]) -> Result<usize> {
        ensure!(
            !targets.is_empty(),
            "at least one witness target is required"
        );
        let scenario = self.document.scenario()?;
        let project = ScenarioProject::new(&scenario)?;
        let selections = project.selection_matrix();
        let mut checked = 0;
        for target in targets {
            let environment = target.markers()?;
            for selection in &selections {
                let oracle = project.oracle(&environment, selection)?;
                let closure = oracle
                    .reachable_selection(&self.assignment)
                    .with_context(|| format!("invalid witness for {selection} on {target}"))?;
                oracle.validate(&closure)?;
                checked += 1;
            }
        }
        Ok(checked)
    }
}

fn validate_witness_requirement(requirement: &Requirement, assignment: &Selection) -> Result<()> {
    let version = assignment
        .get(&requirement.name)
        .with_context(|| format!("the witness is missing `{requirement}`"))?;
    match &requirement.version_or_url {
        Some(VersionOrUrl::VersionSpecifier(specifier)) => ensure!(
            specifier.contains(version),
            "{}=={version} does not satisfy `{requirement}`",
            requirement.name
        ),
        Some(VersionOrUrl::Url(_)) => {
            bail!("the scenario oracle does not model direct URLs: {requirement}");
        }
        None => {}
    }
    Ok(())
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
    expected_project_document(
        project_graph_value(seed, options, target, max_states)?,
        target,
        max_states,
    )
}

fn project_graph_value(
    seed: u64,
    options: SmallGraphOptions,
    target: &ScenarioTarget,
    max_states: usize,
) -> Result<Value> {
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
    Ok(value)
}

/// Generate a project graph around an independently checked satisfying assignment.
///
/// Every root requirement and every dependency of an assigned version accepts that assignment.
/// Alternative versions retain their arbitrary constraints, so the resolver can still encounter
/// backtracking and marker-dependent candidates. This only uses ordinary markers and additive
/// extras; non-additive extras and conflict declarations remain outside the oracle's contract.
pub fn generate_satisfiable_project_graph(
    seed: u64,
    options: SmallGraphOptions,
    targets: &[ScenarioTarget],
    max_states: usize,
) -> Result<WitnessedProjectGraph> {
    let target = targets
        .first()
        .context("at least one witness target is required")?;
    let mut value = project_graph_value(seed, options, target, max_states)?;
    let major = target.python.major();
    let minor = u16::from(target.python.minor());
    let middle = format!("{major}.{}", minor + 1);
    let upper = format!("{major}.{}", minor + 2);
    let names = value["packages"]
        .as_object()
        .context("generated packages must be a table")?
        .keys()
        .map(|name| name.parse())
        .collect::<Result<Vec<PackageName>, _>>()?;
    // Each generator owns its stream so adding a corpus cannot change existing replay seeds.
    let mut random = fastrand::Rng::with_seed(seed ^ 0x9cf6_80a2_b417_e35d);
    let versions = names
        .iter()
        .map(|name| (name.clone(), random.usize(1..=options.versions)))
        .collect::<BTreeMap<_, _>>();
    let assignment = versions
        .iter()
        .map(|(name, version)| Ok((name.clone(), format!("{version}.0.0").parse()?)))
        .collect::<Result<Selection>>()?;

    // Ensure the base project is nonempty, and give each optional root its own marked edge.
    value["root"]["requires"]
        .as_array_mut()
        .context("generated root requirements must be an array")?
        .push(json!(names[0].to_string()));
    for (root, name, marker) in [
        (
            "optional_dependencies",
            "one",
            "sys_platform == 'linux'".to_string(),
        ),
        (
            "optional_dependencies",
            "two",
            format!("python_version >= '{middle}'"),
        ),
        (
            "dependency_groups",
            "shared",
            "sys_platform != 'win32'".to_string(),
        ),
        (
            "dependency_groups",
            "dev",
            format!("python_version < '{upper}'"),
        ),
    ] {
        let dependency = &names[random.usize(0..names.len())];
        value["root"][root][name]
            .as_array_mut()
            .context("generated project requirements must be an array")?
            .push(json!(format!("{dependency}[feature]; {marker}")));
    }

    make_requirements_compatible(
        &mut value["root"]["requires"],
        &versions,
        options.versions,
        &mut random,
    )?;
    for kind in ["optional_dependencies", "dependency_groups"] {
        for requirements in value["root"][kind]
            .as_object_mut()
            .context("generated project dependencies must be a table")?
            .values_mut()
        {
            make_requirements_compatible(requirements, &versions, options.versions, &mut random)?;
        }
    }

    let python_range = value["root"]["requires_python"].clone();
    for (name, version) in &versions {
        let metadata = &mut value["packages"][name.as_str()]["versions"][format!("{version}.0.0")];
        metadata["requires_python"] = python_range.clone();
        make_requirements_compatible(
            &mut metadata["requires"],
            &versions,
            options.versions,
            &mut random,
        )?;
        for requirements in metadata["extras"]
            .as_object_mut()
            .context("generated extras must be a table")?
            .values_mut()
        {
            make_requirements_compatible(requirements, &versions, options.versions, &mut random)?;
        }
    }
    value["name"] = json!(format!(
        "satisfiable-project-graph-{seed:016x}-{}p-{}v-py{}-through{major}.{}-{}",
        options.packages,
        options.versions,
        target.python,
        minor + 2,
        target.platform,
    ));
    value["description"] = json!(format!(
        "Finite project graph generated from seed {seed} with a compatible version assignment. Its expected outcome includes all project roots for {target}."
    ));
    let document = expected_project_document(value, target, max_states)?;
    let graph = WitnessedProjectGraph {
        document,
        assignment,
    };
    graph.certify_universal_witness()?;
    graph.check_witness(targets)?;
    ensure!(
        graph.document.scenario()?.expected.satisfiable,
        "the generated witness disagrees with the exhaustive search"
    );
    Ok(graph)
}

fn make_requirements_compatible(
    requirements: &mut Value,
    versions: &BTreeMap<PackageName, usize>,
    version_count: usize,
    random: &mut fastrand::Rng,
) -> Result<()> {
    for value in requirements
        .as_array_mut()
        .context("generated requirements must be an array")?
    {
        if let Some(object) = value.as_object() {
            ensure!(
                object.len() == 1 && object.get("include-group").is_some_and(Value::is_string),
                "generated dependency objects must be group includes"
            );
            continue;
        }
        let mut requirement: Requirement = value
            .as_str()
            .context("generated requirements must be strings")?
            .parse()?;
        let version = *versions
            .get(&requirement.name)
            .context("generated requirement must name an assigned package")?;
        let lower = random.usize(1..=version);
        let upper = random.usize(version + 1..=version_count + 1);
        let specifiers = [
            String::new(),
            format!("=={version}"),
            format!(">={lower}"),
            format!("<{upper}"),
            format!("!={}", version + 1),
            format!(">={lower},<{upper}"),
        ];
        let specifier = &specifiers[random.usize(0..specifiers.len())];
        requirement.version_or_url = if specifier.is_empty() {
            None
        } else {
            Some(VersionOrUrl::VersionSpecifier(specifier.parse()?))
        };
        *value = json!(requirement.to_string());
    }
    Ok(())
}

fn expected_project_document(
    mut value: Value,
    target: &ScenarioTarget,
    max_states: usize,
) -> Result<ScenarioDocument> {
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
    use std::collections::BTreeSet;
    use std::str::FromStr;

    use sha2::{Digest, Sha256};
    use uv_pep440::Version;
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

    #[test]
    fn satisfiable_project_graphs_have_checked_witnesses() -> Result<()> {
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
        let options = SmallGraphOptions {
            packages: 3,
            versions: 2,
        };
        let highest: Version = "2.0.0".parse()?;
        let mut non_highest_assignment = false;
        let mut varying_roots = false;
        let mut varying_environments = false;
        for seed in 0..32 {
            let graph = generate_satisfiable_project_graph(seed, options, &targets, 27)?;
            let repeated = generate_satisfiable_project_graph(seed, options, &targets, 27)?;
            assert_eq!(graph.document.to_toml()?, repeated.document.to_toml()?);
            assert_eq!(graph.assignment, repeated.assignment);
            let certificate = graph.certify_universal_witness()?;
            assert_eq!(certificate.assigned_packages, options.packages);
            assert!(certificate.checked_requirements > 0);
            assert_eq!(certificate.requires_python, ">=3.12,<3.15".parse()?);
            assert_eq!(graph.check_witness(&targets)?, 99);
            let replay: ScenarioDocument = graph.document.to_toml()?.parse()?;
            let scenario = replay.scenario()?;
            assert!(scenario.expected.satisfiable);
            assert_eq!(scenario.packages.len(), options.packages);
            assert_eq!(graph.assignment.len(), options.packages);
            for (name, version) in &graph.assignment {
                assert_eq!(scenario.packages[name].versions.len(), options.versions);
                assert!(scenario.packages[name].versions.contains_key(version));
                non_highest_assignment |= *version != highest;
            }
            let project = ScenarioProject::new(&scenario)?;
            let selections = project.selection_matrix();
            let all = project.all_selection();
            let mut all_closures = BTreeSet::new();
            for target in &targets {
                let environment = target.markers()?;
                let all_closure = project
                    .oracle(&environment, &all)?
                    .reachable_selection(&graph.assignment)?;
                all_closures.insert(all_closure.clone());
                for selection in &selections {
                    let oracle = project.oracle(&environment, selection)?;
                    let closure = oracle.reachable_selection(&graph.assignment)?;
                    oracle.validate(&closure)?;
                    let search = oracle.find_solution(27)?;
                    assert!(search.checked <= 27);
                    assert!(search.solution.is_some());
                    varying_roots |= closure != all_closure;
                }
            }
            varying_environments |= all_closures.len() > 1;
        }
        assert!(non_highest_assignment);
        assert!(varying_roots);
        assert!(varying_environments);
        insta::assert_snapshot!(
            generate_satisfiable_project_graph(0, options, &targets, 26)
                .err().expect("search should be bounded"),
            @"scenario search space exceeds 26 selections"
        );
        Ok(())
    }

    #[test]
    fn rejects_invalid_project_witnesses() -> Result<()> {
        let target = ScenarioTarget {
            python: PythonVersion::from_str("3.12").expect("valid Python version"),
            platform: ScenarioPlatform::Linux,
        };
        let graph = WitnessedProjectGraph {
            document: r#"
name = "invalid-witness"
[root]
requires = ["a>=2"]
[expected]
satisfiable = true
[packages.a.versions."1"]
[packages.a.versions."2"]
"#
            .parse()?,
            assignment: [("a".parse()?, "1".parse()?)].into(),
        };
        insta::assert_snapshot!(
            format!("{:#}", graph.check_witness(&[target]).expect_err("incompatible witness")),
            @"invalid witness for project on CPython 3.12 on x86_64-unknown-linux-gnu: a==1 does not satisfy `a>=2`"
        );
        insta::assert_snapshot!(
            graph.check_witness(&[]).expect_err("a target is required"),
            @"at least one witness target is required"
        );
        Ok(())
    }

    #[test]
    fn universal_witness_checks_unsampled_marked_edges() -> Result<()> {
        let target = ScenarioTarget {
            python: PythonVersion::from_str("3.12").expect("valid Python version"),
            platform: ScenarioPlatform::Linux,
        };
        let graph = WitnessedProjectGraph {
            document: r#"
name = "unsampled-witness-edge"
[root]
requires_python = ">=3.12,<3.15"
requires = ["a"]
[expected]
satisfiable = true
[packages.a.versions."1"]
requires = ["b>=2; sys_platform == 'freebsd'"]
[packages.b.versions."1"]
[packages.b.versions."2"]
"#
            .parse()?,
            assignment: [("a".parse()?, "1".parse()?), ("b".parse()?, "1".parse()?)].into(),
        };
        assert_eq!(graph.check_witness(&[target])?, 1);
        insta::assert_snapshot!(
            graph.certify_universal_witness().expect_err("unsampled incompatible edge"),
            @"b==1 does not satisfy `b>=2 ; sys_platform == 'freebsd'`"
        );
        Ok(())
    }

    #[test]
    fn universal_witness_checks_the_entire_python_domain() -> Result<()> {
        let target = ScenarioTarget {
            python: PythonVersion::from_str("3.14").expect("valid Python version"),
            platform: ScenarioPlatform::Linux,
        };
        let graph = WitnessedProjectGraph {
            document: r#"
name = "narrow-witness-python"
[root]
requires_python = ">=3.12,<3.15"
requires = ["a"]
[expected]
satisfiable = true
[packages.a.versions."1"]
requires_python = ">=3.12,<3.13"
"#
            .parse()?,
            assignment: [("a".parse()?, "1".parse()?)].into(),
        };
        // The concrete oracle follows uv's lower-bound-only dependency policy. Certification is
        // deliberately stricter, so an uncertified assignment is not an unsatisfiable scenario.
        assert_eq!(graph.check_witness(&[target])?, 1);
        insta::assert_snapshot!(
            graph.certify_universal_witness().expect_err("narrow declared Python range"),
            @"a==1 does not declare support for the entire project Python range `>=3.12, <3.15`"
        );

        let mut value = graph.document.value().clone();
        value["packages"]["a"]["versions"]["1"]["requires_python"] =
            toml::Value::String(">=3.11,<3.16".to_string());
        let graph = WitnessedProjectGraph {
            document: ScenarioDocument::from_value(value)?,
            assignment: graph.assignment,
        };
        let certificate = graph.certify_universal_witness()?;
        assert_eq!(certificate.requires_python, ">=3.12,<3.15".parse()?);
        assert_eq!(certificate.assigned_packages, 1);
        assert_eq!(certificate.checked_requirements, 1);
        Ok(())
    }

    #[test]
    fn universal_witness_rejects_unsupported_domains() -> Result<()> {
        let graph = WitnessedProjectGraph {
            document: r#"
name = "unbounded-witness-python"
[root]
requires_python = ">=3.12"
requires = ["a"]
[expected]
satisfiable = true
[packages.a.versions."1"]
"#
            .parse()?,
            assignment: [("a".parse()?, "1".parse()?)].into(),
        };
        insta::assert_snapshot!(
            graph.certify_universal_witness().expect_err("py3 wheels need a Python 3 domain"),
            @"a universal project witness requires a Python 3 root range"
        );

        let mut value = graph.document.value().clone();
        value["root"]["requires_python"] = toml::Value::String(">=3.12,<3.15".to_string());
        value["packages"]["a"]["versions"]["1"]
            .as_table_mut()
            .context("package metadata must be a table")?
            .insert(
                "requires".to_string(),
                toml::Value::Array(vec![toml::Value::String(
                    "a; extra != 'feature'".to_string(),
                )]),
            );
        let graph = WitnessedProjectGraph {
            document: ScenarioDocument::from_value(value)?,
            assignment: graph.assignment,
        };
        insta::assert_snapshot!(
            graph.certify_universal_witness().expect_err("non-additive extra"),
            @"the scenario oracle does not model non-additive extras: a ; extra != 'feature'"
        );
        Ok(())
    }
}
