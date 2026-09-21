use std::str::FromStr;

use anyhow::{Context, Result, bail};

use uv_python::PythonVersion;
use uv_resolver::ForkStrategy;
use uv_static::EnvVars;
use uv_test::packse::PackseServer;
use uv_test::packse::check::{
    LockCheckOptions, LockCheckResult, LockEvidenceMode, LockScenarioFailureKind, LockfileMode,
    ScenarioPlatform, ScenarioTarget, check_lock_scenario, check_lock_scenario_with_artifacts,
    check_project_lock_scenario, check_project_lock_scenario_with_artifacts, check_scenario,
    check_witnessed_project_lock_scenario, check_witnessed_project_lock_scenario_with_artifacts,
};
use uv_test::packse::generate::{
    SmallGraphOptions, WitnessedProjectGraph, generate_marker_graph, generate_project_graph,
    generate_satisfiable_project_graph, generate_small_graph,
};
use uv_test::packse::lock_score::score_lock_versions;
use uv_test::packse::minimize::minimize_witnessed_project_lock_scenario;
use uv_test::packse::oracle::Selection;
use uv_test::packse::project::{ProjectSelection, ScenarioProject};
use uv_test::packse::scenario::{Resolution, Scenario, ScenarioDocument};

#[test]
fn prerelease_scenarios_match_the_exhaustive_oracle() -> Result<()> {
    let target = ScenarioTarget {
        python: PythonVersion::from_str("3.12").expect("valid Python version"),
        platform: ScenarioPlatform::Linux,
    };
    for path in [
        "prereleases/package-only-prereleases-in-range.toml",
        "prereleases/transitive-prerelease-and-stable-dependency.toml",
    ] {
        for prereleases in [false, true] {
            let context = uv_test::test_context!("3.12");
            let mut scenario =
                Scenario::from_path(&context.workspace_root.join("test/scenarios").join(path))?;
            scenario.resolver_options.prereleases = prereleases;
            let result = check_scenario(&context, &scenario, &target, 100_000)?;
            assert_eq!(result.selection, Some(scenario.expected.packages));
        }
    }
    Ok(())
}

#[test]
fn prerelease_locks_match_their_concrete_projections() -> Result<()> {
    let contents = r#"
name = "prerelease-lock-domain"
[root]
requires_python = ">=3.12,<3.14"
requires = ["a"]
[expected]
satisfiable = true
[packages.a.versions."1rc1"]
[packages.a.versions."2"]
requires = ["missing"]
"#;
    let targets = ScenarioTarget::matrix(
        &["3.12", "3.13"]
            .map(|version| PythonVersion::from_str(version).expect("valid Python version")),
        &[ScenarioPlatform::Linux, ScenarioPlatform::Windows],
    );
    for prereleases in [false, true] {
        let graph = WitnessedProjectGraph {
            document: format!("{contents}\n[resolver_options]\nprereleases = {prereleases}\n")
                .parse()?,
            assignment: [("a".parse()?, "1rc1".parse()?)].into_iter().collect(),
        };
        let scenario = graph.document.scenario()?;
        let selections = ScenarioProject::new(&scenario)?.selection_matrix();
        for lockfile in [LockfileMode::Standard, LockfileMode::WithoutMetadata] {
            let options = LockCheckOptions {
                max_states: 100_000,
                lockfile,
                evidence: LockEvidenceMode::PrintedV1,
            };
            let context = uv_test::test_context!("3.12");
            assert!(matches!(
                check_lock_scenario(&context, &scenario, &targets, options)?,
                LockCheckResult::Satisfiable { projections, .. }
                    if projections == targets.len()
            ));
            let context = uv_test::test_context!("3.12");
            assert!(matches!(
                check_witnessed_project_lock_scenario(
                    &context,
                    &graph,
                    &targets,
                    &selections,
                    options,
                    100_000,
                )?,
                LockCheckResult::Satisfiable { projections, .. }
                    if projections == targets.len() * selections.len()
            ));
        }
    }
    Ok(())
}

#[test]
fn fixed_scenarios_match_the_exhaustive_oracle() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let mut target = ScenarioTarget {
        python: PythonVersion::from_str("3.12").expect("valid Python version"),
        platform: ScenarioPlatform::Linux,
    };
    let paths = [
        "backtracking/wrong-backtracking-basic.toml",
        "incompatible_versions/transitive-incompatible-with-root-version.toml",
        "extras/all-extras-required.toml",
        "fork/basic.toml",
        "requires_python/python-less-than-current.toml",
    ];
    let mut outcomes = Vec::new();
    for path in paths {
        let scenario =
            Scenario::from_path(&context.workspace_root.join("test/scenarios").join(path))?;
        let result = check_scenario(&context, &scenario, &target, 100_000)?;
        outcomes.push((scenario.name, result.selection.is_some()));
    }
    target.platform = ScenarioPlatform::Macos;
    let scenario = Scenario::from_path(
        &context
            .workspace_root
            .join("test/scenarios/fork/basic.toml"),
    )?;
    let result = check_scenario(&context, &scenario, &target, 100_000)?;
    outcomes.push((
        format!("{}-macos", scenario.name),
        result.selection.is_some(),
    ));
    insta::assert_debug_snapshot!(outcomes, @r#"
    [
        (
            "wrong-backtracking-basic",
            true,
        ),
        (
            "transitive-incompatible-with-root-version",
            false,
        ),
        (
            "all-extras-required",
            true,
        ),
        (
            "fork-basic",
            true,
        ),
        (
            "python-less-than-current",
            true,
        ),
        (
            "fork-basic-macos",
            true,
        ),
    ]
    "#);
    Ok(())
}

