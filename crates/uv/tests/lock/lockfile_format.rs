//! Lockfile formatting validation and repair.

use anyhow::Result;
use assert_fs::prelude::*;
use indoc::formatdoc;
use uv_test::{TestContext, uv_snapshot};

/// The local dependency sorts after the project, keeping its first indented entry on line 13.
fn formatting_project() -> Result<TestContext> {
    let context = uv_test::test_context!("3.12");
    let filename = "validation-1.0.0-py3-none-any.whl";
    fs_err::copy(
        context.workspace_root.join("test/links").join(filename),
        context.temp_dir.child(filename),
    )?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["validation==1.0.0"]

        [tool.uv.sources]
        validation = {{ path = "{filename}" }}
    "#})?;
    Ok(context)
}

/// Checks that formatting-only changes are rejected when lockfile-format-check preview is enabled.
#[test]
fn preview_rejects_noncanonical_formatting() -> Result<()> {
    let context = formatting_project()?;

    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    let formatted = context.read("uv.lock");
    let unformatted = formatted.replacen("\n    {", "\n{", 1);
    assert_ne!(formatted, unformatted);
    context.temp_dir.child("uv.lock").write_str(&unformatted)?;

    // Formatting is not part of the stable lockfile contract.
    uv_snapshot!(context.filters(), context.lock().arg("--check").arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.lock().arg("--check").arg("--offline").arg("--preview-features").arg("lockfile-format-check"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: The lockfile at `uv.lock` has non-canonical formatting at line 13, but `--check` was provided.

    hint: To regenerate the lockfile, run `uv lock --refresh --preview-features lockfile-format-check`.
    ");

    uv_snapshot!(context.filters(), context.sync().arg("--locked").arg("--offline").arg("--preview-features").arg("lockfile-format-check"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: The lockfile at `uv.lock` has non-canonical formatting at line 13, but `--locked` was provided.

    hint: To regenerate the lockfile, run `uv lock --refresh --preview-features lockfile-format-check`.
    ");

    assert_eq!(context.read("uv.lock"), unformatted);

    Ok(())
}

#[test]
fn refresh_restores_canonical_formatting() -> Result<()> {
    let context = formatting_project()?;

    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    let original_lock = context.read("uv.lock");
    let dedented_lock = original_lock
        .lines()
        .map(str::trim_start)
        .collect::<Vec<_>>()
        .join("\n");
    context
        .temp_dir
        .child("uv.lock")
        .write_str(&dedented_lock)?;

    uv_snapshot!(context.filters(), context.lock().arg("--refresh").arg("--dry-run"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    No lockfile changes detected
    ");
    assert_eq!(context.read("uv.lock"), dedented_lock);

    uv_snapshot!(context.filters(), context.lock().arg("--refresh"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    assert_eq!(context.read("uv.lock"), dedented_lock);

    uv_snapshot!(context.filters(), context.lock().arg("--refresh").arg("--dry-run").arg("--preview-features").arg("lockfile-format-check"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Lockfile changes detected
    ");
    assert_eq!(context.read("uv.lock"), dedented_lock);

    uv_snapshot!(context.filters(), context.lock().arg("--refresh").arg("--preview-features").arg("lockfile-format-check"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    assert_eq!(context.read("uv.lock"), original_lock);

    uv_snapshot!(context.filters(), context.lock().arg("--check").arg("--preview-features").arg("lockfile-format-check"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.lock().arg("--refresh").arg("--dry-run").arg("--preview-features").arg("lockfile-format-check"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    No lockfile changes detected
    ");

    Ok(())
}
