use std::path::Path;
use std::process::Command;

use anyhow::Result;
use assert_cmd::prelude::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::indoc;
use predicates::prelude::predicate;

use uv_static::EnvVars;
use uv_test::TestContext;

fn init_child(context: &TestContext, child: &Path) -> Command {
    let mut command = context.init();
    command.arg(child).args([
        "--bare",
        "--vcs",
        "none",
        "--no-pin-python",
        "--python",
        "3.12",
        "--offline",
        "--preview-features",
        "nested-workspaces",
    ]);
    command.env_remove(EnvVars::VIRTUAL_ENV);
    command
}

fn lock(context: &TestContext, workspace: &Path) -> Command {
    let mut command = context.lock();
    command.current_dir(workspace).args([
        "--python",
        "3.12",
        "--offline",
        "--preview-features",
        "nested-workspaces",
    ]);
    command.env_remove(EnvVars::VIRTUAL_ENV);
    command
}

fn read_pyproject(project: &Path) -> Result<toml::Value> {
    Ok(toml::from_str(&fs_err::read_to_string(
        project.join("pyproject.toml"),
    )?)?)
}

fn has_workspace(pyproject: &toml::Value) -> bool {
    pyproject
        .get("tool")
        .and_then(|tool| tool.get("uv"))
        .and_then(|uv| uv.get("workspace"))
        .and_then(toml::Value::as_table)
        .is_some()
}

