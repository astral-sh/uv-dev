//! Builds and file-listing checks for the bundled build backend.

use std::collections::BTreeMap;

use anyhow::Result;
use assert_fs::assert::PathAssert;
use assert_fs::fixture::{FileWriteBin, PathChild};
use predicates::prelude::predicate;
use uv_test::packse::generate_wheel;
use uv_test::{TestContext, assert_path_missing, uv_snapshot};

/// The default plan builds a wheel from the source distribution.
#[test]
fn build_default_plan() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    let source = context.workspace_root.join("test/packages/built-by-uv");
    let output = context.temp_dir.child("dist");

    uv_snapshot!(context.build()
        .arg(&source)
        .arg("--out-dir")
        .arg(output.path())
        .arg("--no-index"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building source distribution...
    Building wheel from source distribution...
    Successfully built dist/built_by_uv-0.1.0.tar.gz
    Successfully built dist/built_by_uv-0.1.0-py3-none-any.whl
    ");
    output
        .child("built_by_uv-0.1.0.tar.gz")
        .assert(predicate::path::is_file());
    output
        .child("built_by_uv-0.1.0-py3-none-any.whl")
        .assert(predicate::path::is_file());
}

#[test]
fn build_sdist_plan() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    let source = context.workspace_root.join("test/packages/built-by-uv");
    let output = context.temp_dir.child("dist");

    uv_snapshot!(context.build()
        .arg(&source)
        .arg("--out-dir")
        .arg(output.path())
        .arg("--sdist")
        .arg("--no-index"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building source distribution...
    Successfully built dist/built_by_uv-0.1.0.tar.gz
    ");
    output
        .child("built_by_uv-0.1.0.tar.gz")
        .assert(predicate::path::is_file());
    assert_path_missing(output.child("built_by_uv-0.1.0-py3-none-any.whl"));
}

#[test]
fn build_wheel_plan() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    let source = context.workspace_root.join("test/packages/built-by-uv");
    let output = context.temp_dir.child("dist");

    uv_snapshot!(context.build()
        .arg(&source)
        .arg("--out-dir")
        .arg(output.path())
        .arg("--wheel")
        .arg("--no-index"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building wheel...
    Successfully built dist/built_by_uv-0.1.0-py3-none-any.whl
    ");
    output
        .child("built_by_uv-0.1.0-py3-none-any.whl")
        .assert(predicate::path::is_file());
    assert_path_missing(output.child("built_by_uv-0.1.0.tar.gz"));
}

/// Explicitly requesting both artifacts builds each directly from the source tree.
#[test]
fn build_sdist_and_wheel_plan() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    let source = context.workspace_root.join("test/packages/built-by-uv");
    let output = context.temp_dir.child("dist");

    uv_snapshot!(context.build()
        .arg(&source)
        .arg("--out-dir")
        .arg(output.path())
        .arg("--sdist")
        .arg("--wheel")
        .arg("--no-index"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building source distribution...
    Building wheel...
    Successfully built dist/built_by_uv-0.1.0.tar.gz
    Successfully built dist/built_by_uv-0.1.0-py3-none-any.whl
    ");
    output
        .child("built_by_uv-0.1.0.tar.gz")
        .assert(predicate::path::is_file());
    output
        .child("built_by_uv-0.1.0-py3-none-any.whl")
        .assert(predicate::path::is_file());
}

fn write_unhashed_backend_wheel(context: &TestContext) -> Result<()> {
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
    Ok(())
}

#[test]
fn require_build_hashes_bundled_backend() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    let source = context.workspace_root.join("test/packages/built-by-uv");
    let output = context.temp_dir.child("dist");
    write_unhashed_backend_wheel(&context)?;

    uv_snapshot!(context.filters(), context.build()
        .arg(&source)
        .arg("--out-dir")
        .arg(output.path())
        .args([
            "--no-index",
            "--find-links=wheels",
            "--no-cache",
            "--preview-features=build-dependency-hashes",
            "--require-build-hashes",
        ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building source distribution...
    Building wheel from source distribution...
    Successfully built dist/built_by_uv-0.1.0.tar.gz
    Successfully built dist/built_by_uv-0.1.0-py3-none-any.whl
    ");
    output
        .child("built_by_uv-0.1.0.tar.gz")
        .assert(predicate::path::is_file());
    output
        .child("built_by_uv-0.1.0-py3-none-any.whl")
        .assert(predicate::path::is_file());
    Ok(())
}

#[test]
fn require_build_hashes_forced_pep517() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    let source = context.workspace_root.join("test/packages/built-by-uv");
    let output = context.temp_dir.child("dist");
    write_unhashed_backend_wheel(&context)?;

    uv_snapshot!(context.filters(), context.build()
        .arg(&source)
        .arg("--out-dir")
        .arg(output.path())
        .args([
            "--no-index",
            "--find-links=wheels",
            "--no-cache",
            "--preview-features=build-dependency-hashes",
            "--require-build-hashes",
            "--force-pep517",
        ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Building source distribution...
    error: Failed to build `[WORKSPACE]/test/packages/built-by-uv`
      cause: Failed to resolve requirements from `build-system.requires`
      cause: No solution found when resolving: `uv-build>=0.8.0, <0.14`
      cause: Hash-checking is required, but no trusted hash was provided for: `uv-build`
    ");
    assert_path_missing(output.child("built_by_uv-0.1.0.tar.gz"));
    assert_path_missing(output.child("built_by_uv-0.1.0-py3-none-any.whl"));
    Ok(())
}

/// Listing a wheel built from an sdist requires producing the sdist first.
#[test]
fn list_default_plan() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    let built_by_uv = context.workspace_root.join("test/packages/built-by-uv");
    let output = context.temp_dir.child("dist");

    uv_snapshot!(context.build()
        .arg(&built_by_uv)
        .arg("--out-dir")
        .arg(output.path())
        .arg("--no-index")
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
    Successfully built dist/built_by_uv-0.1.0.tar.gz
    ");
    output
        .child("built_by_uv-0.1.0.tar.gz")
        .assert(predicate::path::is_file());
    assert_path_missing(output.child("built_by_uv-0.1.0-py3-none-any.whl"));
}

/// Listing artifacts built directly from the source tree produces neither artifact.
#[test]
fn list_sdist_and_wheel_plan() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    let built_by_uv = context.workspace_root.join("test/packages/built-by-uv");
    let output = context.temp_dir.child("dist");
    uv_snapshot!(context.build()
        .arg(&built_by_uv)
        .arg("--out-dir")
        .arg(output.path())
        .arg("--no-index")
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
    assert_path_missing(output.child("built_by_uv-0.1.0.tar.gz"));
    assert_path_missing(output.child("built_by_uv-0.1.0-py3-none-any.whl"));
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
