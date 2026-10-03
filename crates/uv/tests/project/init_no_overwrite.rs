use anyhow::Result;
use assert_cmd::prelude::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::indoc;
use predicates::prelude::predicate;

use uv_static::EnvVars;

#[test]
fn init_does_not_overwrite_project_through_missing_parent() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let child = context.temp_dir.child("child");
    let child_toml = indoc! {r#"
        [project]
        name = "child"
        version = "7.0.0"
        requires-python = ">=3.12"
    "#};
    child.child("pyproject.toml").write_str(child_toml)?;

    context
        .init()
        .arg(context.temp_dir.join("missing/../child"))
        .args([
            "--bare",
            "--vcs",
            "none",
            "--no-pin-python",
            "--python",
            "3.12",
            "--offline",
            "--no-workspace",
        ])
        .env_remove(EnvVars::VIRTUAL_ENV)
        .assert()
        .failure()
        .stderr(predicate::str::contains("Project is already initialized"));

    assert_eq!(
        fs_err::read_to_string(child.join("pyproject.toml"))?,
        child_toml
    );
    Ok(())
}
