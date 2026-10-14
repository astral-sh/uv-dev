use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::{FileWriteStr, PathChild, PathCreateDir};
use indoc::{formatdoc, indoc};
use uv_static::EnvVars;
use uv_test::uv_snapshot;

use crate::workspace_fixture::{MixedWorkspace, WorkspaceWithChild, WorkspaceWithUnrequiredMember};

#[test]
fn tool_install_preserves_mixed_workspace_member_editability() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let fixture = MixedWorkspace::new(&context)?;

    context
        .tool_install()
        .arg("--with-editable")
        .arg(fixture.other_workspace.path())
        .arg(fixture.tool_root.path())
        .env(EnvVars::PATH, fixture.bin_dir.as_os_str())
        .assert()
        .success();
    fixture.assert_tool_output(&context, "0.1.0 OK");

    fixture.child_module.write_str("MESSAGE = 'CHANGED'\n")?;
    fixture.assert_tool_output(&context, "0.1.0 CHANGED");
    Ok(())
}

#[test]
fn tool_install_preserves_mixed_workspace_member_non_editability() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let fixture = MixedWorkspace::new(&context)?;

    context
        .tool_install()
        .arg("--editable")
        .arg("--with")
        .arg(fixture.other_workspace.path())
        .arg(fixture.tool_root.path())
        .env(EnvVars::PATH, fixture.bin_dir.as_os_str())
        .assert()
        .success();
    fixture.assert_tool_output(&context, "0.1.0 OK");

    fixture.child_module.write_str("MESSAGE = 'CHANGED'\n")?;
    fixture.assert_tool_output(&context, "0.1.0 OK");
    Ok(())
}