#[test]
fn universal_locks_match_their_concrete_projections() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let targets = ["3.12", "3.13", "3.14"]
        .into_iter()
        .flat_map(|python| {
            [
                ScenarioPlatform::Linux,
                ScenarioPlatform::Macos,
                ScenarioPlatform::Windows,
            ]
            .into_iter()
            .map(move |platform| ScenarioTarget {
                python: PythonVersion::from_str(python).expect("valid Python version"),
                platform,
            })
        })
        .collect::<Vec<_>>();
    let paths = [
        "backtracking/wrong-backtracking-basic.toml",
        "incompatible_versions/transitive-incompatible-with-root-version.toml",
        "extras/all-extras-required.toml",
        "fork/basic.toml",
        "fork/incomplete-markers.toml",
    ];
    let mut outcomes = Vec::new();
    for path in paths {
        let scenario =
            Scenario::from_path(&context.workspace_root.join("test/scenarios").join(path))?;
        let result = check_lock_scenario(
            &context,
            &scenario,
            &targets,
            LockCheckOptions::new(100_000),
        )?;
        outcomes.push((
            scenario.name,
            matches!(result, LockCheckResult::Satisfiable { .. }),
        ));
    }
    insta::assert_debug_snapshot!(outcomes, @r#"
    [
        (
            "wrong-backtracking-basic",
            true,
        ),
        (
            "transitive-incompatible-with-root-version",
            false,
        ),
        (
            "all-extras-required",
            true,
        ),
        (
            "fork-basic",
            true,
        ),
        (
            "fork-incomplete-markers",
            true,
        ),
    ]
    "#);
    Ok(())
}

#[test]
fn lock_version_scores_use_the_actual_universal_lock() -> Result<()> {
    let targets = ScenarioTarget::matrix(
        &[PythonVersion::from_str("3.12").expect("valid Python version")],
        &[
            ScenarioPlatform::Linux,
            ScenarioPlatform::Macos,
            ScenarioPlatform::Windows,
        ],
    );
    for lockfile in [LockfileMode::Standard, LockfileMode::WithoutMetadata] {
        let context = uv_test::test_context!("3.12");
        let scenario = Scenario::from_path(
            &context
                .workspace_root
                .join("test/scenarios/fork/basic.toml"),
        )?;
        let result = check_lock_scenario(
            &context,
            &scenario,
            &targets,
            LockCheckOptions {
                lockfile,
                ..LockCheckOptions::new(100_000)
            },
        )?;
        assert!(matches!(result, LockCheckResult::Satisfiable { .. }));

        let lock = context.read("uv.lock");
        assert_eq!(score_lock_versions(&lock)?.excess_versions(), 1);
        assert_eq!(context.read("uv.lock"), lock);
    }
    Ok(())
}

