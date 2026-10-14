//! Sources available to `uv pip sync` when index access is disabled.

use anyhow::Result;
use assert_fs::prelude::*;

use uv_test::packse::PackseServer;
use uv_test::uv_snapshot;

#[test]
fn install_no_index() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("iniconfig==2.0.0")?;

    uv_snapshot!(context.pip_sync()
        .args(["requirements.txt", "--no-index", "--strict"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because iniconfig was not found in the provided package locations and you require iniconfig==2.0.0, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because index lookups were disabled and no additional package locations were provided (try: `--find-links <uri>`)
    ");
    context.assert_not_installed("iniconfig");
    Ok(())
}

/// Disabling indexes prevents use of cached registry artifacts, even when they work offline.
#[test]
fn install_no_index_cached() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let index = PackseServer::new("simple/dependency-groups.toml");
    let index_url = index.index_url();
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("iniconfig==2.0.0")?;

    uv_snapshot!(context.pip_sync()
        .args(["requirements.txt", "--strict", "--default-index"])
        .arg(&index_url), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");
    context.assert_installed("iniconfig", "2.0.0");
    drop(index);

    uv_snapshot!(context.pip_uninstall().arg("iniconfig"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - iniconfig==2.0.0
    ");

    uv_snapshot!(context.pip_sync()
        .args(["requirements.txt", "--no-index", "--strict", "--default-index"])
        .arg(&index_url), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because iniconfig was not found in the provided package locations and you require iniconfig==2.0.0, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because index lookups were disabled and no additional package locations were provided (try: `--find-links <uri>`)
    ");
    context.assert_not_installed("iniconfig");

    uv_snapshot!(context.pip_sync()
        .args(["requirements.txt", "--offline", "--strict", "--default-index"])
        .arg(&index_url), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");
    context.assert_installed("iniconfig", "2.0.0");
    Ok(())
}

#[test]
fn find_links_no_index_match() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("tqdm==1000.0.0")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--no-index", "--find-links"])
        .arg(context.workspace_root.join("test/links")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==1000.0.0
    ");
    context
        .assert_command(
            "import tqdm; from importlib.metadata import version; print(version('tqdm'))",
        )
        .success()
        .stdout("1000.0.0\n");
    Ok(())
}

#[test]
fn find_links_offline_match() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("tqdm==1000.0.0")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--offline", "--find-links"])
        .arg(context.workspace_root.join("test/links")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==1000.0.0
    ");
    context
        .assert_command(
            "import tqdm; from importlib.metadata import version; print(version('tqdm'))",
        )
        .success()
        .stdout("1000.0.0\n");
    Ok(())
}

/// An unavailable requirement must not partially sync the environment.
#[test]
fn find_links_offline_no_match() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let links = context.workspace_root.join("test/links");
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("numpy\ntqdm==1000.0.0\n")?;

    uv_snapshot!(context.filters(), context.pip_install()
        .args(["ok==1.0.0", "--no-index", "--find-links"])
        .arg(&links), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==1.0.0
    ");

    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--offline", "--find-links"])
        .arg(&links), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because numpy was not found in the cache and you require numpy, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because the network was disabled. When the network is disabled, registry packages may only be read from the cache.
    ");
    context.assert_not_installed("tqdm");
    context.assert_installed("ok", "1.0.0");
    Ok(())
}
