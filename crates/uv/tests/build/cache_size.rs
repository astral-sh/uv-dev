use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::{FileWriteBin, PathChild};

use uv_test::{assert_path_missing, uv_snapshot};

/// Reporting the size of a missing cache should not create it.
#[test]
fn cache_size_missing_cache_remains_missing() {
    let context = uv_test::test_context_with_versions!(&[]).with_cache_dir("missing-cache");
    assert_path_missing(&context.cache_dir);

    uv_snapshot!(context.cache_size().arg("--preview"), @"
    exit_code: 0 (success)
    ----- stdout -----
    0
    ");

    assert_path_missing(&context.cache_dir);
}

/// Files added to the cache contribute to its reported disk usage.
#[test]
fn cache_size_populated_raw() -> anyhow::Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let read_cache_size = || -> anyhow::Result<u64> {
        let output = context
            .cache_size()
            .arg("--preview")
            .assert()
            .success()
            .stderr("");
        Ok(std::str::from_utf8(&output.get_output().stdout)?
            .trim()
            .parse()?)
    };

    let empty_size = read_cache_size()?;
    context
        .cache_dir
        .child("payload.bin")
        .write_binary(&[42; 8192])?;
    let populated_size = read_cache_size()?;
    assert!(
        populated_size > empty_size,
        "adding a file should increase cache usage (empty: {empty_size}, populated: {populated_size})"
    );
    Ok(())
}

/// A populated cache reports human-readable units.
#[test]
fn cache_size_populated_human() -> anyhow::Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_cache_size();
    context
        .cache_dir
        .child("payload.bin")
        .write_binary(&[42; 8192])?;

    uv_snapshot!(context.filters(), context.cache_size().arg("--preview").arg("--human"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [SIZE]KiB
    ");
    Ok(())
}

/// Explicit output formats override terminal detection.
#[test]
fn cache_size_output_formats() {
    let context = uv_test::test_context_with_versions!(&[]).with_cache_dir("missing-cache");

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
    let context = uv_test::test_context_with_versions!(&[]).with_cache_dir("missing-cache");

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
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.cache_size().arg("--preview").arg("--human").arg("--output-format").arg("machine"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--human' cannot be used with '--output-format <OUTPUT_FORMAT>'

    Usage: uv cache size --cache-dir [CACHE_DIR] --human

    For more information, try '--help'.
    ");
}