#[test]
fn explicit_resolution_policies_survive_lock_round_trips() -> Result<()> {
    let targets = ScenarioTarget::matrix(
        &["3.12", "3.13"]
            .map(|version| PythonVersion::from_str(version).expect("valid Python version")),
        &[ScenarioPlatform::Linux],
    );
    let document: ScenarioDocument = r#"
name = "fork-policy-oracle"
[root]
requires_python = ">=3.12,<3.14"
requires = ["a"]
[expected]
satisfiable = true
[packages.a.versions."1.0.0"]
requires_python = ">=3.12"
[packages.a.versions."2.0.0"]
requires_python = ">=3.13"
"#
    .parse()?;
    for lockfile in [LockfileMode::Standard, LockfileMode::WithoutMetadata] {
        for resolution in [
            Resolution::Highest,
            Resolution::Lowest,
            Resolution::LowestDirect,
        ] {
            for fork_strategy in [ForkStrategy::RequiresPython, ForkStrategy::Fewest] {
                let context = uv_test::test_context!("3.12");
                let mut scenario = document.scenario()?;
                scenario.resolver_options.resolution = Some(resolution);
                scenario.resolver_options.fork_strategy = Some(fork_strategy);
                let result = check_lock_scenario(
                    &context,
                    &scenario,
                    &targets,
                    LockCheckOptions {
                        lockfile,
                        ..LockCheckOptions::new(100)
                    },
                )?;
                assert!(matches!(result, LockCheckResult::Satisfiable { .. }));
                let expected = usize::from(
                    resolution == Resolution::Highest
                        && fork_strategy == ForkStrategy::RequiresPython,
                );
                assert_eq!(
                    score_lock_versions(&context.read("uv.lock"))?.excess_versions(),
                    expected,
                    "{lockfile:?}, {resolution}, {fork_strategy}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn metadata_free_locks_match_their_concrete_projections() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let targets = ScenarioTarget::matrix(
        &["3.12", "3.13", "3.14"]
            .map(|version| PythonVersion::from_str(version).expect("valid Python version")),
        &[
            ScenarioPlatform::Linux,
            ScenarioPlatform::Macos,
            ScenarioPlatform::Windows,
        ],
    );
    let options = LockCheckOptions {
        max_states: 100_000,
        lockfile: LockfileMode::WithoutMetadata,
        evidence: LockEvidenceMode::PrintedV1,
    };
    let scenario = Scenario::from_path(
        &context
            .workspace_root
            .join("test/scenarios/fork/incomplete-markers.toml"),
    )?;
    assert!(matches!(
        check_lock_scenario(&context, &scenario, &targets, options)?,
        LockCheckResult::Satisfiable { projections, .. } if projections == targets.len()
    ));
    assert_metadata_free_lock(&context.read("uv.lock"))?;

    let project_targets = ScenarioTarget::matrix(
        &["3.12", "3.13"]
            .map(|version| PythonVersion::from_str(version).expect("valid Python version")),
        &[
            ScenarioPlatform::Linux,
            ScenarioPlatform::Macos,
            ScenarioPlatform::Windows,
        ],
    );
    for (path, targets) in [
        ("fork/empty-extra-lock-roundtrip.toml", targets.as_slice()),
        (
            "project/selection-projections.toml",
            project_targets.as_slice(),
        ),
    ] {
        let context = uv_test::test_context!("3.12");
        let scenario =
            Scenario::from_path(&context.workspace_root.join("test/scenarios").join(path))?;
        let selections = ScenarioProject::new(&scenario)?.selection_matrix();
        assert!(matches!(
            check_project_lock_scenario(&context, &scenario, targets, &selections, options)?,
            LockCheckResult::Satisfiable { projections, .. }
                if projections == targets.len() * selections.len()
        ));
        assert_metadata_free_lock(&context.read("uv.lock"))?;
    }
    Ok(())
}

fn assert_metadata_free_lock(lock: &str) -> Result<()> {
    let lock: toml::Value = toml::from_str(lock)?;
    assert_eq!(lock["version"].as_integer(), Some(1));
    assert_eq!(lock["revision"].as_integer(), Some(4));
    assert!(
        lock["package"]
            .as_array()
            .expect("lockfile packages")
            .iter()
            .all(|package| package.get("metadata").is_none())
    );
    Ok(())
}

#[test]
fn captures_unsampled_universal_conflicts() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = ScenarioTarget {
        python: PythonVersion::from_str("3.12").expect("valid Python version"),
        platform: ScenarioPlatform::Linux,
    };
    let document = ScenarioDocument::from_path(
        &context
            .workspace_root
            .join("test/scenarios/fork/conflict-in-fork.toml"),
    )?;
    let directory = context.temp_dir.join("failure");
    let error = check_lock_scenario_with_artifacts(
        &context,
        &document,
        std::slice::from_ref(&target),
        LockCheckOptions::new(100_000),
        &directory,
    )
    .expect_err("the Linux projection does not witness the other platform's conflict");
    assert!(format!("{error:#}").contains("requested projections are satisfiable"));
    assert_eq!(LockScenarioFailureKind::from_error(&error), None);

    let failure: serde_json::Value =
        serde_json::from_slice(&fs_err::read(directory.join("failure.json"))?)?;
    assert!(failure["kind"].is_null());
    assert_eq!(failure["targets"][0]["python"], "3.12");
    assert_eq!(failure["options"]["max_states"], 100_000);
    assert_eq!(failure["options"]["lockfile"], "standard");
    let command: serde_json::Value = serde_json::from_slice(&fs_err::read(
        directory.join("commands/01-lock/command.json"),
    )?)?;
    assert_eq!(command["status"], 1);
    assert_eq!(command["sha256"].as_str().expect("binary digest").len(), 64);
    assert!(directory.join("pyproject.toml").is_file());
    let index: serde_json::Value =
        serde_json::from_slice(&fs_err::read(directory.join("index/index.json"))?)?;
    for file in index["files"].as_array().expect("served distributions") {
        assert!(
            directory
                .join("index/files")
                .join(file["filename"].as_str().expect("distribution filename"))
                .is_file()
        );
    }
    let original = fs_err::read(directory.join("scenario.toml"))?;
    let error = check_lock_scenario_with_artifacts(
        &context,
        &document,
        std::slice::from_ref(&target),
        LockCheckOptions::new(100_000),
        &directory,
    )
    .expect_err("evidence directories cannot be replaced");
    assert!(format!("{error:#}").contains("failed to save lock evidence"));
    assert_eq!(fs_err::read(directory.join("scenario.toml"))?, original);

    let directory = context.temp_dir.join("metadata-free-failure");
    let error = check_lock_scenario_with_artifacts(
        &context,
        &document,
        std::slice::from_ref(&target),
        LockCheckOptions {
            max_states: 100_000,
            lockfile: LockfileMode::WithoutMetadata,
            evidence: LockEvidenceMode::PrintedV1,
        },
        &directory,
    )
    .expect_err("the unsampled conflict remains unclassified in metadata-free mode");
    assert_eq!(LockScenarioFailureKind::from_error(&error), None);
    let failure: serde_json::Value =
        serde_json::from_slice(&fs_err::read(directory.join("failure.json"))?)?;
    assert_eq!(failure["options"]["lockfile"], "without-metadata");
    let command: serde_json::Value = serde_json::from_slice(&fs_err::read(
        directory.join("commands/01-lock/command.json"),
    )?)?;
    assert!(
        command["args"]
            .as_array()
            .expect("command arguments")
            .windows(2)
            .any(|args| args[0] == "--preview-features" && args[1] == "lock-without-metadata")
    );

    let document = ScenarioDocument::from_path(
        &context
            .workspace_root
            .join("test/scenarios/fork/conflict-unsatisfiable.toml"),
    )?;
    let matched = context.temp_dir.join("matched");
    assert!(matches!(
        check_lock_scenario_with_artifacts(
            &context,
            &document,
            &[target],
            LockCheckOptions::new(100_000),
            &matched,
        )?,
        LockCheckResult::Unsatisfiable { .. }
    ));
    assert!(!matched.exists());
    Ok(())
}

#[test]
fn project_locks_match_explicit_root_selections() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let scenario = Scenario::from_path(
        &context
            .workspace_root
            .join("test/scenarios/project/selection-projections.toml"),
    )?;
    let targets = ScenarioTarget::matrix(
        &["3.12", "3.13"]
            .map(|version| PythonVersion::from_str(version).expect("valid Python version")),
        &[
            ScenarioPlatform::Linux,
            ScenarioPlatform::Macos,
            ScenarioPlatform::Windows,
        ],
    );
    let selections = ScenarioProject::new(&scenario)?.selection_matrix();
    assert_eq!(selections.len(), 11);
    match check_project_lock_scenario(
        &context,
        &scenario,
        &targets,
        &selections,
        LockCheckOptions::new(100_000),
    )? {
        LockCheckResult::Satisfiable { projections, .. } => {
            assert_eq!(projections, targets.len() * selections.len());
        }
        LockCheckResult::Unsatisfiable { witness, .. } => {
            panic!("project roots are satisfiable for {witness}");
        }
    }
    Ok(())
}

#[test]
fn project_locks_check_unselected_roots() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = ScenarioTarget {
        python: PythonVersion::from_str("3.12").expect("valid Python version"),
        platform: ScenarioPlatform::Linux,
    };
    let document = ScenarioDocument::from_path(
        &context
            .workspace_root
            .join("test/scenarios/project/combined-roots-conflict.toml"),
    )?;
    let scenario = document.scenario()?;
    let project = ScenarioProject::new(&scenario)?;
    let all = project.all_selection();
    let environment = target.markers()?;
    for selection in [
        ProjectSelection::default(),
        ProjectSelection {
            extras: all.extras.clone(),
            ..ProjectSelection::default()
        },
        ProjectSelection {
            groups: all.groups.clone(),
            ..ProjectSelection::default()
        },
    ] {
        assert!(
            project
                .oracle(&environment, &selection)?
                .find_solution(100_000)?
                .solution
                .is_some()
        );
    }
    assert!(
        project
            .oracle(&environment, &all)?
            .find_solution(100_000)?
            .solution
            .is_none()
    );
    let directory = context.temp_dir.join("failure");
    assert!(matches!(
        check_project_lock_scenario_with_artifacts(
            &context,
            &document,
            &[target],
            &[ProjectSelection::default()],
            LockCheckOptions::new(100_000),
            &directory,
        )?,
        LockCheckResult::Unsatisfiable { .. }
    ));
    assert!(!directory.exists());
    Ok(())
}

