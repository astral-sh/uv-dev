use std::collections::BTreeMap;
#[cfg(windows)]
use std::path::{Component, Prefix};

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_static::EnvVars;
use uv_test::package_server::PackageServer;
use uv_test::packse::generate_wheel_with_files;
use uv_test::uv_snapshot;

#[test]
fn no_arguments() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.filters(), context.pip_uninstall(), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the following required arguments were not provided:
      <PACKAGE|--requirements <REQUIREMENTS>>

    Usage: uv pip uninstall --cache-dir [CACHE_DIR] --exclude-newer <EXCLUDE_NEWER> <PACKAGE|--requirements <REQUIREMENTS>>

    For more information, try '--help'.
    "
    );
}

#[test]
fn invalid_requirement() {
    let context = uv_test::test_context!("3.12");

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
    let context = uv_test::test_context!("3.12");

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
    let context = uv_test::test_context!("3.12");

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

#[test]
#[cfg(feature = "test-pypi")]
fn uninstall() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("MarkupSafe==2.1.3")?;

    context
        .pip_sync()
        .arg("requirements.txt")
        .assert()
        .success();

    context.assert_command("import markupsafe").success();

    uv_snapshot!(context.pip_uninstall()
        .arg("MarkupSafe"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - markupsafe==2.1.3
    "
    );

    context.assert_command("import markupsafe").failure();

    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn missing_record() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("MarkupSafe==2.1.3")?;

    context
        .pip_sync()
        .arg("requirements.txt")
        .assert()
        .success();

    context.assert_command("import markupsafe").success();

    // Delete the RECORD file.
    let dist_info = context.site_packages().join("MarkupSafe-2.1.3.dist-info");
    fs_err::remove_file(dist_info.join("RECORD"))?;

    uv_snapshot!(context.filters(), context.pip_uninstall()
        .arg("MarkupSafe"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Cannot uninstall package; `RECORD` file not found at: [SITE_PACKAGES]/MarkupSafe-2.1.3.dist-info/RECORD
    "
    );

    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn uninstall_editable_by_name() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "-e {}",
        context
            .workspace_root
            .join("test/packages/flit_editable")
            .as_os_str()
            .to_str()
            .expect("Path is valid unicode")
    ))?;
    context
        .pip_sync()
        .arg(requirements_txt.path())
        .assert()
        .success();

    context.assert_command("import flit_editable").success();

    // Uninstall the editable by name.
    uv_snapshot!(context.filters(), context.pip_uninstall()
        .arg("flit-editable"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - flit-editable==0.1.0 (from file://[WORKSPACE]/test/packages/flit_editable)
    "
    );

    context.assert_command("import flit_editable").failure();

    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn uninstall_by_path() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(
        context
            .workspace_root
            .join("test/packages/flit_editable")
            .as_os_str()
            .to_str()
            .expect("Path is valid unicode"),
    )?;

    context
        .pip_sync()
        .arg(requirements_txt.path())
        .assert()
        .success();

    context.assert_command("import flit_editable").success();

    // Uninstall the editable by path.
    uv_snapshot!(context.filters(), context.pip_uninstall()
        .arg(context.workspace_root.join("test/packages/flit_editable")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - flit-editable==0.1.0 (from file://[WORKSPACE]/test/packages/flit_editable)
    "
    );

    context.assert_command("import flit_editable").failure();

    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn uninstall_duplicate_by_path() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(
        context
            .workspace_root
            .join("test/packages/flit_editable")
            .as_os_str()
            .to_str()
            .expect("Path is valid unicode"),
    )?;

    context
        .pip_sync()
        .arg(requirements_txt.path())
        .assert()
        .success();

    context.assert_command("import flit_editable").success();

    // Uninstall the editable by both path and name.
    uv_snapshot!(context.filters(), context.pip_uninstall()
        .arg("flit-editable")
        .arg(context.workspace_root.join("test/packages/flit_editable")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - flit-editable==0.1.0 (from file://[WORKSPACE]/test/packages/flit_editable)
    "
    );

    context.assert_command("import flit_editable").failure();

    Ok(())
}

/// Uninstall a duplicate package in a virtual environment.
#[test]
#[cfg(feature = "test-pypi")]
fn uninstall_duplicate() -> Result<()> {
    use uv_fs::copy_dir_all;

    // Sync a version of `pip` into a virtual environment.
    let context1 = uv_test::test_context!("3.12");
    let requirements_txt = context1.temp_dir.child("requirements.txt");
    requirements_txt.write_str("pip==21.3.1")?;

    // Run `pip sync`.
    context1
        .pip_sync()
        .arg(requirements_txt.path())
        .assert()
        .success();

    // Sync a different version of `pip` into a virtual environment.
    let context2 = uv_test::test_context!("3.12");
    let requirements_txt = context2.temp_dir.child("requirements.txt");
    requirements_txt.write_str("pip==22.1.1")?;

    // Run `pip sync`.
    context2
        .pip_sync()
        .arg(requirements_txt.path())
        .assert()
        .success();

    // Copy the virtual environment to a new location.
    copy_dir_all(
        context2.site_packages().join("pip-22.1.1.dist-info"),
        context1.site_packages().join("pip-22.1.1.dist-info"),
    )?;

    // Run `pip uninstall`.
    uv_snapshot!(context1.pip_uninstall()
        .arg("pip"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 2 packages in [TIME]
     - pip==21.3.1
     - pip==22.1.1
    "
    );

    Ok(())
}

/// Uninstall a `.egg-info` package in a virtual environment.
#[test]
fn uninstall_egg_info() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let site_packages = ChildPath::new(context.site_packages());

    // Manually create a `.egg-info` directory.
    site_packages
        .child("zstandard-0.22.0-py3.12.egg-info")
        .create_dir_all()?;
    site_packages
        .child("zstandard-0.22.0-py3.12.egg-info")
        .child("top_level.txt")
        .write_str("zstd")?;
    site_packages
        .child("zstandard-0.22.0-py3.12.egg-info")
        .child("SOURCES.txt")
        .write_str("")?;
    site_packages
        .child("zstandard-0.22.0-py3.12.egg-info")
        .child("PKG-INFO")
        .write_str("")?;
    site_packages
        .child("zstandard-0.22.0-py3.12.egg-info")
        .child("dependency_links.txt")
        .write_str("")?;
    site_packages
        .child("zstandard-0.22.0-py3.12.egg-info")
        .child("entry_points.txt")
        .write_str("")?;

    // Manually create the package directory.
    site_packages.child("zstd").create_dir_all()?;
    site_packages
        .child("zstd")
        .child("__init__.py")
        .write_str("")?;

    // Run `pip uninstall`.
    uv_snapshot!(context.pip_uninstall()
        .arg("zstandard"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - zstandard==0.22.0
    ");

    Ok(())
}

/// Refuse to uninstall a versionless `.egg-info` file without the metadata required to do so safely.
#[test]
fn uninstall_versionless_egg_info_file() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let egg_info = ChildPath::new(context.site_packages()).child("demo.egg-info");
    egg_info.write_str("Metadata-Version: 1.1\nName: demo\nVersion: 1.0\n")?;

    uv_snapshot!(context.pip_uninstall().arg("demo"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Unable to uninstall `demo==1.0`. distutils-installed distributions do not include the metadata required to uninstall safely.
    "
    );

    assert!(egg_info.exists());

    Ok(())
}

fn normcase(s: &str) -> String {
    if cfg!(windows) {
        s.replace('/', "\\").to_lowercase()
    } else {
        s.to_owned()
    }
}

/// Uninstall a legacy editable package in a virtual environment.
#[test]
fn uninstall_legacy_editable() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let site_packages = ChildPath::new(context.site_packages());

    let target = context.temp_dir.child("zstandard_project");
    target.child("zstd").create_dir_all()?;
    target.child("zstd").child("__init__.py").write_str("")?;

    target.child("zstandard.egg-info").create_dir_all()?;
    target
        .child("zstandard.egg-info")
        .child("PKG-INFO")
        .write_str(
            "Metadata-Version: 2.1
Name: zstandard
Version: 0.22.0
",
        )?;

    site_packages
        .child("zstandard.egg-link")
        .write_str(target.path().to_str().unwrap())?;

    site_packages.child("easy-install.pth").write_str(&format!(
        "something\n{}\nanother thing\n",
        normcase(target.path().to_str().unwrap())
    ))?;

    // Run `pip uninstall`.
    uv_snapshot!(context.pip_uninstall()
        .arg("zstandard"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - zstandard==0.22.0
    ");

    // The entry in `easy-install.pth` should be removed.
    assert_eq!(
        context.read(site_packages.child("easy-install.pth")),
        "something\nanother thing\n",
        "easy-install.pth should not contain the path to the uninstalled package"
    );
    // The `.egg-link` file should be removed.
    assert!(!site_packages.child("zstandard.egg-link").exists());
    // The `.egg-info` directory should still exist.
    assert!(target.child("zstandard.egg-info").exists());

    Ok(())
}

#[test]
fn dry_run_uninstall_egg_info() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let site_packages = ChildPath::new(context.site_packages());

    // Manually create a `.egg-info` directory.
    site_packages
        .child("zstandard-0.22.0-py3.12.egg-info")
        .create_dir_all()?;
    site_packages
        .child("zstandard-0.22.0-py3.12.egg-info")
        .child("top_level.txt")
        .write_str("zstd")?;
    site_packages
        .child("zstandard-0.22.0-py3.12.egg-info")
        .child("SOURCES.txt")
        .write_str("")?;
    site_packages
        .child("zstandard-0.22.0-py3.12.egg-info")
        .child("PKG-INFO")
        .write_str("")?;
    site_packages
        .child("zstandard-0.22.0-py3.12.egg-info")
        .child("dependency_links.txt")
        .write_str("")?;
    site_packages
        .child("zstandard-0.22.0-py3.12.egg-info")
        .child("entry_points.txt")
        .write_str("")?;

    // Manually create the package directory.
    site_packages.child("zstd").create_dir_all()?;
    site_packages
        .child("zstd")
        .child("__init__.py")
        .write_str("")?;

    // Run `pip uninstall`.
    uv_snapshot!(context.pip_uninstall()
        .arg("--dry-run")
        .arg("zstandard"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would uninstall 1 package
     - zstandard==0.22.0
    ");

    // The `.egg-info` directory should still exist.
    assert!(
        site_packages
            .child("zstandard-0.22.0-py3.12.egg-info")
            .exists()
    );
    // The package directory should still exist.
    assert!(site_packages.child("zstd").child("__init__.py").exists());

    Ok(())
}

/// Uninstall must not remove files outside the install scheme.
///
/// A malformed or malicious wheel can include path-traversal entries
/// (e.g. `../../../../../etc/passwd`) in its RECORD file. During uninstall those entries are joined
/// with the site-packages directory and could cause deletion of files outside the installation
/// scheme.
#[test]
fn uninstall_record_path_traversal() -> Result<()> {
    // The traversal-depth count differs between Unix (`.venv/lib/pythonX.Y/site-packages`)
    // and Windows (`.venv/Lib/site-packages`), so normalize the `../` sequence in the warning.
    let context = uv_test::test_context!("3.12").with_filter((
        r"(\.\./)+traversal_target\.txt",
        "[..]/traversal_target.txt",
    ));

    context
        .init()
        .arg("--lib")
        .arg("evilpkg")
        .assert()
        .success();
    context.pip_install().arg("./evilpkg").assert().success();

    // Build the relative traversal path from site-packages to a target file outside
    // site-packages but inside the test temp dir. RECORD uses forward slashes, even on
    // Windows, and the environment layout (and thus the traversal depth) differs by platform,
    // so we construct the path manually and filter the leading `../` sequence out of the
    // snapshot above.
    let target_file = context.temp_dir.child("traversal_target.txt");
    target_file.write_str("I should not be deleted")?;
    // Canonicalize the temp dir, since `site_packages` is built from a canonicalized path
    // (with `\\?\`), which would otherwise make `strip_prefix` fail.
    let canonical_temp_dir = context.temp_dir.canonicalize()?;
    let depth = context
        .site_packages()
        .strip_prefix(&canonical_temp_dir)?
        .components()
        .count();
    let traversal_record = format!("{}traversal_target.txt", "../".repeat(depth));

    let record_file = context
        .site_packages()
        .join("evilpkg-0.1.0.dist-info/RECORD");
    let record = context.read(&record_file);
    let record = format!("{}\n{},,0\n", record.trim(), traversal_record);
    fs_err::write(record_file, &record)?;

    let init_py = context.site_packages().join("evilpkg/__init__.py");
    assert!(context.site_packages().join(&traversal_record).exists());
    assert!(init_py.exists());

    uv_snapshot!(context.filters(), context.pip_uninstall()
        .arg("evilpkg"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Invalid `RECORD` entry in `evilpkg==0.1.0 (from file://[TEMP_DIR]/evilpkg)` that escapes the Python environment, skipping: [..]/traversal_target.txt
    Uninstalled 1 package in [TIME]
     - evilpkg==0.1.0 (from file://[TEMP_DIR]/evilpkg)
    ");

    // The regular package files have been removed, while the file outside the scheme still exists.
    assert!(target_file.exists());
    assert!(!init_py.exists());

    Ok(())
}

/// Egg `top_level.txt` entries must be top-level names, not paths.
#[test]
fn uninstall_egg_info_top_level_path_traversal() -> Result<()> {
    // The traversal-depth count differs between Unix (`.venv/lib/pythonX.Y/site-packages`)
    // and Windows (`.venv/Lib/site-packages`), so normalize the `../` sequence in the warning.
    let context = uv_test::test_context!("3.12")
        .with_filter((r"(\.\./)+traversal_target", "[..]/traversal_target"));

    let site_packages = ChildPath::new(context.site_packages());

    // Manually create a `name-version.egg-info` directory, which is recognized by the shared egg
    // filename parser.
    let egg_info = site_packages.child("evilpkg-0.1.0.egg-info");
    egg_info.create_dir_all()?;

    // The traversal target is outside site-packages but inside the environment, so a wheel RECORD entry
    // could validly target this scheme area. An egg `top_level.txt` entry must not.
    let target_dir = context.venv.child("traversal_target");
    let target_file = target_dir.child("secret.txt");
    target_file.write_str("I should not be deleted")?;

    let depth = context
        .site_packages()
        .strip_prefix(context.venv.path())?
        .components()
        .count();
    let traversal_entry = format!("{}traversal_target", "../".repeat(depth));
    assert!(context.site_packages().join(&traversal_entry).exists());

    egg_info
        .child("top_level.txt")
        .write_str(&format!("evilpkg\n{traversal_entry}\n"))?;

    let init_py = site_packages.child("evilpkg").child("__init__.py");
    init_py.touch()?;

    uv_snapshot!(context.filters(), context.pip_uninstall()
        .arg("evilpkg"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Invalid `top_level.txt` entry in evilpkg==0.1.0 that is not a top-level module or package, skipping: [..]/traversal_target
    Uninstalled 1 package in [TIME]
     - evilpkg==0.1.0
    ");

    assert!(target_dir.exists());
    assert!(target_file.exists());
    assert!(!init_py.exists());
    assert!(!egg_info.exists());

    Ok(())
}

/// Windows drive-relative paths are not valid `top_level.txt` entries.
#[cfg(windows)]
#[test]
fn uninstall_egg_info_top_level_drive_relative() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filter((r"[A-Za-z]:traversal_target", "[DRIVE]:traversal_target"));
    let site_packages = ChildPath::new(context.site_packages());

    let egg_info = site_packages.child("evilpkg-0.1.0.egg-info");
    egg_info.create_dir_all()?;

    let drive = match context.temp_dir.path().components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => drive,
            prefix => anyhow::bail!("expected a disk path, found {prefix:?}"),
        },
        component => anyhow::bail!("expected a Windows path prefix, found {component:?}"),
    };
    let traversal_entry = format!("{}:traversal_target", char::from(drive));

    // Commands run from `context.temp_dir`, so this drive-relative path resolves there rather than
    // below `site-packages`.
    let target_file = context
        .temp_dir
        .child("traversal_target")
        .child("secret.txt");
    target_file.write_str("I should not be deleted")?;
    egg_info
        .child("top_level.txt")
        .write_str(&format!("evilpkg\n{traversal_entry}\n"))?;

    let init_py = site_packages.child("evilpkg").child("__init__.py");
    init_py.touch()?;

    uv_snapshot!(context.filters(), context.pip_uninstall()
        .arg("evilpkg"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Invalid `top_level.txt` entry in evilpkg==0.1.0 that is not a top-level module or package, skipping: [DRIVE]:traversal_target
    Uninstalled 1 package in [TIME]
     - evilpkg==0.1.0
    ");

    assert!(target_file.exists());
    assert!(!init_py.exists());
    assert!(!egg_info.exists());

    Ok(())
}

/// `--yes` is accepted for `pip uninstall` compatibility, but emits a warning.
#[test]
fn yes_flag() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.filters(), context.pip_uninstall()
        .arg("--yes")
        .arg("flask"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: `--yes` has no effect (uv never asks for confirmation)
    warning: Skipping flask as it is not installed
    warning: No packages to uninstall
    "
    );
}

/// `-y` is accepted for `pip uninstall` compatibility, but emits a warning.
#[test]
fn yes_short_flag() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.filters(), context.pip_uninstall()
        .arg("-y")
        .arg("flask"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: `--yes` has no effect (uv never asks for confirmation)
    warning: Skipping flask as it is not installed
    warning: No packages to uninstall
    "
    );
}

/// Lockfile selection uses the current environment and synthetic default groups without reading artifacts.
#[test]
#[cfg(feature = "test-pypi")]
fn uninstall_pylock_stdin() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .pip_install()
        .args(["iniconfig==2.0.0", "idna==3.6", "sniffio==1.3.0"])
        .assert()
        .success();
    let input = context.temp_dir.child("input");
    input.write_str(indoc! {r#"
        lock-version = "1.0"
        created-by = "uv"
        default-groups = ["default"]
        extras = ["test"]
        [[packages]]
        name = "iniconfig"
        version = "1.1.1"
        marker = "'default' in dependency_groups"
        archive = { path = "missing.tar.gz", subdirectory = "missing", hashes = { sha256 = "0000000000000000000000000000000000000000000000000000000000000000" } }
        [[packages]]
        name = "idna"
        version = "3.6"
        marker = "python_version < '3'"
        [[packages]]
        name = "sniffio"
        version = "1.3.0"
        marker = "'test' in extras"
    "#})?;

    uv_snapshot!(context.filters(), context.pip_uninstall().args(["--offline", "-r", "-"])
        .stdin(fs_err::File::open(input.path())?.into_file()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - iniconfig==2.0.0
    ");
    uv_snapshot!(context.filters(), context.pip_freeze(), @"
    exit_code: 0 (success)
    ----- stdout -----
    idna==3.6
    sniffio==1.3.0
    ");
    Ok(())
}

/// Base metadata dependencies follow requirements-file behavior: versions and markers are ignored.
/// Static metadata needs neither installation sources nor configured build inputs to be available.
#[test]
#[cfg(feature = "test-pypi")]
fn uninstall_pyproject_stdin() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .pip_install()
        .args(["iniconfig==2.0.0", "sniffio==1.3.0"])
        .assert()
        .success();
    let input = context.temp_dir.child("input");
    input.write_str(indoc! {r#"
        [project]
        name = "stdin-project"
        version = "1.0"
        dependencies = ["iniconfig>=99; python_version < '3'"]
        [project.optional-dependencies]
        test = ["sniffio"]
        [tool.uv.sources]
        iniconfig = { index = "private" }
    "#})?;

    uv_snapshot!(context.filters(), context.pip_uninstall().args(["--offline", "-r", "-"])
        .env(EnvVars::UV_FIND_LINKS, "missing-wheels")
        .env(EnvVars::UV_BUILD_CONSTRAINT, "missing-build-constraints.txt")
        .stdin(fs_err::File::open(input.path())?.into_file()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - iniconfig==2.0.0
    ");
    uv_snapshot!(context.filters(), context.pip_freeze(), @"
    exit_code: 0 (success)
    ----- stdout -----
    sniffio==1.3.0
    ");
    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn uninstall_setup_cfg_stdin() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .pip_install()
        .arg("iniconfig==2.0.0")
        .assert()
        .success();
    let input = context.temp_dir.child("input");
    input.write_str(indoc! {r"
        [metadata]
        name = stdin-project
        version = 1.0
        [options]
        install_requires = iniconfig>=99
    "})?;

    uv_snapshot!(context.filters(), context.pip_uninstall().args(["-r", "-"])
        .stdin(fs_err::File::open(input.path())?.into_file()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - iniconfig==2.0.0
    ");
    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn uninstall_setup_py_stdin() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .pip_install()
        .args(["iniconfig==2.0.0", "setuptools==69.2.0", "wheel==0.43.0"])
        .assert()
        .success();
    let input = context.temp_dir.child("input");
    input.write_str(indoc! {r#"
        from setuptools import setup
        setup(name="stdin-project", version="1.0", install_requires=["iniconfig>=99"])
    "#})?;

    uv_snapshot!(context.filters(), context.pip_uninstall()
        .args(["--no-index", "-r", "-", "--input-format", "setup.py"])
        .env(EnvVars::UV_NO_BUILD_ISOLATION, "1")
        .stdin(fs_err::File::open(input.path())?.into_file()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - iniconfig==2.0.0
    ");
    Ok(())
}

/// Dynamic metadata uses the configured index to install its backend and passes build settings.
#[tokio::test]
async fn uninstall_pyproject_build_index() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .pip_install()
        .arg(
            context
                .workspace_root
                .join("test/links/simple_launcher-0.1.0-py3-none-any.whl"),
        )
        .assert()
        .success();

    let name = "uninstall-build-backend".parse()?;
    let (filename, wheel) = generate_wheel_with_files(
        &name,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::default(),
        None,
        "py3-none-any",
        &[(
            "uninstall_build_backend/backend.py",
            indoc! {r#"
            from pathlib import Path

            def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
                dist_info = Path(metadata_directory) / "metadata_project-1.0.dist-info"
                dist_info.mkdir()
                dependency = config_settings["dependency"]
                (dist_info / "METADATA").write_text(
                    "Metadata-Version: 2.3\nName: metadata-project\nVersion: 1.0\n"
                    f"Requires-Dist: {dependency}\n"
                )
                return dist_info.name
        "#},
        )],
    );
    let server = PackageServer::new(&name).await;
    server.serve(&filename, &wheel, None).await;

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "metadata-project"
        version = "1.0"
        dynamic = ["dependencies"]

        [build-system]
        requires = ["uninstall-build-backend==1.0.0"]
        build-backend = "uninstall_build_backend.backend"
    "#})?;

    uv_snapshot!(context.filters(), context.pip_uninstall()
        .args(["--dry-run", "-r", "pyproject.toml", "-C", "dependency=simple-launcher", "--only-binary", "uninstall-build-backend"])
        .env(EnvVars::UV_INDEX_URL, server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would uninstall 1 package
     - simple-launcher==0.1.0 (from file://[WORKSPACE]/test/links/simple_launcher-0.1.0-py3-none-any.whl)
    ");

    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&format!(
            "--index-url {}\nsimple-launcher\n",
            server.index_url()
        ))?;
    uv_snapshot!(context.filters(), context.pip_uninstall()
        .args(["--no-cache", "-r", "pyproject.toml", "-r", "requirements.txt", "-C", "dependency=simple-launcher"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - simple-launcher==0.1.0 (from file://[WORKSPACE]/test/links/simple_launcher-0.1.0-py3-none-any.whl)
    ");
    Ok(())
}

/// Reading dynamic metadata must not resolve unrelated unnamed requirements before matching
/// installed URLs.
#[test]
#[cfg(feature = "test-pypi")]
fn uninstall_pyproject_with_unnamed_requirement() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .pip_install()
        .arg("iniconfig==2.0.0")
        .assert()
        .success();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "metadata-project"
        version = "1.0"
        dynamic = ["dependencies"]

        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    context.temp_dir.child("backend.py").write_str(indoc! {r#"
        from pathlib import Path

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            dist_info = Path(metadata_directory) / "metadata_project-1.0.dist-info"
            dist_info.mkdir()
            (dist_info / "METADATA").write_text(
                "Metadata-Version: 2.3\nName: metadata-project\nVersion: 1.0\n"
                "Requires-Dist: iniconfig\n"
            )
            return dist_info.name
    "#})?;
    uv_snapshot!(context.filters(), context.pip_uninstall()
        .args(["--offline", "-r", "pyproject.toml", "https://example.com/missing.tar.gz"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Skipping `https://example.com/missing.tar.gz` as it is not installed
    Uninstalled 1 package in [TIME]
     - iniconfig==2.0.0
    ");
    Ok(())
}

/// Metadata build settings are enforced before any installed distribution is removed.
#[test]
#[cfg(feature = "test-pypi")]
fn uninstall_setup_py_build_hashes() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .pip_install()
        .arg("iniconfig==2.0.0")
        .assert()
        .success();
    context.temp_dir.child("uv.toml").write_str(indoc! {r"
        [pip]
        require-build-hashes = true
    "})?;
    context
        .temp_dir
        .child("build-constraints.txt")
        .write_str("setuptools>=40")?;
    context.temp_dir.child("setup.py").write_str(indoc! {r#"
        from setuptools import setup
        setup(name="metadata-project", version="1.0", install_requires=["iniconfig"])
    "#})?;
    uv_snapshot!(context.filters(), context.pip_uninstall()
        .args(["--offline", "--preview-features", "build-dependency-hashes", "-r", "setup.py"])
        .env(EnvVars::UV_BUILD_CONSTRAINT, "build-constraints.txt"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have their versions pinned with `==`, but found: setuptools>=40
    ");
    uv_snapshot!(context.filters(), context.pip_freeze().arg("--no-config"), @"
    exit_code: 0 (success)
    ----- stdout -----
    iniconfig==2.0.0
    ");
    Ok(())
}

#[test]
fn uninstall_input_format_requires_stdin() {
    let context = uv_test::test_context_with_versions!(&[]);
    uv_snapshot!(context.filters(), context.pip_uninstall()
        .args(["iniconfig", "--input-format", "setup.py"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `--input-format` requires a requirements input of `-`
    ");
}

/// Pip build options consume package names without treating them as uninstall targets.
#[test]
#[cfg(feature = "test-pypi")]
fn uninstall_build_binary_options() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .pip_install()
        .args(["iniconfig==2.0.0", "setuptools==69.2.0"])
        .assert()
        .success();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "metadata-project"
        version = "1.0"
        dependencies = ["iniconfig"]
    "#})?;

    uv_snapshot!(context.filters(), context.pip_uninstall()
        .args(["--offline", "-r", "pyproject.toml", "--no-binary", "setuptools"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - iniconfig==2.0.0
    ");
    uv_snapshot!(context.filters(), context.pip_freeze(), @"
    exit_code: 0 (success)
    ----- stdout -----
    setuptools==69.2.0
    ");
    Ok(())
}
