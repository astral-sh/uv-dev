use anyhow::Result;
use assert_fs::prelude::*;

use uv_test::uv_snapshot;

#[test]
fn no_arguments() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.pip_uninstall(), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the following required arguments were not provided:
      <PACKAGE|--requirements <REQUIREMENTS>>

    Usage: uv pip uninstall --cache-dir [CACHE_DIR] <PACKAGE|--requirements <REQUIREMENTS>>

    For more information, try '--help'.
    "
    );
}

#[test]
fn invalid_requirement() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.pip_uninstall()
        .arg("flask==1.0.x"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse: `flask==1.0.x`
      cause: after parsing `1.0`, found `.x`, which is not part of a valid version
             flask==1.0.x
                  ^^^^^^^
    ");
}

#[test]
fn missing_requirements_txt() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.pip_uninstall()
        .arg("-r")
        .arg("requirements.txt"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: File not found: requirements.txt
    "
    );
}

#[test]
fn invalid_requirements_txt_requirement() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("flask==1.0.x")?;

    uv_snapshot!(context.filters(), context.pip_uninstall()
        .arg("-r")
        .arg("requirements.txt"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Couldn't parse requirement in `requirements.txt` at position 0
      cause: after parsing `1.0`, found `.x`, which is not part of a valid version
             flask==1.0.x
                  ^^^^^^^
    ");

    Ok(())
}
