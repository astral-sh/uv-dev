//! Argument validation before Python discovery or dependency resolution.

use anyhow::Result;
use assert_fs::prelude::*;

use uv_static::EnvVars;
use uv_test::uv_snapshot;

#[test]
fn exclude_newer_package_missing_value() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    context
        .temp_dir
        .child("requirements.in")
        .write_str("tqdm")?;
    uv_snapshot!(context.pip_compile()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .args(["requirements.in", "--exclude-newer-package", "tqdm"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: invalid value 'tqdm' for '--exclude-newer-package <EXCLUDE_NEWER_PACKAGE>': Invalid `exclude-newer-package` value `tqdm`: expected format `PACKAGE=DATE` or `PACKAGE=false`

    For more information, try '--help'.
    ");
    Ok(())
}

#[test]
fn exclude_newer_package_invalid_date() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    context
        .temp_dir
        .child("requirements.in")
        .write_str("tqdm")?;
    uv_snapshot!(context.pip_compile()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .args(["requirements.in", "--exclude-newer-package", "tqdm=invalid-date"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: invalid value 'tqdm=invalid-date' for '--exclude-newer-package <EXCLUDE_NEWER_PACKAGE>': Invalid `exclude-newer-package` value `invalid-date`: `invalid-date` could not be parsed as a valid exclude-newer value (expected a date like `2024-01-01`, a timestamp like `2024-01-01T00:00:00Z`, or a duration like `3 days` or `P3D`)

    For more information, try '--help'.
    ");
    Ok(())
}
