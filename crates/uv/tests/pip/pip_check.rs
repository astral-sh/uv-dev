use anyhow::Result;
use assert_fs::fixture::ChildPath;
use assert_fs::fixture::FileWriteStr;
use assert_fs::fixture::PathChild;
use assert_fs::fixture::PathCreateDir;

use uv_test::uv_snapshot;

#[test]
fn check_compatible_packages() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("requests==2.31.0")?;

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
     + certifi==2024.2.2
     + charset-normalizer==3.3.2
     + idna==3.6
     + requests==2.31.0
     + urllib3==2.2.1
    "
    );

    uv_snapshot!(context.pip_check(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 5 packages in [TIME]
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
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("requests==2.31.0")?;

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
     + certifi==2024.2.2
     + charset-normalizer==3.3.2
     + idna==3.6
     + requests==2.31.0
     + urllib3==2.2.1
    "
    );

    let requirements_txt_idna = context.temp_dir.child("requirements_idna.txt");
    requirements_txt_idna.write_str("idna==2.4")?;

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements_idna.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - idna==3.6
     + idna==2.4
    warning: The package `requests` requires `idna>=2.5,<4`, but `2.4` is installed
    "
    );

    uv_snapshot!(context.pip_check(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Checked 5 packages in [TIME]
    Found 1 incompatibility
    The package `requests` requires `idna>=2.5,<4`, but `2.4` is installed
    "
    );

    Ok(())
}

// requests 2.31.0 requires idna (<4,>=2.5) and urllib3<3,>=1.21.1
// this test force-installs idna 2.4 and urllib3 1.20 to trigger a failure
// with multiple incompatible packages.
#[test]
fn check_multiple_incompatible_packages() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("requests==2.31.0")?;

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
     + certifi==2024.2.2
     + charset-normalizer==3.3.2
     + idna==3.6
     + requests==2.31.0
     + urllib3==2.2.1
    "
    );

    let requirements_txt_two = context.temp_dir.child("requirements_two.txt");
    requirements_txt_two.write_str("idna==2.4\nurllib3==1.20")?;

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
     - idna==3.6
     + idna==2.4
     - urllib3==2.2.1
     + urllib3==1.20
    warning: The package `requests` requires `idna>=2.5,<4`, but `2.4` is installed
    warning: The package `requests` requires `urllib3>=1.21.1,<3`, but `1.20` is installed
    "
    );

    uv_snapshot!(context.pip_check(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Checked 5 packages in [TIME]
    Found 2 incompatibilities
    The package `requests` requires `idna>=2.5,<4`, but `2.4` is installed
    The package `requests` requires `urllib3>=1.21.1,<3`, but `1.20` is installed
    "
    );

    Ok(())
}

#[test]
fn check_python_version() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context
        .pip_install()
        .arg("urllib3")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + urllib3==2.2.1
    "
    );

    uv_snapshot!(context.filters(), context.pip_check().arg("--python-version").arg("3.7"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Checked 1 package in [TIME]
    Found 1 incompatibility
    The package `urllib3` requires Python >=3.8, but `3.12.[X]` is installed
    "
    );
}

#[test]
fn check_incompatible_wheel_tags() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let dist_info = ChildPath::new(context.site_packages()).child("demo-1.0.0.dist-info");
    dist_info.create_dir_all()?;
    dist_info
        .child("METADATA")
        .write_str("Metadata-Version: 2.3\nName: demo\nVersion: 1.0.0\n")?;

    let write_tag = |tag: &str| {
        dist_info.child("WHEEL").write_str(&format!(
            "Wheel-Version: 1.0\nRoot-Is-Purelib: false\nTag: {tag}\n"
        ))
    };

    write_tag("cp313-none-any")?;
    uv_snapshot!(context.pip_check().arg("--python-platform").arg("windows"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Checked 1 package in [TIME]
    Found 1 incompatibility
    The package `demo` was built for a different platform. The distribution is compatible with CPython 3.13 (`cp313`), but you're using CPython 3.12 (`cp312`)
    ");

    write_tag("cp312-cp311-any")?;
    uv_snapshot!(context.pip_check().arg("--python-platform").arg("windows"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Checked 1 package in [TIME]
    Found 1 incompatibility
    The package `demo` was built for a different platform. The distribution is compatible with CPython 3.11 (`cp311`), but you're using CPython 3.12 (`cp312`)
    ");

    write_tag("py3-none-linux_x86_64")?;
    uv_snapshot!(context.pip_check().arg("--python-platform").arg("windows"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Checked 1 package in [TIME]
    Found 1 incompatibility
    The package `demo` was built for a different platform. The distribution is compatible with Linux (`linux_x86_64`), but you're on Windows (`win_amd64`)
    ");

    // Unknown tags still receive the generic incompatibility diagnostic.
    write_tag("unknown-none-any")?;
    uv_snapshot!(context.pip_check().arg("--python-platform").arg("windows"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Checked 1 package in [TIME]
    Found 1 incompatibility
    The package `demo` was built for a different platform
    ");

    write_tag("py3-none-any")?;
    uv_snapshot!(context.pip_check().arg("--python-platform").arg("windows"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    All installed packages are compatible
    ");

    Ok(())
}

#[test]
fn check_dependency_metadata_from_config_file() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("requests==2.31.0")?;

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
     + certifi==2024.2.2
     + charset-normalizer==3.3.2
     + idna==3.6
     + requests==2.31.0
     + urllib3==2.2.1
    "
    );

    let requirements_txt_idna = context.temp_dir.child("requirements_idna.txt");
    requirements_txt_idna.write_str("idna==2.4")?;

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements_idna.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - idna==3.6
     + idna==2.4
    warning: The package `requests` requires `idna>=2.5,<4`, but `2.4` is installed
    "
    );

    let uv_toml = context.temp_dir.child("uv.toml");
    uv_toml.write_str(
        r#"
        dependency-metadata = [
          { name = "requests", version = "2.31.0", requires-dist = ["certifi>=2017.4.17", "charset-normalizer>=2,<4", "idna>=2.4,<4", "urllib3>=1.21.1,<3"] },
        ]
        "#,
    )?;

    uv_snapshot!(context
        .pip_check()
        .arg("--config-file")
        .arg("uv.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 5 packages in [TIME]
    All installed packages are compatible
    "
    );

    Ok(())
}
