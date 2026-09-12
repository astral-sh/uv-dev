use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::fixture::ChildPath;
use assert_fs::fixture::FileWriteStr;
use assert_fs::fixture::PathChild;
use assert_fs::prelude::*;

use uv_static::EnvVars;
use uv_test::uv_snapshot;

#[test]
fn list_empty_columns() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    uv_snapshot!(context.pip_list()
        .arg("--format")
        .arg("columns"), @"
    exit_code: 0 (success)
    "
    );
}

#[test]
fn list_empty_freeze() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    uv_snapshot!(context.pip_list()
        .arg("--format")
        .arg("freeze"), @"
    exit_code: 0 (success)
    "
    );
}

#[test]
fn list_empty_json() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    uv_snapshot!(context.pip_list()
        .arg("--format")
        .arg("json"), @"
    exit_code: 0 (success)
    ----- stdout -----
    []
    "
    );
}

#[test]
fn list_editable_non_file_url() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let dist_info = ChildPath::new(context.site_packages()).child("project-1.0.0.dist-info");
    dist_info.create_dir_all()?;
    dist_info
        .child("METADATA")
        .write_str("Metadata-Version: 2.1\nName: project\nVersion: 1.0.0\n")?;
    dist_info
        .child("direct_url.json")
        .write_str(r#"{"url":"https://example.com/project","dir_info":{"editable":true}}"#)?;

    uv_snapshot!(context.pip_list(), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package Version
    ------- -------
    project 1.0.0
    ");

    uv_snapshot!(context.pip_list().arg("--format=json"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    [{"name":"project","version":"1.0.0"}]
    "#);

    Ok(())
}

#[test]
fn list_single_no_editable() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("simple-package==2.1.3")?;

    uv_snapshot!(context.pip_install()
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

    uv_snapshot!(context.pip_list(), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package        Version
    -------------- -------
    simple-package 2.1.3
    "
    );

    Ok(())
}

#[test]
fn list_outdated_columns() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("outdated-package==3.0.0")?;

    uv_snapshot!(context.pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + outdated-package==3.0.0
    "
    );

    uv_snapshot!(context.pip_list().arg("--outdated"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package          Version Latest Type
    ---------------- ------- ------ -----
    outdated-package 3.0.0   4.3.0  wheel
    "
    );

    Ok(())
}

#[test]
fn list_outdated_json() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("outdated-package==3.0.0")?;

    uv_snapshot!(context.pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + outdated-package==3.0.0
    "
    );

    uv_snapshot!(context.pip_list().arg("--outdated").arg("--format").arg("json"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    [{"name":"outdated-package","version":"3.0.0","latest_version":"4.3.0","latest_filetype":"wheel"}]
    "#
    );

    Ok(())
}

#[test]
fn list_outdated_find_links() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let links_dir = context.workspace_root.join("test/links");
    let first_links_dir = context.temp_dir.child("first-links");
    first_links_dir.create_dir_all()?;
    fs_err::copy(
        links_dir.join("validation-2.0.0-py3-none-any.whl"),
        first_links_dir
            .child("validation-2.0.0-py3-none-any.whl")
            .path(),
    )?;
    let second_links_dir = context.temp_dir.child("second-links");
    second_links_dir.create_dir_all()?;
    fs_err::copy(
        links_dir.join("validation-3.0.0-py3-none-any.whl"),
        second_links_dir
            .child("validation-3.0.0-py3-none-any.whl")
            .path(),
    )?;

    uv_snapshot!(context.filters(), context.pip_install()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("validation==1.0.0")
        .arg("--find-links")
        .arg(&links_dir)
        .arg("--no-index"), @r###"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + validation==1.0.0
    "###
    );

    uv_snapshot!(context.filters(), context.pip_list()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--outdated")
        .arg("--find-links")
        .arg(first_links_dir.path())
        .arg("--find-links")
        .arg(second_links_dir.path())
        .arg("--no-index"), @r###"
    exit_code: 0 (success)
    ----- stdout -----
    Package    Version Latest Type
    ---------- ------- ------ -----
    validation 1.0.0   3.0.0  wheel
    "###
    );

    Ok(())
}

#[test]
fn list_outdated_freeze() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    uv_snapshot!(context.pip_list().arg("--outdated").arg("--format").arg("freeze"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `--outdated` cannot be used with `--format freeze`
    "
    );
}

#[test]
#[cfg(feature = "test-git")]
fn list_outdated_git() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc::indoc! {r"
        iniconfig==1.0.0
        uv-public-pypackage @ git+https://github.com/astral-test/uv-public-pypackage@0.0.1
    "})?;

    uv_snapshot!(context.pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + iniconfig==1.0.0
     + uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage@0dacfd662c64cb4ceb16e6cf65a157a8b715b979)
    "
    );

    uv_snapshot!(context.pip_list().arg("--outdated"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package   Version Latest Type
    --------- ------- ------ -----
    iniconfig 1.0.0   2.0.0  wheel
    "
    );

    Ok(())
}

#[test]
fn list_outdated_index() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("outdated-package==3.0.0")?;

    uv_snapshot!(context.pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + outdated-package==3.0.0
    "
    );

    uv_snapshot!(context.pip_list()
        .arg("--outdated")
        .arg("--index-url")
        .arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package          Version Latest Type
    ---------------- ------- ------ -----
    outdated-package 3.0.0   4.3.0  wheel
    "
    );

    Ok(())
}

#[test]
fn list_editable() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_filter((r"\-\-\-\-\-\-+.*", "[UNDERLINE]"))
        .with_filter(("  +", " "));

    // Install the editable package.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("-e")
        .arg(context.workspace_root.join("test/packages/flit_editable")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + flit-editable==0.1.0 (from file://[WORKSPACE]/test/packages/flit_editable)
    "
    );

    uv_snapshot!(context.filters(), context.pip_list(), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package Version Editable project location
    [UNDERLINE]
    flit-editable 0.1.0 [WORKSPACE]/test/packages/flit_editable
    "
    );
}

#[test]
fn list_editable_only() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_filter((r"\-\-\-\-\-\-+.*", "[UNDERLINE]"))
        .with_filter(("  +", " "));

    // Install the editable package.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("-e")
        .arg(context.workspace_root.join("test/packages/flit_editable")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + flit-editable==0.1.0 (from file://[WORKSPACE]/test/packages/flit_editable)
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
        .arg("--editable"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package Version Editable project location
    [UNDERLINE]
    flit-editable 0.1.0 [WORKSPACE]/test/packages/flit_editable
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
        .arg("--exclude-editable"), @"exit_code: 0 (success)"
    );

    uv_snapshot!(context.filters(), context.pip_list()
        .arg("--editable")
        .arg("--exclude-editable"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--editable' cannot be used with '--exclude-editable'

    Usage: uv pip list --cache-dir [CACHE_DIR] --editable --exclude-newer <EXCLUDE_NEWER>

    For more information, try '--help'.
    "
    );
}

#[test]
fn list_exclude() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_filter((r"\-\-\-\-\-\-+.*", "[UNDERLINE]"))
        .with_filter(("  +", " "));

    // Install the editable package.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("-e")
        .arg(context.workspace_root.join("test/packages/flit_editable")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + flit-editable==0.1.0 (from file://[WORKSPACE]/test/packages/flit_editable)
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--exclude")
    .arg("numpy"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package Version Editable project location
    [UNDERLINE]
    flit-editable 0.1.0 [WORKSPACE]/test/packages/flit_editable
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--exclude")
    .arg("flit-editable"), @"exit_code: 0 (success)"
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--exclude")
    .arg("numpy")
    .arg("--exclude")
    .arg("flit-editable"), @"exit_code: 0 (success)"
    );
}

#[test]
#[cfg(not(windows))]
fn list_format_json() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    // Install the editable package.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("-e")
        .arg(context.workspace_root.join("test/packages/flit_editable")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + flit-editable==0.1.0 (from file://[WORKSPACE]/test/packages/flit_editable)
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--format=json"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    [{"name":"flit-editable","version":"0.1.0","editable_project_location":"[WORKSPACE]/test/packages/flit_editable"}]
    "#
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--format=json")
    .arg("--editable"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    [{"name":"flit-editable","version":"0.1.0","editable_project_location":"[WORKSPACE]/test/packages/flit_editable"}]
    "#
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--format=json")
    .arg("--exclude-editable"), @"
    exit_code: 0 (success)
    ----- stdout -----
    []
    "
    );
}

#[test]
fn list_format_freeze() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    // Install the editable package.
    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("-e")
        .arg(context.workspace_root.join("test/packages/flit_editable")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + flit-editable==0.1.0 (from file://[WORKSPACE]/test/packages/flit_editable)
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--format=freeze"), @"
    exit_code: 0 (success)
    ----- stdout -----
    flit-editable==0.1.0
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--format=freeze")
    .arg("--editable"), @"
    exit_code: 0 (success)
    ----- stdout -----
    flit-editable==0.1.0
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--format=freeze")
    .arg("--exclude-editable"), @"exit_code: 0 (success)"
    );
}

#[test]
fn list_legacy_editable() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_filter((r"\-\-\-\-\-\-+.*", "[UNDERLINE]"))
        .with_filter(("  +", " "));

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
        target.path().to_str().unwrap()
    ))?;

    uv_snapshot!(context.filters(), context.pip_list()
        .arg("--editable"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package Version Editable project location
    [UNDERLINE]
    zstandard 0.22.0 [TEMP_DIR]/zstandard_project
    "
    );

    Ok(())
}

#[test]
fn list_legacy_editable_invalid_version() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_filter(("  +", " "));

    let site_packages = ChildPath::new(context.site_packages());

    let target = context.temp_dir.child("paramiko_project");
    target.child("paramiko.egg-info").create_dir_all()?;
    target
        .child("paramiko.egg-info")
        .child("PKG-INFO")
        .write_str(
            "Metadata-Version: 1.0
Name: paramiko
Version: 0.1-bulbasaur
",
        )?;
    site_packages
        .child("paramiko.egg-link")
        .write_str(target.path().to_str().unwrap())?;

    uv_snapshot!(context.filters(), context.pip_list()
        .arg("--editable"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to read metadata from: `[SITE_PACKAGES]/paramiko.egg-link`
     cause: after parsing `0.1-b`, found `ulbasaur`, which is not part of a valid version
    "
    );

    Ok(())
}

#[test]
fn list_ignores_quiet_flag_format_freeze() {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    // Install the editable package.
    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("-e")
        .arg(context.workspace_root.join("test/packages/flit_editable")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + flit-editable==0.1.0 (from file://[WORKSPACE]/test/packages/flit_editable)
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--format=freeze")
    .arg("--quiet"), @"
    exit_code: 0 (success)
    ----- stdout -----
    flit-editable==0.1.0
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--format=freeze")
    .arg("--editable")
    .arg("--quiet"), @"
    exit_code: 0 (success)
    ----- stdout -----
    flit-editable==0.1.0
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--format=freeze")
    .arg("--exclude-editable")
    .arg("--quiet"), @"exit_code: 0 (success)"
    );
}

#[test]
fn list_target() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("simple-package==2.1.3\nother-package==2.0.1")?;

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

    // List packages in the target directory.
    uv_snapshot!(context.filters(), context.pip_list()
        .arg("--target")
        .arg(target.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package        Version
    -------------- -------
    other-package  2.0.1
    simple-package 2.1.3
    "
    );

    // Without --target, the packages should not be visible.
    uv_snapshot!(context.pip_list(), @"
    exit_code: 0 (success)
    "
    );

    Ok(())
}

#[test]
fn list_prefix() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("simple-package==2.1.3\nother-package==2.0.1")?;

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

    // List packages in the prefix directory.
    uv_snapshot!(context.filters(), context.pip_list()
        .arg("--prefix")
        .arg(prefix.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package        Version
    -------------- -------
    other-package  2.0.1
    simple-package 2.1.3
    "
    );

    // Without --prefix, the packages should not be visible.
    uv_snapshot!(context.pip_list(), @"
    exit_code: 0 (success)
    "
    );

    Ok(())
}
