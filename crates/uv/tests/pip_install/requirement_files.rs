//! File-valued requirement options and environment variables.

use anyhow::Result;
use assert_fs::prelude::*;

use uv_static::EnvVars;
use uv_test::uv_snapshot;

/// Explicit paths with spaces take precedence over their environment-variable counterparts.
#[test]
fn requirement_file_paths_with_spaces() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("requirements.txt").touch()?;

    // Empty files produce path-specific warnings, so each option is independently observable.
    let requirements_dir = context.temp_dir.child("requirements with spaces");
    requirements_dir.create_dir_all()?;
    let constraints = requirements_dir.child("constraints.txt");
    constraints.touch()?;
    let overrides = requirements_dir.child("overrides.txt");
    overrides.touch()?;
    let excludes = requirements_dir.child("excludes.txt");
    excludes.touch()?;
    let build_constraints = requirements_dir.child("build-constraints.txt");
    build_constraints.touch()?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--constraint")
        .arg(constraints.path())
        .arg("--override")
        .arg(overrides.path())
        .arg("--exclude")
        .arg(excludes.path())
        .arg("--build-constraint")
        .arg(build_constraints.path())
        .env(EnvVars::UV_CONSTRAINT, "missing-constraint.txt")
        .env(EnvVars::UV_OVERRIDE, "missing-override.txt")
        .env(EnvVars::UV_EXCLUDE, "missing-exclude.txt")
        .env(EnvVars::UV_BUILD_CONSTRAINT, "missing-build-constraint.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Requirements file `requirements.txt` does not contain any dependencies
    warning: Requirements file `requirements with spaces/constraints.txt` does not contain any dependencies
    warning: Requirements file `requirements with spaces/overrides.txt` does not contain any dependencies
    warning: Requirements file `requirements with spaces/excludes.txt` does not contain any dependencies
    warning: Requirements file `requirements with spaces/build-constraints.txt` does not contain any dependencies
    Checked in [TIME]
    ");
    Ok(())
}

/// Both whitespace-delimited paths must be consumed for every file-valued environment variable.
#[test]
fn requirement_file_paths_from_environment() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("requirements.txt").touch()?;
    for path in [
        "constraint-1.txt",
        "constraint-2.txt",
        "override-1.txt",
        "override-2.txt",
        "exclude-1.txt",
        "exclude-2.txt",
        "build-constraint-1.txt",
        "build-constraint-2.txt",
    ] {
        context.temp_dir.child(path).touch()?;
    }

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .env(EnvVars::UV_CONSTRAINT, "constraint-1.txt constraint-2.txt")
        .env(EnvVars::UV_OVERRIDE, "override-1.txt override-2.txt")
        .env(EnvVars::UV_EXCLUDE, "exclude-1.txt exclude-2.txt")
        .env(EnvVars::UV_BUILD_CONSTRAINT, "build-constraint-1.txt build-constraint-2.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Requirements file `requirements.txt` does not contain any dependencies
    warning: Requirements file `constraint-1.txt` does not contain any dependencies
    warning: Requirements file `constraint-2.txt` does not contain any dependencies
    warning: Requirements file `override-1.txt` does not contain any dependencies
    warning: Requirements file `override-2.txt` does not contain any dependencies
    warning: Requirements file `exclude-1.txt` does not contain any dependencies
    warning: Requirements file `exclude-2.txt` does not contain any dependencies
    warning: Requirements file `build-constraint-1.txt` does not contain any dependencies
    warning: Requirements file `build-constraint-2.txt` does not contain any dependencies
    Checked in [TIME]
    ");
    Ok(())
}
