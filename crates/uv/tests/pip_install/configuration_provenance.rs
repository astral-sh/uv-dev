use anyhow::Result;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_static::EnvVars;
use uv_test::uv_snapshot;

/// Distinguish an environment value from the same value supplied explicitly on the command line.
#[test]
fn hash_environment() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.pip_install()
        .arg("iniconfig")
        .env(EnvVars::UV_REQUIRE_HASHES, "1"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have their versions pinned with `==`, but found: iniconfig

    hint: `--require-hashes` was enabled by environment variable `UV_REQUIRE_HASHES`
    ");

    uv_snapshot!(context.pip_install()
        .arg("iniconfig")
        .arg("--require-hashes")
        .env(EnvVars::UV_REQUIRE_HASHES, "1"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have their versions pinned with `==`, but found: iniconfig
    ");

    // Clap rejects this combination before settings are resolved.
    uv_snapshot!(context.pip_install()
        .arg("iniconfig")
        .arg("--no-require-hashes")
        .arg("--no-index")
        .env(EnvVars::UV_REQUIRE_HASHES, "1"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--no-require-hashes' cannot be used with '--require-hashes'

    Usage: uv pip install --cache-dir [CACHE_DIR] --no-index --exclude-newer <EXCLUDE_NEWER> <PACKAGE|--requirements <REQUIREMENTS>|--editable <EDITABLE>|--group <GROUP>>

    For more information, try '--help'.
    ");
}

/// A false environment flag does not override configuration, but a negated CLI flag does.
#[test]
fn hash_explicit_configuration() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("policy.toml").write_str(indoc! {r"
        no-index = true
        [pip]
        require-hashes = true
    "})?;

    uv_snapshot!(context.pip_install()
        .arg("iniconfig")
        .arg("--config-file")
        .arg("policy.toml")
        .env(EnvVars::UV_REQUIRE_HASHES, "0"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have their versions pinned with `==`, but found: iniconfig

    hint: `--require-hashes` was enabled by `pip.require-hashes` in `policy.toml`
    ");

    uv_snapshot!(context.pip_install()
        .arg("iniconfig")
        .arg("--config-file")
        .arg("policy.toml")
        .arg("--no-require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because iniconfig was not found in the provided package locations and you require iniconfig, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because index lookups were disabled and no additional package locations were provided (try: `--find-links <uri>`)

    hint: `--no-index` was enabled by `no-index` in `policy.toml`
    ");

    Ok(())
}

/// A project setting retains its fully qualified TOML key.
#[test]
fn hash_pyproject_configuration() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r"
        [tool.uv.pip]
        require-hashes = true
    "})?;

    uv_snapshot!(context.pip_install().arg("iniconfig"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have their versions pinned with `==`, but found: iniconfig

    hint: `--require-hashes` was enabled by `tool.uv.pip.require-hashes` in `pyproject.toml`
    ");
    Ok(())
}

/// Requirements directives accumulate independently of the effective scalar setting.
#[test]
fn hash_configuration_and_requirements() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("uv.toml").write_str(indoc! {r"
        [pip]
        require-hashes = true
    "})?;
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(indoc! {r"
        # All requirements need hashes.
        --require-hashes
        iniconfig
    "})?;

    uv_snapshot!(context.pip_install().arg("-r").arg("requirements.txt"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have their versions pinned with `==`, but found: iniconfig

    hint: `--require-hashes` was enabled by `pip.require-hashes` in `uv.toml`

    hint: `--require-hashes` was enabled by `requirements.txt` at line 2
    ");
    uv_snapshot!(context.pip_install().arg("-r").arg("requirements.txt")
        .arg("--no-require-hashes"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have their versions pinned with `==`, but found: iniconfig

    hint: `--require-hashes` was enabled by `requirements.txt` at line 2
    ");
    Ok(())
}

/// A winning false value must discard the source of the shadowed true value.
#[test]
#[cfg_attr(
    windows,
    ignore = "Configuration tests are not yet supported on Windows"
)]
fn hash_configuration_precedence() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let xdg = context.temp_dir.child("config");
    xdg.child("uv/uv.toml").write_str(indoc! {r"
        [pip]
        require-hashes = true
    "})?;
    context.temp_dir.child("uv.toml").write_str(indoc! {r"
        no-index = true
        [pip]
        require-hashes = false
    "})?;

    uv_snapshot!(context.pip_install().arg("iniconfig")
        .env(EnvVars::XDG_CONFIG_HOME, xdg.path()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because iniconfig was not found in the provided package locations and you require iniconfig, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because index lookups were disabled and no additional package locations were provided (try: `--find-links <uri>`)

    hint: `--no-index` was enabled by `no-index` in `uv.toml`
    ");

    context
        .temp_dir
        .child("uv.toml")
        .write_str("no-index = true")?;
    uv_snapshot!(context.pip_install().arg("iniconfig")
        .env(EnvVars::XDG_CONFIG_HOME, xdg.path()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have their versions pinned with `==`, but found: iniconfig

    hint: `--require-hashes` was enabled by `pip.require-hashes` in `config/uv/uv.toml`
    ");
    Ok(())
}

/// Source information survives initial hash collection and reaches transitive resolution errors.
#[test]
fn hash_transitive_requirement() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("requirements.txt").write_str(indoc! {r"
        -r policy.txt
        werkzeug==3.0.0 --hash=sha256:cbb2600f7eabe51dbc0502f58be0b3e1b96b893b05695ea2b35b43d4de2d9962
    "})?;
    context
        .temp_dir
        .child("policy.txt")
        .write_str("--require-hashes")?;

    uv_snapshot!(context.pip_install().arg("-r").arg("requirements.txt"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must be pinned upfront with `==`, but found: `markupsafe`

    hint: `--require-hashes` was enabled by `policy.txt` at line 1 (included from `requirements.txt` at line 1)
    ");
    Ok(())
}

/// Both pip entry points retain environment provenance.
#[test]
fn hash_sync_environment() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("iniconfig")?;

    uv_snapshot!(context.pip_sync().arg("requirements.txt")
        .env(EnvVars::UV_REQUIRE_HASHES, "1"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have their versions pinned with `==`, but found: iniconfig

    hint: `--require-hashes` was enabled by environment variable `UV_REQUIRE_HASHES`
    ");
    Ok(())
}

/// Report all enabling declarations and their include sites, without repeating a visited input.
#[test]
fn no_index_nested_requirements() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(indoc! {r"
        -r nested/first.txt
        -r nested/second.txt
        -r nested/first.txt
        iniconfig
    "})?;
    context
        .temp_dir
        .child("nested/first.txt")
        .write_str(indoc! {r"
        # Use local packages.
        --no-index
    "})?;
    context
        .temp_dir
        .child("nested/second.txt")
        .write_str("--no-index")?;

    uv_snapshot!(context.pip_install().arg("-r").arg("requirements.txt"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because iniconfig was not found in the provided package locations and you require iniconfig, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because index lookups were disabled and no additional package locations were provided (try: `--find-links <uri>`)

    hint: `--no-index` was enabled by `nested/first.txt` at line 2 (included from `requirements.txt` at line 1)

    hint: `--no-index` was enabled by `nested/second.txt` at line 1 (included from `requirements.txt` at line 2)
    ");

    context.temp_dir.child("wheels").create_dir_all()?;
    uv_snapshot!(context.pip_install().arg("-r").arg("requirements.txt")
        .arg("--find-links").arg("wheels"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because iniconfig was not found in the provided package locations and you require iniconfig, we can conclude that your requirements are unsatisfiable.

    hint: `--no-index` was enabled by `nested/first.txt` at line 2 (included from `requirements.txt` at line 1)

    hint: `--no-index` was enabled by `nested/second.txt` at line 1 (included from `requirements.txt` at line 2)
    ");
    Ok(())
}

/// Pip configuration takes precedence over the top-level setting, including its source.
#[test]
fn no_index_configuration() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("uv.toml").write_str(indoc! {r"
        no-index = true
        [pip]
        no-index = true
    "})?;

    uv_snapshot!(context.pip_install().arg("iniconfig"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because iniconfig was not found in the provided package locations and you require iniconfig, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because index lookups were disabled and no additional package locations were provided (try: `--find-links <uri>`)

    hint: `--no-index` was enabled by `pip.no-index` in `uv.toml`
    ");

    context.temp_dir.child("uv.toml").write_str(indoc! {r"
        no-index = true
        [pip]
        no-index = false
    "})?;

    uv_snapshot!(context.pip_install().arg("iniconfig==2.0.0").arg("--dry-run"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Would download 1 package
    Would install 1 package
     + iniconfig==2.0.0
    ");
    Ok(())
}

/// Inline script configuration retains its input and TOML key.
#[test]
fn no_index_script_configuration() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("script.py").write_str(indoc! {r#"
        # /// script
        # dependencies = ["iniconfig"]
        # [tool.uv]
        # no-index = true
        # ///
    "#})?;

    uv_snapshot!(context.pip_install().arg("-r").arg("script.py"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because iniconfig was not found in the provided package locations and you require iniconfig, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because index lookups were disabled and no additional package locations were provided (try: `--find-links <uri>`)

    hint: `--no-index` was enabled by `tool.uv.no-index` in `script.py`
    ");

    uv_snapshot!(context.run().arg("script.py"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving script dependencies
      cause: Because iniconfig was not found in the provided package locations and you require iniconfig, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because index lookups were disabled and no additional package locations were provided (try: `--find-links <uri>`)

    hint: `--no-index` was enabled by `tool.uv.no-index` in `script.py`
    ");
    Ok(())
}
