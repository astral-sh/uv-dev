use std::str::FromStr;

use anyhow::Result;

use uv_python::PythonVersion;
use uv_test::packse::check::{ScenarioPlatform, ScenarioTarget, check_scenario};
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
            "fork-basic-macos",
            true,
        ),
    ]
    "#);
    Ok(())
}
