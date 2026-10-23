//! Search-history checks that complement the generated lockfile scenarios.

use anyhow::Result;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_static::EnvVars;
use uv_test::packse::PackseServer;
use uv_test::packse::scenario::Scenario;
use uv_test::uv_snapshot;

#[test]
fn coordinated_trace_replaces_withdrawn_agreement() -> Result<()> {
    let scenario: Scenario = toml::from_str(include_str!(
        "../../../../test/scenarios/fork/coordinated/coordinated-agreement-replacement.toml"
    ))?;
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::from_scenario(&scenario);
    context.temp_dir.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12,<3.14"
        dependencies = ["switchable; python_version < '3.13'", "later-entry; python_version >= '3.13'"]

        [tool.uv]
        fork-strategy = "fewest"
        environments = ["python_version == '3.12'", "python_version == '3.13'"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::RUST_LOG, "uv_resolver::resolver::coordination=debug")
        .arg("--index-url").arg(server.index_url()), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    DEBUG Trying coordinated backtracking for shared==1.0.0 from http://[LOCALHOST]/simple/ in split `python_full_version == '3.12.*'`
    DEBUG Completed coordinated backtracking fork for split `python_full_version == '3.12.*'`
    DEBUG Accepted coordinated backtracking: duplicate count 1 -> 0
    DEBUG Trying coordinated backtracking for shared==2.0.0 from http://[LOCALHOST]/simple/ in split `python_full_version == '3.12.*'`
    DEBUG Completed coordinated backtracking fork for split `python_full_version == '3.12.*'`
    DEBUG Accepted coordinated backtracking: duplicate count 1 -> 0
    Resolved 15 packages in [TIME]
    "#);
    Ok(())
}

#[test]
fn coordinated_trace_accumulates_agreements() -> Result<()> {
    let scenario: Scenario = toml::from_str(include_str!(
        "../../../../test/scenarios/fork/coordinated/coordinated-agreement-accumulation.toml"
    ))?;
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::from_scenario(&scenario);
    context.temp_dir.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12,<3.14"
        dependencies = ["switchable-a; python_version < '3.13'", "switchable-b; python_version < '3.13'", "later-entry; python_version >= '3.13'"]

        [tool.uv]
        fork-strategy = "fewest"
        environments = ["python_version == '3.12'", "python_version == '3.13'"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::RUST_LOG, "uv_resolver::resolver::coordination=debug")
        .arg("--index-url").arg(server.index_url()), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    DEBUG Trying coordinated backtracking for shared-a==1.0.0 from http://[LOCALHOST]/simple/ in split `python_full_version == '3.12.*'`
    DEBUG Completed coordinated backtracking fork for split `python_full_version == '3.12.*'`
    DEBUG Accepted coordinated backtracking: duplicate count 1 -> 0
    DEBUG Trying coordinated backtracking for shared-b==1.0.0 from http://[LOCALHOST]/simple/ in split `python_full_version == '3.12.*'`
    DEBUG Completed coordinated backtracking fork for split `python_full_version == '3.12.*'`
    DEBUG Accepted coordinated backtracking: duplicate count 1 -> 0
    Resolved 14 packages in [TIME]
    "#);
    Ok(())
}

#[test]
fn coordinated_trace_rejects_equal_score() -> Result<()> {
    let scenario: Scenario = toml::from_str(include_str!(
        "../../../../test/scenarios/fork/coordinated/coordinated-agreement-non-improvement.toml"
    ))?;
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::from_scenario(&scenario);
    context.temp_dir.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12,<3.14"
        dependencies = ["switchable-a; python_version < '3.13'", "switchable-b; python_version < '3.13'", "later-entry; python_version >= '3.13'"]

        [tool.uv]
        fork-strategy = "fewest"
        environments = ["python_version == '3.12'", "python_version == '3.13'"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::RUST_LOG, "uv_resolver::resolver::coordination=debug")
        .arg("--index-url").arg(server.index_url()), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    DEBUG Trying coordinated backtracking for shared-a==1.0.0 from http://[LOCALHOST]/simple/ in split `python_full_version == '3.12.*'`
    DEBUG Completed coordinated backtracking fork for split `python_full_version == '3.12.*'`
    DEBUG Accepted coordinated backtracking: duplicate count 1 -> 0
    DEBUG Trying coordinated backtracking for shared-b==1.0.0 from http://[LOCALHOST]/simple/ in split `python_full_version == '3.12.*'`
    DEBUG Completed coordinated backtracking fork for split `python_full_version == '3.12.*'`
    DEBUG Rejected coordinated backtracking: duplicate count 1 -> 1
    DEBUG Trying coordinated backtracking for shared-b==2.0.0 from http://[LOCALHOST]/simple/ in split `python_full_version == '3.13.*'`
    DEBUG Abandoned coordinated backtracking after an unsatisfiable fork: Because shared-b<2.0.0 cannot be used because a coordinated backtracking trial requires version 2.0.0 and all versions of trade-delay-three depend on shared-b==1.0.0, we can conclude that all versions of trade-delay-three cannot be used.
    And because all versions of trade-delay-two depend on trade-delay-three, we can conclude that all versions of trade-delay-two cannot be used.
    And because all versions of trade-delay-one depend on trade-delay-two and all versions of trade-gate depend on trade-delay-one, we can conclude that all versions of trade-gate cannot be used.
    And because all versions of stage-a depend on trade-gate and all versions of start-five depend on stage-a, we can conclude that all versions of start-five cannot be used.
    And because all versions of start-four depend on start-five and all versions of start-three depend on start-four, we can conclude that all versions of start-three cannot be used.
    And because all versions of start-two depend on start-three and all versions of start-one depend on start-two, we can conclude that all versions of start-one cannot be used.
    And because all versions of later-entry depend on start-one and your project depends on later-entry{python_full_version >= '3.13'}, we can conclude that your project's requirements are unsatisfiable.
    Resolved 18 packages in [TIME]
    "#);
    Ok(())
}