#[test]
fn project_locks_reject_unsupported_roots() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = ScenarioTarget {
        python: PythonVersion::from_str("3.12").expect("valid Python version"),
        platform: ScenarioPlatform::Linux,
    };
    for (root, expected_error) in [
        (
            "[root.dependency_groups]\na = [{include-group = 'b'}]\nb = [{include-group = 'a'}]",
            "dependency group include cycle",
        ),
        (
            "[root]\nrequires = [\"dep; extra == 'docs'\"]",
            "does not model root extra markers",
        ),
        (
            "[root.dependency_groups]\ndev = [\"dep; extra == 'docs'\"]",
            "does not model root extra markers",
        ),
    ] {
        let document = ScenarioDocument::from_str(&format!(
            "name = 'unsupported-project-root'\n{root}\n[expected]\nsatisfiable = true\n"
        ))?;
        let directory = context.temp_dir.join("failure");
        let error = check_project_lock_scenario_with_artifacts(
            &context,
            &document,
            std::slice::from_ref(&target),
            &[ProjectSelection::default()],
            LockCheckOptions::new(100_000),
            &directory,
        )
        .expect_err("the project root is outside the checker contract");
        assert!(format!("{error:#}").contains(expected_error));
        assert!(!context.temp_dir.join("pyproject.toml").exists());
        assert!(!directory.exists());
    }
    Ok(())
}

#[test]
fn generated_small_graphs_match_the_exhaustive_oracle() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = ScenarioTarget {
        python: PythonVersion::from_str("3.12").expect("valid Python version"),
        platform: ScenarioPlatform::Linux,
    };
    let options = SmallGraphOptions {
        packages: 3,
        versions: 2,
    };
    let mut satisfiable = 0;
    let mut unsatisfiable = 0;
    for seed in 0..32 {
        let scenario = generate_small_graph(seed, options, &target, 27)?.scenario()?;
        let result = check_scenario(&context, &scenario, &target, 27)
            .with_context(|| format!("generated graph seed {seed}"))?;
        if result.selection.is_some() {
            satisfiable += 1;
        } else {
            unsatisfiable += 1;
        }
        if seed < 8 {
            let result = check_lock_scenario(
                &context,
                &scenario,
                std::slice::from_ref(&target),
                LockCheckOptions::new(27),
            )
            .with_context(|| format!("generated lock graph seed {seed}"))?;
            assert_eq!(
                scenario.expected.satisfiable,
                matches!(result, LockCheckResult::Satisfiable { .. })
            );
        }
    }
    assert_eq!(satisfiable + unsatisfiable, 32);
    assert!(satisfiable > 0);
    assert!(unsatisfiable > 0);
    Ok(())
}

