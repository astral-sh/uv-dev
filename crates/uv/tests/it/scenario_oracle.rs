use std::str::FromStr;

use anyhow::{Context, Result};

use uv_python::PythonVersion;
use uv_test::packse::check::{
    LockCheckResult, ScenarioPlatform, ScenarioTarget, check_lock_scenario, check_scenario,
};
use uv_test::packse::generate::{SmallGraphOptions, generate_marker_graph, generate_small_graph};
use uv_test::packse::scenario::Scenario;

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
