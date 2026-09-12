use anyhow::Result;
use assert_fs::prelude::*;
use indoc::indoc;
use insta::assert_snapshot;

use uv_test::uv_snapshot;

/// Indexes added for a workspace package should be stored at the workspace root.
///
/// See: <https://github.com/astral-sh/uv/issues/20678>
#[test]
fn add_indexes_for_workspace_package() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
            [tool.uv.workspace]
            members = ["child"]
        "#})?;
    context
        .temp_dir
        .child("child/pyproject.toml")
        .write_str(indoc! {r#"
            [project]
            name = "child"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = []
        "#})?;

    uv_snapshot!(context.filters(), context
        .add()
        .arg("iniconfig==2.0.0")
        .arg("--package")
        .arg("child")
        .arg("--index")
        .arg("test=https://test.pypi.org/simple")
        .arg("--index")
        .arg("pypi=https://pypi.org/simple"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(context.read("pyproject.toml"), @r#"
        [tool.uv.workspace]
        members = ["child"]

        [[tool.uv.index]]
        name = "test"
        url = "https://test.pypi.org/simple"

        [[tool.uv.index]]
        name = "pypi"
        url = "https://pypi.org/simple"
        "#);
        assert_snapshot!(context.read("child/pyproject.toml"), @r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig==2.0.0",
        ]
        "#);
    });

    Ok(())
}
