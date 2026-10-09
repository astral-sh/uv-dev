//! Offline coverage for dependency-group installation checks.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use assert_cmd::prelude::*;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};

use uv_static::EnvVars;
use uv_test::packse::generate_wheel;
use uv_test::{TestContext, uv_snapshot};

fn wheel(
    context: &TestContext,
    name: &str,
    version: &str,
    dependencies: &[&str],
) -> Result<ChildPath> {
    let dependencies = dependencies
        .iter()
        .map(|dependency| dependency.parse())
        .collect::<Result<Vec<_>, _>>()?;
    let (filename, contents) = generate_wheel(
        &name.parse()?,
        &version.parse()?,
        &dependencies,
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let wheel = context.temp_dir.child(filename);
    wheel.write_binary(&contents)?;
    Ok(wheel)
}

#[test]
fn dependency_group_installed_requirements() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_env(EnvVars::UV_NO_CONFIG, "1")
        .with_env(EnvVars::UV_OFFLINE, "1")
        .with_env(EnvVars::UV_NO_BUILD, "1");
    let root = wheel(&context, "group-root", "1.0.0", &["group-leaf==1.0.0"])?;
    let leaf = wheel(&context, "group-leaf", "1.0.0", &[])?;
    context
        .pip_install()
        .arg("--no-index")
        .arg(root.path())
        .arg(leaf.path())
        .assert()
        .success();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [dependency-groups]
        base = ["group-root==1.0.0"]
        dev = [{ include-group = "base" }]
    "#})?;

    uv_snapshot!(context.filters(), context.pip_install().arg("--no-index").args(["--group", "dev"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Checked 2 packages in [TIME]
    ");

    // The direct-only check need not read or resolve the missing transitive dependency.
    context.pip_uninstall().arg("group-leaf").assert().success();
    uv_snapshot!(context.filters(), context.pip_install().arg("--no-index").args(["--group", "dev", "--no-deps"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    ");

    // A missing transitive dependency still requires resolution.
    uv_snapshot!(context.filters(), context.pip_install().arg("--no-index").args(["--group", "dev"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because group-leaf was not found in the provided package locations and group-root==1.0.0 depends on group-leaf==1.0.0, we can conclude that group-root==1.0.0 cannot be used.
             And because group-root was not found in the provided package locations and you require group-root==1.0.0, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because index lookups were disabled and no additional package locations were provided (try: `--find-links <uri>`)
    ");
    uv_snapshot!(context.filters(), context.pip_install().arg("--no-index")
        .args(["--group", "dev", "--find-links"])
        .arg(context.temp_dir.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + group-leaf==1.0.0
    ");
    Ok(())
}

#[test]
fn dependency_group_multiple_sources() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_env(EnvVars::UV_NO_CONFIG, "1")
        .with_env(EnvVars::UV_OFFLINE, "1")
        .with_env(EnvVars::UV_NO_BUILD, "1");
    let first = wheel(&context, "group-first", "1.0.0", &[])?;
    let second = wheel(&context, "group-second", "1.0.0", &[])?;
    context
        .pip_install()
        .arg("--no-index")
        .arg(first.path())
        .arg(second.path())
        .assert()
        .success();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [dependency-groups]
        dev = ["group-first==1.0.0"]
    "#})?;
    let other = context.temp_dir.child("other");
    other.create_dir_all()?;
    other.child("pyproject.toml").write_str(indoc! {r#"
        [dependency-groups]
        dev = ["group-second==1.0.0"]
    "#})?;
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("group-first==1.0.0\n")?;

    uv_snapshot!(context.filters(), context.pip_install().arg("--no-index").args([
        "-r", "requirements.txt", "--group", "dev", "--group", "other/pyproject.toml:dev",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Checked 2 packages in [TIME]
    ");
    Ok(())
}

#[test]
fn dependency_group_sources() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_env(EnvVars::UV_NO_CONFIG, "1")
        .with_env(EnvVars::UV_OFFLINE, "1")
        .with_env(EnvVars::UV_NO_BUILD, "1");
    let first = wheel(&context, "group-source", "1.0.0", &[])?;
    let second = wheel(&context, "group-source", "2.0.0", &[])?;
    context
        .pip_install()
        .arg("--no-index")
        .arg(first.path())
        .assert()
        .success();
    let second = second
        .path()
        .file_name()
        .context("generated wheel has a filename")?
        .to_str()
        .context("generated wheel filename is UTF-8")?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [dependency-groups]
        dev = ["group-source>=1"]

        [tool.uv.sources]
        group-source = {{ path = "{second}" }}
    "#})?;

    uv_snapshot!(context.filters(), context.pip_install().arg("--no-index").args(["--group", "dev", "--no-sources"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    ");
    uv_snapshot!(context.filters(), context.pip_install().arg("--no-index").args([
        "--group", "dev", "--no-sources-package", "group-source",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    ");

    // Disabling a different package's source must not make the existing version sufficient.
    uv_snapshot!(context.filters(), context.pip_install().arg("--no-index")
        .args(["--group", "dev", "--no-sources-package", "another-package"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - group-source==1.0.0 (from file://[TEMP_DIR]/group_source-1.0.0-py3-none-any.whl)
     + group-source==2.0.0 (from file://[TEMP_DIR]/group_source-2.0.0-py3-none-any.whl)
    ");
    uv_snapshot!(context.filters(), context.pip_install().arg("--no-index").args(["--group", "dev"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    ");
    Ok(())
}

#[test]
fn dependency_group_errors_and_hash_modes() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_env(EnvVars::UV_NO_CONFIG, "1")
        .with_env(EnvVars::UV_OFFLINE, "1")
        .with_env(EnvVars::UV_NO_BUILD, "1");
    let wheel = wheel(&context, "group-pinned", "1.0.0", &[])?;
    context
        .pip_install()
        .arg("--no-index")
        .arg(wheel.path())
        .assert()
        .success();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [dependency-groups]
        dev = ["group-pinned==1.0.0"]
    "#})?;

    uv_snapshot!(context.filters(), context.pip_install().arg("--no-index").args(["--group", "missing"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The dependency group 'missing' was not found in the project: pyproject.toml
    ");
    uv_snapshot!(context.filters(), context.pip_install().arg("--no-index").args(["--group", "dev", "--require-hashes"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must be pinned upfront with `==`, but found: `group-pinned`
    ");
    uv_snapshot!(context.filters(), context.pip_install().arg("--no-index").args(["--group", "dev", "--verify-hashes"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    ");
    uv_snapshot!(context.filters(), context.pip_install().arg("--no-index").args(["--group", "dev", "--compile-bytecode"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    Ok(())
}
