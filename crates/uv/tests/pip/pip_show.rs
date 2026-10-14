#[cfg(any(feature = "test-python", feature = "test-pypi"))]
use anyhow::Result;
#[cfg(any(feature = "test-python", feature = "test-pypi"))]
use assert_cmd::prelude::*;
#[cfg(feature = "test-python")]
use assert_fs::fixture::ChildPath;
#[cfg(any(feature = "test-python", feature = "test-pypi"))]
use assert_fs::fixture::{FileWriteStr, PathChild};
#[cfg(feature = "test-python")]
use indoc::indoc;

#[cfg(feature = "test-pypi")]
use uv_static::EnvVars;

#[cfg(feature = "test-pypi")]
use uv_test::copy_dir_ignore;
#[cfg(feature = "test-python")]
use uv_test::packse::{PackseServer, scenario::Scenario};
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

/// Show active requirements while omitting requirements for another Python version.
#[test]
#[cfg(feature = "test-python")]
fn show_python_version_marker() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let scenario = toml::from_str::<Scenario>(indoc! {r#"
        name = "show-python-marker"
        [root]
        [expected]
        satisfiable = true
        [packages.click.versions."8.1.7"]
        requires = [
            "active-dependency ; python_version >= '3.8'",
            "importlib-metadata ; python_version < '3.8'",
        ]
        [packages.active-dependency.versions."1.0.0"]
    "#})?;
    let index = PackseServer::from_scenario(&scenario);

    uv_snapshot!(context
        .pip_install()
        .args(["click==8.1.7", "--strict", "--default-index"])
        .arg(index.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + active-dependency==1.0.0
     + click==8.1.7
    ");

    uv_snapshot!(context.filters(), context.pip_show().arg("click"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: click
    Version: 8.1.7
    Location: [SITE_PACKAGES]/
    Requires: active-dependency
    Required-by:
    ");

    Ok(())
}

/// Lookup results, missing-package warnings, and quiet exit codes share the same environment.
#[test]
#[cfg(feature = "test-python")]
fn show_package_lookup_results() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let scenario = toml::from_str::<Scenario>(indoc! {r#"
        name = "show-package-lookups"
        [root]
        [expected]
        satisfiable = true
        [packages.markupsafe.versions."2.1.3"]
        [packages.pip.versions."21.3.1"]
    "#})?;
    let index = PackseServer::from_scenario(&scenario);
    let install = |requirement: &str| {
        context
            .pip_install()
            .args([requirement, "--strict", "--default-index"])
            .arg(index.index_url())
            .assert()
            .success();
    };

    install("MarkupSafe==2.1.3");
    uv_snapshot!(context.filters(), context.pip_show().arg("MARKUPSAFE"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: markupsafe
    Version: 2.1.3
    Location: [SITE_PACKAGES]/
    Requires:
    Required-by:
    ");

    install("pip==21.3.1");
    uv_snapshot!(context.filters(), context.pip_show().args(["pip", "MarkupSafe", "markupsafe"]), @"
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
    ");

    uv_snapshot!(context.filters(), context.pip_show().args(["markupsafe", "flask", "django"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: markupsafe
    Version: 2.1.3
    Location: [SITE_PACKAGES]/
    Requires:
    Required-by:

    ----- stderr -----
    warning: Package(s) not found for: django, flask
    ");

    uv_snapshot!(context.pip_show().args(["markupsafe", "flask", "--quiet"]), @"
    exit_code: 0 (success)
    ");
    uv_snapshot!(context.pip_show().args(["flask", "--quiet"]), @"
    exit_code: 1 (failure)
    ");
    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn show_editable() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    copy_dir_ignore(
        context.workspace_root.join("test/packages/poetry_editable"),
        context.temp_dir.child("poetry_editable"),
    )?;

    // Install the editable package.
    context
        .pip_install()
        .arg("-e")
        .arg("poetry_editable")
        .env(EnvVars::CARGO_TARGET_DIR, context.temp_dir.join("target"))
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_show()
        .arg("poetry-editable"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: poetry-editable
    Version: 0.1.0
    Location: [SITE_PACKAGES]/
    Editable project location: [TEMP_DIR]/poetry_editable
    Requires: anyio
    Required-by:
    "
    );

    Ok(())
}

#[test]
#[cfg(feature = "test-python")]
fn show_required_by_multiple() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let site_packages = ChildPath::new(context.site_packages());
    site_packages
        .child("idna-3.6.dist-info/METADATA")
        .write_str("Metadata-Version: 2.1\nName: idna\nVersion: 3.6\n")?;
    site_packages
        .child("anyio-4.0.0.dist-info/METADATA")
        .write_str("Metadata-Version: 2.1\nName: anyio\nVersion: 4.0.0\nRequires-Dist: idna\n")?;
    site_packages
        .child("requests-2.31.0.dist-info/METADATA")
        .write_str("Metadata-Version: 2.1\nName: requests\nVersion: 2.31.0\nRequires-Dist: idna; python_version >= '3.8'\n")?;
    site_packages
        .child("legacy-1.0.0.dist-info/METADATA")
        .write_str("Metadata-Version: 2.1\nName: legacy\nVersion: 1.0.0\nRequires-Dist: idna; python_version < '3.8'\n")?;

    // Reverse dependencies include active requirements and omit inactive markers.
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

/// Strict validation reports problems throughout the environment, including packages not shown.
#[test]
#[cfg(feature = "test-python")]
fn show_strict_validates_entire_environment() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let site_packages = ChildPath::new(context.site_packages());
    let wheel_metadata = indoc! {"
        Wheel-Version: 1.0
        Root-Is-Purelib: true
        Tag: py3-none-any
    "};
    let visible = site_packages.child("visible-1.0.0.dist-info");
    visible
        .child("METADATA")
        .write_str("Metadata-Version: 2.1\nName: visible\nVersion: 1.0.0\n")?;
    visible.child("WHEEL").write_str(wheel_metadata)?;
    let broken = site_packages.child("broken-1.0.0.dist-info");
    broken.child("METADATA").write_str(indoc! {"
        Metadata-Version: 2.1
        Name: broken
        Version: 1.0.0
        Requires-Dist: missing-dependency>=2
    "})?;
    broken.child("WHEEL").write_str(wheel_metadata)?;

    uv_snapshot!(context.filters(), context.pip_show().args(["visible", "--strict"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: visible
    Version: 1.0.0
    Location: [SITE_PACKAGES]/
    Requires:
    Required-by:

    ----- stderr -----
    warning: The package `broken` requires `missing-dependency>=2`, but it's not installed
    ");

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

    let snapshot = uv_test::capture_uv_snapshot!(
        context.filters(),
        context.pip_show().arg("requests").arg("--files")
    );
    let (header, files) = snapshot.split_once("Files:\n").expect("missing file list");
    let mut files = files.lines().collect::<Vec<_>>();
    assert!(
        files.iter().all(|line| line.starts_with("  ")),
        "unexpected output after file list: {snapshot}"
    );
    // RECORD ordering varies by platform; the listed paths should not.
    files.sort_unstable();
    insta::assert_snapshot!(format!("{header}Files:\n{}\n", files.join("\n")), @"
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
