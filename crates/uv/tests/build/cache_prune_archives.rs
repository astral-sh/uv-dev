use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;

use uv_cache::{Cache, CacheBucket};
use uv_test::packse::PackseServer;
use uv_test::{assert_path_missing, uv_snapshot};

/// Pruning retains the archive referenced by an installed wheel.
#[test]
fn prune_referenced_archive() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let index = PackseServer::new("simple/single-package.toml");

    context
        .pip_install()
        .args(["a==1.0.0", "--index-url"])
        .arg(index.index_url())
        .assert()
        .success();
    let cached_files = context.cache_files(CacheBucket::Archive)?;
    assert!(!cached_files.is_empty(), "expected cached wheel files");

    uv_snapshot!(context.filters(), context.prune().arg("--verbose"), @"
    exit_code: 0 (success)
    ----- stderr -----
    DEBUG Searching for user configuration in: [UV_USER_CONFIG_DIR]/uv.toml
    DEBUG uv [VERSION] ([COMMIT] DATE)
    Pruning cache at: [CACHE_DIR]/
    No unused entries found
    ");

    assert_eq!(context.cache_files(CacheBucket::Archive)?, cached_files);
    Ok(())
}

/// Pruning removes a wheel archive after its cache reference is removed.
#[test]
fn prune_unreferenced_archive() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units()
        .with_filter((
            r"\[CACHE_DIR\](\\|\/)(.*?)(\\|\/).*",
            "[CACHE_DIR]/$2/[ENTRY]",
        ));
    let index = PackseServer::new("simple/single-package.toml");

    context
        .pip_install()
        .args(["a==1.0.0", "--index-url"])
        .arg(index.index_url())
        .assert()
        .success();
    let cache = Cache::from_path(context.cache_dir.path());
    let archives = fs_err::read_dir(cache.bucket(CacheBucket::Archive))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    assert_eq!(archives.len(), 1, "cached wheel archives: {archives:?}");
    fs_err::remove_dir_all(cache.bucket(CacheBucket::Wheels))?;

    uv_snapshot!(context.filters(), context.prune().arg("--verbose"), @"
    exit_code: 0 (success)
    ----- stderr -----
    DEBUG Searching for user configuration in: [UV_USER_CONFIG_DIR]/uv.toml
    DEBUG uv [VERSION] ([COMMIT] DATE)
    Pruning cache at: [CACHE_DIR]/
    DEBUG Removing dangling cache archive: [CACHE_DIR]/archive-v0/[ENTRY]
    Removed [N] files ([SIZE])
    ");

    assert_path_missing(&archives[0]);
    Ok(())
}
