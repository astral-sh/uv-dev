//! Builds and file-listing checks for the bundled build backend.

use std::collections::BTreeMap;

use anyhow::Result;
use assert_fs::assert::PathAssert;
use assert_fs::fixture::{FileWriteBin, PathChild};
use predicates::prelude::predicate;
use uv_test::packse::generate_wheel;
use uv_test::uv_snapshot;

/// Test the uv fast path. Tests all four possible build plans:
/// * Defaults
/// * `--sdist`
/// * `--wheel`
/// * `--sdist --wheel`
#[test]
fn build_plans() {
    let context = uv_test::test_context!("3.12");

    let built_by_uv = context.workspace_root.join("test/packages/built-by-uv");

    uv_snapshot!(context.build()
        .arg(&built_by_uv)
        .arg("--out-dir")
        .arg(context.temp_dir.join("output1")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building source distribution...
    Building wheel from source distribution...
    Successfully built output1/built_by_uv-0.1.0.tar.gz
    Successfully built output1/built_by_uv-0.1.0-py3-none-any.whl
    ");
    context
        .temp_dir
        .child("output1")
        .child("built_by_uv-0.1.0.tar.gz")
        .assert(predicate::path::is_file());
    context
        .temp_dir
        .child("output1")
        .child("built_by_uv-0.1.0-py3-none-any.whl")
        .assert(predicate::path::is_file());

    uv_snapshot!(context.build()
        .arg(&built_by_uv)
        .arg("--out-dir")
        .arg(context.temp_dir.join("output2"))
        .arg("--sdist"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building source distribution...
    Successfully built output2/built_by_uv-0.1.0.tar.gz
    ");
    context
        .temp_dir
        .child("output2")
        .child("built_by_uv-0.1.0.tar.gz")
        .assert(predicate::path::is_file());

    uv_snapshot!(context.build()
        .arg(&built_by_uv)
        .arg("--out-dir")
        .arg(context.temp_dir.join("output3"))
        .arg("--wheel"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building wheel...
    Successfully built output3/built_by_uv-0.1.0-py3-none-any.whl
    ");
    context
        .temp_dir
        .child("output3")
        .child("built_by_uv-0.1.0-py3-none-any.whl")
        .assert(predicate::path::is_file());

    uv_snapshot!(context.build()
        .arg(&built_by_uv)
        .arg("--out-dir")
        .arg(context.temp_dir.join("output4"))
        .arg("--sdist")
        .arg("--wheel"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building source distribution...
    Building wheel...
    Successfully built output4/built_by_uv-0.1.0.tar.gz
    Successfully built output4/built_by_uv-0.1.0-py3-none-any.whl
    ");
    context
        .temp_dir
        .child("output4")
        .child("built_by_uv-0.1.0.tar.gz")
        .assert(predicate::path::is_file());
    context
        .temp_dir
        .child("output4")
        .child("built_by_uv-0.1.0-py3-none-any.whl")
        .assert(predicate::path::is_file());
}

#[test]
fn require_build_hashes() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let built_by_uv = context.workspace_root.join("test/packages/built-by-uv");
    let (filename, wheel) = generate_wheel(
        &"uv-build".parse()?,
        &uv_version::version().parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    context
        .temp_dir
        .child("wheels")
        .child(filename)
        .write_binary(&wheel)?;

    let build = || {
        let mut command = context.build();
        command
            .arg(&built_by_uv)
            .arg("--out-dir")
            .arg(context.temp_dir.join("dist"))
            .args([
                "--no-index",
                "--find-links=wheels",
                "--no-cache",
                "--preview-features=build-dependency-hashes",
                "--require-build-hashes",
            ]);
        command
    };

    uv_snapshot!(context.filters(), build(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building source distribution...
    Building wheel from source distribution...
    Successfully built dist/built_by_uv-0.1.0.tar.gz
    Successfully built dist/built_by_uv-0.1.0-py3-none-any.whl
    ");

    // Installing the backend instead of using the bundled version still requires hashes.
    uv_snapshot!(context.filters(), build().arg("--force-pep517"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Building source distribution...
    error: Failed to build `[WORKSPACE]/test/packages/built-by-uv`
      cause: Failed to resolve requirements from `build-system.requires`
      cause: No solution found when resolving: `uv-build>=0.8.0, <0.14`
      cause: Hash-checking is required, but no trusted hash was provided for: `uv-build`
    ");

    Ok(())
}

/// Test the `--list` option.
#[test]
fn list_files() {
    let context = uv_test::test_context!("3.12");

    let built_by_uv = context.workspace_root.join("test/packages/built-by-uv");

    // By default, we build the wheel from the source dist, which we need to do even for the list
    // task.
    uv_snapshot!(context.build()
        .arg(&built_by_uv)
        .arg("--out-dir")
        .arg(context.temp_dir.join("output1"))
        .arg("--list"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Building built_by_uv-0.1.0.tar.gz will include the following files:
    built_by_uv-0.1.0/PKG-INFO (generated)
    built_by_uv-0.1.0/pyproject.toml (generated)
    built_by_uv-0.1.0/pyproject.toml.orig (pyproject.toml)
    built_by_uv-0.1.0/LICENSE-APACHE (LICENSE-APACHE)
    built_by_uv-0.1.0/LICENSE-MIT (LICENSE-MIT)
    built_by_uv-0.1.0/README.md (README.md)
    built_by_uv-0.1.0/assets/data.csv (assets/data.csv)
    built_by_uv-0.1.0/header/built_by_uv.h (header/built_by_uv.h)
    built_by_uv-0.1.0/scripts/whoami.sh (scripts/whoami.sh)
    built_by_uv-0.1.0/src/built_by_uv/__init__.py (src/built_by_uv/__init__.py)
    built_by_uv-0.1.0/src/built_by_uv/arithmetic/__init__.py (src/built_by_uv/arithmetic/__init__.py)
    built_by_uv-0.1.0/src/built_by_uv/arithmetic/circle.py (src/built_by_uv/arithmetic/circle.py)
    built_by_uv-0.1.0/src/built_by_uv/arithmetic/pi.txt (src/built_by_uv/arithmetic/pi.txt)
    built_by_uv-0.1.0/src/built_by_uv/build-only.h (src/built_by_uv/build-only.h)
    built_by_uv-0.1.0/src/built_by_uv/cli.py (src/built_by_uv/cli.py)
    built_by_uv-0.1.0/third-party-licenses/PEP-401.txt (third-party-licenses/PEP-401.txt)
    Building built_by_uv-0.1.0-py3-none-any.whl will include the following files:
    built_by_uv/__init__.py (src/built_by_uv/__init__.py)
    built_by_uv/arithmetic/__init__.py (src/built_by_uv/arithmetic/__init__.py)
    built_by_uv/arithmetic/circle.py (src/built_by_uv/arithmetic/circle.py)
    built_by_uv/arithmetic/pi.txt (src/built_by_uv/arithmetic/pi.txt)
    built_by_uv/cli.py (src/built_by_uv/cli.py)
    built_by_uv-0.1.0.dist-info/licenses/LICENSE-APACHE (LICENSE-APACHE)
    built_by_uv-0.1.0.dist-info/licenses/LICENSE-MIT (LICENSE-MIT)
    built_by_uv-0.1.0.dist-info/licenses/third-party-licenses/PEP-401.txt (third-party-licenses/PEP-401.txt)
    built_by_uv-0.1.0.data/headers/built_by_uv.h (header/built_by_uv.h)
    built_by_uv-0.1.0.data/scripts/whoami.sh (scripts/whoami.sh)
    built_by_uv-0.1.0.data/data/data.csv (assets/data.csv)
    built_by_uv-0.1.0.dist-info/WHEEL (generated)
    built_by_uv-0.1.0.dist-info/entry_points.txt (generated)
    built_by_uv-0.1.0.dist-info/METADATA (generated)

    ----- stderr -----
    Building source distribution...
    Successfully built output1/built_by_uv-0.1.0.tar.gz
    ");
    context
        .temp_dir
        .child("output1")
        .child("built_by_uv-0.1.0.tar.gz")
        .assert(predicate::path::is_file());
    context
        .temp_dir
        .child("output1")
        .child("built_by_uv-0.1.0-py3-none-any.whl")
        .assert(predicate::path::missing());

    uv_snapshot!(context.build()
        .arg(&built_by_uv)
        .arg("--out-dir")
        .arg(context.temp_dir.join("output2"))
        .arg("--list")
        .arg("--sdist")
        .arg("--wheel"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Building built_by_uv-0.1.0.tar.gz will include the following files:
    built_by_uv-0.1.0/PKG-INFO (generated)
    built_by_uv-0.1.0/pyproject.toml (generated)
    built_by_uv-0.1.0/pyproject.toml.orig (pyproject.toml)
    built_by_uv-0.1.0/LICENSE-APACHE (LICENSE-APACHE)
    built_by_uv-0.1.0/LICENSE-MIT (LICENSE-MIT)
    built_by_uv-0.1.0/README.md (README.md)
    built_by_uv-0.1.0/assets/data.csv (assets/data.csv)
    built_by_uv-0.1.0/header/built_by_uv.h (header/built_by_uv.h)
    built_by_uv-0.1.0/scripts/whoami.sh (scripts/whoami.sh)
    built_by_uv-0.1.0/src/built_by_uv/__init__.py (src/built_by_uv/__init__.py)
    built_by_uv-0.1.0/src/built_by_uv/arithmetic/__init__.py (src/built_by_uv/arithmetic/__init__.py)
    built_by_uv-0.1.0/src/built_by_uv/arithmetic/circle.py (src/built_by_uv/arithmetic/circle.py)
    built_by_uv-0.1.0/src/built_by_uv/arithmetic/pi.txt (src/built_by_uv/arithmetic/pi.txt)
    built_by_uv-0.1.0/src/built_by_uv/build-only.h (src/built_by_uv/build-only.h)
    built_by_uv-0.1.0/src/built_by_uv/cli.py (src/built_by_uv/cli.py)
    built_by_uv-0.1.0/third-party-licenses/PEP-401.txt (third-party-licenses/PEP-401.txt)
    Building built_by_uv-0.1.0-py3-none-any.whl will include the following files:
    built_by_uv/__init__.py (src/built_by_uv/__init__.py)
    built_by_uv/arithmetic/__init__.py (src/built_by_uv/arithmetic/__init__.py)
    built_by_uv/arithmetic/circle.py (src/built_by_uv/arithmetic/circle.py)
    built_by_uv/arithmetic/pi.txt (src/built_by_uv/arithmetic/pi.txt)
    built_by_uv/cli.py (src/built_by_uv/cli.py)
    built_by_uv-0.1.0.dist-info/licenses/LICENSE-APACHE (LICENSE-APACHE)
    built_by_uv-0.1.0.dist-info/licenses/LICENSE-MIT (LICENSE-MIT)
    built_by_uv-0.1.0.dist-info/licenses/third-party-licenses/PEP-401.txt (third-party-licenses/PEP-401.txt)
    built_by_uv-0.1.0.data/headers/built_by_uv.h (header/built_by_uv.h)
    built_by_uv-0.1.0.data/scripts/whoami.sh (scripts/whoami.sh)
    built_by_uv-0.1.0.data/data/data.csv (assets/data.csv)
    built_by_uv-0.1.0.dist-info/WHEEL (generated)
    built_by_uv-0.1.0.dist-info/entry_points.txt (generated)
    built_by_uv-0.1.0.dist-info/METADATA (generated)
    ");
    context
        .temp_dir
        .child("output2")
        .child("built_by_uv-0.1.0.tar.gz")
        .assert(predicate::path::missing());
    context
        .temp_dir
        .child("output2")
        .child("built_by_uv-0.1.0-py3-none-any.whl")
        .assert(predicate::path::missing());
}

/// Test `--list` option errors.
#[test]
fn list_files_errors() {
    let context = uv_test::test_context!("3.12");

    let built_by_uv = context.workspace_root.join("test/packages/built-by-uv");

    uv_snapshot!(context.filters(), context.build()
        .arg(&built_by_uv)
        .arg("--out-dir")
        .arg(context.temp_dir.join("output1"))
        .arg("--list")
        .arg("--force-pep517"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--list' cannot be used with '--force-pep517'

    Usage: uv build --cache-dir [CACHE_DIR] --out-dir <OUT_DIR> --exclude-newer <EXCLUDE_NEWER> <SRC>

    For more information, try '--help'.
    ");

    // Not a uv build backend package, we can't list it.
    let anyio_local = context.workspace_root.join("test/packages/anyio_local");
    uv_snapshot!(context.filters(), context.build()
        .arg(&anyio_local)
        .arg("--out-dir")
        .arg(context.temp_dir.join("output2"))
        .arg("--list"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to build `[WORKSPACE]/test/packages/anyio_local`
      cause: Can only use `--list` with a compatible uv build backend, but `[WORKSPACE]/test/packages/anyio_local` is not compatible because `build_system.build-backend` is not `uv_build`, but `flit_core.buildapi`
    ");
}