#[test]
fn init_nested_workspace_registrations() -> Result<()> {
    for (pattern, create_child) in [
        ("services/child", false),
        ("services/*", false),
        ("services/*", true),
    ] {
        let context = uv_test::test_context!("3.12");
        let parent = context.temp_dir.child("parent");
        let child = parent.child("services/child");
        let parent_toml = format!("[tool.uv.workspace]\nworkspaces = [{pattern:?}]\n");
        parent.child("pyproject.toml").write_str(&parent_toml)?;
        if create_child {
            child.create_dir_all()?;
        }

        init_child(&context, child.path()).assert().success();

        assert_eq!(
            fs_err::read_to_string(parent.join("pyproject.toml"))?,
            parent_toml
        );
        assert_eq!(
            fs_err::read_to_string(child.join("pyproject.toml"))?,
            indoc! {r#"
                [project]
                name = "child"
                version = "0.1.0"
                requires-python = ">=3.12"
                dependencies = []

                [tool.uv.workspace]
            "#}
        );

        // Each generated workspace is independently lockable once its parent has a lock.
        lock(&context, parent.path()).assert().success();
        lock(&context, child.path()).assert().success();
        parent.child("uv.lock").assert(predicate::path::is_file());
        child.child("uv.lock").assert(predicate::path::is_file());
    }
    Ok(())
}

#[test]
fn init_nested_workspace_uses_registering_ancestor() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let parent = context.temp_dir.child("parent");
    let middle = parent.child("group");
    let child = middle.child("child");
    let parent_toml = indoc! {r#"
        [tool.uv.workspace]
        workspaces = ["group", "group/child"]
    "#};
    let middle_toml = "[tool.uv.workspace]\n";
    parent.child("pyproject.toml").write_str(parent_toml)?;
    middle.child("pyproject.toml").write_str(middle_toml)?;

    init_child(&context, child.path()).assert().success();

    assert!(has_workspace(&read_pyproject(child.path())?));
    assert_eq!(
        fs_err::read_to_string(parent.join("pyproject.toml"))?,
        parent_toml
    );
    assert_eq!(
        fs_err::read_to_string(middle.join("pyproject.toml"))?,
        middle_toml
    );

    // The intervening workspace does not register the child, so it needs no lockfile.
    lock(&context, parent.path()).assert().success();
    lock(&context, child.path()).assert().success();
    middle.child("uv.lock").assert(predicate::path::missing());
    Ok(())
}

#[test]
fn init_nested_workspace_rejects_parent_member_overlap() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let parent = context.temp_dir.child("parent");
    let child = parent.child("services/child");
    let parent_toml = indoc! {r#"
        [tool.uv.workspace]
        members = ["services/*"]
        workspaces = ["services/*"]
    "#};
    parent.child("pyproject.toml").write_str(parent_toml)?;

    init_child(&context, child.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "overlaps independently locked child workspace",
        ));
    child.assert(predicate::path::missing());
    assert_eq!(
        fs_err::read_to_string(parent.join("pyproject.toml"))?,
        parent_toml
    );

    let excluded_toml = indoc! {r#"
        [tool.uv.workspace]
        members = ["services/*"]
        exclude = ["services/*"]
        workspaces = ["services/*"]
    "#};
    parent.child("pyproject.toml").write_str(excluded_toml)?;
    init_child(&context, child.path()).assert().success();
    assert!(has_workspace(&read_pyproject(child.path())?));
    assert_eq!(
        fs_err::read_to_string(parent.join("pyproject.toml"))?,
        excluded_toml
    );
    Ok(())
}

#[test]
fn init_nested_workspace_rejects_nearer_member_overlap() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let parent = context.temp_dir.child("parent");
    let middle = parent.child("group");
    let child = middle.child("child");
    let parent_toml = indoc! {r#"
        [tool.uv.workspace]
        workspaces = ["group", "group/child"]
    "#};
    let middle_toml = indoc! {r#"
        [tool.uv.workspace]
        members = ["child"]
    "#};
    parent.child("pyproject.toml").write_str(parent_toml)?;
    middle.child("pyproject.toml").write_str(middle_toml)?;

    init_child(&context, child.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "overlaps independently locked child workspace",
        ));
    child.assert(predicate::path::missing());
    assert_eq!(
        fs_err::read_to_string(middle.join("pyproject.toml"))?,
        middle_toml
    );

    let excluded_toml = indoc! {r#"
        [tool.uv.workspace]
        members = ["child"]
        exclude = ["child"]
    "#};
    middle.child("pyproject.toml").write_str(excluded_toml)?;
    init_child(&context, child.path()).assert().success();
    assert!(has_workspace(&read_pyproject(child.path())?));
    assert_eq!(
        fs_err::read_to_string(parent.join("pyproject.toml"))?,
        parent_toml
    );
    assert_eq!(
        fs_err::read_to_string(middle.join("pyproject.toml"))?,
        excluded_toml
    );
    Ok(())
}

#[test]
fn init_nested_workspace_rejects_owned_descendants() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let parent = context.temp_dir.child("parent");
    let child = parent.child("child");
    let member = child.child("packages/member");
    let parent_toml = indoc! {r#"
        [tool.uv.workspace]
        members = ["child/packages/*"]
        workspaces = ["child"]
    "#};
    let member_toml = indoc! {r#"
        [project]
        name = "member"
        version = "1.0.0"
    "#};
    parent.child("pyproject.toml").write_str(parent_toml)?;
    member.child("pyproject.toml").write_str(member_toml)?;

    init_child(&context, child.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "overlaps independently locked child workspace",
        ));
    child
        .child("pyproject.toml")
        .assert(predicate::path::missing());
    assert_eq!(
        fs_err::read_to_string(parent.join("pyproject.toml"))?,
        parent_toml
    );
    assert_eq!(
        fs_err::read_to_string(member.join("pyproject.toml"))?,
        member_toml
    );
    Ok(())
}

#[test]
fn init_nested_workspace_only_ignores_its_target() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let parent = context.temp_dir.child("parent");
    let child = parent.child("services/child");
    let parent_toml = indoc! {r#"
        [tool.uv.workspace]
        workspaces = ["services/child", "services/missing"]
    "#};
    parent.child("pyproject.toml").write_str(parent_toml)?;

    init_child(&context, child.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("matches: `services/missing`"));
    child.assert(predicate::path::missing());
    assert_eq!(
        fs_err::read_to_string(parent.join("pyproject.toml"))?,
        parent_toml
    );
    Ok(())
}

#[test]
fn init_nested_workspace_python_is_independent() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let parent = context.temp_dir.child("parent");
    let child = parent.child("child");
    let parent_toml = indoc! {r#"
        [project]
        name = "parent"
        version = "1.0.0"
        requires-python = ">=3.10,<3.12"

        [tool.uv.workspace]
        workspaces = ["child"]
    "#};
    parent.child("pyproject.toml").write_str(parent_toml)?;
    parent.child(".python-version").write_str("3.11\n")?;

    context
        .init()
        .arg(child.path())
        .args([
            "--bare",
            "--vcs",
            "none",
            "--pin-python",
            "--offline",
            "--preview-features",
            "nested-workspaces",
        ])
        .env_remove(EnvVars::VIRTUAL_ENV)
        .assert()
        .success();

    let pyproject = read_pyproject(child.path())?;
    assert!(has_workspace(&pyproject));
    assert_eq!(
        pyproject["project"]["requires-python"].as_str(),
        Some(">=3.12")
    );
    assert_eq!(
        fs_err::read_to_string(child.join(".python-version"))?,
        "3.12\n"
    );
    assert_eq!(
        fs_err::read_to_string(parent.join("pyproject.toml"))?,
        parent_toml
    );
    assert_eq!(
        fs_err::read_to_string(parent.join(".python-version"))?,
        "3.11\n"
    );
    Ok(())
}

#[test]
fn init_nested_workspace_no_workspace() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let parent = context.temp_dir.child("parent");
    let child = parent.child("child");
    let parent_toml = "[tool.uv.workspace]\nworkspaces = [\"child\"]\n";
    parent.child("pyproject.toml").write_str(parent_toml)?;

    init_child(&context, child.path())
        .arg("--no-workspace")
        .assert()
        .success();

    assert!(!has_workspace(&read_pyproject(child.path())?));
    assert_eq!(
        fs_err::read_to_string(parent.join("pyproject.toml"))?,
        parent_toml
    );
    Ok(())
}

#[test]
fn init_nested_workspace_without_preview() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let parent = context.temp_dir.child("parent");
    let child = parent.child("child");
    let parent_toml = "[tool.uv.workspace]\nworkspaces = [\"child\"]\n";
    parent.child("pyproject.toml").write_str(parent_toml)?;

    context
        .init()
        .arg(child.path())
        .args([
            "--bare",
            "--vcs",
            "none",
            "--no-pin-python",
            "--python",
            "3.12",
            "--offline",
            "--no-preview",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "Nested workspaces are experimental",
        ));

    assert!(has_workspace(&read_pyproject(child.path())?));
    assert_eq!(
        fs_err::read_to_string(parent.join("pyproject.toml"))?,
        parent_toml
    );
    Ok(())
}
