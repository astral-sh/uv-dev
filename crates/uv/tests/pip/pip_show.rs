use std::env::current_dir;

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::fixture::FileWriteStr;
use assert_fs::fixture::PathChild;
use indoc::indoc;

use uv_static::EnvVars;

use uv_test::uv_snapshot;

#[test]
fn show_empty() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    uv_snapshot!(context.pip_show(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    warning: Please provide a package name or names.
    "
    );
}

#[test]
fn show_requires_multiple() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("tree-parent==2.31.0")?;

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 5 packages in [TIME]
    Installed 5 packages in [TIME]
     + tree-leaf-a==2024.2.2
     + tree-leaf-b==3.3.2
     + tree-leaf-c==3.6
     + tree-leaf-d==2.2.1
     + tree-parent==2.31.0
    "
    );

    context.assert_command("import tree_parent").success();
    uv_snapshot!(context.filters(), context.pip_show()
        .arg("tree-parent"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: tree-parent
    Version: 2.31.0
    Location: [SITE_PACKAGES]/
    Requires: tree-leaf-a, tree-leaf-b, tree-leaf-c, tree-leaf-d
    Required-by:
    "
    );

    Ok(())
}

/// Asserts that the Python version marker in the metadata is correctly evaluated.
/// `click` v8.1.7 requires `importlib-metadata`, but only when `python_version < "3.8"`.
#[test]
fn show_python_version_marker() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("marker-parent==8.1.7")?;

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + marker-parent==8.1.7
    "
    );

    context.assert_command("import marker_parent").success();

    uv_snapshot!(context.filters(), context.pip_show()
        .arg("marker-parent"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: marker-parent
    Version: 8.1.7
    Location: [SITE_PACKAGES]/
    Requires:
    Required-by:
    "
    );

    Ok(())
}

#[test]
fn show_found_single_package() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("simple-package==2.1.3")?;

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + simple-package==2.1.3
    "
    );

    context.assert_command("import simple_package").success();

    uv_snapshot!(context.filters(), context.pip_show()
        .arg("simple-package"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: simple-package
    Version: 2.1.3
    Location: [SITE_PACKAGES]/
    Requires:
    Required-by:
    "
    );

    Ok(())
}

#[test]
fn show_found_multiple_packages() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {r"
        simple-package==2.1.3
        other-package==2.0.1
    "
    })?;

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + other-package==2.0.1
     + simple-package==2.1.3
    "
    );

    context.assert_command("import simple_package").success();

    uv_snapshot!(context.filters(), context.pip_show()
        .arg("simple-package")
        .arg("other-package"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: other-package
    Version: 2.0.1
    Location: [SITE_PACKAGES]/
    Requires:
    Required-by:
    ---
    Name: simple-package
    Version: 2.1.3
    Location: [SITE_PACKAGES]/
    Requires:
    Required-by:
    "
    );

    Ok(())
}

#[test]
fn show_found_one_out_of_three() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {r"
        simple-package==2.1.3
        other-package==2.0.1
    "
    })?;

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + other-package==2.0.1
     + simple-package==2.1.3
    "
    );

    context.assert_command("import simple_package").success();

    uv_snapshot!(context.filters(), context.pip_show()
        .arg("simple-package")
        .arg("absent-package")
        .arg("second-absent-package"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: simple-package
    Version: 2.1.3
    Location: [SITE_PACKAGES]/
    Requires:
    Required-by:

    ----- stderr -----
    warning: Package(s) not found for: absent-package, second-absent-package
    "
    );

    Ok(())
}

#[test]
fn show_found_one_out_of_two_quiet() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {r"
        simple-package==2.1.3
        other-package==2.0.1
    "
    })?;

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + other-package==2.0.1
     + simple-package==2.1.3
    "
    );

    context.assert_command("import simple_package").success();

    // `absent-package` isn't installed, but `simple-package` is, so the command should succeed.
    uv_snapshot!(context.pip_show()
        .arg("simple-package")
        .arg("absent-package")
        .arg("--quiet"), @"
    exit_code: 0 (success)
    "
    );

    Ok(())
}

