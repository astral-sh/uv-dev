use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::FileWriteStr;
use uv_static::EnvVars;
use uv_test::uv_snapshot;

use crate::workspace_fixture::{MixedWorkspace, WorkspaceWithChild};

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
