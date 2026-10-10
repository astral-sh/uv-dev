use assert_cmd::assert::OutputAssertExt;

use uv_test::uv_snapshot;

/// An empty cache can retain filesystem space in its coordination directory.
#[test]
fn cache_size_empty_raw() {
    let context = uv_test::test_context!("3.12").with_filtered_cache_size();

    // Remove entries while retaining the root coordination lock.
    context.clean().assert().success();

    assert!(context.cache_dir.join(".lock").is_file());

    uv_snapshot!(context.filters(), context.cache_size().arg("--preview"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [SIZE]
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

/// Explicit output formats override terminal detection.
#[test]
fn cache_size_output_formats() -> anyhow::Result<()> {
    let context = uv_test::test_context!("3.12");
    context.clean().assert().success();
    // Use a missing cache so directory accounting cannot affect the format assertions.
    fs_err::remove_dir_all(context.cache_dir.path())?;

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
    Ok(())
}

/// Existing human-readable flags remain equivalent to `--output-format human`.
#[test]
fn cache_size_human_aliases() -> anyhow::Result<()> {
    let context = uv_test::test_context!("3.12");
    context.clean().assert().success();
    // Use a missing cache so directory accounting cannot affect the format assertions.
    fs_err::remove_dir_all(context.cache_dir.path())?;

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
    Ok(())
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