#[test]
fn coordinated_trace_discards_completed_nested_child() -> Result<()> {
    let scenario: Scenario = toml::from_str(include_str!(
        "../../../../test/scenarios/fork/coordinated/coordinated-nested-platform-unsatisfiable.toml"
    ))?;
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::from_scenario(&scenario);
    context.temp_dir.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12,<3.14"
        dependencies = ["parent; python_version < '3.13'", "blocker==2.0.0; python_version < '3.13'", "delayed; python_version >= '3.13'"]

        [tool.uv]
        fork-strategy = "fewest"
        environments = ["python_version < '3.13'", "python_version >= '3.13'"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::RUST_LOG, "uv_resolver::resolver::coordination=debug")
        .arg("--index-url").arg(server.index_url()), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    DEBUG Trying coordinated backtracking for aux==1.0.0 from http://[LOCALHOST]/simple/ in split `python_full_version < '3.13'`
    DEBUG Split coordinated backtracking into 2 forks
    DEBUG Completed coordinated backtracking fork for split `python_full_version < '3.13' and sys_platform == 'win32'`
    DEBUG Abandoned coordinated backtracking after an unsatisfiable fork: Because branch>=2.0.0 depends on blocker==1.0.0 and parent<=1.0.0 depends on branch{sys_platform != 'win32'}==2.0.0, we can conclude that parent<=1.0.0 depends on blocker==1.0.0. (1)

    Because aux>1.0.0 cannot be used because a coordinated backtracking trial requires version 1.0.0 and parent>=2.0.0 depends on aux==2.0.0, we can conclude that parent>=2.0.0 cannot be used.
    And because we know from (1) that parent<=1.0.0 depends on blocker==1.0.0, we can conclude that all versions of parent depend on blocker==1.0.0.
    And because your project depends on blocker{python_full_version < '3.13'}==2.0.0 and parent{python_full_version < '3.13'}, we can conclude that your project's requirements are unsatisfiable.
    DEBUG Trying coordinated backtracking for shared==1.0.0 from http://[LOCALHOST]/simple/ in split `python_full_version < '3.13'`
    DEBUG Split coordinated backtracking into 2 forks
    DEBUG Completed coordinated backtracking fork for split `python_full_version < '3.13' and sys_platform == 'win32'`
    DEBUG Abandoned coordinated backtracking after an unsatisfiable fork: Because branch>=2.0.0 depends on blocker==1.0.0 and parent<=1.0.0 depends on branch{sys_platform != 'win32'}==2.0.0, we can conclude that parent<=1.0.0 depends on blocker==1.0.0. (1)

    Because shared>1.0.0 cannot be used because a coordinated backtracking trial requires version 1.0.0 and parent>=2.0.0 depends on shared==2.0.0, we can conclude that parent>=2.0.0 cannot be used.
    And because we know from (1) that parent<=1.0.0 depends on blocker==1.0.0, we can conclude that all versions of parent depend on blocker==1.0.0.
    And because your project depends on blocker{python_full_version < '3.13'}==2.0.0 and parent{python_full_version < '3.13'}, we can conclude that your project's requirements are unsatisfiable.
    DEBUG Trying coordinated backtracking for aux==2.0.0 from http://[LOCALHOST]/simple/ in split `python_full_version >= '3.13'`
    DEBUG Abandoned coordinated backtracking after an unsatisfiable fork: Because aux<2.0.0 cannot be used because a coordinated backtracking trial requires version 2.0.0 and all versions of constrained depend on aux==1.0.0, we can conclude that all versions of constrained cannot be used.
    And because all versions of delay-eight depend on constrained and all versions of delay-seven depend on delay-eight, we can conclude that all versions of delay-seven cannot be used.
    And because all versions of delay-six depend on delay-seven and all versions of delay-five depend on delay-six, we can conclude that all versions of delay-five cannot be used.
    And because all versions of delay-four depend on delay-five and all versions of delay-three depend on delay-four, we can conclude that all versions of delay-three cannot be used.
    And because all versions of delay-two depend on delay-three and all versions of delay-one depend on delay-two, we can conclude that all versions of delay-one cannot be used.
    And because all versions of delayed depend on delay-one and your project depends on delayed{python_full_version >= '3.13'}, we can conclude that your project's requirements are unsatisfiable.
    DEBUG Trying coordinated backtracking for shared==2.0.0 from http://[LOCALHOST]/simple/ in split `python_full_version >= '3.13'`
    DEBUG Abandoned coordinated backtracking after an unsatisfiable fork: Because shared<2.0.0 cannot be used because a coordinated backtracking trial requires version 2.0.0 and all versions of constrained depend on shared==1.0.0, we can conclude that all versions of constrained cannot be used.
    And because all versions of delay-eight depend on constrained and all versions of delay-seven depend on delay-eight, we can conclude that all versions of delay-seven cannot be used.
    And because all versions of delay-six depend on delay-seven and all versions of delay-five depend on delay-six, we can conclude that all versions of delay-five cannot be used.
    And because all versions of delay-four depend on delay-five and all versions of delay-three depend on delay-four, we can conclude that all versions of delay-three cannot be used.
    And because all versions of delay-two depend on delay-three and all versions of delay-one depend on delay-two, we can conclude that all versions of delay-one cannot be used.
    And because all versions of delayed depend on delay-one and your project depends on delayed{python_full_version >= '3.13'}, we can conclude that your project's requirements are unsatisfiable.
    Resolved 17 packages in [TIME]
    "#);
    Ok(())
}