#[test]
fn generated_marker_graphs_match_their_concrete_projections() -> Result<()> {
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
    for seed in 0..4 {
        let scenario = generate_marker_graph(seed, options, anchor, 27)?.scenario()?;
        for target in &targets {
            let context = uv_test::test_context!("3.12");
            let result = check_scenario(&context, &scenario, target, 27)
                .with_context(|| format!("generated marker graph seed {seed} for {target}"))?;
            if result.selection.is_some() {
                satisfiable += 1;
            } else {
                unsatisfiable += 1;
            }
        }
        if seed < 2 {
            let context = uv_test::test_context!("3.12");
            check_lock_scenario(&context, &scenario, &targets, LockCheckOptions::new(27))
                .with_context(|| format!("generated marker lock graph seed {seed}"))?;
        }
    }
    assert_eq!(satisfiable + unsatisfiable, 36);
    assert!(satisfiable > 0);
    assert!(unsatisfiable > 0);
    Ok(())
}

#[test]
fn generated_project_graphs_match_selected_exports() -> Result<()> {
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
    for (seed, satisfiable) in [(0, false), (39, true)] {
        let scenario = generate_project_graph(seed, options, anchor, 27)?.scenario()?;
        let selections = ScenarioProject::new(&scenario)?.selection_matrix();
        let context = uv_test::test_context!("3.12");
        let result = check_project_lock_scenario(
            &context,
            &scenario,
            &targets,
            &selections,
            LockCheckOptions::new(27),
        )
        .with_context(|| format!("generated project lock graph seed {seed}"))?;
        assert_eq!(
            matches!(result, LockCheckResult::Satisfiable { .. }),
            satisfiable
        );
        if let LockCheckResult::Satisfiable { projections, .. } = result {
            assert_eq!(projections, targets.len() * selections.len());
        }
    }
    Ok(())
}

#[test]
fn witnessed_project_graphs_match_selected_exports() -> Result<()> {
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
    for seed in [0, 1] {
        let graph = generate_satisfiable_project_graph(seed, options, &targets, 27)?;
        assert_eq!(graph.certify_universal_witness()?.assigned_packages, 3);
        assert_eq!(graph.check_witness(&targets)?, 99);
        let scenario = graph.document.scenario()?;
        let selections = ScenarioProject::new(&scenario)?.selection_matrix();
        let context = uv_test::test_context!("3.12");
        let result = check_witnessed_project_lock_scenario(
            &context,
            &graph,
            &targets,
            &selections,
            LockCheckOptions::new(27),
            100_000,
        )
        .with_context(|| format!("witnessed project lock graph seed {seed}"))?;
        let LockCheckResult::Satisfiable { projections, .. } = result else {
            bail!("witnessed project lock graph seed {seed} must be satisfiable");
        };
        assert_eq!(projections, targets.len() * selections.len());
    }
    Ok(())
}

#[test]
fn certified_project_locks_match_their_concrete_projections() -> Result<()> {
    let targets = ScenarioTarget::matrix(
        &["3.12", "3.13", "3.14"]
            .map(|version| PythonVersion::from_str(version).expect("valid Python version")),
        &[
            ScenarioPlatform::Linux,
            ScenarioPlatform::Macos,
            ScenarioPlatform::Windows,
        ],
    );
    for lockfile in [LockfileMode::Standard, LockfileMode::WithoutMetadata] {
        let context = uv_test::test_context!("3.12");
        let graph = WitnessedProjectGraph {
            document: ScenarioDocument::from_path(
                &context
                    .workspace_root
                    .join("test/scenarios/fork/non-local-fork-marker-unreachable.toml"),
            )?,
            assignment: [("a".parse()?, "1.0.0".parse()?)].into_iter().collect(),
        };
        let scenario = graph.document.scenario()?;
        let selections = ScenarioProject::new(&scenario)?.selection_matrix();
        let result = check_witnessed_project_lock_scenario(
            &context,
            &graph,
            &targets,
            &selections,
            LockCheckOptions {
                max_states: 100_000,
                lockfile,
                evidence: LockEvidenceMode::PrintedV1,
            },
            100_000,
        )?;
        assert!(matches!(
            result,
            LockCheckResult::Satisfiable { projections, .. }
                if projections == targets.len() * selections.len()
        ));
    }
    Ok(())
}

