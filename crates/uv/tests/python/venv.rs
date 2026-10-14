use std::path::{Path, PathBuf};

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use indoc::indoc;
use predicates::prelude::*;
use uv_cache_key::cache_digest;
use uv_fs::{LockedFile, LockedFileMode, verbatim_path};
use uv_python_discovery::{PYTHON_VERSION_FILENAME, PYTHON_VERSIONS_FILENAME};
use uv_static::EnvVars;

#[cfg(unix)]
use fs_err::os::unix::fs::symlink;
#[cfg(unix)]
use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
#[cfg(windows)]
use std::{ffi::OsString, os::windows::ffi::OsStringExt};

use uv_test::{assert_link_target, assert_path_missing, site_packages_path, uv_snapshot};

#[track_caller]
fn assert_no_distutils_patch(site_packages: &Path) {
    assert!(
        site_packages.is_dir(),
        "missing site-packages directory: {}",
        site_packages.display()
    );
    assert_path_missing(site_packages.join("_virtualenv.py"));
    assert_path_missing(site_packages.join("_virtualenv.pth"));
}

#[test]
fn create_venv() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    // Create a virtual environment at `.venv`.
    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    context.venv.assert(predicates::path::is_dir());
    let marker = context.venv.child("original-environment.txt");
    marker.write_str("original environment\n")?;

    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    error: Failed to create virtual environment
      cause: A virtual environment already exists at: .venv

    hint: Use the `--clear` flag or set `UV_VENV_CLEAR=1` to replace the existing virtual environment
    "
    );
    marker.assert("original environment\n");

    // Create a virtual environment at the same location using `--clear`,
    // which should replace it.
    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--clear")
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    context.venv.assert(predicates::path::is_dir());
    assert_path_missing(marker);
    Ok(())
}

#[test]
#[cfg(unix)]
fn create_venv_ksh() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    // OpenBSD ksh requires `.` to source the activation script.
    // See astral-sh/uv#22537.
    uv_snapshot!(context.filters(), context.venv()
        .arg("--python")
        .arg("3.12")
        .env(EnvVars::SHELL, "/bin/ksh"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: . .venv/bin/activate
    ");

    context.venv.assert(predicates::path::is_dir());
}

#[test]
fn reset_fixture_venv_ignores_project_configuration() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let marker = context.venv.child("before-reset");
    marker.touch()?;
    context.temp_dir.child("uv.toml").write_str("invalid = [")?;

    context.reset_venv();

    assert_path_missing(marker);
    Ok(())
}

#[test]
fn create_venv_skips_distutils_patch_on_py310() {
    let context = uv_test::test_context_with_versions!(&["3.10"]);

    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.10"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.10.[X] interpreter at: [PYTHON-3.10]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    assert_no_distutils_patch(&site_packages_path(context.venv.path(), "python3.10"));
}

#[test]
#[cfg(feature = "test-python-eol")]
fn create_venv_keeps_distutils_patch_on_py39() {
    let context = uv_test::test_context_with_versions!(&["3.9"]);

    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.9"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.9.[X] interpreter at: [PYTHON-3.9]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    context.venv.assert(predicates::path::is_dir());
    let site_packages = site_packages_path(context.venv.path(), "python3.9");
    assert!(site_packages.join("_virtualenv.py").is_file());
    assert!(site_packages.join("_virtualenv.pth").is_file());
}

