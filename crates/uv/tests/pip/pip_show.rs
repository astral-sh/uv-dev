use std::collections::BTreeMap;
use std::env::current_dir;
use std::path::PathBuf;

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::fixture::FileWriteBin;
use assert_fs::fixture::FileWriteStr;
use assert_fs::fixture::PathChild;
use indoc::indoc;

use uv_static::EnvVars;

use uv_test::packse::generate_wheel;
use uv_test::uv_snapshot;

#[test]
fn show_empty() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.pip_show(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    warning: Please provide a package name or names.
    "
    );
}

fn install_metadata_warning_fixture(context: &uv_test::TestContext) -> Result<PathBuf> {
    let (filename, bytes) = generate_wheel(
        &"metadata-warning".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
    );
    let wheel = context.temp_dir.child(filename);
    wheel.write_binary(&bytes)?;
    context
        .pip_install()
        .arg(wheel.path())
        .arg("--no-index")
        .arg("--no-deps")
        .arg("--offline")
        .assert()
        .success();
    Ok(context
        .site_packages()
        .join("metadata_warning-1.0.0.dist-info/METADATA"))
}

#[test]
fn show_lenient_requirement_warning_omits_source() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let metadata_path = install_metadata_warning_fixture(&context)?;
    let metadata = indoc! {"
        Metadata-Version: 2.3
        Name: metadata-warning
        Version: 1.0.0
        Requires-Dist: dependency @ 'https://user:requires-secret@example.com/private-1.0-py3-none-any.whl?sig=signature-secret'
    "};
    fs_err::write(&metadata_path, metadata)?;

    let assert = context
        .pip_show()
        .arg("metadata-warning")
        .env(EnvVars::RUST_LOG, "warn")
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stdout.contains("Requires: dependency"));
    assert!(stderr.contains("Fixing invalid requirement by removing stray quotes"));
    for output in [stdout, stderr] {
        assert!(!output.contains("requires-secret"));
        assert!(!output.contains("signature-secret"));
    }
    assert_eq!(fs_err::read_to_string(metadata_path)?, metadata);
    Ok(())
}