#[test]
fn restricted_lock_domains_match_their_concrete_projections() -> Result<()> {
    let graph = WitnessedProjectGraph {
        document: r#"
name = "restricted-lock-domain"
[root]
requires_python = ">=3.12,<3.15"
requires = ["a", "missing; sys_platform != 'win32'"]
optional_dependencies = { feature = ["a"] }
dependency_groups = { dev = ["a"] }
[expected]
satisfiable = true
[resolver_options]
fork_strategy = "fewest"
environments = ["sys_platform == 'win32' and python_version >= '3.13'"]
[packages.a.versions."1.0.0"]
requires_python = ">=3.13,<3.15"
"#
        .parse()?,
        assignment: [("a".parse()?, "1.0.0".parse()?)].into_iter().collect(),
    };
    let scenario = graph.document.scenario()?;
    let selections = ScenarioProject::new(&scenario)?.selection_matrix();
    let targets = ScenarioTarget::matrix(
        &["3.13", "3.14"]
            .map(|version| PythonVersion::from_str(version).expect("valid Python version")),
        &[ScenarioPlatform::Windows],
    );
    for lockfile in [LockfileMode::Standard, LockfileMode::WithoutMetadata] {
        let options = LockCheckOptions {
            max_states: 100_000,
            lockfile,
            evidence: LockEvidenceMode::PrintedV1,
        };
        let context = uv_test::test_context!("3.13");
        let result =
            check_project_lock_scenario(&context, &scenario, &targets, &selections, options)?;
        let LockCheckResult::Satisfiable { projections, .. } = result else {
            bail!("the restricted project must be satisfiable");
        };
        assert_eq!(projections, targets.len() * selections.len());

        let context = uv_test::test_context!("3.13");
        let result = check_witnessed_project_lock_scenario(
            &context,
            &graph,
            &targets,
            &selections,
            options,
            100_000,
        )?;
        let LockCheckResult::Satisfiable { projections, .. } = result else {
            bail!("the restricted witness must be satisfiable");
        };
        assert_eq!(projections, targets.len() * selections.len());

        let mut base = graph.document.scenario()?;
        base.root.optional_dependencies.clear();
        base.root.dependency_groups = None;
        let context = uv_test::test_context!("3.13");
        let result = check_lock_scenario(&context, &base, &targets, options)?;
        let LockCheckResult::Satisfiable { projections, .. } = result else {
            bail!("the restricted base project must be satisfiable");
        };
        assert_eq!(projections, targets.len());
    }
    Ok(())
}

#[test]
fn structured_restricted_project_locks_match_their_concrete_projections() -> Result<()> {
    let graph = WitnessedProjectGraph {
        document: r#"
name = "structured-restricted-lock-domain"
[root]
requires_python = ">=3.12,<3.15"
requires = ["a", "missing; sys_platform != 'win32' and sys_platform != 'linux'"]
optional_dependencies = { feature = ["a"] }
dependency_groups = { dev = ["a"] }
[expected]
satisfiable = true
[resolver_options]
fork_strategy = "fewest"
environments = ["sys_platform == 'win32' and python_version >= '3.13'", "sys_platform == 'linux' and python_version >= '3.13'"]
[packages.a.versions."1.0.0"]
requires_python = ">=3.13,<3.15"
"#
        .parse()?,
        assignment: [("a".parse()?, "1.0.0".parse()?)].into_iter().collect(),
    };
    let scenario = graph.document.scenario()?;
    let selections = ScenarioProject::new(&scenario)?.selection_matrix();
    let targets = ScenarioTarget::matrix(
        &["3.13", "3.14"]
            .map(|version| PythonVersion::from_str(version).expect("valid Python version")),
        &[ScenarioPlatform::Windows, ScenarioPlatform::Linux],
    );
    for lockfile in [LockfileMode::Standard, LockfileMode::WithoutMetadata] {
        let context = uv_test::test_context!("3.13");
        let result = check_witnessed_project_lock_scenario(
            &context,
            &graph,
            &targets,
            &selections,
            LockCheckOptions {
                max_states: 100_000,
                lockfile,
                evidence: LockEvidenceMode::StructuredV1,
            },
            100_000,
        )?;
        let LockCheckResult::Satisfiable { projections, .. } = result else {
            bail!("the structured restricted witness must be satisfiable");
        };
        assert_eq!(projections, targets.len() * selections.len());
    }
    Ok(())
}

#[test]
fn structured_project_locks_match_their_concrete_projections() -> Result<()> {
    let targets = ScenarioTarget::matrix(
        &["3.12", "3.13", "3.14"]
            .map(|version| PythonVersion::from_str(version).expect("valid Python version")),
        &[
            ScenarioPlatform::Linux,
            ScenarioPlatform::Macos,
            ScenarioPlatform::Windows,
        ],
    );
    for lockfile in [LockfileMode::Standard, LockfileMode::WithoutMetadata] {
        let context = uv_test::test_context!("3.12")
            .with_env(EnvVars::UV_OFFLINE, "1")
            .with_env(EnvVars::UV_INDEX_STRATEGY, "unsafe-best-match")
            .with_env(EnvVars::UV_TORCH_BACKEND, "cu129")
            .with_env("UV_INDEX_PRIVATE_PASSWORD", "unexpected")
            .with_env(EnvVars::UV_CONFIG_FILE, "unexpected.toml")
            .with_env(EnvVars::UV_INTERNAL__SHOW_DERIVATION_TREE, "1")
            .with_env(EnvVars::UV_INTERNAL__RESOLVER_CAPTURE, "inherited.json")
            .with_env(
                EnvVars::UV_INTERNAL__RESOLVER_CAPTURE_REQUEST,
                "ffffffffffffffffffffffffffffffff",
            );
        let graph = WitnessedProjectGraph {
            document: ScenarioDocument::from_path(
                &context
                    .workspace_root
                    .join("test/scenarios/fork/non-local-fork-marker-unreachable.toml"),
            )?,
            assignment: [("a".parse()?, "1.0.0".parse()?)].into_iter().collect(),
        };
        let scenario = graph.document.scenario()?;
        let selections = ScenarioProject::new(&scenario)?.selection_matrix();
        let result = check_witnessed_project_lock_scenario(
            &context,
            &graph,
            &targets,
            &selections,
            LockCheckOptions {
                max_states: 100_000,
                lockfile,
                evidence: LockEvidenceMode::StructuredV1,
            },
            100_000,
        )?;
        let LockCheckResult::Satisfiable { projections, .. } = result else {
            bail!("the structured witnessed lock must be satisfiable");
        };
        assert_eq!(projections, targets.len() * selections.len());
        assert!(!context.temp_dir.join("inherited.json").exists());
    }
    Ok(())
}

