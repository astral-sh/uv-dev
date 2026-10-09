use assert_fs::fixture::{FileWriteStr, PathChild, PathCreateDir};

use uv_static::EnvVars;
use uv_test::uv_snapshot;

#[test]
fn tool_uninstall_all_keeps_unrelated_files() -> anyhow::Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_tool_dirs();
    let parent = context.temp_dir.child("custom");
    let tools = parent.child("tools");
    tools.create_dir_all()?;
    tools.child("notes.txt").write_str("tool root notes")?;
    parent.child("notes.txt").write_str("parent notes")?;

    uv_snapshot!(context.filters(), context.tool_uninstall()
        .arg("--all")
        .env(EnvVars::UV_TOOL_DIR, tools.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Nothing to uninstall
    ");

    assert_eq!(context.read("custom/tools/notes.txt"), "tool root notes");
    assert_eq!(context.read("custom/notes.txt"), "parent notes");
    assert!(tools.join(".lock").is_file());
    assert!(tools.join(".gitignore").is_file());
    Ok(())
}

#[test]
fn tool_uninstall_last_environment_keeps_parent_files() -> anyhow::Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_tool_dirs();
    let parent = context.temp_dir.child("custom");
    let tools = parent.child("tools");
    tools.child("example").create_dir_all()?;
    parent.child("notes.txt").write_str("parent notes")?;

    uv_snapshot!(context.filters(), context.tool_uninstall()
        .arg("example")
        .env(EnvVars::UV_TOOL_DIR, tools.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed dangling environment for `example`
    ");

    assert!(!tools.join("example").exists());
    assert_eq!(context.read("custom/notes.txt"), "parent notes");
    assert!(tools.join(".lock").is_file());
    Ok(())
}

#[test]
fn tool_uninstall_all_keeps_unrecognized_temporary_directories() -> anyhow::Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_tool_dirs();
    let directory = context.temp_dir.child("tools/.tmp-user-data");
    directory.create_dir_all()?;
    directory.child("notes.txt").write_str("unrelated notes")?;

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("--all"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Ignoring tool directory `tools/[TMP]` with an invalid package name; move it outside the tool directory, or remove it if no longer needed
    Nothing to uninstall
    ");

    assert_eq!(
        context.read("tools/.tmp-user-data/notes.txt"),
        "unrelated notes"
    );
    Ok(())
}

#[test]
#[cfg(unix)]
fn tool_uninstall_all_keeps_unrecognized_symlinks() -> anyhow::Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_tool_dirs();
    let tools = context.temp_dir.child("tools");
    tools.create_dir_all()?;
    let target = context.temp_dir.child("unrelated");
    target.create_dir_all()?;
    target.child("notes.txt").write_str("unrelated notes")?;
    fs_err::os::unix::fs::symlink(target.path(), tools.join("directory-link"))?;
    fs_err::os::unix::fs::symlink("missing", tools.join("dangling-link"))?;

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("--all"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Nothing to uninstall
    ");

    assert_eq!(
        fs_err::read_link(tools.join("directory-link"))?,
        target.path()
    );
    assert_eq!(
        fs_err::read_link(tools.join("dangling-link"))?,
        Path::new("missing")
    );
    assert_eq!(context.read("unrelated/notes.txt"), "unrelated notes");
    Ok(())
}
#[cfg(unix)]
use std::path::Path;
