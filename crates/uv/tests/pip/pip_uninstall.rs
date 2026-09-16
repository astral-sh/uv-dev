#[cfg(windows)]
use std::path::{Component, Prefix};

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;

use uv_test::uv_snapshot;

#[test]
fn no_arguments() {
    let context = uv_test::test_context!("3.12");

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
    error: File not found: `requirements.txt`
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

/// Remove every adjacent module file while keeping namespaces and directory precedence intact.
#[test]
fn uninstall_egg_info_adjacent_module_files() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let site_packages = ChildPath::new(context.site_packages());
    let egg_info = site_packages.child("adjacent_bytecode-0.1.0.egg-info");
    egg_info.create_dir_all()?;
    egg_info
        .child("PKG-INFO")
        .write_str("Metadata-Version: 2.1\nName: adjacent-bytecode\nVersion: 0.1.0\n")?;
    egg_info
        .child("top_level.txt")
        .write_str("legacy_module\nshared_module\nlegacy_package\n")?;
    egg_info
        .child("namespace_packages.txt")
        .write_str("shared_module\n")?;

    let package = site_packages.child("legacy_package");
    package
        .child("__init__.py")
        .write_str("VALUE = 'package'\n")?;
    for extension in ["py", "pyc", "pyo"] {
        site_packages
            .child(format!("legacy_module.{extension}"))
            .write_str("owned module")?;
        site_packages
            .child(format!("shared_module.{extension}"))
            .write_str("namespace sentinel")?;
        site_packages
            .child(format!("legacy_package.{extension}"))
            .write_str("adjacent sentinel")?;
    }

    uv_snapshot!(context.pip_uninstall()
        .arg("--python")
        .arg(context.interpreter())
        .arg("adjacent-bytecode"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - adjacent-bytecode==0.1.0
    ");

    assert!(!egg_info.exists());
    assert!(!package.exists());
    for extension in ["py", "pyc", "pyo"] {
        assert!(
            !site_packages
                .child(format!("legacy_module.{extension}"))
                .exists()
        );
        assert_eq!(
            fs_err::read_to_string(site_packages.child(format!("shared_module.{extension}")))?,
            "namespace sentinel"
        );
        assert_eq!(
            fs_err::read_to_string(site_packages.child(format!("legacy_package.{extension}")))?,
            "adjacent sentinel"
        );
    }

    context
        .assert_command("import sys; assert sys.prefix != sys.base_prefix")
        .success();

    Ok(())
}

/// Uninstall files and generated scripts recorded by a legacy `.egg-info` installation.
#[test]
fn uninstall_egg_info_recorded_files() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let site_packages = ChildPath::new(context.site_packages());
    let egg_info = site_packages.child("zstandard-0.22.0-py3.12.egg-info");
    egg_info.create_dir_all()?;
    egg_info
        .child("PKG-INFO")
        .write_str("Metadata-Version: 2.1\nName: zstandard\nVersion: 0.22.0\n")?;
    egg_info
        .child("entry_points.txt")
        .write_str("[console_scripts]\nzstd = zstd:main\n")?;
    egg_info.child("top_level.txt").write_str("zstd\nowned\n")?;

    let module = site_packages.child("zstd/__init__.py");
    module.write_str("def main(): pass\n")?;
    let sibling = site_packages.child("zstd/sibling.py");
    sibling.write_str("VALUE = 'sibling'\n")?;
    let bytecode = site_packages.child("zstd/__pycache__/__init__.cpython-312.pyc");
    bytecode.write_str("bytecode")?;
    let sibling_bytecode = site_packages.child("zstd/__pycache__/sibling.cpython-312.pyc");
    sibling_bytecode.write_str("sibling bytecode")?;
    let owned = site_packages.child("owned/module.py");
    owned.write_str("VALUE = 'owned'\n")?;
    let owned_bytecode = site_packages.child("owned/__pycache__/module.cpython-312.opt-1.pyc");
    owned_bytecode.write_str("owned bytecode")?;
    let data = context.venv.child("share/zstandard/data.txt");
    data.write_str("data")?;
    let depth = egg_info
        .path()
        .strip_prefix(context.venv.canonicalize()?)?
        .components()
        .count();
    let data_path = format!("{}share/zstandard/data.txt", "../".repeat(depth));
    egg_info.child("installed-files.txt").write_str(&format!(
        "../zstd/__init__.py\n../owned/module.py\n{data_path}\n"
    ))?;

    let script = if cfg!(windows) {
        context.venv.child("Scripts/zstd.exe")
    } else {
        context.venv.child("bin/zstd")
    };
    script.write_str("launcher")?;

    uv_snapshot!(context.pip_uninstall()
        .arg("--python")
        .arg(context.interpreter())
        .arg("zstandard"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - zstandard==0.22.0
    ");

    assert!(!module.exists());
    assert!(sibling.exists());
    assert!(!bytecode.exists());
    assert!(sibling_bytecode.exists());
    assert!(!owned.exists());
    assert!(!owned_bytecode.exists());
    assert!(!site_packages.child("owned").exists());
    assert!(!data.exists());
    assert!(!script.exists());
    assert!(!egg_info.exists());

    Ok(())
}

/// Remove legacy launchers without validating the entry-point targets, which are never imported.
#[test]
fn uninstall_egg_info_invalid_entry_point_targets() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let site_packages = ChildPath::new(context.site_packages());
    let egg_info = site_packages.child("zstandard-0.22.0-py3.12.egg-info");
    egg_info.create_dir_all()?;
    egg_info
        .child("PKG-INFO")
        .write_str("Metadata-Version: 2.1\nName: zstandard\nVersion: 0.22.0\n")?;
    egg_info
        .child("entry_points.txt")
        .write_str("[console_scripts]\nzstd = invalid-entry-point\n")?;
    egg_info
        .child("installed-files.txt")
        .write_str("../zstd/__init__.py\n")?;

    let module = site_packages.child("zstd/__init__.py");
    module.write_str("def main(): pass\n")?;
    let script = if cfg!(windows) {
        context.venv.child("Scripts/zstd.exe")
    } else {
        context.venv.child("bin/zstd")
    };
    script.write_str("launcher")?;

    uv_snapshot!(context.pip_uninstall()
        .arg("--python")
        .arg(context.interpreter())
        .arg("zstandard"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 package in [TIME]
     - zstandard==0.22.0
    ");

    assert!(!module.exists());
    assert!(!script.exists());
    assert!(!egg_info.exists());

    Ok(())
}

/// A nested directory link cannot give legacy entry-point metadata authority outside `scripts`.
#[test]
fn uninstall_egg_info_script_directory_alias() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let site_packages = ChildPath::new(context.site_packages());
    let egg_info = site_packages.child("legacy_authority-0.1.0.egg-info");
    egg_info.create_dir_all()?;
    egg_info
        .child("PKG-INFO")
        .write_str("Metadata-Version: 2.1\nName: legacy-authority\nVersion: 0.1.0\n")?;
    egg_info
        .child("installed-files.txt")
        .write_str("../legacy_authority.py\n")?;
    egg_info
        .child("entry_points.txt")
        .write_str("[console_scripts]\nnested/tool = missing:main\n")?;
    let payload = site_packages.child("legacy_authority.py");
    payload.write_str("VALUE = 'owned'\n")?;

    let outside = context.temp_dir.child("outside-launchers");
    for name in ["tool", "tool.exe", "tool.exe.manifest", "tool-script.py"] {
        outside.child(name).write_str("outside launcher")?;
    }
    let scripts = context
        .venv
        .child(if cfg!(windows) { "Scripts" } else { "bin" });
    uv_fs::create_symlink(outside.path(), scripts.child("nested").path())?;

    uv_snapshot!(context.pip_uninstall()
        .arg("--python")
        .arg(context.interpreter())
        .arg("legacy-authority"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The wheel is invalid: Script path must resolve to a file within the scripts directory: `nested/tool`
    ");

    assert_eq!(fs_err::read_to_string(&payload)?, "VALUE = 'owned'\n");
    assert!(egg_info.exists());
    for name in ["tool", "tool.exe", "tool.exe.manifest", "tool-script.py"] {
        assert_eq!(
            fs_err::read_to_string(outside.child(name))?,
            "outside launcher"
        );
    }
    context
        .assert_command("import sys; assert sys.prefix != sys.base_prefix")
        .success();

    Ok(())
}

/// The raw Windows launcher candidate must not remove the selected interpreter.
#[test]
#[cfg(windows)]
fn uninstall_egg_info_raw_python_exe() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let interpreter = context.interpreter();
    assert!(interpreter.starts_with(context.venv.path()));
    assert!(fs_err::symlink_metadata(&interpreter)?.is_file());
    let identity = context
        .python_command()
        .arg("-c")
        .arg("import os, sys; print(os.path.abspath(sys._base_executable)); raise SystemExit(1 if os.path.samefile(sys.argv[1], sys._base_executable) else 0)")
        .arg(&interpreter)
        .output()?;
    assert!(
        identity.status.success(),
        "the fixture interpreter must be a private launcher copy: {}",
        String::from_utf8_lossy(&identity.stderr),
    );
    let interpreter_bytes = fs_err::read(&interpreter)?;
    let configuration = context.venv.child("pyvenv.cfg");
    let configuration_bytes = fs_err::read(&configuration)?;

    let site_packages = ChildPath::new(context.site_packages());
    let egg_info = site_packages.child("legacy_authority-0.1.0.egg-info");
    egg_info.create_dir_all()?;
    egg_info
        .child("PKG-INFO")
        .write_str("Metadata-Version: 2.1\nName: legacy-authority\nVersion: 0.1.0\n")?;
    egg_info
        .child("installed-files.txt")
        .write_str("../legacy_authority.py\n")?;
    egg_info
        .child("entry_points.txt")
        .write_str("[console_scripts]\npython.exe = missing:main\n")?;
    let payload = site_packages.child("legacy_authority.py");
    payload.write_str("VALUE = 'owned'\n")?;

    uv_snapshot!(context.pip_uninstall()
        .arg("--python")
        .arg(&interpreter)
        .arg("legacy-authority"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The wheel is invalid: Script path targets a core Python environment file: `python.exe`
    ");

    assert_eq!(fs_err::read_to_string(&payload)?, "VALUE = 'owned'\n");
    assert!(egg_info.exists());
    assert_eq!(fs_err::read(&interpreter)?, interpreter_bytes);
    assert_eq!(fs_err::read(&configuration)?, configuration_bytes);
    context
        .assert_command("import sys; assert sys.prefix != sys.base_prefix")
        .success();

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
        fs_err::read_to_string(site_packages.child("easy-install.pth"))?,
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
    let record = fs_err::read_to_string(&record_file)?;
    let record = format!("{}\n{},,0\n", record.trim(), traversal_record);
    fs_err::write(record_file, &record)?;

    let init_py = context.site_packages().join("evilpkg/__init__.py");
    assert!(context.site_packages().join(&traversal_record).exists());
    assert!(init_py.exists());

    uv_snapshot!(context.filters(), context.pip_uninstall()
        .arg("evilpkg"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Invalid RECORD entry in evilpkg==0.1.0 (from file://[TEMP_DIR]/evilpkg) that escapes the Python environment, skipping: [..]/traversal_target.txt
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