#[test]
fn missing_index_packages_only_create_an_empty_credential_lock() -> Result<()> {
    let scenario = r#"
name = "missing-index-package"
[root]
requires_python = ">=3.12,<3.13"
requires = ["missing"]
[expected]
satisfiable = false
"#
    .parse::<ScenarioDocument>()?
    .scenario()?;
    for lockfile in [LockfileMode::Standard, LockfileMode::WithoutMetadata] {
        let context = uv_test::test_context!("3.12");
        fs_err::write(
            context.temp_dir.join("pyproject.toml"),
            r#"[project]
name = "missing-index-package"
version = "0.1.0"
requires-python = ">=3.12,<3.13"
dependencies = ["missing"]
"#,
        )?;
        let credentials = context.temp_dir.join("credentials");
        fs_err::create_dir(&credentials)?;
        let server = PackseServer::from_scenario_without_build_dependencies(&scenario);
        let mut command = context.lock();
        command
            .args([
                "--no-config",
                "--no-build",
                "--no-offline",
                "--index-strategy",
                "first-index",
                "--keyring-provider",
                "disabled",
                "--python",
                "3.12",
                "--no-python-downloads",
            ])
            .arg("--index-url")
            .arg(server.index_url())
            .env(EnvVars::UV_CREDENTIALS_DIR, &credentials);
        if matches!(lockfile, LockfileMode::WithoutMetadata) {
            command.args(["--preview-features", "lock-without-metadata"]);
        }
        let output = command.output()?;
        assert_eq!(
            output.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        let mut entries = fs_err::read_dir(&credentials)?;
        let lock = entries.next().context("the credential lock")??;
        assert_eq!(lock.file_name(), "credentials.toml.lock");
        assert!(lock.file_type()?.is_file());
        assert!(fs_err::read(lock.path())?.is_empty());
        assert!(entries.next().is_none());
        assert!(!context.temp_dir.join("uv.lock").exists());
    }
    Ok(())
}

#[test]
fn structured_project_locks_do_not_apply_an_available_version_cutoff() -> Result<()> {
    let targets = ScenarioTarget::matrix(
        &[PythonVersion::from_str("3.12").expect("valid Python version")],
        &[ScenarioPlatform::Linux],
    );
    for lockfile in [LockfileMode::Standard, LockfileMode::WithoutMetadata] {
        let context = uv_test::test_context!("3.12").with_env(
            EnvVars::UV_TEST_AVAILABLE_VERSION_CUTOFF,
            "2024-03-25T00:00:00Z",
        );
        let graph = WitnessedProjectGraph {
            document: ScenarioDocument::from_path(
                &context
                    .workspace_root
                    .join("test/scenarios/project/available-version-cutoff.toml"),
            )?,
            assignment: [
                ("a".parse()?, "2.0.0".parse()?),
                ("b".parse()?, "2.0.0".parse()?),
            ]
            .into_iter()
            .collect(),
        };
        let certificate = graph.certify_universal_witness()?;
        assert_eq!(certificate.assigned_packages, 2);
        assert_eq!(certificate.checked_requirements, 3);
        let scenario = graph.document.scenario()?;
        let selections = ScenarioProject::new(&scenario)?.selection_matrix();
        let result = check_witnessed_project_lock_scenario(
            &context,
            &graph,
            &targets,
            &selections,
            LockCheckOptions {
                max_states: 100_000,
                lockfile,
                evidence: LockEvidenceMode::StructuredV1,
            },
            100_000,
        )?;
        let LockCheckResult::Satisfiable { projections, .. } = result else {
            bail!("the complete candidate inventory must admit a==2 and b==2");
        };
        assert_eq!(projections, targets.len() * selections.len());

        // A separate ordinary lock of the same generated project demonstrates the policy being
        // excluded from structured evidence: the cutoff hides the only compatible `a` candidate.
        let cutoff_context = uv_test::test_context!("3.12");
        fs_err::write(
            cutoff_context.temp_dir.join("pyproject.toml"),
            fs_err::read(context.temp_dir.join("pyproject.toml"))?,
        )?;
        let server = PackseServer::from_scenario_without_build_dependencies(&scenario);
        let mut command = cutoff_context.lock();
        command
            .args([
                "--no-config",
                "--no-build",
                "--no-offline",
                "--index-strategy",
                "first-index",
                "--keyring-provider",
                "disabled",
                "--python",
                "3.12",
                "--no-python-downloads",
            ])
            .arg("--index-url")
            .arg(server.index_url())
            .env_remove(EnvVars::UV_EXCLUDE_NEWER)
            .env(
                EnvVars::UV_TEST_AVAILABLE_VERSION_CUTOFF,
                "2024-03-25T00:00:00Z",
            );
        match lockfile {
            LockfileMode::Standard => {}
            LockfileMode::WithoutMetadata => {
                command.args(["--preview-features", "lock-without-metadata"]);
            }
        }
        let output = command.output()?;
        assert_eq!(
            output.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        assert!(!cutoff_context.temp_dir.join("uv.lock").exists());
    }
    Ok(())
}

#[test]
fn structured_project_locks_cannot_certify_a_genuine_conflict() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let document = ScenarioDocument::from_path(
        &context
            .workspace_root
            .join("test/scenarios/project/combined-roots-conflict.toml"),
    )?;
    let scenario = document.scenario()?;
    let selections = ScenarioProject::new(&scenario)?.selection_matrix();
    let targets = ScenarioTarget::matrix(
        &[PythonVersion::from_str("3.12").expect("valid Python version")],
        &[ScenarioPlatform::Linux],
    );
    let result = check_project_lock_scenario(
        &context,
        &scenario,
        &targets,
        &selections,
        LockCheckOptions::new(100_000),
    )?;
    assert!(matches!(result, LockCheckResult::Unsatisfiable { .. }));

    let context = uv_test::test_context!("3.12");
    let graph = WitnessedProjectGraph {
        document,
        assignment: [
            ("base".parse()?, "1".parse()?),
            ("dep".parse()?, "1".parse()?),
        ]
        .into_iter()
        .collect(),
    };
    let directory = context.temp_dir.join("failure");
    let error = check_witnessed_project_lock_scenario_with_artifacts(
        &context,
        &graph,
        &targets,
        &selections,
        LockCheckOptions {
            max_states: 100_000,
            lockfile: LockfileMode::Standard,
            evidence: LockEvidenceMode::StructuredV1,
        },
        100_000,
        &directory,
    )
    .expect_err("the conflicting project has no whole-domain assignment");
    assert_eq!(LockScenarioFailureKind::from_error(&error), None);
    insta::assert_snapshot!(error, @"dep==1 does not satisfy `dep==2`");
    assert!(!context.temp_dir.join("pyproject.toml").exists());
    assert!(!directory.exists());
    Ok(())
}

#[test]
fn witnessed_reduction_requires_a_reproducing_lock_failure() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let graph = WitnessedProjectGraph {
        document: ScenarioDocument::from_path(
            &context
                .workspace_root
                .join("test/scenarios/fork/non-local-fork-marker-unreachable.toml"),
        )?,
        assignment: [("a".parse()?, "1.0.0".parse()?)].into_iter().collect(),
    };
    let targets = ScenarioTarget::matrix(
        &[PythonVersion::from_str("3.12").expect("valid Python version")],
        &[ScenarioPlatform::Linux, ScenarioPlatform::Windows],
    );
    for lockfile in [LockfileMode::Standard, LockfileMode::WithoutMetadata] {
        let error = minimize_witnessed_project_lock_scenario(
            &graph,
            &targets,
            100_000,
            10,
            100_000,
            |candidate| {
                let context = uv_test::test_context!("3.12");
                let scenario = candidate.document.scenario()?;
                let selections = ScenarioProject::new(&scenario)?.selection_matrix();
                check_witnessed_project_lock_scenario(
                    &context,
                    candidate,
                    &targets,
                    &selections,
                    LockCheckOptions {
                        max_states: 100_000,
                        lockfile,
                        evidence: LockEvidenceMode::PrintedV1,
                    },
                    100_000,
                )
            },
        )
        .expect_err("the real resolver accepts this certified satisfiable graph");
        assert_eq!(LockScenarioFailureKind::from_error(&error), None);
        insta::allow_duplicates! {
            insta::assert_snapshot!(error, @"the input does not reproduce a lockfile mismatch");
        }
    }
    Ok(())
}

