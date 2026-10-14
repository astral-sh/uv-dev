use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::{ChildPath, FileTouch, FileWriteStr, PathChild, PathCreateDir};
use indoc::{formatdoc, indoc};
use uv_static::EnvVars;
use uv_test::{TestContext, uv_snapshot};

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
    assert_tool_output(&context, &fixture.bin_dir, "0.1.0 OK");

    fixture.child_module.write_str("MESSAGE = 'CHANGED'\n")?;
    assert_tool_output(&context, &fixture.bin_dir, "0.1.0 CHANGED");
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
    assert_tool_output(&context, &fixture.bin_dir, "0.1.0 OK");

    fixture.child_module.write_str("MESSAGE = 'CHANGED'\n")?;
    assert_tool_output(&context, &fixture.bin_dir, "0.1.0 OK");
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

    assert_tool_output(&context, &fixture.bin_dir, "OK");

    fixture
        .child_module
        .write_str("MESSAGE = 'PRE-REINSTALL'\n")?;
    assert_tool_output(&context, &fixture.bin_dir, "PRE-REINSTALL");

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

    assert_tool_output(&context, &fixture.bin_dir, "PRE-REINSTALL");

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

    assert_tool_output(&context, &fixture.bin_dir, "OK");

    fixture.child_module.write_str("MESSAGE = 'CHANGED'\n")?;

    assert_tool_output(&context, &fixture.bin_dir, "OK");

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

    assert_tool_output(&context, &fixture.bin_dir, "OK");

    fixture.child_module.write_str("MESSAGE = 'CHANGED'\n")?;

    assert_tool_output(&context, &fixture.bin_dir, "CHANGED");

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
    assert_tool_output(&context, &fixture.bin_dir, "0.1.0 OK");

    fixture.set_tool_version("0.1.1")?;
    context
        .tool_upgrade()
        .arg("tool-root")
        .env(EnvVars::PATH, fixture.bin_dir.as_os_str())
        .assert()
        .success();
    assert_tool_output(&context, &fixture.bin_dir, "0.1.1 OK");

    fixture
        .child_module
        .write_str("MESSAGE = 'POST-UPGRADE'\n")?;
    assert_tool_output(&context, &fixture.bin_dir, "0.1.1 POST-UPGRADE");
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
    assert_tool_output(&context, &fixture.bin_dir, "0.1.0 OK");

    fixture.set_tool_version("0.1.1")?;
    context
        .tool_upgrade()
        .arg("tool-root")
        .env(EnvVars::PATH, fixture.bin_dir.as_os_str())
        .assert()
        .success();
    assert_tool_output(&context, &fixture.bin_dir, "0.1.1 OK");

    fixture
        .child_module
        .write_str("MESSAGE = 'POST-UPGRADE'\n")?;
    assert_tool_output(&context, &fixture.bin_dir, "0.1.1 OK");
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
    assert_tool_output(&context, &fixture.bin_dir, "OK");

    fixture.member_module.write_str("MESSAGE = 'CHANGED'\n")?;
    assert_tool_output(&context, &fixture.bin_dir, "CHANGED");
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
    assert_tool_output(&context, &fixture.bin_dir, "OK");

    fixture.member_module.write_str("MESSAGE = 'CHANGED'\n")?;
    assert_tool_output(&context, &fixture.bin_dir, "OK");
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

/// A tool that depends on a member of its own workspace.
struct WorkspaceWithChild {
    child_module: ChildPath,
    bin_dir: ChildPath,
}

