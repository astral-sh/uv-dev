use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use assert_fs::prelude::*;

use uv_cache::CacheBucket;
use uv_static::EnvVars;
use uv_test::{TestContext, assert_path_missing, uv_snapshot};

fn sole_project_revision(context: &TestContext) -> Result<PathBuf> {
    let wheels = context
        .cache_files(CacheBucket::SourceDistributions)?
        .into_iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name == "project-0.1.0-py3-none-any.whl")
        })
        .collect::<Vec<_>>();
    let [wheel] = wheels.as_slice() else {
        bail!("expected one cached project wheel, found {wheels:?}");
    };
    Ok(wheel
        .parent()
        .context("cached wheel has no revision directory")?
        .to_path_buf())
}

fn write_project_fixture(context: &TestContext) -> Result<()> {
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#,
    )?;

    context
        .temp_dir
        .child("src")
        .child("project")
        .child("__init__.py")
        .touch()?;
    context.temp_dir.child("README").touch()?;
    Ok(())
}

/// `cache prune` should remove any stale source distribution revisions.
#[test]
fn prune_stale_revision() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units()
        // The cache entry does not have a stable key, so we filter it out.
        .with_filter((
            r"\[CACHE_DIR\](\\|\/)(.*?)(\\|\/).*",
            "[CACHE_DIR]/$2/[ENTRY]",
        ));

    write_project_fixture(&context)?;

    // Install the same package twice, with `--reinstall`.
    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg(".")
        .arg("--reinstall"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + project==0.1.0 (from file://[TEMP_DIR]/)
    ");
    let stale_revision = sole_project_revision(&context)?;

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg(".")
        .arg("--reinstall"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ project==0.1.0 (from file://[TEMP_DIR]/)
    ");
    assert!(
        stale_revision.is_dir(),
        "first revision disappeared before pruning: {}",
        stale_revision.display()
    );

    // Pruning should remove the unused revision.
    uv_snapshot!(context.filters(), context.prune().arg("--verbose"), @"
    exit_code: 0 (success)
    ----- stderr -----
    DEBUG Found workspace root: [TEMP_DIR]/
    DEBUG Adding root workspace member: [TEMP_DIR]/
    DEBUG Skipping `pyproject.toml` in `[TEMP_DIR]/` (no `[tool]` section)
    DEBUG Searching for user configuration in: [UV_USER_CONFIG_DIR]/uv.toml
    DEBUG uv [VERSION] ([COMMIT] DATE)
    Pruning cache at: [CACHE_DIR]/
    DEBUG Removing dangling source revision: [CACHE_DIR]/sdists-v10/[ENTRY]
    DEBUG Removing dangling cache archive: [CACHE_DIR]/archive-v0/[ENTRY]
    Removed [N] files ([SIZE])
    ");
    assert_ne!(sole_project_revision(&context)?, stale_revision);
    assert_path_missing(stale_revision);

    // Reinstall the project after pruning.
    uv_snapshot!(context.filters(), context
        .pip_uninstall()
        .arg("."), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - project==0.1.0 (from file://[TEMP_DIR]/)
    ");

    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("."), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + project==0.1.0 (from file://[TEMP_DIR]/)
    ");

    Ok(())
}

/// Content-addressed cache entries should remain reachable across equivalent stale revisions.
#[test]
fn prune_stale_revision_content_addressed_cache() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units()
        // The cache entry does not have a stable key, so we filter it out.
        .with_filter((
            r"\[CACHE_DIR\](\\|\/)(.*?)(\\|\/).*",
            "[CACHE_DIR]/$2/[ENTRY]",
        ));

    write_project_fixture(&context)?;

    // Install the same package twice, with `--reinstall`.
    uv_snapshot!(context.filters(), context
        .pip_install()
        .env(EnvVars::UV_PREVIEW_FEATURES, "content-addressed-cache")
        .arg(".")
        .arg("--reinstall"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + project==0.1.0 (from file://[TEMP_DIR]/)
    ");
    let stale_revision = sole_project_revision(&context)?;
    let archive_files = context.cache_files(CacheBucket::Archive)?;
    assert!(
        !archive_files.is_empty(),
        "expected a cached project archive"
    );

    uv_snapshot!(context.filters(), context
        .pip_install()
        .env(EnvVars::UV_PREVIEW_FEATURES, "content-addressed-cache")
        .arg(".")
        .arg("--reinstall"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ project==0.1.0 (from file://[TEMP_DIR]/)
    ");
    assert!(
        stale_revision.is_dir(),
        "first revision disappeared before pruning: {}",
        stale_revision.display()
    );

    // Pruning should remove the unused revision but retain the shared archive.
    uv_snapshot!(context.filters(), context.prune().arg("--verbose"), @"
    exit_code: 0 (success)
    ----- stderr -----
    DEBUG Found workspace root: [TEMP_DIR]/
    DEBUG Adding root workspace member: [TEMP_DIR]/
    DEBUG Skipping `pyproject.toml` in `[TEMP_DIR]/` (no `[tool]` section)
    DEBUG Searching for user configuration in: [UV_USER_CONFIG_DIR]/uv.toml
    DEBUG uv [VERSION] ([COMMIT] DATE)
    Pruning cache at: [CACHE_DIR]/
    DEBUG Removing dangling source revision: [CACHE_DIR]/sdists-v10/[ENTRY]
    Removed [N] files ([SIZE])
    ");
    assert_ne!(sole_project_revision(&context)?, stale_revision);
    assert_path_missing(stale_revision);
    assert_eq!(context.cache_files(CacheBucket::Archive)?, archive_files);

    // Reinstall the project after pruning.
    uv_snapshot!(context.filters(), context
        .pip_uninstall()
        .arg("."), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - project==0.1.0 (from file://[TEMP_DIR]/)
    ");

    uv_snapshot!(context.filters(), context
        .pip_install()
        .env(EnvVars::UV_PREVIEW_FEATURES, "content-addressed-cache")
        .arg("."), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + project==0.1.0 (from file://[TEMP_DIR]/)
    ");

    Ok(())
}