#[test]
fn certified_project_locks_reject_invalid_proofs_before_running_uv() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = ScenarioTarget {
        python: PythonVersion::from_str("3.12").expect("valid Python version"),
        platform: ScenarioPlatform::Linux,
    };
    let document = ScenarioDocument::from_path(
        &context
            .workspace_root
            .join("test/scenarios/fork/non-local-fork-marker-unreachable.toml"),
    )?;
    let assignment = [("a".parse()?, "1.0.0".parse()?)].into_iter().collect();
    for (assignment, max_work, expected) in [
        (
            Selection::new(),
            100_000,
            "must assign every scenario package",
        ),
        (assignment, 1, "marker witness work exceeds 1 requirements"),
    ] {
        let graph = WitnessedProjectGraph {
            document: document.clone(),
            assignment,
        };
        let directory = context.temp_dir.join("failure");
        let error = check_witnessed_project_lock_scenario_with_artifacts(
            &context,
            &graph,
            std::slice::from_ref(&target),
            &[ProjectSelection::default()],
            LockCheckOptions::new(100_000),
            max_work,
            &directory,
        )
        .expect_err("the proposed proof cannot classify a resolver result");
        assert!(format!("{error:#}").contains(expected));
        assert_eq!(LockScenarioFailureKind::from_error(&error), None);
        assert!(!context.temp_dir.join("pyproject.toml").exists());
        assert!(!directory.exists());
    }
    Ok(())
}