#[test]
fn show_empty_quiet() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {r"
        simple-package==2.1.3
        other-package==2.0.1
    "
    })?;

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + other-package==2.0.1
     + simple-package==2.1.3
    "
    );

    context.assert_command("import simple_package").success();

    // `absent-package` isn't installed, so the command should fail.
    uv_snapshot!(context.pip_show()
        .arg("absent-package")
        .arg("--quiet"), @"
    exit_code: 1 (failure)
    "
    );

    Ok(())
}

#[test]
fn show_editable() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    // Install the editable package.
    context
        .pip_install()
        .arg("-e")
        .arg("../../test/packages/flit_editable")
        .current_dir(current_dir()?)
        .env(
            EnvVars::CARGO_TARGET_DIR,
            "../../../target/target_install_editable",
        )
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_show()
        .arg("flit-editable"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: flit-editable
    Version: 0.1.0
    Location: [SITE_PACKAGES]/
    Editable project location: [WORKSPACE]/test/packages/flit_editable
    Requires:
    Required-by:
    "
    );

    Ok(())
}

#[test]
fn show_required_by_multiple() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {r"
        tree-parent-two==4.0.0
        tree-parent==2.31.0
    "
    })?;

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + tree-leaf-a==2024.2.2
     + tree-leaf-b==3.3.2
     + tree-leaf-c==3.6
     + tree-leaf-d==2.2.1
     + tree-parent==2.31.0
     + tree-parent-two==4.0.0
    "
    );

    context.assert_command("import tree_parent").success();

    // tree-leaf-c is required by tree-parent-two and tree-parent
    uv_snapshot!(context.filters(), context.pip_show()
        .arg("tree-leaf-c"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: tree-leaf-c
    Version: 3.6
    Location: [SITE_PACKAGES]/
    Requires:
    Required-by: tree-parent, tree-parent-two
    "
    );

    Ok(())
}

#[test]
fn show_files() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    uv_snapshot!(context
        .pip_install()
        .arg("tree-parent==2.31.0")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 5 packages in [TIME]
    Installed 5 packages in [TIME]
     + tree-leaf-a==2024.2.2
     + tree-leaf-b==3.3.2
     + tree-leaf-c==3.6
     + tree-leaf-d==2.2.1
     + tree-parent==2.31.0
    "
    );

    // Windows has a different files order.
    #[cfg(not(windows))]
    uv_snapshot!(context.filters(), context.pip_show().arg("tree-parent").arg("--files"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: tree-parent
    Version: 2.31.0
    Location: [SITE_PACKAGES]/
    Requires: tree-leaf-a, tree-leaf-b, tree-leaf-c, tree-leaf-d
    Required-by:
    Files:
      tree_parent-2.31.0.dist-info/INSTALLER
      tree_parent-2.31.0.dist-info/METADATA
      tree_parent-2.31.0.dist-info/RECORD
      tree_parent-2.31.0.dist-info/REQUESTED
      tree_parent-2.31.0.dist-info/WHEEL
      tree_parent/__init__.py
    ");
}

#[test]
fn show_target() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("simple-package==2.1.3")?;

    let target = context.temp_dir.child("target");

    // Install packages to a target directory.
    context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--target")
        .arg(target.path())
        .assert()
        .success();

    // Show package in the target directory.
    uv_snapshot!(context.filters(), context.pip_show()
        .arg("simple-package")
        .arg("--target")
        .arg(target.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: simple-package
    Version: 2.1.3
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    "
    );

    // Without --target, the package should not be found.
    uv_snapshot!(context.pip_show().arg("simple-package"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    warning: Package(s) not found for: simple-package
    "
    );

    Ok(())
}

#[test]
fn show_prefix() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("simple-package==2.1.3")?;

    let prefix = context.temp_dir.child("prefix");

    // Install packages to a prefix directory.
    context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--prefix")
        .arg(prefix.path())
        .assert()
        .success();

    // Show package in the prefix directory.
    uv_snapshot!(context.filters(), context.pip_show()
        .arg("simple-package")
        .arg("--prefix")
        .arg(prefix.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: simple-package
    Version: 2.1.3
    Location: [TEMP_DIR]/prefix/[PYTHON-LIB]/site-packages
    Requires:
    Required-by:
    "
    );

    // Without --prefix, the package should not be found.
    uv_snapshot!(context.pip_show().arg("simple-package"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    warning: Package(s) not found for: simple-package
    "
    );

    Ok(())
}
