use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};

use uv_test::TestContext;

/// A tool that depends on a member of its own workspace.
pub(super) struct WorkspaceWithChild {
    pub(super) child_module: ChildPath,
    pub(super) bin_dir: ChildPath,
}

impl WorkspaceWithChild {
    pub(super) fn new(context: &TestContext) -> Result<Self> {
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

    pub(super) fn assert_tool_output(&self, context: &TestContext, expected: &str) {
        assert_tool_output(context, &self.bin_dir, expected);
    }
}

/// A tool whose workspace member is not a declared dependency.
pub(super) struct WorkspaceWithUnrequiredMember {
    pub(super) member: ChildPath,
    pub(super) member_module: ChildPath,
    pub(super) bin_dir: ChildPath,
}

impl WorkspaceWithUnrequiredMember {
    pub(super) fn new(context: &TestContext) -> Result<Self> {
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

    pub(super) fn assert_tool_output(&self, context: &TestContext, expected: &str) {
        assert_tool_output(context, &self.bin_dir, expected);
    }
}

/// A tool with an additional requirement that belongs to a separate workspace.
pub(super) struct MixedWorkspace {
    pub(super) tool_root: ChildPath,
    pub(super) other_workspace: ChildPath,
    pub(super) child_module: ChildPath,
    pub(super) bin_dir: ChildPath,
}

impl MixedWorkspace {
    pub(super) fn new(context: &TestContext) -> Result<Self> {
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

    pub(super) fn set_tool_version(&self, version: &str) -> Result<()> {
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

    pub(super) fn assert_tool_output(&self, context: &TestContext, expected: &str) {
        assert_tool_output(context, &self.bin_dir, expected);
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
