use std::collections::BTreeMap;
#[cfg(windows)]
use std::path::{Component, Prefix};

use anyhow::{Context, Result};
use assert_cmd::prelude::*;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;

use uv_test::packse::generate_wheel;
use uv_test::uv_snapshot;

#[test]
fn uninstall_ignores_version_specifier() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("ok==1.0.0")?;

    context
        .pip_sync()
        .args(["requirements.txt", "--no-index", "--find-links"])
        .arg(context.workspace_root.join("test/links"))
        .assert()
        .success();

    context.assert_installed("ok", "1.0.0");

    uv_snapshot!(context.pip_uninstall()
        .arg("ok==999.0.0"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - ok==1.0.0
    "
    );

    context.assert_not_installed("ok");

    Ok(())
}

#[test]
fn missing_record() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let (filename, bytes) = generate_wheel(
        &"demo".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let wheel = context.temp_dir.child(filename);
    wheel.write_binary(&bytes)?;
    context
        .pip_install()
        .arg("--no-index")
        .arg(wheel.path())
        .assert()
        .success();

    let dist_info = ChildPath::new(context.site_packages()).child("demo-1.0.0.dist-info");
    let metadata = dist_info.child("METADATA");
    let contents = context.read(metadata.path());
    fs_err::remove_file(dist_info.child("RECORD"))?;

    uv_snapshot!(context.filters(), context.pip_uninstall()
        .arg("demo"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Cannot uninstall package; `RECORD` file not found at: [SITE_PACKAGES]/demo-1.0.0.dist-info/RECORD
    "
    );

    metadata.assert(contents);
    context.assert_installed("demo", "1.0.0");

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

    context.assert_not_installed("flit_editable");

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

    context.assert_not_installed("flit_editable");

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

    context.assert_not_installed("flit_editable");

    Ok(())
}

/// Uninstall a duplicate package in a virtual environment.
#[test]
fn uninstall_duplicate() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let site_packages = ChildPath::new(context.site_packages());
    let package = site_packages.child("demo");
    package.child("__init__.py").write_str("VALUE = 42\n")?;
    let first = site_packages.child("demo-1.0.0.dist-info");
    let second = site_packages.child("demo-2.0.0.dist-info");
    for (version, distribution) in [("1.0.0", &first), ("2.0.0", &second)] {
        distribution.child("METADATA").write_str(&format!(
            "Metadata-Version: 2.1\nName: demo\nVersion: {version}\n"
        ))?;
        distribution.child("RECORD").write_str(&format!(
            "demo/__init__.py,,\ndemo-{version}.dist-info/METADATA,,\ndemo-{version}.dist-info/RECORD,,\n"
        ))?;
    }

    uv_snapshot!(context.pip_uninstall().arg("demo"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 2 packages in [TIME]
     - demo==1.0.0
     - demo==2.0.0
    ");

    package.assert(predicates::path::missing());
    first.assert(predicates::path::missing());
    second.assert(predicates::path::missing());

    Ok(())
}

/// Uninstall a `.egg-info` package in a virtual environment.
#[test]
fn uninstall_egg_info() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let site_packages = ChildPath::new(context.site_packages());

    let egg_info = site_packages.child("zstandard-0.22.0-py3.12.egg-info");
    egg_info.child("top_level.txt").write_str("zstd")?;
    egg_info.child("SOURCES.txt").touch()?;
    egg_info.child("PKG-INFO").touch()?;
    egg_info.child("dependency_links.txt").touch()?;
    egg_info.child("entry_points.txt").touch()?;
    let package = site_packages.child("zstd");
    package.child("__init__.py").touch()?;

    // Run `pip uninstall`.
    uv_snapshot!(context.pip_uninstall()
        .arg("zstandard"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - zstandard==0.22.0
    ");

    egg_info.assert(predicates::path::missing());
    package.assert(predicates::path::missing());

    Ok(())
}

/// Refuse to uninstall a versionless `.egg-info` file without the metadata required to do so safely.
#[test]
fn uninstall_versionless_egg_info_file() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let site_packages = ChildPath::new(context.site_packages());
    let egg_info = site_packages.child("demo.egg-info");
    let metadata = "Metadata-Version: 1.1\nName: demo\nVersion: 1.0\n";
    egg_info.write_str(metadata)?;
    let module = site_packages.child("demo.py");
    let source = "VALUE = 42\n";
    module.write_str(source)?;

    uv_snapshot!(context.pip_uninstall().arg("demo"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Unable to uninstall `demo==1.0`. distutils-installed distributions do not include the metadata required to uninstall safely.
    "
    );

    egg_info.assert(metadata);
    module.assert(source);

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
    let source = target.child("zstd").child("__init__.py");
    source.write_str("MARKER = 1\n")?;

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

    let target_path = target.path().to_str().context("non-UTF-8 editable path")?;
    site_packages
        .child("zstandard.egg-link")
        .write_str(target_path)?;

    site_packages.child("easy-install.pth").write_str(&format!(
        "something\n{}\nanother thing\n",
        normcase(target_path)
    ))?;

    // Run `pip uninstall`.
    uv_snapshot!(context.pip_uninstall()
        .arg("zstandard"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - zstandard==0.22.0
    ");

    site_packages
        .child("easy-install.pth")
        .assert("something\nanother thing\n");
    site_packages
        .child("zstandard.egg-link")
        .assert(predicates::path::missing());
    target
        .child("zstandard.egg-info")
        .assert(predicates::path::is_dir());
    source.assert("MARKER = 1\n");

    Ok(())
}

#[test]
fn dry_run_uninstall_egg_info() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let site_packages = ChildPath::new(context.site_packages());

    let egg_info = site_packages.child("zstandard-0.22.0-py3.12.egg-info");
    egg_info.child("top_level.txt").write_str("zstd")?;
    egg_info.child("SOURCES.txt").touch()?;
    egg_info.child("PKG-INFO").touch()?;
    egg_info.child("dependency_links.txt").touch()?;
    egg_info.child("entry_points.txt").touch()?;
    let package = site_packages.child("zstd");
    package.child("__init__.py").touch()?;

    // Run `pip uninstall`.
    uv_snapshot!(context.pip_uninstall()
        .arg("--dry-run")
        .arg("zstandard"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would uninstall 1 package
     - zstandard==0.22.0
    ");

    egg_info.child("top_level.txt").assert("zstd");
    package.child("__init__.py").assert("");

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

    let (filename, bytes) = generate_wheel(
        &"evilpkg".parse()?,
        &"0.1.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let wheel = context.temp_dir.child(filename);
    wheel.write_binary(&bytes)?;
    context.pip_install().arg(wheel.path()).assert().success();

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

    let site_packages = ChildPath::new(context.site_packages());
    let init_py = site_packages.child("evilpkg/__init__.py");
    site_packages
        .child(&traversal_record)
        .assert("I should not be deleted");
    init_py.assert(predicates::path::is_file().follow_links(true));

    uv_snapshot!(context.filters(), context.pip_uninstall()
        .arg("evilpkg"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Invalid `RECORD` entry in `evilpkg==0.1.0 (from file://[TEMP_DIR]/evilpkg-0.1.0-py3-none-any.whl)` that escapes the Python environment, skipping: [..]/traversal_target.txt
    Uninstalled 1 package in [TIME]
     - evilpkg==0.1.0 (from file://[TEMP_DIR]/evilpkg-0.1.0-py3-none-any.whl)
    ");

    target_file.assert("I should not be deleted");
    init_py.assert(predicates::path::missing());

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
    assert_eq!(
        site_packages.child(&traversal_entry).canonicalize()?,
        target_dir.canonicalize()?,
        "the traversal entry must resolve to the protected target"
    );

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

    target_file.assert("I should not be deleted");
    init_py.assert(predicates::path::missing());
    egg_info.assert(predicates::path::missing());

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

    target_file.assert("I should not be deleted");
    init_py.assert(predicates::path::missing());
    egg_info.assert(predicates::path::missing());

    Ok(())
}

/// The uninstall warning redacts the password and sensitive query value.
#[test]
#[cfg(feature = "test-python")]
fn uninstall_url_redacts_credentials() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.filters(), context.pip_uninstall()
        .arg("https://user:password@example.com/demo-1.0-py3-none-any.whl?sig=signature"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Skipping `https://user:****@example.com/demo-1.0-py3-none-any.whl?sig=****` as it is not installed
    warning: No packages to uninstall
    ");
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