#[test]
fn tool_install_reinstall_converts_workspace_members_to_non_editable() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let fixture = WorkspaceWithChild::new(&context)?;

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("--editable")
        .arg(context.temp_dir.path())
        .env(EnvVars::PATH, fixture.bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + child==0.1.0 (from file://[TEMP_DIR]/child)
     + root==0.1.0 (from file://[TEMP_DIR]/)
    Installed 1 executable: root_cli
    ");

    fixture.assert_tool_output(&context, "OK");

    fixture
        .child_module
        .write_str("MESSAGE = 'PRE-REINSTALL'\n")?;
    fixture.assert_tool_output(&context, "PRE-REINSTALL");

    context
        .tool_install()
        .arg("--reinstall")
        .arg(context.temp_dir.path())
        .env(EnvVars::PATH, fixture.bin_dir.as_os_str())
        .assert()
        .success();

    fixture
        .child_module
        .write_str("MESSAGE = 'POST-REINSTALL'\n")?;

    fixture.assert_tool_output(&context, "PRE-REINSTALL");

    Ok(())
}

#[test]
fn tool_install_workspace_members_are_non_editable_by_default() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let fixture = WorkspaceWithChild::new(&context)?;

    uv_snapshot!(context.filters(), context.tool_install()
        .arg(context.temp_dir.path())
        .env(EnvVars::PATH, fixture.bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + child==0.1.0 (from file://[TEMP_DIR]/child)
     + root==0.1.0 (from file://[TEMP_DIR]/)
    Installed 1 executable: root_cli
    ");

    fixture.assert_tool_output(&context, "OK");

    fixture.child_module.write_str("MESSAGE = 'CHANGED'\n")?;

    fixture.assert_tool_output(&context, "OK");

    Ok(())
}

#[test]
fn tool_install_workspace_members_honor_editable_flag() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let fixture = WorkspaceWithChild::new(&context)?;

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("--editable")
        .arg(context.temp_dir.path())
        .env(EnvVars::PATH, fixture.bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + child==0.1.0 (from file://[TEMP_DIR]/child)
     + root==0.1.0 (from file://[TEMP_DIR]/)
    Installed 1 executable: root_cli
    ");

    fixture.assert_tool_output(&context, "OK");

    fixture.child_module.write_str("MESSAGE = 'CHANGED'\n")?;

    fixture.assert_tool_output(&context, "CHANGED");

    Ok(())
}

#[test]
fn tool_upgrade_preserves_mixed_workspace_member_editability() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let fixture = MixedWorkspace::new(&context)?;

    context
        .tool_install()
        .arg("--with-editable")
        .arg(fixture.other_workspace.path())
        .arg(fixture.tool_root.path())
        .env(EnvVars::PATH, fixture.bin_dir.as_os_str())
        .assert()
        .success();
    fixture.assert_tool_output(&context, "0.1.0 OK");

    fixture.set_tool_version("0.1.1")?;
    context
        .tool_upgrade()
        .arg("tool-root")
        .env(EnvVars::PATH, fixture.bin_dir.as_os_str())
        .assert()
        .success();
    fixture.assert_tool_output(&context, "0.1.1 OK");

    fixture
        .child_module
        .write_str("MESSAGE = 'POST-UPGRADE'\n")?;
    fixture.assert_tool_output(&context, "0.1.1 POST-UPGRADE");
    Ok(())
}

#[test]
fn tool_upgrade_preserves_mixed_workspace_member_non_editability() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let fixture = MixedWorkspace::new(&context)?;

    context
        .tool_install()
        .arg("--editable")
        .arg("--with")
        .arg(fixture.other_workspace.path())
        .arg(fixture.tool_root.path())
        .env(EnvVars::PATH, fixture.bin_dir.as_os_str())
        .assert()
        .success();
    fixture.assert_tool_output(&context, "0.1.0 OK");

    fixture.set_tool_version("0.1.1")?;
    context
        .tool_upgrade()
        .arg("tool-root")
        .env(EnvVars::PATH, fixture.bin_dir.as_os_str())
        .assert()
        .success();
    fixture.assert_tool_output(&context, "0.1.1 OK");

    fixture
        .child_module
        .write_str("MESSAGE = 'POST-UPGRADE'\n")?;
    fixture.assert_tool_output(&context, "0.1.1 OK");
    Ok(())
}

#[test]
fn tool_install_workspace_member_honors_with_editable() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let fixture = WorkspaceWithUnrequiredMember::new(&context)?;

    context
        .tool_install()
        .arg("--with-editable")
        .arg(fixture.member.path())
        .arg(context.temp_dir.path())
        .env(EnvVars::PATH, fixture.bin_dir.as_os_str())
        .assert()
        .success();
    fixture.assert_tool_output(&context, "OK");

    fixture.member_module.write_str("MESSAGE = 'CHANGED'\n")?;
    fixture.assert_tool_output(&context, "CHANGED");
    Ok(())
}

#[test]
fn tool_install_workspace_member_honors_non_editable_with() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let fixture = WorkspaceWithUnrequiredMember::new(&context)?;

    context
        .tool_install()
        .arg("--editable")
        .arg("--with")
        .arg(fixture.member.path())
        .arg(context.temp_dir.path())
        .env(EnvVars::PATH, fixture.bin_dir.as_os_str())
        .assert()
        .success();
    fixture.assert_tool_output(&context, "OK");

    fixture.member_module.write_str("MESSAGE = 'CHANGED'\n")?;
    fixture.assert_tool_output(&context, "OK");
    Ok(())
}

#[test]
fn tool_install_workspace_members_honor_source_editable_flag() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let executable = bin_dir.child(format!("root_cli{}", std::env::consts::EXE_SUFFIX));

    let root_pyproject = context.temp_dir.child("pyproject.toml");
    root_pyproject.write_str(indoc! {
        r#"
        [project]
        name = "root"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["child"]

        [project.scripts]
        root_cli = "root:main"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"

        [tool.uv.sources]
        child = { workspace = true, editable = true }

        [tool.uv.workspace]
        members = ["child"]
        "#
    })?;

    let root_src = context.temp_dir.child("src").child("root");
    root_src.create_dir_all()?;
    root_src.child("__init__.py").write_str(indoc! {
        r"
        ROOT_MESSAGE = 'ROOT'

        def main():
            import child
            print(f'{ROOT_MESSAGE} {child.MESSAGE}')
        "
    })?;

    let child = context.temp_dir.child("child");
    child.create_dir_all()?;
    child.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;

    let child_src = child.child("src").child("child");
    child_src.create_dir_all()?;
    child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'OK'\n")?;

    uv_snapshot!(context.filters(), context.tool_install()
        .arg(context.temp_dir.path())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + child==0.1.0 (from file://[TEMP_DIR]/child)
     + root==0.1.0 (from file://[TEMP_DIR]/)
    Installed 1 executable: root_cli
    ");

    uv_snapshot!(context.filters(), context.external_command(executable.path()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    ROOT OK
    ");

    root_src.child("__init__.py").write_str(indoc! {
        r"
        ROOT_MESSAGE = 'CHANGED'

        def main():
            import child
            print(f'{ROOT_MESSAGE} {child.MESSAGE}')
        "
    })?;

    uv_snapshot!(context.filters(), context.external_command(executable.path()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    ROOT OK
    ");

    child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'CHANGED'\n")?;

    uv_snapshot!(context.filters(), context.external_command(executable.path()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    ROOT CHANGED
    ");

    Ok(())
}

#[test]
fn tool_upgrade_preserves_workspace_member_non_editability() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let executable = bin_dir.child(format!("root_cli{}", std::env::consts::EXE_SUFFIX));

    let root_pyproject = context.temp_dir.child("pyproject.toml");
    let write_root_manifest = |version: &str| {
        root_pyproject.write_str(&formatdoc! {r#"
            [project]
            name = "root"
            version = "{version}"
            requires-python = ">=3.12"
            dependencies = ["child"]

            [project.scripts]
            root_cli = "root:main"

            [build-system]
            requires = ["uv_build>=0.7,<10000"]
            build-backend = "uv_build"

            [tool.uv.sources]
            child = {{ workspace = true }}

            [tool.uv.workspace]
            members = ["child"]
        "#})
    };
    write_root_manifest("0.1.0")?;

    let root_src = context.temp_dir.child("src").child("root");
    root_src.create_dir_all()?;
    root_src.child("__init__.py").write_str(indoc! {
        r#"
        def main():
            from importlib.metadata import version

            import child

            print(version("root"), child.MESSAGE)
        "#
    })?;

    let child = context.temp_dir.child("child");
    child.create_dir_all()?;
    child.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;

    let child_src = child.child("src").child("child");
    child_src.create_dir_all()?;
    child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'OK'\n")?;

    context
        .tool_install()
        .arg(context.temp_dir.path())
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.external_command(executable.path()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    0.1.0 OK
    ");

    child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'PRE-UPGRADE'\n")?;

    uv_snapshot!(context.filters(), context.external_command(executable.path()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    0.1.0 OK
    ");

    write_root_manifest("0.1.1")?;
    context
        .tool_upgrade()
        .arg("root")
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.external_command(executable.path()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    0.1.1 OK
    ");

    child_src
        .child("__init__.py")
        .write_str("MESSAGE = 'POST-UPGRADE'\n")?;

    uv_snapshot!(context.filters(), context.external_command(executable.path()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    0.1.1 OK
    ");

    Ok(())
}
