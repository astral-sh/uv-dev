use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_cache::{Cache, CacheBucket};
use uv_static::EnvVars;

use uv_test::{assert_path_missing, uv_snapshot};

/// `cache prune` should remove all cached environments from the cache.
#[test]
fn prune_cached_env() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_sizes_and_units()
        // The cache entry does not have a stable key, so we filter it out.
        .with_filter((
            r"\[CACHE_DIR\](\\|\/)(.*?)(\\|\/).*",
            "[CACHE_DIR]/$2/[ENTRY]",
        ));
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("pytest@8.0.0")
        .arg("--version")
        .env(EnvVars::UV_TOOL_DIR, tool_dir.as_os_str())
        .env(EnvVars::XDG_BIN_HOME, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    pytest 8.0.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + iniconfig==2.0.0
     + packaging==24.0
     + pluggy==1.4.0
     + pytest==8.0.0
    ");

    let environments = fs_err::read_dir(
        Cache::from_path(context.cache_dir.path()).bucket(CacheBucket::Environments),
    )?
    .map(|entry| entry.map(|entry| entry.path()))
    .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        environments.len(),
        1,
        "cached environments: {environments:?}"
    );

    uv_snapshot!(context.filters(), context.prune().arg("--verbose"), @"
    exit_code: 0 (success)
    ----- stderr -----
    DEBUG Searching for user configuration in: [UV_USER_CONFIG_DIR]/uv.toml
    DEBUG uv [VERSION] ([COMMIT] DATE)
    Pruning cache at: [CACHE_DIR]/
    DEBUG Removing cached environment: [CACHE_DIR]/environments-v2/[ENTRY]
    DEBUG Removing dangling cache archive: [CACHE_DIR]/archive-v0/[ENTRY]
    Removed [N] files ([SIZE])
    ");
    assert_path_missing(&environments[0]);
    Ok(())
}

/// `cache prune --ci` should remove all unzipped archives.
#[test]
fn prune_unzipped() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_exclude_newer("2025-01-01T00:00Z")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! { r"
        source-distribution==0.0.1
        iniconfig
    " })?;

    // Install a requirement, to populate the cache.
    uv_snapshot!(context.filters(), context.pip_install().arg("-r").arg("requirements.txt").arg("--reinstall"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + iniconfig==2.0.0
     + source-distribution==0.0.1
    ");

    uv_snapshot!(context.filters(), context.prune().arg("--ci"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Pruning cache at: [CACHE_DIR]/
    Removed [N] files ([SIZE])
    ");

    context.venv().arg("--clear").assert().success();

    // Reinstalling the source distribution should not require re-downloading the source
    // distribution.
    requirements_txt.write_str(indoc! { r"
        source-distribution==0.0.1
    " })?;
    uv_snapshot!(context.filters(), context.pip_install().arg("-r").arg("requirements.txt").arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.1
    ");

    // But reinstalling the other package should require a download, since we pruned the wheel.
    requirements_txt.write_str(indoc! { r"
        iniconfig
    " })?;
    uv_snapshot!(context.filters(), context.pip_install().arg("-r").arg("requirements.txt").arg("--offline"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because all versions of iniconfig need to be downloaded from a registry and you require iniconfig, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because the network was disabled. When the network is disabled, registry packages may only be read from the cache.
    ");

    Ok(())
}
