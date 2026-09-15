#[cfg(unix)]
use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
#[cfg(unix)]
use assert_fs::prelude::*;

#[cfg(unix)]
use uv_fs::link::{LinkMode, LinkOptions, link_dir};
use uv_test::uv_snapshot;

/// Test that `cache size` returns 0 for an empty cache directory (raw output).
#[test]
fn cache_size_empty_raw() {
    let context = uv_test::test_context!("3.12");

    // Clean cache first to ensure truly empty state
    context.clean().assert().success();

    uv_snapshot!(context.cache_size().arg("--preview"), @"
    exit_code: 0 (success)
    ----- stdout -----
    0
    ");
}

/// Test that `cache size` returns raw bytes after installing packages.
#[test]
fn cache_size_with_packages_raw() {
    let context = uv_test::test_context!("3.12").with_filtered_cache_size();

    // Install a requirement to populate the cache.
    context.pip_install().arg("iniconfig").assert().success();

    // Check cache size is now positive (raw bytes).
    uv_snapshot!(context.filters(), context.cache_size().arg("--preview"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [SIZE]
    ");
}

/// Test that `cache size --human` returns human-readable format after installing packages.
#[test]
fn cache_size_with_packages_human() {
    let context = uv_test::test_context!("3.12").with_filtered_cache_size();

    // Install a requirement to populate the cache.
    context.pip_install().arg("iniconfig").assert().success();

    // Check cache size with --human flag
    uv_snapshot!(context.filters(), context.cache_size().arg("--preview").arg("--human"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [SIZE]KiB
    ");
}

/// Physical cache sizing should count copy-on-write clones within the cache only once.
#[cfg(unix)]
#[test]
fn cache_size_physical_cached_clones() -> Result<()> {
    let Some(context) = uv_test::test_context!("3.12").with_cache_on_cow_fs()? else {
        return Ok(());
    };
    context.clean().assert().success();

    let original = context.cache_dir.child("original");
    original.create_dir_all()?;
    original
        .child("original.bin")
        .write_binary(&vec![42; 1024 * 1024])?;

    let cloned = context.cache_dir.child("cloned");
    let link_mode = link_dir(&original, &cloned, &LinkOptions::new(LinkMode::Clone))?;
    assert_eq!(
        link_mode,
        LinkMode::Clone,
        "the configured copy-on-write filesystem did not clone the cached file"
    );

    uv_snapshot!(context.filters(), context.cache_size().arg("--preview-features").arg("cache-size").arg("--human"), @"
    exit_code: 0 (success)
    ----- stdout -----
    2.0MiB
    ");

    uv_snapshot!(context.filters(), context.cache_size().arg("--preview-features").arg("cache-size,cache-physical-space").arg("--human"), @"
    exit_code: 0 (success)
    ----- stdout -----
    1.0MiB
    ");

    assert!(original.child("original.bin").is_file());
    assert!(cloned.child("original.bin").is_file());

    Ok(())
}

/// A clone retained outside the cache still contributes once to the cache's physical footprint.
#[cfg(unix)]
#[test]
fn cache_size_physical_external_clone() -> Result<()> {
    let Some(context) = uv_test::test_context!("3.12").with_cache_on_cow_fs()? else {
        return Ok(());
    };
    context.clean().assert().success();

    let retained = context.cache_dir.path().with_file_name("retained");
    fs_err::create_dir_all(&retained)?;
    let original = retained.join("original.bin");
    fs_err::write(&original, vec![42; 1024 * 1024])?;

    let cloned = context.cache_dir.child("cloned");
    let link_mode = link_dir(&retained, &cloned, &LinkOptions::new(LinkMode::Clone))?;
    assert_eq!(
        link_mode,
        LinkMode::Clone,
        "the configured copy-on-write filesystem did not clone the cached file"
    );

    uv_snapshot!(context.filters(), context.cache_size().arg("--preview-features").arg("cache-size,cache-physical-space").arg("--human"), @"
    exit_code: 0 (success)
    ----- stdout -----
    1.0MiB
    ");

    assert!(original.is_file());

    Ok(())
}

/// Physical cache sizing should count hardlinked files once, including links retained elsewhere.
#[cfg(unix)]
#[test]
fn cache_size_physical_hardlinked_files() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.clean().assert().success();
    context.cache_dir.create_dir_all()?;

    let original = context.cache_dir.child("original.bin");
    original.write_binary(&vec![42; 1024 * 1024])?;
    fs_err::OpenOptions::new()
        .write(true)
        .open(original.path())?
        .sync_all()?;
    let cached = context.cache_dir.child("hardlinked.bin");
    fs_err::hard_link(&original, &cached)?;
    let retained = context.cache_dir.path().with_file_name("retained.bin");
    fs_err::hard_link(&original, &retained)?;

    uv_snapshot!(context.filters(), context.cache_size().arg("--preview-features").arg("cache-size,cache-physical-space").arg("--human"), @"
    exit_code: 0 (success)
    ----- stdout -----
    1.0MiB
    ");

    assert!(retained.is_file());

    Ok(())
}

/// Explicit output formats override terminal detection.
#[test]
fn cache_size_output_formats() {
    let context = uv_test::test_context!("3.12");
    context.clean().assert().success();

    uv_snapshot!(context.cache_size().arg("--preview").arg("--output-format").arg("auto"), @"
    exit_code: 0 (success)
    ----- stdout -----
    0
    ");

    uv_snapshot!(context.filters(), context.cache_size().arg("--preview").arg("--output-format").arg("human"), @"
    exit_code: 0 (success)
    ----- stdout -----
    0B
    ");

    uv_snapshot!(context.cache_size().arg("--preview").arg("--output-format").arg("machine"), @"
    exit_code: 0 (success)
    ----- stdout -----
    0
    ");
}

/// Existing human-readable flags remain equivalent to `--output-format human`.
#[test]
fn cache_size_human_aliases() {
    let context = uv_test::test_context!("3.12");
    context.clean().assert().success();

    uv_snapshot!(context.filters(), context.cache_size().arg("--preview").arg("--human"), @"
    exit_code: 0 (success)
    ----- stdout -----
    0B
    ");

    uv_snapshot!(context.filters(), context.cache_size().arg("--preview").arg("-H"), @"
    exit_code: 0 (success)
    ----- stdout -----
    0B
    ");

    uv_snapshot!(context.filters(), context.cache_size().arg("--preview").arg("--human-readable"), @"
    exit_code: 0 (success)
    ----- stdout -----
    0B
    ");
}

/// Legacy human-readable flags cannot be combined with an explicit output format.
#[test]
fn cache_size_output_format_conflicts_with_human() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.filters(), context.cache_size().arg("--preview").arg("--human").arg("--output-format").arg("machine"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--human' cannot be used with '--output-format <OUTPUT_FORMAT>'

    Usage: uv cache size --cache-dir [CACHE_DIR] --human

    For more information, try '--help'.
    ");
}
