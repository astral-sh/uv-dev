#[cfg(feature = "test-git")]
use std::process::Command;

use anyhow::Result;
#[cfg(feature = "test-git")]
use anyhow::anyhow;
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_fs::copy_dir_all;
#[cfg(feature = "test-git")]
use uv_redacted::SafeUrl;
#[cfg(feature = "test-git-lfs")]
use uv_static::EnvVars;
use uv_test::packse::PackseServer;
#[cfg(feature = "test-git")]
use uv_test::packse::WheelBuilder;
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

/// Competing upgrades use the normal resolver with roots in normalized package-name order.
#[test]
fn competing_upgrades() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::from_scenario(&toml::from_str(indoc! {r#"
        name = "competing-upgrades"
        [root]
        requires = ["a", "b"]
        [expected]
        satisfiable = true
        [packages.a.versions."1.0.0"]
        [packages.a.versions."2.0.0"]
        requires = ["b<2"]
        [packages.b.versions."1.0.0"]
        [packages.b.versions."2.0.0"]
        requires = ["a<2"]
    "#})?);

    context
        .pip_install()
        .args(["b==1.0.0", "a==1.0.0"])
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
    b==1.0.0
    ");
    Ok(())
}

/// An obsolete dependency remains a root when its former parent switches dependencies.
#[test]
fn retains_unused_dependencies() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::from_scenario(&toml::from_str(indoc! {r#"
        name = "retains-unused-dependencies"
        [root]
        requires = ["a"]
        [expected]
        satisfiable = true
        [packages.a.versions."1.0.0"]
        requires = ["b"]
        [packages.a.versions."2.0.0"]
        requires = ["c"]
        [packages.b.versions."1.0.0"]
        [packages.c.versions."1.0.0"]
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
    Resolved 3 packages in [TIME]
    Prepared 2 packages in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 2 packages in [TIME]
     - a==1.0.0
     + a==2.0.0
     + c==1.0.0
    ");
    uv_snapshot!(context.filters(), context.pip_freeze(), @"
    exit_code: 0 (success)
    ----- stdout -----
    a==2.0.0
    b==1.0.0
    c==1.0.0
    ");
    Ok(())
}

/// Installed dependencies do not identify which extras the user originally requested.
#[test]
fn unrecorded_extras() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::from_scenario(&toml::from_str(indoc! {r#"
        name = "unrecorded-extras"
        [root]
        requires = ["a[z]"]
        [expected]
        satisfiable = true
        [packages.a.versions."1.0.0".extras]
        z = ["b<2"]
        [packages.b.versions."1.0.0"]
        [packages.b.versions."2.0.0"]
    "#})?);

    context
        .pip_install()
        .args(["a[z]", "--no-installer-metadata"])
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
     - b==1.0.0
     + b==2.0.0
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

/// A registry duplicate must not discard the direct URL recorded for the same package.
#[test]
fn duplicate_registry_and_url() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let duplicate = uv_test::test_context!("3.12");
    let server = PackseServer::new("simple/single-package.toml");
    context
        .pip_install()
        .arg("a==1.0.0")
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();
    duplicate
        .pip_install()
        .arg(server.file_url("a-2.0.0-py3-none-any.whl"))
        .assert()
        .success();
    copy_dir_all(
        duplicate.site_packages().join("a-2.0.0.dist-info"),
        context.site_packages().join("a-2.0.0.dist-info"),
    )?;

    uv_snapshot!(context.filters(), context.pip_upgrade()
        .arg("--all")
        .arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 2 packages in [TIME]
    Installed 1 package in [TIME]
     - a==1.0.0
     ~ a==2.0.0 (from http://[LOCALHOST]/files/a-2.0.0-py3-none-any.whl)
    ");
    uv_snapshot!(context.filters(), context.pip_freeze(), @"
    exit_code: 0 (success)
    ----- stdout -----
    a @ http://[LOCALHOST]/files/a-2.0.0-py3-none-any.whl
    ");
    Ok(())
}

/// Conflicting installed URLs must fail instead of arbitrarily selecting one source.
#[test]
fn duplicate_urls() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let duplicate = uv_test::test_context!("3.12");
    let server = PackseServer::new("simple/single-package.toml");
    context
        .pip_install()
        .arg(server.file_url("a-1.0.0-py3-none-any.whl"))
        .assert()
        .success();
    duplicate
        .pip_install()
        .arg(server.file_url("a-2.0.0-py3-none-any.whl"))
        .assert()
        .success();
    copy_dir_all(
        duplicate.site_packages().join("a-2.0.0.dist-info"),
        context.site_packages().join("a-2.0.0.dist-info"),
    )?;

    uv_snapshot!(context.filters(), context.pip_upgrade()
        .arg("--all")
        .arg("--index-url").arg(server.index_url()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Requirements contain conflicting URLs for package `a`:
    - http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl
    - http://[LOCALHOST]/files/a-2.0.0-py3-none-any.whl
    ");
    uv_snapshot!(context.filters(), context.pip_freeze(), @"
    exit_code: 0 (success)
    ----- stdout -----
    a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl
    a @ http://[LOCALHOST]/files/a-2.0.0-py3-none-any.whl
    ");
    Ok(())
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
fn editable_directory_with_git_suffix() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("simple/single-package.toml");
    context
        .temp_dir
        .child("project.git/pyproject.toml")
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
        .child("project.git/src/editable/__init__.py")
        .write_str("")?;

    context
        .pip_install()
        .args(["-e", "./project.git", "a==1.0.0"])
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
    -e file://[TEMP_DIR]/project.git
    ");
    Ok(())
}

/// Structured subdirectories may contain characters that delimit URL fragments.
#[test]
#[cfg(feature = "test-git")]
fn git_structured_subdirectory() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filters([(r"@[0-9a-f]{40}".to_string(), "@[COMMIT]".to_string())]);
    let repository = context.temp_dir.child("project@team.git");
    repository
        .child("packages/a&b/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "example"
        version = "0.1.0"
        [build-system]
        requires = ["uv_build>=0.13,<0.14"]
        build-backend = "uv_build"
    "#})?;
    repository
        .child("packages/a&b/src/example/__init__.py")
        .write_str("value = 'installed'\n")?;
    Command::new("git")
        .args(["init", "--initial-branch", "main"])
        .arg(repository.path())
        .assert()
        .success();
    Command::new("git")
        .current_dir(repository.path())
        .args(["add", "."])
        .assert()
        .success();
    Command::new("git")
        .current_dir(repository.path())
        .args([
            "-c",
            "user.name=ferris",
            "-c",
            "user.email=ferris@example.com",
            "commit",
            "-m",
            "Initial commit",
        ])
        .assert()
        .success();
    let repository_url = SafeUrl::from_directory_path(repository.path())
        .map_err(|()| anyhow!("failed to convert repository path to file URL"))?;
    let repository_url = repository_url.as_str().trim_end_matches('/');
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&format!(
            r#"
        [project]
        name = "requirements"
        version = "0.1.0"
        dependencies = ["example"]
        [tool.uv.sources]
        example = {{ git = "{repository_url}", subdirectory = "packages/a&b" }}
    "#
        ))?;
    context
        .pip_install()
        .args(["-r", "pyproject.toml", "--no-index"])
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_upgrade()
        .args(["--all", "--reinstall", "--no-cache", "--no-index"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ example==0.1.0 (from git+file://[TEMP_DIR]/project@team.git@[COMMIT]#subdirectory=packages/a&b)
    ");
    uv_snapshot!(context.filters(), context.python_command()
        .args(["-c", "import example; print(example.value)"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    installed
    ");
    uv_snapshot!(context.filters(), context.pip_upgrade().args(["--all", "--no-index"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    ");
    Ok(())
}

/// Archive paths inside Git repositories also retain their structured spelling.
#[test]
#[cfg(feature = "test-git")]
fn git_structured_archive_path() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filters([(r"@[0-9a-f]{40}".to_string(), "@[COMMIT]".to_string())]);
    let repository = context.temp_dir.child("project@team.git");
    let (filename, wheel) = WheelBuilder::new(&"example".parse()?, &"0.1.0".parse()?).build();
    repository
        .child(format!("a&b/{filename}"))
        .write_binary(&wheel)?;
    Command::new("git")
        .args(["init", "--initial-branch", "main"])
        .arg(repository.path())
        .assert()
        .success();
    Command::new("git")
        .current_dir(repository.path())
        .args(["add", "."])
        .assert()
        .success();
    Command::new("git")
        .current_dir(repository.path())
        .args([
            "-c",
            "user.name=ferris",
            "-c",
            "user.email=ferris@example.com",
            "commit",
            "-m",
            "Initial commit",
        ])
        .assert()
        .success();
    let repository_url = SafeUrl::from_directory_path(repository.path())
        .map_err(|()| anyhow!("failed to convert repository path to file URL"))?;
    let repository_url = repository_url.as_str().trim_end_matches('/');
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&format!(
            r#"
        [project]
        name = "requirements"
        version = "0.1.0"
        dependencies = ["example"]
        [tool.uv.sources]
        example = {{ git = "{repository_url}", path = "a&b/{filename}" }}
    "#
        ))?;
    context
        .pip_install()
        .args(["-r", "pyproject.toml", "--no-index"])
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_upgrade()
        .args(["--all", "--reinstall", "--no-cache", "--no-index"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ example==0.1.0 (from git+file://[TEMP_DIR]/project@team.git@[COMMIT]#path=a&b/example-0.1.0-py3-none-any.whl)
    ");
    uv_snapshot!(context.filters(), context.pip_upgrade().args(["--all", "--no-index"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    ");
    Ok(())
}
