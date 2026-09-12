use std::str::FromStr;

use anyhow::{Context, Result};

use uv_python::PythonVersion;
use uv_test::packse::check::{
    LockCheckResult, LockScenarioFailureKind, ScenarioPlatform, ScenarioTarget,
    check_lock_scenario, check_lock_scenario_with_artifacts, check_project_lock_scenario,
    check_project_lock_scenario_with_artifacts, check_scenario,
};
use uv_test::packse::generate::{
    SmallGraphOptions, generate_marker_graph, generate_project_graph, generate_small_graph,
};
use uv_test::packse::project::{ProjectSelection, ScenarioProject};
use uv_test::packse::scenario::{Scenario, ScenarioDocument};

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
        let result = check_lock_scenario(&context, &scenario, &targets, 100_000)?;
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
        100_000,
        &directory,
    )
    .expect_err("the Linux projection does not witness the other platform's conflict");
    assert!(format!("{error:#}").contains("requested projections are satisfiable"));
    assert_eq!(LockScenarioFailureKind::from_error(&error), None);

    let failure: serde_json::Value =
        serde_json::from_slice(&fs_err::read(directory.join("failure.json"))?)?;
    assert!(failure["kind"].is_null());
    assert_eq!(failure["targets"][0]["python"], "3.12");
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
        100_000,
        &directory,
    )
    .expect_err("evidence directories cannot be replaced");
    assert!(format!("{error:#}").contains("failed to save lock evidence"));
    assert_eq!(fs_err::read(directory.join("scenario.toml"))?, original);

    let document = ScenarioDocument::from_path(
        &context
            .workspace_root
            .join("test/scenarios/fork/conflict-unsatisfiable.toml"),
    )?;
    let matched = context.temp_dir.join("matched");
    assert!(matches!(
        check_lock_scenario_with_artifacts(&context, &document, &[target], 100_000, &matched,)?,
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
    match check_project_lock_scenario(&context, &scenario, &targets, &selections, 100_000)? {
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
            100_000,
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
            100_000,
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
            let result =
                check_lock_scenario(&context, &scenario, std::slice::from_ref(&target), 27)
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
            check_lock_scenario(&context, &scenario, &targets, 27)
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
        let result = check_project_lock_scenario(&context, &scenario, &targets, &selections, 27)
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