impl WorkspaceWithChild {
    fn new(context: &TestContext) -> Result<Self> {
        context
            .temp_dir
            .child("pyproject.toml")
            .write_str(indoc! {r#"
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
            child = { workspace = true }

            [tool.uv.workspace]
            members = ["child"]
        "#})?;
        let root_src = context.temp_dir.child("src").child("root");
        root_src.create_dir_all()?;
        root_src.child("__init__.py").write_str(indoc! {r"
            def main():
                import child
                print(child.MESSAGE)
        "})?;

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
        let child_module = child_src.child("__init__.py");
        child_module.write_str("MESSAGE = 'OK'\n")?;

        Ok(Self {
            child_module,
            bin_dir: context.temp_dir.child("bin"),
        })
    }
}

/// A tool whose workspace member is not a declared dependency.
struct WorkspaceWithUnrequiredMember {
    member: ChildPath,
    member_module: ChildPath,
    bin_dir: ChildPath,
}

impl WorkspaceWithUnrequiredMember {
    fn new(context: &TestContext) -> Result<Self> {
        context
            .temp_dir
            .child("pyproject.toml")
            .write_str(indoc! {r#"
                [project]
                name = "root"
                version = "0.1.0"
                requires-python = ">=3.12"

                [project.scripts]
                root_cli = "root:main"

                [build-system]
                requires = ["uv_build>=0.7,<10000"]
                build-backend = "uv_build"

                [tool.uv.workspace]
                members = ["child"]
            "#})?;
        let root_src = context.temp_dir.child("src").child("root");
        root_src.create_dir_all()?;
        root_src.child("__init__.py").write_str(indoc! {r"
            def main():
                import child
                print(child.MESSAGE)
        "})?;

        let member = context.temp_dir.child("child");
        member.create_dir_all()?;
        member.child("pyproject.toml").write_str(indoc! {r#"
            [project]
            name = "child"
            version = "0.1.0"
            requires-python = ">=3.12"

            [build-system]
            requires = ["uv_build>=0.7,<10000"]
            build-backend = "uv_build"
        "#})?;
        let member_src = member.child("src").child("child");
        member_src.create_dir_all()?;
        let member_module = member_src.child("__init__.py");
        member_module.write_str("MESSAGE = 'OK'\n")?;

        Ok(Self {
            member,
            member_module,
            bin_dir: context.temp_dir.child("bin"),
        })
    }
}

/// A tool with an additional requirement that belongs to a separate workspace.
struct MixedWorkspace {
    tool_root: ChildPath,
    other_workspace: ChildPath,
    child_module: ChildPath,
    bin_dir: ChildPath,
}

impl MixedWorkspace {
    fn new(context: &TestContext) -> Result<Self> {
        let tool_root = context.temp_dir.child("tool-root");
        let tool_root_src = tool_root.child("src").child("tool_root");
        tool_root_src.create_dir_all()?;
        tool_root_src.child("__init__.py").write_str(indoc! {r#"
            def main():
                import importlib.metadata
                import other_child

                print(f"{importlib.metadata.version('tool-root')} {other_child.MESSAGE}")
        "#})?;

        let other_workspace = context.temp_dir.child("other-workspace");
        other_workspace.create_dir_all()?;
        other_workspace
            .child("pyproject.toml")
            .write_str(indoc! {r#"
                [project]
                name = "other-workspace"
                version = "0.1.0"
                requires-python = ">=3.12"
                dependencies = ["other-child"]

                [build-system]
                requires = ["uv_build>=0.7,<10000"]
                build-backend = "uv_build"

                [tool.uv.sources]
                other-child = { workspace = true }

                [tool.uv.workspace]
                members = ["packages/*"]
            "#})?;
        let other_workspace_src = other_workspace.child("src").child("other_workspace");
        other_workspace_src.create_dir_all()?;
        other_workspace_src.child("__init__.py").touch()?;

        let other_child = other_workspace.child("packages").child("other-child");
        other_child.create_dir_all()?;
        other_child.child("pyproject.toml").write_str(indoc! {r#"
            [project]
            name = "other-child"
            version = "0.1.0"
            requires-python = ">=3.12"

            [build-system]
            requires = ["uv_build>=0.7,<10000"]
            build-backend = "uv_build"
        "#})?;
        let other_child_src = other_child.child("src").child("other_child");
        other_child_src.create_dir_all()?;
        let child_module = other_child_src.child("__init__.py");
        child_module.write_str("MESSAGE = 'OK'\n")?;

        let fixture = Self {
            tool_root,
            other_workspace,
            child_module,
            bin_dir: context.temp_dir.child("bin"),
        };
        fixture.set_tool_version("0.1.0")?;
        Ok(fixture)
    }

    fn set_tool_version(&self, version: &str) -> Result<()> {
        self.tool_root
            .child("pyproject.toml")
            .write_str(&formatdoc! {r#"
                [project]
                name = "tool-root"
                version = "{version}"
                requires-python = ">=3.12"

                [project.scripts]
                root_cli = "tool_root:main"

                [build-system]
                requires = ["uv_build>=0.7,<10000"]
                build-backend = "uv_build"
            "#})?;
        Ok(())
    }
}

fn assert_tool_output(context: &TestContext, bin_dir: &ChildPath, expected: &str) {
    let launcher = bin_dir.child(format!("root_cli{}", std::env::consts::EXE_SUFFIX));
    let output = context
        .external_command(launcher.path())
        .assert()
        .success()
        .stderr("");
    let stdout = std::str::from_utf8(&output.get_output().stdout)
        .expect("tool output should be valid UTF-8");
    assert_eq!(stdout.replace("\r\n", "\n"), format!("{expected}\n"));
}
