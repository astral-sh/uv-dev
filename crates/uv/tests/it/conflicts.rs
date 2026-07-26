use anyhow::Result;
use assert_fs::prelude::*;

use uv_test::uv_snapshot;

/// Selecting an extra always selects its package, so the conflict is impossible to satisfy.
///
/// Regression test for: <https://github.com/astral-sh/uv/issues/20694>
#[test]
fn reject_self_conflicting_extra() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "self-conflict"
        version = "0.1.0"
        requires-python = ">=3.12"

        [project.optional-dependencies]
        foo = []

        [tool.uv]
        conflicts = [[
            { package = "self-conflict" },
            { package = "self-conflict", extra = "foo" },
        ]]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: Declaring conflicts for packages (`package = ...`) is experimental and may change without warning. Pass `--preview-features package-conflicts` to disable this warning.
    error: Extra `foo` and package `self-conflict` are incompatible with the declared conflicts: {`self-conflict[foo]`, self-conflict}
    ");

    Ok(())
}
