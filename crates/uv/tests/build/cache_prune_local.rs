use anyhow::Result;
use assert_fs::prelude::*;

use uv_test::{assert_path_missing, uv_snapshot};

/// Cache pruning should not count storage retained by another hardlink.
#[cfg(unix)]
#[test]
fn prune_hardlinked_file() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    // Keep both hardlinks on the selected filesystem.
    let retained = context.cache_dir.path().with_file_name("retained.bin");
    fs_err::write(&retained, vec![42; 1024 * 1024])?;
    fs_err::OpenOptions::new()
        .write(true)
        .open(&retained)?
        .sync_all()?;

    let stale = context.cache_dir.child("stale-v0");
    stale.create_dir_all()?;
    fs_err::hard_link(&retained, stale.child("hardlinked.bin"))?;

    // Counting the externally retained hardlink would incorrectly report 1.0MiB.
    uv_snapshot!(context.filters(), context.prune(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Pruning cache at: [CACHE_DIR]/
    Removed 1 file (0B)
    ");

    stale.create_dir_all()?;
    fs_err::hard_link(&retained, stale.child("hardlinked.bin"))?;

    uv_snapshot!(context.filters(), context.prune().arg("--preview-features").arg("cache-physical-space"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Pruning cache at: [CACHE_DIR]/
    Removed 1 file (0B)
    ");

    assert_path_missing(&stale);
    assert_eq!(context.read_bytes(&retained), vec![42; 1024 * 1024]);

    Ok(())
}

/// `cache prune` should fall back to logical space on unsupported filesystems.
#[cfg(unix)]
#[test]
fn prune_physical_space_unsupported_fs() -> Result<()> {
    let Some(context) = uv_test::test_context_with_versions!(&[]).with_cache_on_alt_fs()? else {
        return Ok(());
    };

    let stale = context.cache_dir.child("stale-v0");
    stale.create_dir_all()?;
    stale
        .child("cached.bin")
        .write_binary(&vec![42; 1024 * 1024])?;

    uv_snapshot!(context.filters(), context.prune().arg("--preview-features").arg("cache-physical-space"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Pruning cache at: [ALT_FS]/[CACHE_DIR]/
    Removed 1 file (1.0MiB)
    ");

    Ok(())
}

/// `cache prune` should preserve cached Python downloads.
#[test]
fn prune_python_downloads() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    let python_cache = context.cache_dir.child("python-v1");
    python_cache.create_dir_all()?;
    let download = python_cache.child("python.tar.gz");
    download.write_binary(b"cached Python download")?;

    uv_snapshot!(context.filters(), context.prune(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Pruning cache at: [CACHE_DIR]/
    No unused entries found
    ");

    assert_eq!(context.read_bytes(&download), b"cached Python download");

    Ok(())
}

/// `cache prune` should remove any temporary build environments left in the cache.
#[test]
fn prune_temporary_build_environment() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_counts()
        .with_filtered_sizes_and_units()
        .with_filter((
            r"\[CACHE_DIR\](\\|\/)(.*?)(\\|\/).*",
            "[CACHE_DIR]/$2/[ENTRY]",
        ));

    // Populate the cache with a temporary build environment.
    let builds = context.cache_dir.child("builds-v0").child(".tmp123456");
    builds.create_dir_all()?;
    builds.child("pyvenv.cfg").write_str("home = /usr/bin")?;

    uv_snapshot!(context.filters(), context.prune().arg("--verbose"), @"
    exit_code: 0 (success)
    ----- stderr -----
    DEBUG Searching for user configuration in: [UV_USER_CONFIG_DIR]/uv.toml
    DEBUG uv [VERSION] ([COMMIT] DATE)
    Pruning cache at: [CACHE_DIR]/
    DEBUG Removing temporary build environment: [CACHE_DIR]/builds-v0/[ENTRY]
    Removed [N] files ([SIZE])
    ");

    assert_path_missing(&builds);

    Ok(())
}