#[test]
fn show_invalid_extra_warning_omits_source() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let metadata_path = install_metadata_warning_fixture(&context)?;
    let metadata = indoc! {"
        Metadata-Version: 2.3
        Name: metadata-warning
        Version: 1.0.0
        Provides-Extra: https://user:extra-secret@example.com/private.whl?sig=signature-secret
        Provides-Extra: valid
    "};
    fs_err::write(&metadata_path, metadata)?;

    let assert = context
        .pip_show()
        .arg("metadata-warning")
        .env(EnvVars::RUST_LOG, "warn")
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stdout.contains("Name: metadata-warning"));
    assert!(stderr.contains("Ignoring invalid extra"));
    for output in [stdout, stderr] {
        assert!(!output.contains("extra-secret"));
        assert!(!output.contains("signature-secret"));
    }
    assert_eq!(fs_err::read_to_string(metadata_path)?, metadata);
    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn show_requires_multiple() -> Result<()> {
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

    context.assert_command("import requests").success();
    uv_snapshot!(context.filters(), context.pip_show()
        .arg("requests"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: requests
    Version: 2.31.0
    Location: [SITE_PACKAGES]/
    Requires: certifi, charset-normalizer, idna, urllib3
    Required-by:
    "
    );

    Ok(())
}

/// Asserts that the Python version marker in the metadata is correctly evaluated.
/// `click` v8.1.7 requires `importlib-metadata`, but only when `python_version < "3.8"`.
#[test]
#[cfg(feature = "test-pypi")]
fn show_python_version_marker() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("click==8.1.7")?;

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
     + click==8.1.7
    "
    );

    context.assert_command("import click").success();

    let mut filters = context.filters();
    if cfg!(windows) {
        filters.push(("Requires: colorama", "Requires:"));
    }

    uv_snapshot!(filters, context.pip_show()
        .arg("click"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: click
    Version: 8.1.7
    Location: [SITE_PACKAGES]/
    Requires:
    Required-by:
    "
    );

    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn show_found_single_package() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("MarkupSafe==2.1.3")?;

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
     + markupsafe==2.1.3
    "
    );

    context.assert_command("import markupsafe").success();

    uv_snapshot!(context.filters(), context.pip_show()
        .arg("markupsafe"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: markupsafe
    Version: 2.1.3
    Location: [SITE_PACKAGES]/
    Requires:
    Required-by:
    "
    );

    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn show_found_multiple_packages() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {r"
        MarkupSafe==2.1.3
        pip==21.3.1
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
     + markupsafe==2.1.3
     + pip==21.3.1
    "
    );

    context.assert_command("import markupsafe").success();

    uv_snapshot!(context.filters(), context.pip_show()
        .arg("markupsafe")
        .arg("pip"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: markupsafe
    Version: 2.1.3
    Location: [SITE_PACKAGES]/
    Requires:
    Required-by:
    ---
    Name: pip
    Version: 21.3.1
    Location: [SITE_PACKAGES]/
    Requires:
    Required-by:
    "
    );

    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn show_found_one_out_of_three() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {r"
        MarkupSafe==2.1.3
        pip==21.3.1
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
     + markupsafe==2.1.3
     + pip==21.3.1
    "
    );

    context.assert_command("import markupsafe").success();

    uv_snapshot!(context.filters(), context.pip_show()
        .arg("markupsafe")
        .arg("flask")
        .arg("django"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: markupsafe
    Version: 2.1.3
    Location: [SITE_PACKAGES]/
    Requires:
    Required-by:

    ----- stderr -----
    warning: Package(s) not found for: django, flask
    "
    );

    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn show_found_one_out_of_two_quiet() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {r"
        MarkupSafe==2.1.3
        pip==21.3.1
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
     + markupsafe==2.1.3
     + pip==21.3.1
    "
    );

    context.assert_command("import markupsafe").success();

    // Flask isn't installed, but markupsafe is, so the command should succeed.
    uv_snapshot!(context.pip_show()
        .arg("markupsafe")
        .arg("flask")
        .arg("--quiet"), @"
    exit_code: 0 (success)
    "
    );

    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn show_empty_quiet() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {r"
        MarkupSafe==2.1.3
        pip==21.3.1
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
     + markupsafe==2.1.3
     + pip==21.3.1
    "
    );

    context.assert_command("import markupsafe").success();

    // Flask isn't installed, so the command should fail.
    uv_snapshot!(context.pip_show()
        .arg("flask")
        .arg("--quiet"), @"
    exit_code: 1 (failure)
    "
    );

    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn show_editable() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Install the editable package.
    context
        .pip_install()
        .arg("-e")
        .arg("../../test/packages/poetry_editable")
        .current_dir(current_dir()?)
        .env(
            EnvVars::CARGO_TARGET_DIR,
            "../../../target/target_install_editable",
        )
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_show()
        .arg("poetry-editable"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: poetry-editable
    Version: 0.1.0
    Location: [SITE_PACKAGES]/
    Editable project location: [WORKSPACE]/test/packages/poetry_editable
    Requires: anyio
    Required-by:
    "
    );

    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn show_required_by_multiple() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {r"
        anyio==4.0.0
        requests==2.31.0
    "
    })?;

    uv_snapshot!(context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    Prepared 7 packages in [TIME]
    Installed 7 packages in [TIME]
     + anyio==4.0.0
     + certifi==2024.2.2
     + charset-normalizer==3.3.2
     + idna==3.6
     + requests==2.31.0
     + sniffio==1.3.1
     + urllib3==2.2.1
    "
    );

    context.assert_command("import requests").success();

    // idna is required by anyio and requests
    uv_snapshot!(context.filters(), context.pip_show()
        .arg("idna"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: idna
    Version: 3.6
    Location: [SITE_PACKAGES]/
    Requires:
    Required-by: anyio, requests
    "
    );

    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn show_files() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context
        .pip_install()
        .arg("requests==2.31.0")
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

    // Windows has a different files order.
    #[cfg(not(windows))]
    uv_snapshot!(context.filters(), context.pip_show().arg("requests").arg("--files"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: requests
    Version: 2.31.0
    Location: [SITE_PACKAGES]/
    Requires: certifi, charset-normalizer, idna, urllib3
    Required-by:
    Files:
      requests-2.31.0.dist-info/INSTALLER
      requests-2.31.0.dist-info/LICENSE
      requests-2.31.0.dist-info/METADATA
      requests-2.31.0.dist-info/RECORD
      requests-2.31.0.dist-info/REQUESTED
      requests-2.31.0.dist-info/WHEEL
      requests-2.31.0.dist-info/top_level.txt
      requests/__init__.py
      requests/__version__.py
      requests/_internal_utils.py
      requests/adapters.py
      requests/api.py
      requests/auth.py
      requests/certs.py
      requests/compat.py
      requests/cookies.py
      requests/exceptions.py
      requests/help.py
      requests/hooks.py
      requests/models.py
      requests/packages.py
      requests/sessions.py
      requests/status_codes.py
      requests/structures.py
      requests/utils.py
    ");
}

#[test]
#[cfg(feature = "test-pypi")]
fn show_target() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("MarkupSafe==2.1.3")?;

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
        .arg("markupsafe")
        .arg("--target")
        .arg(target.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: markupsafe
    Version: 2.1.3
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    "
    );

    // Without --target, the package should not be found.
    uv_snapshot!(context.pip_show().arg("markupsafe"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    warning: Package(s) not found for: markupsafe
    "
    );

    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn show_prefix() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("MarkupSafe==2.1.3")?;

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
        .arg("markupsafe")
        .arg("--prefix")
        .arg(prefix.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: markupsafe
    Version: 2.1.3
    Location: [TEMP_DIR]/prefix/[PYTHON-LIB]/site-packages
    Requires:
    Required-by:
    "
    );

    // Without --prefix, the package should not be found.
    uv_snapshot!(context.pip_show().arg("markupsafe"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    warning: Package(s) not found for: markupsafe
    "
    );

    Ok(())
}
