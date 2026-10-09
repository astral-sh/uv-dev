use anyhow::Result;
use assert_fs::fixture::{FileWriteStr, PathChild};
use indoc::indoc;
use uv_test::uv_snapshot;

#[test]
fn sync_consolidates_tilde_declarations_once() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "example"
        version = "0.1.0"
        requires-python = "~=3.12"

        [dependency-groups]
        dev = []

        [tool.uv]
        package = false

        [tool.uv.dependency-groups]
        dev = { requires-python = "~=3.12" }
    "#})?;
    uv_snapshot!(context.filters(), context.sync().args(["--offline", "--no-index"]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    warning: The following `requires-python` specifiers use the tilde specifier (`~=`) without a patch version:
    - `example`: `~=3.12` is interpreted as `>=3.12, <4`; use `~=3.12.[X]` to constrain the version as `>=3.12.[X], <3.13`
    - `example:dev`: `~=3.12` is interpreted as `>=3.12, <4`; use `~=3.12.[X]` to constrain the version as `>=3.12.[X], <3.13`
    We recommend only using the tilde specifier with a patch version to avoid ambiguity.
    Resolved 1 package in [TIME]
    Checked in [TIME]
    "#);
    Ok(())
}

#[test]
fn sync_tilde_warning_only_selected_groups() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "example"
        version = "0.1.0"
        requires-python = ">=3.12"

        [dependency-groups]
        dev = []
        docs = []

        [tool.uv]
        package = false
        default-groups = []

        [tool.uv.dependency-groups]
        dev = { requires-python = "~=3.12" }
        docs = { requires-python = "~=3.12" }
    "#})?;
    uv_snapshot!(context.filters(), context.sync().args(["--offline", "--no-index", "--group", "docs"]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    warning: The `requires-python` specifier (`~=3.12`) in `example:docs` uses the tilde specifier (`~=`) without a patch version. This will be interpreted as `>=3.12, <4`. Did you mean `~=3.12.[X]` to constrain the version as `>=3.12.[X], <3.13`? We recommend only using the tilde specifier with a patch version to avoid ambiguity.
    Resolved 1 package in [TIME]
    Checked in [TIME]
    "#);
    uv_snapshot!(context.filters(), context.sync().args(["--offline", "--no-index"]), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked in [TIME]
    "#);
    Ok(())
}