#[test]
fn create_venv_with_removed_distutils_patch_preview() {
    let context = uv_test::test_context_with_versions!(&["3.10"]);

    uv_snapshot!(context.filters(), context.venv()
        .arg("--python")
        .arg("3.10")
        .arg("--preview-features")
        .arg("no-distutils-patch"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Unknown preview feature: `no-distutils-patch`
    Using CPython 3.10.[X] interpreter at: [PYTHON-3.10]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    ");

    assert_no_distutils_patch(&site_packages_path(context.venv.path(), "python3.10"));
}

#[test]
fn create_venv_preserves_existing_distutils_patch() -> Result<()> {
    let context = uv_test::test_context!("3.10");
    let site_packages = site_packages_path(context.venv.path(), "python3.10");
    fs_err::write(site_packages.join("_virtualenv.py"), "# existing patch\n")?;
    fs_err::write(
        site_packages.join("_virtualenv.pth"),
        "import _virtualenv\n",
    )?;

    uv_snapshot!(context.filters(), context.venv()
        .arg("--allow-existing")
        .arg("--python")
        .arg("3.10"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.10.[X] interpreter at: [PYTHON-3.10]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    ");

    assert_eq!(
        context.read(site_packages.join("_virtualenv.py")),
        "# existing patch\n"
    );
    assert_eq!(
        context.read(site_packages.join("_virtualenv.pth")),
        "import _virtualenv\n"
    );

    context
        .venv()
        .arg("--clear")
        .arg("--python")
        .arg("3.10")
        .assert()
        .success();
    assert_no_distutils_patch(&site_packages);
    Ok(())
}

#[test]
fn create_centralized_project_environment_bypasses() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    // Centralized environments are only enabled for projects.
    uv_snapshot!(context.filters(), context.venv()
        .arg("--preview-features")
        .arg("centralized-project-envs"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );
    context.venv.assert(predicates::path::is_dir());

    fs_err::remove_dir_all(&context.venv)?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
    "#})?;

    // Explicit project environment paths are not centralized.
    uv_snapshot!(context.filters(), context.venv()
        .env(EnvVars::UV_PROJECT_ENVIRONMENT, "explicit")
        .arg("--preview-features")
        .arg("centralized-project-envs"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: explicit
    Activate with: source explicit/[BIN]/activate
    "
    );
    let explicit = context.temp_dir.child("explicit");
    explicit.assert(predicates::path::is_dir());

    // Pathless invocations outside the project root are not centralized.
    let child = context.temp_dir.child("child");
    child.create_dir_all()?;
    context
        .venv()
        .current_dir(child.path())
        .arg("--preview-features")
        .arg("centralized-project-envs")
        .assert()
        .success();
    let environment = child.child(".venv");
    environment.assert(predicates::path::is_dir());
    Ok(())
}

#[test]
fn create_venv_313() {
    let context = uv_test::test_context_with_versions!(&["3.13"]);

    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.13"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    context.venv.assert(predicates::path::is_dir());
}

#[test]
fn create_venv_project_environment() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"])
        .with_env(EnvVars::UV_PROJECT_ENVIRONMENT, "foo");

    // `uv venv` ignores `UV_PROJECT_ENVIRONMENT` when it's not a project
    uv_snapshot!(context.filters(), context.venv(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    context.venv.assert(predicates::path::is_dir());
    context
        .temp_dir
        .child("foo")
        .assert(predicates::path::missing());

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
            [project]
            name = "project"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = ["iniconfig"]
            "#,
    )?;

    // But, if we're in a project we'll respect it
    uv_snapshot!(context.filters(), context.venv(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: foo
    Activate with: source foo/[BIN]/activate
    "
    );

    context
        .temp_dir
        .child("foo")
        .assert(predicates::path::is_dir());

    // Unless we're in a child directory
    let child = context.temp_dir.child("child");
    child.create_dir_all()?;

    uv_snapshot!(context.filters(), context.venv().current_dir(child.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    // In which case, we'll use the default name of `.venv`
    child.child("foo").assert(predicates::path::missing());
    child.child(".venv").assert(predicates::path::is_dir());

    // Or, if a name is provided
    uv_snapshot!(context.filters(), context.venv().arg("bar"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: bar
    Activate with: source bar/[BIN]/activate
    "
    );

    context
        .temp_dir
        .child("bar")
        .assert(predicates::path::is_dir());

    // Or, if project discovery is disabled with `--no-workspace` or `--no-project`.
    uv_snapshot!(context.filters(), context.venv().arg("--clear").arg("--no-workspace"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    uv_snapshot!(context.filters(), context.venv().arg("--clear").arg("--no-project"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    Ok(())
}

/// Hold a project's environment lock in a temporary directory owned by the test.
async fn locked_project_environment() -> Result<(uv_test::TestContext, LockedFile)> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    context.temp_dir.child("pyproject.toml").write_str(
        r#"
            [project]
            name = "project"
            version = "0.1.0"
            requires-python = ">=3.12"
            "#,
    )?;

    let system_temp = context.root.child("system-temp");
    system_temp.create_dir_all()?;
    let install_path = dunce::canonicalize(context.temp_dir.path())?;
    let lock_path = system_temp.join(format!("uv-{}.lock", cache_digest(&install_path)));
    let lock = LockedFile::acquire(
        &lock_path,
        LockedFileMode::Exclusive,
        context.temp_dir.path().display(),
    )
    .await?;
    let context = context
        .with_env(
            if cfg!(windows) { "TMP" } else { "TMPDIR" },
            system_temp.as_os_str(),
        )
        .with_filtered_path(&lock_path, "PROJECT_ENVIRONMENT_LOCK");
    Ok((context, lock))
}

#[tokio::test]
async fn create_venv_project_environment_lock() -> Result<()> {
    let (context, lock) = locked_project_environment().await?;

    // A pathless invocation from the project root uses the workspace-derived lock, including when
    // `UV_PROJECT_ENVIRONMENT` changes the environment path. Lock errors warn and continue.
    uv_snapshot!(context.filters(), context.venv()
        .env(EnvVars::UV_PROJECT_ENVIRONMENT, "foo")
        .env(EnvVars::RUST_LOG, "warn")
        .env(EnvVars::UV_LOCK_TIMEOUT, "1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: foo
    WARN Failed to acquire project environment lock: Timeout ([TIME]) when waiting for lock on `[TEMP_DIR]/` at `[PROJECT_ENVIRONMENT_LOCK]/`, is another uv process running? You can set `UV_LOCK_TIMEOUT` to increase the timeout.
    Activate with: source foo/[BIN]/activate
    ");
    drop(lock);

    context
        .temp_dir
        .child("foo")
        .assert(predicates::path::is_dir());

    Ok(())
}

/// Project aliases share the workspace lock even before the environment exists.
#[tokio::test]
async fn create_venv_project_environment_lock_symlink() -> Result<()> {
    let (context, lock) = locked_project_environment().await?;
    let project_link = context.temp_dir.child("project-link");
    uv_fs::create_symlink(context.temp_dir.path(), project_link.path())?;

    // A project alias uses the same lock, including when `UV_PROJECT_ENVIRONMENT` changes the
    // environment path. Lock errors warn and continue.
    uv_snapshot!(context.filters(), context.venv()
        .arg("--project")
        .arg(project_link.path())
        .env(EnvVars::UV_PROJECT_ENVIRONMENT, "foo")
        .env(EnvVars::RUST_LOG, "warn")
        .env(EnvVars::UV_LOCK_TIMEOUT, "1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: project-link/foo
    WARN Failed to acquire project environment lock: Timeout ([TIME]) when waiting for lock on `[TEMP_DIR]/project-link` at `[PROJECT_ENVIRONMENT_LOCK]/`, is another uv process running? You can set `UV_LOCK_TIMEOUT` to increase the timeout.
    Activate with: source project-link/foo/[BIN]/activate
    ");
    drop(lock);

    context
        .temp_dir
        .child("foo")
        .assert(predicates::path::is_dir());

    Ok(())
}

#[test]
fn create_centralized_project_environment() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"])
        .with_filtered_centralized_environment_hashes();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Explicit paths remain local; expose one through a directory link.
    let external = context.temp_dir.child("external");
    context
        .venv()
        .arg(external.path())
        .arg("--preview-features")
        .arg("centralized-project-envs")
        .assert()
        .success();
    external.child("marker").touch()?;
    uv_fs::create_symlink(external.path(), context.temp_dir.child(".venv").path())?;

    // Migrate to centralized storage without removing the linked environment.
    uv_snapshot!(context.filters(), context.venv()
        .arg("--preview-features")
        .arg("centralized-project-envs"), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment `project-cp3.12.[X]-[HASH]`
    Activate with: source .venv/[BIN]/activate
    "#);

    let target = fs_err::read_link(context.temp_dir.child(".venv").path())?;
    let cache_root = context.cache_dir.child("environments-v2");
    assert_eq!(
        target.parent().map(verbatim_path),
        Some(verbatim_path(cache_root.path()))
    );
    assert!(target.join("pyvenv.cfg").is_file());
    assert!(external.child("marker").is_file());

    let marker = context.temp_dir.child(".venv").child("marker");
    marker.touch()?;

    // `--allow-existing` preserves the contents of the centralized environment.
    context
        .venv()
        .arg("--allow-existing")
        .arg("--preview-features")
        .arg("centralized-project-envs")
        .assert()
        .success();
    assert_link_target(context.temp_dir.child(".venv"), &target);
    assert!(marker.exists());

    // Preserve an existing centralized environment with `--no-clear`.
    uv_snapshot!(context.filters(), context.venv()
        .arg("--no-clear")
        .arg("--preview-features")
        .arg("centralized-project-envs"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment `project-cp3.12.[X]-[HASH]`
    error: Failed to create virtual environment
      cause: A virtual environment already exists at: [CACHE_DIR]/environments-v2/project-cp3.12.[X]-[HASH]

    hint: Use the `--clear` flag or set `UV_VENV_CLEAR=1` to replace the existing virtual environment
    "#);

    assert_link_target(context.temp_dir.child(".venv"), &target);
    assert!(marker.exists());

    let environment = context.temp_dir.child(".venv");
    let cache_marker = target.join("marker");
    assert!(cache_marker.is_file());

    // Without the preview, `--allow-existing` operates on the environment through the link.
    context.venv().arg("--allow-existing").assert().success();
    assert_link_target(&environment, &target);
    assert!(cache_marker.is_file());

    // Without the preview, `.venv` is replaced locally without clearing its cached target.
    context.venv().assert().success();

    environment.assert(predicates::path::is_dir());
    assert!(cache_marker.is_file());
    let local_marker = environment.child("local-marker");
    local_marker.touch()?;

    // With the preview, `--allow-existing` operates on the cached environment.
    context
        .venv()
        .arg("--allow-existing")
        .arg("--preview-features")
        .arg("centralized-project-envs")
        .assert()
        .success();

    assert_link_target(&environment, &target);
    assert_path_missing(&local_marker);
    assert!(cache_marker.is_file());

    // A plain invocation recreates the centralized environment without prompting.
    uv_snapshot!(context.filters(), context.venv()
        .arg("--preview-features")
        .arg("centralized-project-envs"), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment `project-cp3.12.[X]-[HASH]`
    Activate with: source .venv/[BIN]/activate
    "#);

    assert_link_target(&environment, &target);
    assert_path_missing(&cache_marker);
    Ok(())
}

#[test]
fn create_centralized_project_environment_path_file() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_filtered_centralized_environment_hashes();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = []
    "#})?;

    context
        .venv()
        .arg("--preview-features")
        .arg("centralized-project-envs")
        .arg("--python")
        .arg("3.11")
        .assert()
        .success();

    let environment = context.temp_dir.child(".venv");
    let target = fs_err::read_link(environment.path())?;
    let marker = target.join("marker");
    fs_err::write(&marker, "")?;

    uv_fs::remove_virtualenv(environment.path())?;
    environment.write_str(&target.to_string_lossy())?;

    // With the preview, `--allow-existing` selects the root for the requested interpreter.
    uv_snapshot!(context.filters(), context.venv()
        .arg("--preview-features")
        .arg("centralized-project-envs")
        .arg("--allow-existing")
        .arg("--python")
        .arg("3.12"), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment `project-cp3.12.[X]-[HASH]`
    Activate with: source .venv/[BIN]/activate
    "#);
    assert_ne!(target, fs_err::read_link(environment.path())?);
    assert!(marker.is_file());

    uv_fs::remove_virtualenv(environment.path())?;
    environment.write_str(&target.to_string_lossy())?;

    // Without the preview, `--allow-existing` replaces the path file without clearing its target.
    context.venv().arg("--allow-existing").assert().success();
    assert!(uv_fs::is_virtualenv_base(environment.path()));
    assert!(marker.is_file());
    Ok(())
}

#[test]
fn create_centralized_project_environment_link_failure() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"])
        .with_filtered_centralized_environment_hashes();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;
    let environment = context.temp_dir.child(".venv");
    environment.create_dir_all()?;
    environment.child("keep").touch()?;

    let mut command = context.venv();
    command
        .arg("--preview-features")
        .arg("centralized-project-envs");
    cfg_select! {
        unix => {
            uv_snapshot!(context.filters(), command, @r#"
            exit_code: 0 (success)
            ----- stderr -----
            Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
            Creating virtual environment `project-cp3.12.[X]-[HASH]`
            warning: Failed to write the environment path: failed to rename file from [TEMP_DIR]/[TMP] to [VENV]/: Is a directory (os error 21)
            Activate with: source [CACHE_DIR]/environments-v2/project-cp3.12.[X]-[HASH]/[BIN]/activate
            "#);
        },
        windows => {
            uv_snapshot!(context.filters(), command, @r#"
            exit_code: 0 (success)
            ----- stderr -----
            Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
            Creating virtual environment `project-cp3.12.[X]-[HASH]`
            warning: Failed to create link to project environment: failed to remove directory `[VENV]/`: The directory is not empty. (os error 145)
            Activate with: source [CACHE_DIR]/environments-v2/project-cp3.12.[X]-[HASH]/[BIN]/activate
            "#);
        },
    }

    assert!(environment.child("keep").is_file());
    // The activation hint must lead to a usable environment despite the project link failure.
    let cache = uv_cache::Cache::from_path(context.cache_dir.to_path_buf());
    let cached = fs_err::read_dir(cache.bucket(uv_cache::CacheBucket::Environments))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    let [fallback] = cached.as_slice() else {
        panic!("expected one fallback environment, found {cached:?}");
    };
    context
        .external_command(
            uv_test::venv_bin_path(fallback).join(format!("python{}", std::env::consts::EXE_SUFFIX)),
        )
        .args([
            "-I",
            "-c",
            "import sys; assert sys.prefix != sys.base_prefix; print(f'{sys.version_info.major}.{sys.version_info.minor}', end='')",
        ])
        .assert()
        .success()
        .stdout("3.12");
    Ok(())
}

#[test]
fn create_centralized_project_environment_no_cache() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.venv()
        .arg("--no-cache")
        .arg("--preview-features")
        .arg("centralized-project-envs"), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    warning: The `centralized-project-envs` feature has no effect when `--no-cache` is enabled
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "#);

    context
        .temp_dir
        .child(".venv")
        .assert(predicates::path::is_dir());
    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn create_centralized_project_environment_with_seed_packages() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"])
        .with_filtered_centralized_environment_hashes();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.venv()
        .arg("--seed")
        .arg("--preview-features")
        .arg("centralized-project-envs"), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment `project-cp3.12.[X]-[HASH]` with seed packages
     + pip==24.0
    Activate with: source .venv/[BIN]/activate
    "#);

    let target = fs_err::read_link(context.temp_dir.child(".venv").path())?;
    assert!(target.join("pyvenv.cfg").is_file());

    // Seed the existing environment without clearing its contents.
    let marker = target.join("marker");
    fs_err::write(&marker, "")?;
    context
        .venv()
        .arg("--seed")
        .arg("--allow-existing")
        .arg("--preview-features")
        .arg("centralized-project-envs")
        .assert()
        .success();

    assert_link_target(context.temp_dir.child(".venv"), &target);
    assert!(marker.is_file());
    Ok(())
}

#[test]
fn virtual_empty() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [tool.mycooltool]
        wow = "someconfig"
    "#})?;

    uv_snapshot!(context.filters(), context.venv(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    ");

    Ok(())
}

#[test]
fn virtual_dependency_group() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [dependency-groups]
        foo = ["sortedcontainers"]
        bar = ["iniconfig"]
        dev = ["sniffio"]
    "#})?;

    uv_snapshot!(context.filters(), context.venv(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    ");

    Ok(())
}

#[test]
fn create_venv_defaults_to_cwd() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    uv_snapshot!(context.filters(), context.venv()
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    context.venv.assert(predicates::path::is_dir());
}

#[test]
fn create_venv_ignores_virtual_env_variable() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    // A recognized environment without an interpreter would fail discovery if inspected.
    let active_venv = context.temp_dir.child("active");
    active_venv.create_dir_all()?;
    active_venv.child("pyvenv.cfg").touch()?;

    uv_snapshot!(context.filters(), context.venv()
        .env(EnvVars::VIRTUAL_ENV, active_venv.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    Ok(())
}

#[test]
fn create_venv_reads_request_from_python_version_file() {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);

    // Without the file, we should use the first on the PATH
    uv_snapshot!(context.filters(), context.venv(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    // With a version file, we should prefer that version
    context
        .temp_dir
        .child(PYTHON_VERSION_FILENAME)
        .write_str("3.12")
        .unwrap();

    uv_snapshot!(context.filters(), context.venv().arg("--clear"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    context.venv.assert(predicates::path::is_dir());
}

#[test]
fn create_venv_reads_request_from_python_versions_file() {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);

    // Without the file, we should use the first on the PATH
    uv_snapshot!(context.filters(), context.venv(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    // With a versions file, we should prefer the first listed version
    context
        .temp_dir
        .child(PYTHON_VERSIONS_FILENAME)
        .write_str(indoc! {"
            3.12
            3.11
        "})
        .unwrap();

    uv_snapshot!(context.filters(), context.venv().arg("--clear"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    context.venv.assert(predicates::path::is_dir());
}

#[test]
fn create_venv_respects_pyproject_requires_python() -> Result<()> {
    let mut context = uv_test::test_context_with_versions!(&["3.11", "3.9", "3.10", "3.12"]);

    // Without a Python requirement, we use the first on the PATH
    uv_snapshot!(context.filters(), context.venv(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    // With `requires-python = "<3.11"`, we prefer the first available version
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = "<3.11"
        dependencies = []
        "#
    })?;

    uv_snapshot!(context.filters(), context.venv().arg("--clear"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.9.[X] interpreter at: [PYTHON-3.9]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    // Put an incompatible interpreter first so the following requirements must filter it out.
    context.python_versions.swap(0, 1);

    // With `requires-python = "==3.11.*"`, we prefer exact version (3.11)
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = "==3.11.*"
        dependencies = []
        "#
    })?;

    uv_snapshot!(context.filters(), context.venv().arg("--clear"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    // With `requires-python = ">=3.11,<3.12"`, we prefer exact version (3.11)
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11,<3.12"
        dependencies = []
        "#
    })?;

    uv_snapshot!(context.filters(), context.venv().arg("--clear"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    // With `requires-python = ">=3.10"`, we prefer first compatible version (3.11)
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.10"
        dependencies = []
        "#
    })?;

    uv_snapshot!(context.filters(), context.venv().arg("--clear"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    // With `requires-python = ">=3.11"`, we prefer first compatible version (3.11)
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11"
        dependencies = []
        "#
    })?;

    uv_snapshot!(context.filters(), context.venv().arg("--clear"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    // With `requires-python = ">3.11"`, we prefer first compatible version (3.11)
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">3.11"
        dependencies = []
        "#
    })?;

    uv_snapshot!(context.filters(), context.venv().arg("--clear"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    // With `requires-python = ">=3.12"`, we prefer first compatible version (3.12)
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = []
        "#
    })?;

    uv_snapshot!(context.filters(), context.venv().arg("--clear"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    context.venv.assert(predicates::path::is_dir());

    // We warn if we receive an incompatible version
    uv_snapshot!(context.filters(), context.venv().arg("--clear").arg("--python").arg("3.11"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    warning: The requested interpreter resolved to Python 3.11.[X], which is incompatible with the project's Python requirement: `>=3.12` (from `project.requires-python`)
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    Ok(())
}

#[test]
fn create_venv_respects_group_requires_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.9", "3.10", "3.11", "3.12"]);

    // Without a Python requirement, we use the first on the PATH
    uv_snapshot!(context.filters(), context.venv(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.9.[X] interpreter at: [PYTHON-3.9]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    // With `requires-python = ">=3.10"` on the default group, we pick 3.10
    // However non-default groups should not be consulted!
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        dependencies = []

        [dependency-groups]
        dev = ["sortedcontainers"]
        other = ["sniffio"]

        [tool.uv.dependency-groups]
        dev = {requires-python = ">=3.10"}
        other = {requires-python = ">=3.12"}
        "#
    })?;

    uv_snapshot!(context.filters(), context.venv().arg("--clear"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.10.[X] interpreter at: [PYTHON-3.10]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    // When the top-level requires-python and default group requires-python
    // both apply, their intersection is used. However non-default groups
    // should not be consulted! (here the top-level wins)
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11"
        dependencies = []

        [dependency-groups]
        dev = ["sortedcontainers"]
        other = ["sniffio"]

        [tool.uv.dependency-groups]
        dev = {requires-python = ">=3.10"}
        other = {requires-python = ">=3.12"}
        "#
    })?;

    uv_snapshot!(context.filters(), context.venv().arg("--clear"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    // When the top-level requires-python and default group requires-python
    // both apply, their intersection is used. However non-default groups
    // should not be consulted! (here the group wins)
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.10"
        dependencies = []

        [dependency-groups]
        dev = ["sortedcontainers"]
        other = ["sniffio"]

        [tool.uv.dependency-groups]
        dev = {requires-python = ">=3.11"}
        other = {requires-python = ">=3.12"}
        "#
    })?;

    uv_snapshot!(context.filters(), context.venv().arg("--clear"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    // We warn if we receive an incompatible version
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        dependencies = []

        [dependency-groups]
        dev = ["sortedcontainers"]

        [tool.uv.dependency-groups]
        dev = {requires-python = ">=3.12"}
        "#
    })?;

    uv_snapshot!(context.filters(), context.venv().arg("--clear").arg("--python").arg("3.11"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    warning: The requested interpreter resolved to Python 3.11.[X], which is incompatible with the project's Python requirement: `>=3.12` (from `tool.uv.dependency-groups.dev.requires-python`).
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    // Conflicting project and default-group requirements fail before clearing the environment;
    // non-default groups do not participate in the conflict.
    let marker = context.venv.child("marker");
    marker.write_str("existing environment\n")?;
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = "<3.12"
        dependencies = []

        [dependency-groups]
        dev = ["sortedcontainers"]
        other = ["sniffio"]

        [tool.uv.dependency-groups]
        dev = {requires-python = ">=3.12"}
        other = {requires-python = ">=3.11"}
        "#
    })?;

    uv_snapshot!(context.filters(), context.venv().arg("--clear").arg("--python").arg("3.11"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Found conflicting Python requirements:
    - foo: <3.12
    - foo:dev: >=3.12
    "
    );
    assert_eq!(context.read(marker), "existing environment\n");

    Ok(())
}

#[test]
fn create_venv_ignores_missing_pyproject_metadata() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r"[tool.no.project.here]" })?;

    uv_snapshot!(context.filters(), context.venv(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    context.venv.assert(predicates::path::is_dir());

    Ok(())
}

#[test]
fn create_venv_warns_user_on_requires_python_discovery_error() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r"invalid toml" })?;

    uv_snapshot!(context.filters(), context.venv(), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Failed to parse `pyproject.toml` during settings discovery:
      TOML parse error at line 1, column 9
        |
      1 | invalid toml
        |         ^
      key with no value, expected `=`

    warning: Failed to parse `pyproject.toml` during environment creation:
      TOML parse error at line 1, column 9
        |
      1 | invalid toml
        |         ^
      key with no value, expected `=`

    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    context.venv.assert(predicates::path::is_dir());

    Ok(())
}

#[test]
fn create_venv_explicit_request_takes_priority_over_python_version_file() {
    // Both default discovery and the version file select 3.12, so only the explicit request can
    // select 3.11.
    let context = uv_test::test_context_with_versions!(&["3.12", "3.11"]);

    context
        .temp_dir
        .child(PYTHON_VERSION_FILENAME)
        .write_str("3.12")
        .unwrap();

    uv_snapshot!(context.filters(), context.venv().arg("--python").arg("3.11"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    context.venv.assert(predicates::path::is_dir());
}

#[cfg(feature = "test-pypi")]
fn assert_seed_packages(context: &uv_test::TestContext, expected: &[&str]) {
    context
        .external_command(context.interpreter())
        .args([
            "-I",
            "-B",
            "-c",
            "from importlib.metadata import distributions; print(','.join(sorted(dist.name.lower() for dist in distributions())), end='')",
        ])
        .assert()
        .success()
        .stdout(expected.join(","));
}

#[test]
#[cfg(feature = "test-pypi")]
fn seed() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--seed")
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment with seed packages at: .venv
     + pip==24.0
    Activate with: source .venv/[BIN]/activate
    "
    );

    assert_seed_packages(&context, &["pip"]);
    assert_no_distutils_patch(&site_packages_path(context.venv.path(), "python3.12"));
}

#[test]
#[cfg(feature = "test-pypi")]
fn seed_older_python_version() {
    let context = uv_test::test_context_with_versions!(&["3.11"]);
    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--seed")
        .arg("--python")
        .arg("3.11"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    Creating virtual environment with seed packages at: .venv
     + pip==24.0
     + setuptools==69.2.0
     + wheel==0.43.0
    Activate with: source .venv/[BIN]/activate
    "
    );

    assert_seed_packages(&context, &["pip", "setuptools", "wheel"]);
}

#[test]
#[cfg(feature = "test-pypi")]
fn seed_skips_distutils_patch_on_py310() {
    let context = uv_test::test_context_with_versions!(&["3.10"]);

    uv_snapshot!(context.filters(), context.venv()
        .arg("--seed")
        .arg("--python")
        .arg("3.10"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.10.[X] interpreter at: [PYTHON-3.10]
    Creating virtual environment with seed packages at: .venv
     + pip==24.0
     + setuptools==69.2.0
     + wheel==0.43.0
    Activate with: source .venv/[BIN]/activate
    ");

    assert_no_distutils_patch(&site_packages_path(context.venv.path(), "python3.10"));
}

#[test]
#[cfg(all(feature = "test-pypi", feature = "test-python-eol"))]
fn seed_keeps_distutils_patch_on_py39() {
    let context = uv_test::test_context_with_versions!(&["3.9"]);

    uv_snapshot!(context.filters(), context.venv()
        .arg("--seed")
        .arg("--python")
        .arg("3.9"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.9.[X] interpreter at: [PYTHON-3.9]
    Creating virtual environment with seed packages at: .venv
     + pip==24.0
     + setuptools==69.2.0
     + wheel==0.43.0
    Activate with: source .venv/[BIN]/activate
    ");

    let site_packages = site_packages_path(context.venv.path(), "python3.9");
    assert!(site_packages.join("_virtualenv.py").is_file());
    assert!(site_packages.join("_virtualenv.pth").is_file());
}

#[test]
fn create_venv_unknown_python_minor() {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_python_sources();

    let mut command = context.venv();
    command
        .arg(context.venv.as_os_str())
        // Request a version we know we'll never see
        .arg("--python")
        .arg("3.100")
        // Unset this variable to force what the user would see
        .env_remove(EnvVars::UV_PYTHON_SEARCH_PATH);

    uv_snapshot!(context.filters(), &mut command, @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.100 in [PYTHON SOURCES]
    "
    );

    context.venv.assert(predicates::path::missing());
}

#[test]
fn create_venv_unknown_python_patch() {
    let context = uv_test::test_context_with_versions!(&[]).with_filtered_python_sources();

    let mut command = context.venv();
    command
        .arg(context.venv.as_os_str())
        // Request a version we know we'll never see
        .arg("--python")
        .arg("3.12.100")
        // Unset this variable to force what the user would see
        .env_remove(EnvVars::UV_PYTHON_SEARCH_PATH);

    uv_snapshot!(context.filters(), &mut command, @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.12.100 in [PYTHON SOURCES]
    "
    );

    context.venv.assert(predicates::path::missing());
}

#[cfg(feature = "test-python-patch")]
#[test]
fn create_venv_python_patch() {
    let context = uv_test::test_context_with_versions!(&["3.12.15"]);

    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12.15"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.15 interpreter at: [PYTHON-3.12.15]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    context.venv.assert(predicates::path::is_dir());
}

#[test]
fn create_venv_preserves_existing_file() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    // Create a file at `.venv`. Creating a virtualenv at the same path should fail.
    context.venv.write_str("existing file\n")?;

    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    error: Failed to create virtual environment
      cause: File exists at `.venv`
    "
    );

    assert_eq!(context.read(&context.venv), "existing file\n");

    Ok(())
}

#[test]
fn non_utf8_path() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    let path = PathBuf::from(cfg_select! {
        unix => OsStr::from_bytes(b".venv-\xff"),
        windows => OsString::from_wide(&[0x002e, 0x0076, 0x0065, 0x006e, 0x0076, 0x002d, 0xd800]),
    });

    uv_snapshot!(context.filters(), context.venv()
        .arg(&path)
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv-�
    error: Failed to create virtual environment
      cause: Virtual environment path is not valid UTF-8: .venv-�
    "
    );

    assert_path_missing(context.temp_dir.join(path));
}

#[test]
fn empty_dir_exists() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    // Create an empty directory at `.venv`. Creating a virtualenv at the same path should succeed.
    context.venv.create_dir_all()?;
    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    context
        .venv
        .child("pyvenv.cfg")
        .assert(predicates::path::is_file());

    Ok(())
}

#[test]
fn non_empty_dir_exists() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    // Neither ordinary creation nor `--clear` may remove files from a non-venv directory.
    context.venv.create_dir_all()?;
    let existing_file = context.venv.child("file");
    existing_file.write_str("existing contents\n")?;

    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    error: Failed to create virtual environment
      cause: A directory already exists at: .venv

    hint: Use the `--clear` flag or set `UV_VENV_CLEAR=1` to replace the existing directory
    ");
    assert_eq!(context.read(&existing_file), "existing contents\n");

    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--clear")
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    error: Failed to create virtual environment
      cause: uv will not clear a directory that is not a virtual environment

    hint: Use the `--force` flag to remove the existing directory anyway
    "
    );

    assert_eq!(context.read(&existing_file), "existing contents\n");

    Ok(())
}

#[test]
fn non_empty_dir_exists_clear_force() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    let directory = context.temp_dir.child("not-a-virtualenv");
    directory.create_dir_all()?;
    directory.child("file").touch()?;

    uv_snapshot!(context.filters(), context.venv()
        .arg(directory.as_os_str())
        .arg("--clear")
        .arg("--force")
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: not-a-virtualenv
    Activate with: source not-a-virtualenv/[BIN]/activate
    ");

    assert_path_missing(directory.child("file"));
    directory
        .child("pyvenv.cfg")
        .assert(predicates::path::is_file());

    Ok(())
}

#[test]
fn non_empty_dir_exists_allow_existing() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    // Create a non-empty directory that is not yet a virtual environment.
    context.venv.create_dir_all()?;
    let existing_file = context.venv.child("file");
    existing_file.write_str("existing contents\n")?;

    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--allow-existing")
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );
    assert_eq!(context.read(&existing_file), "existing contents\n");

    // Running again should _also_ succeed, overwriting existing symlinks and respecting existing
    // directories.
    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--allow-existing")
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );
    assert_eq!(context.read(&existing_file), "existing contents\n");

    Ok(())
}

/// Run `uv venv` followed by `uv venv --allow-existing`.
#[test]
fn create_venv_then_allow_existing() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    // Create a venv
    uv_snapshot!(context.filters(), context.venv(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    // Create a venv again with `--allow-existing`
    uv_snapshot!(context.filters(), context.venv()
        .arg("--allow-existing"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );
}

#[test]
#[cfg(windows)]
fn windows_shims() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.10", "3.9"]);
    let shim_path = context.temp_dir.child("shim");

    // Route the shim to 3.9 while the ordinary search path prefers 3.10.
    let (version, python) = context
        .python_versions
        .last()
        .expect("Python 3.9 interpreter");
    assert_eq!(version.to_string(), "3.9");

    // Pass the executable through the environment so its path retains native encoding.
    let shim_python = "UV_TEST_SHIM_PYTHON";
    fs_err::create_dir(&shim_path)?;
    fs_err::write(
        shim_path.child("python.bat"),
        format!("@echo off\r\n\"%{shim_python}%\" %*\r\n"),
    )?;
    let search_path = std::env::join_paths(
        std::iter::once(shim_path.to_path_buf())
            .chain(std::env::split_paths(&context.python_path())),
    )?;

    // Create a virtual environment at `.venv` with the shim
    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .env(EnvVars::UV_PYTHON_SEARCH_PATH, search_path)
        .env(shim_python, python), @r###"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.9.[X] interpreter at: [PYTHON-3.9]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "###
    );

    context.venv.assert(predicates::path::is_dir());

    Ok(())
}

#[test]
fn verify_pyvenv_cfg() {
    let context = uv_test::test_context!("3.12");
    let pyvenv_cfg = context.venv.child("pyvenv.cfg");

    context.venv.assert(predicates::path::is_dir());

    // Check pyvenv.cfg exists
    pyvenv_cfg.assert(predicates::path::is_file());

    // Check if "uv = version" is present in the file
    let version = env!("CARGO_PKG_VERSION").to_string();
    let search_string = format!("uv = {version}");
    pyvenv_cfg.assert(predicates::str::contains(search_string));

    // Not relocatable by default.
    pyvenv_cfg.assert(predicates::str::contains("relocatable").not());
}

#[test]
fn verify_pyvenv_cfg_relocatable() {
    let context = uv_test::test_context!("3.12");
    let prompt = "résumé \"quoted\"\\path\n";
    let scripts = if cfg!(windows) {
        context.venv.child("Scripts")
    } else {
        context.venv.child("bin")
    };
    let activate_csh = scripts.child("activate.csh");
    activate_csh.assert(predicates::path::exists());

    // Recreate the environment as relocatable.
    context
        .venv()
        .arg(context.venv.as_os_str())
        .arg("--clear")
        .arg("--python")
        .arg("3.12")
        .arg("--prompt")
        .arg(prompt)
        .arg("--relocatable")
        .assert()
        .success();

    let pyvenv_cfg = context.venv.child("pyvenv.cfg");

    context.venv.assert(predicates::path::is_dir());

    // Check pyvenv.cfg exists
    pyvenv_cfg.assert(predicates::path::is_file());

    // Relocatable flag is set.
    pyvenv_cfg.assert(predicates::str::contains("relocatable = true"));

    // Activate scripts contain the relocatable boilerplate
    let activate_sh = scripts.child("activate");
    activate_sh.assert(predicates::path::is_file());
    activate_sh.assert(predicates::str::contains(
        r#"VIRTUAL_ENV=''"$(dirname -- "$(dirname -- "$(realpath -- "$SCRIPT_PATH")")")"''"#,
    ));

    let activate_bat = scripts.child("activate.bat");
    activate_bat.assert(predicates::path::is_file());
    activate_bat.assert(predicates::str::contains(
        r#"@for %%i in ("%~dp0..") do @set "VIRTUAL_ENV=%%~fi""#,
    ));

    let activate_fish = scripts.child("activate.fish");
    activate_fish.assert(predicates::path::is_file());
    activate_fish.assert(predicates::str::contains(
        r"set -gx VIRTUAL_ENV ''(dirname -- (dirname -- (realpath -- (status -f))))''",
    ));
    activate_fish.assert(predicates::str::contains(
        "if string match -qr 'CYGWIN|MSYS|MINGW' (uname); and command -s cygpath >/dev/null",
    ));
    activate_fish.assert(predicates::str::contains(
        r#"set -gx VIRTUAL_ENV (cygpath -u "$VIRTUAL_ENV")"#,
    ));

    let activate_nu = scripts.child("activate.nu");
    activate_nu.assert(predicates::path::is_file());
    activate_nu.assert(predicates::str::contains(
        r"const virtual_env = (path self | path dirname | path dirname)",
    ));

    // csh cannot determine its own script location, so activate.csh should not
    // be generated when --relocatable is used.
    assert_path_missing(activate_csh);

    let activate_xsh = scripts.child("activate.xsh");
    activate_xsh.assert(predicates::path::is_file());
    activate_xsh.assert(predicates::str::contains(
        r"dirname(dirname(realpath(__file__)))",
    ));
    activate_xsh.assert(predicates::str::contains(
        r#"self.embedded_virtual_prompt = b"r\xc3\xa9sum\xc3\xa9 \"quoted\"\\path\n".decode("utf-8")"#,
    ));
}

/// With `UV_VENV_RELOCATABLE=1`, the virtual environment is relocatable.
#[test]
fn verify_pyvenv_cfg_relocatable_env_var() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    // Create a virtual environment at `.venv` with the env var.
    context
        .venv()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12")
        .env(EnvVars::UV_VENV_RELOCATABLE, "1")
        .assert()
        .success();

    let pyvenv_cfg = context.venv.child("pyvenv.cfg");
    pyvenv_cfg.assert(predicates::path::is_file());

    // Relocatable flag is set.
    pyvenv_cfg.assert(predicates::str::contains("relocatable = true"));
}

/// `--no-relocatable` takes precedence over `UV_VENV_RELOCATABLE=1`.
#[test]
fn no_relocatable_overrides_relocatable_env_var() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    context
        .venv()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12")
        .arg("--no-relocatable")
        .env(EnvVars::UV_VENV_RELOCATABLE, "1")
        .assert()
        .success();

    let pyvenv_cfg = context.venv.child("pyvenv.cfg");
    pyvenv_cfg.assert(predicates::path::is_file());
    pyvenv_cfg.assert(predicates::str::contains("relocatable").not());
}

/// With `relocatable-envs-default` preview feature, venvs are relocatable by default.
#[test]
fn relocatable_envs_default_preview() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    // Create a virtual environment with the preview feature enabled.
    context
        .venv()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12")
        .arg("--preview-features")
        .arg("relocatable-envs-default")
        .assert()
        .success();

    let pyvenv_cfg = context.venv.child("pyvenv.cfg");
    pyvenv_cfg.assert(predicates::path::is_file());

    // Relocatable flag is set by default under preview.
    pyvenv_cfg.assert(predicates::str::contains("relocatable = true"));
}

/// With `relocatable-envs-default` preview feature, `--no-relocatable` opts out.
#[test]
fn relocatable_envs_default_no_relocatable() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    // Create a virtual environment with the preview feature but opt out.
    context
        .venv()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12")
        .arg("--preview-features")
        .arg("relocatable-envs-default")
        .arg("--no-relocatable")
        .assert()
        .success();

    let pyvenv_cfg = context.venv.child("pyvenv.cfg");
    pyvenv_cfg.assert(predicates::path::is_file());

    // Relocatable flag is NOT set because of --no-relocatable.
    pyvenv_cfg.assert(predicates::str::contains("relocatable").not());
}

/// Ensure that a nested virtual environment uses the same `home` directory as the parent.
#[test]
fn verify_nested_pyvenv_cfg() {
    let context = uv_test::test_context!("3.12");
    let parent_config = context.read(context.venv.join("pyvenv.cfg"));
    let parent_home = parent_config
        .lines()
        .find_map(|line| line.strip_prefix("home = "))
        .expect("parent pyvenv.cfg should contain a home entry");

    let subvenv = context.temp_dir.child(".subvenv");
    context
        .venv()
        .arg(subvenv.as_os_str())
        .arg("--python")
        .arg(context.interpreter())
        .assert()
        .success();

    let nested_config = context.read(subvenv.join("pyvenv.cfg"));
    let nested_home = nested_config
        .lines()
        .find_map(|line| line.strip_prefix("home = "))
        .expect("nested pyvenv.cfg should contain a home entry");

    assert_eq!(nested_home, parent_home);
}

/// See <https://github.com/astral-sh/uv/issues/3280>
#[test]
#[cfg(windows)]
fn path_with_trailing_space_gives_proper_error() {
    let context = uv_test::test_context_with_versions!(&[]);

    // Set a custom cache directory with a trailing space
    let mut cache_dir = context.cache_dir.as_os_str().to_owned();
    cache_dir.push(" ");
    // Windows translates error messages, for example i get:
    // ": Das System kann den angegebenen Pfad nicht finden. (os error 3)"
    let context = context.with_filter((
        r"\.lock`: .* \(os error 3\)",
        ".lock`: The system cannot find the path specified. (os error 3)",
    ));
    uv_snapshot!(context.filters(), context.external_command(uv_test::get_bin!())
        .arg("venv")
        .env(EnvVars::UV_CACHE_DIR, cache_dir), @r###"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Could not acquire lock
      cause: failed to open file `[CACHE_DIR]/ /.lock`: The system cannot find the path specified. (os error 3)
    "###
    );
    // Note the extra trailing `/` in the snapshot is due to the filters, not the actual output.
}

/// Activate a virtual environment through a UNC path.
///
/// Requires `UV_INTERNAL__TEST_SMB_FS`.
#[test]
#[cfg(windows)]
fn create_venv_powershell_unc() -> Result<()> {
    let Some(smb_fs) = std::env::var_os(EnvVars::UV_INTERNAL__TEST_SMB_FS) else {
        return Ok(());
    };
    let temp_dir = assert_fs::TempDir::new_in(smb_fs)?;
    let venv_dir = temp_dir.child("test env");
    let context =
        uv_test::test_context_with_versions!(&["3.12"]).with_filtered_path(temp_dir.path(), "SMB");

    context
        .venv()
        .arg(venv_dir.path())
        .arg("--python")
        .arg("3.12")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.external_command("powershell.exe")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(indoc! {r#"
            $ErrorActionPreference = "Stop"
            . $env:UV_TEST_ACTIVATE
            $env:VIRTUAL_ENV
            & $env:UV_TEST_BIN python find
            exit $LASTEXITCODE
        "#})
        .env("UV_TEST_ACTIVATE", venv_dir.child("Scripts/activate.ps1").path())
        .env("UV_TEST_BIN", uv_test::get_bin!())
        .env(EnvVars::UV_CACHE_DIR, context.cache_dir.path()), @r"
    exit_code: 0 (success)
    ----- stdout -----
    [SMB]/test env
    [SMB]/test env/Scripts/python.exe
    ");

    Ok(())
}

/// Check that the activate script still works with the path contains an apostrophe.
#[test]
#[cfg(target_os = "linux")]
fn create_venv_apostrophe() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    let venv_dir = context.temp_dir.join("Testing's");

    uv_snapshot!(context.filters(), context.venv()
        .arg(&venv_dir)
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: Testing's
    Activate with: source 'Testing'\"'\"'s/[BIN]/activate'
    "
    );

    context
        .external_command("sh")
        .args([
            "-c",
            ". bin/activate && python -c 'import sys; print(sys.prefix)'",
        ])
        .current_dir(&venv_dir)
        .assert()
        .success()
        .stdout(format!("{}\n", venv_dir.display()).into_bytes());
}

/// Emulate a ksh without ksh93's `${.sh.file}` parameter using a POSIX shell.
#[test]
#[cfg(unix)]
fn activate_non_relocatable_venv_ksh() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    context
        .venv()
        .arg("--no-relocatable")
        .arg("--python")
        .arg("3.12")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.external_command("sh")
        .arg("-c")
        .arg(indoc! {r"
            unset BASH_VERSION ZSH_VERSION
            readonly SCRIPT_PATH=caller
            . .venv/bin/activate && python -c 'import sys; print(sys.prefix)'
        "})
        .env(EnvVars::KSH_VERSION, "test"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [VENV]/
    ");
}

#[test]
fn venv_python_preference() {
    let context =
        uv_test::test_context_with_versions!(&["3.12", "3.11"]).with_versions_as_managed(&["3.12"]);

    // Create a managed interpreter environment
    uv_snapshot!(context.filters(), context.venv(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    ");

    uv_snapshot!(context.filters(), context.venv().arg("--no-managed-python"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    Creating virtual environment at: .venv
    error: Failed to create virtual environment
      cause: A virtual environment already exists at: .venv

    hint: Use the `--clear` flag or set `UV_VENV_CLEAR=1` to replace the existing virtual environment
    ");

    uv_snapshot!(context.filters(), context.venv().arg("--clear").arg("--no-managed-python"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    ");

    uv_snapshot!(context.filters(), context.venv(), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.12.[X]
    Creating virtual environment at: .venv
    error: Failed to create virtual environment
      cause: A virtual environment already exists at: .venv

    hint: Use the `--clear` flag or set `UV_VENV_CLEAR=1` to replace the existing virtual environment
    ");

    uv_snapshot!(context.filters(), context.venv().arg("--clear").arg("--managed-python"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    ");
}

#[test]
#[cfg(unix)]
fn create_venv_symlink_clear_preservation() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    let target_dir = context.temp_dir.child("target");
    target_dir.create_dir_all()?;
    let symlink_path = context.temp_dir.child(".venv");
    symlink(&target_dir, &symlink_path)?;

    uv_snapshot!(context.filters(), context.venv()
        .arg(symlink_path.as_os_str())
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    assert_link_target(&symlink_path, &target_dir);
    let marker = target_dir.child("marker");
    marker.touch()?;

    uv_snapshot!(context.filters(), context.venv()
        .arg(symlink_path.as_os_str())
        .arg("--clear")
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    assert_link_target(&symlink_path, &target_dir);
    assert_path_missing(marker);
    target_dir
        .child("pyvenv.cfg")
        .assert(predicates::path::is_file());

    Ok(())
}

#[test]
#[cfg(unix)]
fn create_venv_nested_symlink_preservation() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);
    let target_dir = context.temp_dir.child("target");
    target_dir.create_dir_all()?;
    let intermediate_link = context.temp_dir.child("intermediate");
    symlink(&target_dir, &intermediate_link)?;
    let symlink_path = context.temp_dir.child(".venv");
    symlink(&intermediate_link, &symlink_path)?;

    uv_snapshot!(context.filters(), context.venv()
        .arg(symlink_path.as_os_str())
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    assert_link_target(&symlink_path, &intermediate_link);
    assert_link_target(&intermediate_link, &target_dir);
    let marker = target_dir.child("marker");
    marker.touch()?;

    uv_snapshot!(context.filters(), context.venv()
        .arg(symlink_path.as_os_str())
        .arg("--clear")
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    assert_link_target(&symlink_path, &intermediate_link);
    assert_link_target(&intermediate_link, &target_dir);
    assert_path_missing(marker);
    target_dir
        .child("pyvenv.cfg")
        .assert(predicates::path::is_file());

    Ok(())
}

/// On Unix, creating a virtual environment in the current working directory should work.
#[test]
#[cfg(unix)]
fn create_venv_current_working_directory() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );
    let marker = context.venv.child("recreation-marker");
    marker.write_str("existing environment\n")?;

    uv_snapshot!(context.filters(), context.venv()
        .arg(".")
        .arg("--clear")
        .arg("--python")
        .arg("3.12")
        .current_dir(&context.venv), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .
    Activate with: source [BIN]/activate
    "
    );

    assert_path_missing(marker);
    context
        .venv
        .child("pyvenv.cfg")
        .assert(predicates::path::is_file());

    Ok(())
}

/// On Windows, creating a virtual environment in the current working directory should fail,
/// as you can't delete the current working directory.
#[test]
#[cfg(windows)]
fn create_venv_current_working_directory() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12"), @r"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    uv_snapshot!(context.filters(), context.venv()
        .arg(".")
        .arg("--clear")
        .arg("--python")
        .arg("3.12")
        .current_dir(&context.venv), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .
    error: Failed to create virtual environment
      cause: failed to remove directory `[VENV]/`: The process cannot access the file because it is being used by another process. (os error 32)
    "
    );
}

#[test]
fn no_clear_with_existing_directory() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    // Create a virtual environment first
    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );
    let marker = context.venv.child("marker");
    marker.write_str("existing environment\n")?;

    // Try to create again with --no-clear (should fail)
    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--no-clear")
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    error: Failed to create virtual environment
      cause: A virtual environment already exists at: .venv

    hint: Use the `--clear` flag or set `UV_VENV_CLEAR=1` to replace the existing virtual environment
    "
    );

    marker.assert("existing environment\n");
    Ok(())
}

#[test]
fn no_clear_with_non_existent_directory() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    // Create with --no-clear on non-existent directory (should succeed)
    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--no-clear")
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Activate with: source .venv/[BIN]/activate
    "
    );

    context.venv.assert(predicates::path::is_dir());
}

#[test]
fn no_clear_overrides_clear() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    // Create a non-empty directory at `.venv`
    context.venv.create_dir_all()?;
    let marker = context.venv.child("file");
    marker.write_str("original contents\n")?;

    // --no-clear should override --clear and fail without prompting
    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--clear")
        .arg("--no-clear")
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    error: Failed to create virtual environment
      cause: A directory already exists at: .venv

    hint: Use the `--clear` flag or set `UV_VENV_CLEAR=1` to replace the existing directory
    "
    );

    marker.assert("original contents\n");
    Ok(())
}

#[test]
fn no_clear_overrides_clear_env_var() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    // Create a non-empty directory at `.venv`
    context.venv.create_dir_all()?;
    let marker = context.venv.child("file");
    marker.write_str("original contents\n")?;

    // --no-clear should override UV_VENV_CLEAR=1 and fail without prompting
    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--no-clear")
        .arg("--python")
        .arg("3.12")
        .env(EnvVars::UV_VENV_CLEAR, "1"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    error: Failed to create virtual environment
      cause: A directory already exists at: .venv

    hint: Use the `--clear` flag or set `UV_VENV_CLEAR=1` to replace the existing directory
    "
    );

    marker.assert("original contents\n");
    Ok(())
}

#[test]
fn no_clear_conflicts_with_allow_existing() {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    // Try to use --no-clear with --allow-existing (should fail)
    uv_snapshot!(context.filters(), context.venv()
        .arg(context.venv.as_os_str())
        .arg("--no-clear")
        .arg("--allow-existing")
        .arg("--python")
        .arg("3.12"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--no-clear' cannot be used with '--allow-existing'

    Usage: uv venv --cache-dir [CACHE_DIR] --python <PYTHON> --exclude-newer <EXCLUDE_NEWER> <PATH>

    For more information, try '--help'.
    "
    );
}
