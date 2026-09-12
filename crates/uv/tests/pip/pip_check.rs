use anyhow::Result;
use assert_fs::fixture::ChildPath;
use assert_fs::fixture::FileWriteStr;
use assert_fs::fixture::PathChild;

use uv_test::packse::PackseServer;
use uv_test::uv_snapshot;

#[test]
fn check_compatible_packages() -> Result<()> {
    let server = PackseServer::new("packages/pip-check.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("parent==1.0.0")?;

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + child==3.6
     + parent==1.0.0
     + second-child==2.2.1
    "
    );

    uv_snapshot!(context.pip_check(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 3 packages in [TIME]
    All installed packages are compatible
    "
    );

    Ok(())
}

/// Check a versionless `.egg-info` file installed by distutils.
#[test]
fn check_versionless_egg_info_file() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    ChildPath::new(context.site_packages())
        .child("demo.egg-info")
        .write_str("Metadata-Version: 1.1\nName: demo\nVersion: 1.0\n")?;

    uv_snapshot!(context.pip_check(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    All installed packages are compatible
    "
    );

    Ok(())
}

// requests 2.31.0 requires idna (<4,>=2.5)
// this test force-installs idna 2.4 to trigger a failure.
#[test]
fn check_incompatible_packages() -> Result<()> {
    let server = PackseServer::new("packages/pip-check.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("parent==1.0.0")?;

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + child==3.6
     + parent==1.0.0
     + second-child==2.2.1
    "
    );

    let requirements_txt_child = context.temp_dir.child("requirements_child.txt");
    requirements_txt_child.write_str("child==2.4")?;

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements_child.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - child==3.6
     + child==2.4
    warning: The package `parent` requires `child>=2.5,<4`, but `2.4` is installed
    "
    );

    uv_snapshot!(context.pip_check(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Checked 3 packages in [TIME]
    Found 1 incompatibility
    The package `parent` requires `child>=2.5,<4`, but `2.4` is installed
    "
    );

    Ok(())
}

// requests 2.31.0 requires idna (<4,>=2.5) and urllib3<3,>=1.21.1
// this test force-installs idna 2.4 and urllib3 1.20 to trigger a failure
// with multiple incompatible packages.
#[test]
fn check_multiple_incompatible_packages() -> Result<()> {
    let server = PackseServer::new("packages/pip-check.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("parent==1.0.0")?;

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + child==3.6
     + parent==1.0.0
     + second-child==2.2.1
    "
    );

    let requirements_txt_two = context.temp_dir.child("requirements_two.txt");
    requirements_txt_two.write_str("child==2.4\nsecond-child==1.20")?;

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements_two.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Uninstalled 2 packages in [TIME]
    Installed 2 packages in [TIME]
     - child==3.6
     + child==2.4
     - second-child==2.2.1
     + second-child==1.20
    warning: The package `parent` requires `child>=2.5,<4`, but `2.4` is installed
    warning: The package `parent` requires `second-child>=1.21.1,<3`, but `1.20` is installed
    "
    );

    uv_snapshot!(context.pip_check(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Checked 3 packages in [TIME]
    Found 2 incompatibilities
    The package `parent` requires `child>=2.5,<4`, but `2.4` is installed
    The package `parent` requires `second-child>=1.21.1,<3`, but `1.20` is installed
    "
    );

    Ok(())
}

#[test]
fn check_python_version() {
    let server = PackseServer::new("packages/pip-check.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&server.index_url());

    uv_snapshot!(context
        .pip_install()
        .arg("requires-python")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + requires-python==1.0.0
    "
    );

    uv_snapshot!(context.filters(), context.pip_check().arg("--python-version").arg("3.7"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Checked 1 package in [TIME]
    Found 1 incompatibility
    The package `requires-python` requires Python >=3.8, but `3.12.[X]` is installed
    "
    );
}

#[test]
fn check_dependency_metadata_from_config_file() -> Result<()> {
    let server = PackseServer::new("packages/pip-check.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("parent==1.0.0")?;

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + child==3.6
     + parent==1.0.0
     + second-child==2.2.1
    "
    );

    let requirements_txt_child = context.temp_dir.child("requirements_child.txt");
    requirements_txt_child.write_str("child==2.4")?;

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements_child.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - child==3.6
     + child==2.4
    warning: The package `parent` requires `child>=2.5,<4`, but `2.4` is installed
    "
    );

    let uv_toml = context.temp_dir.child("uv.toml");
    uv_toml.write_str(
        r#"
        dependency-metadata = [
          { name = "parent", version = "1.0.0", requires-dist = ["child>=2.4,<4", "second-child>=1.21.1,<3"] },
        ]
        "#,
    )?;

    uv_snapshot!(context
        .pip_check()
        .arg("--config-file")
        .arg("uv.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 3 packages in [TIME]
    All installed packages are compatible
    "
    );

    Ok(())
}
