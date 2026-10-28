use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use indoc::indoc;

#[cfg(feature = "test-git-lfs")]
use uv_static::EnvVars;
use uv_test::packse::PackseServer;
use uv_test::uv_snapshot;

#[test]
fn requires_all() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.filters(), context.pip_upgrade(), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the following required arguments were not provided:
      --all

    Usage: uv pip upgrade --all --cache-dir [CACHE_DIR] --exclude-newer <EXCLUDE_NEWER>

    For more information, try '--help'.
    ");
}

#[test]
fn empty_environment() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.filters(), context.pip_upgrade().arg("--all").arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");
}

#[test]
fn no_upgrade_conflicts() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.filters(), context.pip_upgrade().args(["--all", "--no-upgrade"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--all' cannot be used with '--no-upgrade'

    Usage: uv pip upgrade --all --cache-dir [CACHE_DIR] --exclude-newer <EXCLUDE_NEWER>

    For more information, try '--help'.
    ");
}

/// All installed packages, including dependencies, participate in the same resolution.
#[test]
fn upgrade_all() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::from_scenario(&toml::from_str(indoc! {r#"
        name = "upgrade-all"
        [root]
        requires = ["a", "c"]
        [expected]
        satisfiable = true
        [packages.a.versions."1.0.0"]
        requires = ["b==1.0.0"]
        [packages.a.versions."2.0.0"]
        requires = ["b==2.0.0"]
        [packages.b.versions."1.0.0"]
        [packages.b.versions."2.0.0"]
        [packages.c.versions."1.0.0"]
        [packages.c.versions."2.0.0"]
    "#})?);

    context
        .pip_install()
        .args(["a==1.0.0", "c==1.0.0"])
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_upgrade()
        .arg("--all")
        .arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 3 packages in [TIME]
    Uninstalled 3 packages in [TIME]
    Installed 3 packages in [TIME]
     - a==1.0.0
     + a==2.0.0
     - b==1.0.0
     + b==2.0.0
     - c==1.0.0
     + c==2.0.0
    ");

    uv_snapshot!(context.filters(), context.pip_freeze(), @"
    exit_code: 0 (success)
    ----- stdout -----
    a==2.0.0
    b==2.0.0
    c==2.0.0
    ");

    uv_snapshot!(context.filters(), context.pip_upgrade()
        .arg("--all")
        .arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Checked 3 packages in [TIME]
    ");

    Ok(())
}

#[test]
fn dry_run() {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("simple/single-package.toml");

    context
        .pip_install()
        .arg("a==1.0.0")
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_upgrade()
        .args(["--all", "--dry-run"])
        .arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Would download 1 package
    Would uninstall 1 package
    Would install 1 package
     - a==1.0.0
     + a==2.0.0
    ");

    uv_snapshot!(context.filters(), context.pip_freeze(), @"
    exit_code: 0 (success)
    ----- stdout -----
    a==1.0.0
    ");
}

/// Joint upgrades can downgrade an installed dependency to select a newer compatible package.
#[test]
fn compatible_downgrade() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::from_scenario(&toml::from_str(indoc! {r#"
        name = "upgrade-downgrade"
        [root]
        requires = ["a", "b"]
        [expected]
        satisfiable = true
        [packages.a.versions."1.0.0"]
        requires = ["b==2.0.0"]
        [packages.a.versions."2.0.0"]
        requires = ["b==1.0.0"]
        [packages.b.versions."1.0.0"]
        [packages.b.versions."2.0.0"]
    "#})?);

    context
        .pip_install()
        .arg("a==1.0.0")
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_upgrade()
        .arg("--all")
        .arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Uninstalled 2 packages in [TIME]
    Installed 2 packages in [TIME]
     - a==1.0.0
     + a==2.0.0
     - b==2.0.0
     + b==1.0.0
    ");

    uv_snapshot!(context.filters(), context.pip_freeze(), @"
    exit_code: 0 (success)
    ----- stdout -----
    a==2.0.0
    b==1.0.0
    ");
    Ok(())
}

#[test]
fn constraints() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("simple/single-package.toml");
    let constraints = context.temp_dir.child("constraints.txt");
    constraints.write_str("a<2")?;

    context
        .pip_install()
        .arg("a==1.0.0")
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_upgrade()
        .arg("--all")
        .arg("--constraint").arg(constraints.path())
        .arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    ");

    uv_snapshot!(context.filters(), context.pip_freeze(), @"
    exit_code: 0 (success)
    ----- stdout -----
    a==1.0.0
    ");
    Ok(())
}

#[test]
fn configured_no_upgrade() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("simple/single-package.toml");
    context.temp_dir.child("uv.toml").write_str(indoc! {"
        [pip]
        upgrade = false
    "})?;

    context
        .pip_install()
        .arg("a==1.0.0")
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_upgrade()
        .arg("--all")
        .arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - a==1.0.0
     + a==2.0.0
    ");
    Ok(())
}

/// Configured project sources must not add requirements to the installed package set.
#[test]
fn configured_source_selections() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("simple/single-package.toml");

    context
        .pip_install()
        .arg("a==1.0.0")
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    context.temp_dir.child("uv.toml").write_str(indoc! {r#"
        [pip]
        extra = ["dev"]
        no-extra = ["docs"]
        all-extras = true
        group = ["dev"]
    "#})?;

    uv_snapshot!(context.filters(), context.pip_upgrade()
        .arg("--all")
        .arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - a==1.0.0
     + a==2.0.0
    ");
    Ok(())
}

/// Reinstallation can defer scanning ordinary install inputs, but upgrades need the installed set.
#[test]
fn reinstall() {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("simple/single-package.toml");

    context
        .pip_install()
        .arg("a==1.0.0")
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_upgrade()
        .args(["--all", "--reinstall"])
        .arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - a==1.0.0
     + a==2.0.0
    ");
}

/// A failed joint resolution must leave every installed package untouched.
#[test]
fn resolution_failure() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::from_scenario(&toml::from_str(indoc! {r#"
        name = "upgrade-conflict"
        [root]
        requires = ["a", "b"]
        [expected]
        satisfiable = false
        [packages.a.versions."1.0.0"]
        requires = ["b<2"]
        [packages.b.versions."2.0.0"]
    "#})?);

    context
        .pip_install()
        .args(["a", "b", "--no-deps"])
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_upgrade()
        .arg("--all")
        .arg("--index-url").arg(server.index_url()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because all versions of a depend on b<2 and only b==2.0.0 is available, we can conclude that all versions of a cannot be used.
             And because you require a, we can conclude that your requirements are unsatisfiable.
    ");

    uv_snapshot!(context.filters(), context.pip_freeze(), @"
    exit_code: 0 (success)
    ----- stdout -----
    a==1.0.0
    b==2.0.0
    ");
    Ok(())
}

#[test]
fn target() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_virtualenv_bin()
        .with_filtered_python_names();
    let server = PackseServer::new("simple/single-package.toml");
    let target = context.temp_dir.child("target");

    context
        .pip_install()
        .arg("a==1.0.0")
        .arg("--target")
        .arg(target.path())
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_upgrade()
        .arg("--all")
        .arg("--target").arg(target.path())
        .arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: .venv/[BIN]/[PYTHON]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - a==1.0.0
     + a==2.0.0
    ");

    uv_snapshot!(context.filters(), context.pip_freeze()
        .arg("--target").arg(target.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    a==2.0.0
    ");
    uv_snapshot!(context.filters(), context.pip_freeze(), @"exit_code: 0 (success)");
}

/// A direct URL must not be replaced by the newer registry package with the same name.
#[test]
fn direct_url() {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("simple/single-package.toml");

    context
        .pip_install()
        .arg(server.file_url("a-1.0.0-py3-none-any.whl"))
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_upgrade()
        .arg("--all")
        .arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    ");

    uv_snapshot!(context.filters(), context.pip_freeze(), @"
    exit_code: 0 (success)
    ----- stdout -----
    a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl
    ");
}

/// The recorded LFS mode is part of a Git source, independent of the current environment setting.
#[test]
#[cfg(feature = "test-git-lfs")]
fn git_lfs() {
    let context = uv_test::test_context!("3.13").with_git_lfs_config();

    context
        .pip_install()
        .arg("git+https://github.com/astral-sh/test-lfs-repo.git@261c828b8e05251f3a3e4f6b47b149d691c7efbb")
        .env(EnvVars::UV_GIT_LFS, "true")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_upgrade()
        .arg("--all")
        .env(EnvVars::UV_GIT_LFS, "false"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    ");

    uv_snapshot!(context.filters(), context.python_command()
        .arg("-c")
        .arg("import test_lfs_repo.lfs_module"), @"exit_code: 0 (success)");
}

#[test]
fn editable() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("simple/single-package.toml");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "editable"
        version = "0.1.0"
        dependencies = ["a"]
        [build-system]
        requires = ["uv_build>=0.13,<0.14"]
        build-backend = "uv_build"
    "#})?;
    context
        .temp_dir
        .child("src/editable/__init__.py")
        .write_str("")?;

    context
        .pip_install()
        .args(["-e", ".", "a==1.0.0"])
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_upgrade()
        .arg("--all")
        .arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - a==1.0.0
     + a==2.0.0
    ");

    uv_snapshot!(context.filters(), context.pip_freeze(), @"
    exit_code: 0 (success)
    ----- stdout -----
    a==2.0.0
    -e file://[TEMP_DIR]/
    ");
    Ok(())
}
