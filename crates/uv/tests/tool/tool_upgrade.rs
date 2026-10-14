use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::indoc;
use insta::assert_snapshot;
use predicates::prelude::{PredicateStrExt, predicate};

use uv_static::EnvVars;

use uv_test::packse::{PackseServer, scenario::Scenario};
use uv_test::{TestContext, uv_snapshot, venv_bin_path};

#[test]
fn tool_upgrade_up_to_date() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install the latest `babel`.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("babel")
        .arg("--index-url")
        .arg("https://pypi.org/simple/")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + babel==2.14.0
    Installed 1 executable: pybabel
    ");
    let marker = context.temp_dir.child("tools/babel/retained-environment");
    marker.write_str("original tool environment\n")?;

    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("--all")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Nothing to upgrade
    ");
    assert_eq!(context.read(&marker), "original tool environment\n");

    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("--all")
        .arg("-p")
        .arg("3.12")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Nothing to upgrade
    ");
    assert_eq!(context.read(&marker), "original tool environment\n");
    Ok(())
}

#[test]
fn tool_upgrade_name() {
    let old_index = old_tool_index();
    let new_index = new_tool_index();

    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install `babel` from the old index.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("babel")
        .arg("--index-url")
        .arg(old_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + babel==2.6.0
     + pytz==2018.5
    Installed 1 executable: pybabel
    ");

    // Upgrade `babel` by installing from the new index, which should upgrade to the latest version.
    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("babel")
        .arg("--index-url")
        .arg(new_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Updated babel v2.6.0 -> v2.14.0
     - babel==2.6.0
     + babel==2.14.0
     - pytz==2018.5
    Installed 1 executable: pybabel
    ");
}

#[test]
fn tool_upgrade_recomputes_relative_exclude_newer() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    context
        .tool_install()
        .arg("black")
        .arg("--exclude-newer")
        .arg("3 weeks")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(
            EnvVars::UV_INTERNAL__TEST_CURRENT_TIMESTAMP,
            "2024-03-22T00:00:00Z",
        )
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("black")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_INTERNAL__TEST_CURRENT_TIMESTAMP, "2024-04-15T00:00:00Z")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Updated black v24.2.0 -> v24.3.0
     - black==24.2.0
     + black==24.3.0
     - packaging==23.2
     + packaging==24.0
    Installed 2 executables: black, blackd
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(context.read("tools/black/uv-receipt.toml"), @r#"
        [tool]
        requirements = [{ name = "black" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        exclude-newer-span = "P3W"
        "#);
    });
}

/// An explicit settings override must be saved even when no packages need upgrading.
#[test]
fn tool_upgrade_noop_updates_options() {
    let index = old_tool_index();
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("python-dotenv")
        .arg("--index-url")
        .arg(index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + python-dotenv==0.10.2.post2
    Installed 1 executable: dotenv
    ");

    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("python-dotenv")
        .arg("--exclude-newer")
        .arg("false")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Nothing to upgrade
    ");

    insta::with_settings!({ filters => context.filters() }, {
        assert_snapshot!(context.read("tools/python-dotenv/uv-receipt.toml"), @r#"
        [tool]
        requirements = [{ name = "python-dotenv" }]
        entrypoints = [
            { name = "dotenv", install-path = "[TEMP_DIR]/bin/dotenv", from = "python-dotenv" },
        ]

        [tool.options]
        index-url = "http://[LOCALHOST]/simple/"
        exclude-newer = false
        "#);
    });
}

#[test]
fn tool_upgrade_multiple_names() {
    let old_index = old_tool_index();
    let new_index = new_tool_index();

    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install `python-dotenv` from the old index.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("python-dotenv")
        .arg("--index-url")
        .arg(old_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + python-dotenv==0.10.2.post2
    Installed 1 executable: dotenv
    ");

    // Install `babel` from the old index.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("babel")
        .arg("--index-url")
        .arg(old_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + babel==2.6.0
     + pytz==2018.5
    Installed 1 executable: pybabel
    ");

    // Upgrade `babel` and `python-dotenv` from the new index.
    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("babel")
        .arg("python-dotenv")
        .arg("--index-url")
        .arg(new_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Updated babel v2.6.0 -> v2.14.0
     - babel==2.6.0
     + babel==2.14.0
     - pytz==2018.5
    Installed 1 executable: pybabel
    Updated python-dotenv v0.10.2.post2 -> v1.0.1
     - python-dotenv==0.10.2.post2
     + python-dotenv==1.0.1
    Installed 1 executable: dotenv
    ");
}

#[test]
fn tool_upgrade_pinned_hint() {
    let old_index = old_tool_index();
    let new_index = new_tool_index();

    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install a specific version of `babel` so the receipt records an exact pin.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("babel==2.6.0")
        .arg("--index-url")
        .arg(old_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + babel==2.6.0
     + pytz==2018.5
    Installed 1 executable: pybabel
    ");

    // Attempt to upgrade `babel`; it should remain pinned and emit a hint explaining why.
    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("babel")
        .arg("--index-url")
        .arg(new_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Modified babel environment
     - pytz==2018.5
     + pytz==2024.1

    hint: `babel` is pinned to `2.6.0` (installed with an exact version pin); reinstall with `uv tool install babel@latest` to upgrade to a new version.
    ");
}

#[test]
fn tool_upgrade_pinned_hint_with_mixed_constraint() {
    let old_index = old_tool_index();
    let new_index = new_tool_index();

    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install a specific version of `babel` with an additional constraint to ensure the requirement
    // contains multiple specifiers while still including an exact pin.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("babel>=2.0,==2.6.0")
        .arg("--index-url")
        .arg(old_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + babel==2.6.0
     + pytz==2018.5
    Installed 1 executable: pybabel
    ");

    // Attempt to upgrade `babel`; it should remain pinned and emit a hint explaining why.
    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("babel")
        .arg("--index-url")
        .arg(new_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Modified babel environment
     - pytz==2018.5
     + pytz==2024.1

    hint: `babel` is pinned to `2.6.0` (installed with an exact version pin); reinstall with `uv tool install babel@latest` to upgrade to a new version.
    ");
}

#[test]
fn tool_upgrade_all() -> Result<()> {
    let old_index = old_tool_index();
    let new_index = new_tool_index();

    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Install `python-dotenv` from the old index.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("python-dotenv")
        .arg("--index-url")
        .arg(old_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + python-dotenv==0.10.2.post2
    Installed 1 executable: dotenv
    ");

    // Install `babel` from the old index.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("babel")
        .arg("--index-url")
        .arg(old_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + babel==2.6.0
     + pytz==2018.5
    Installed 1 executable: pybabel
    ");

    // An invalid directory must not prevent valid tools from being upgraded.
    let backup = tool_dir.child("tool backup/notes.txt");
    backup.write_str("unrelated backup\n")?;

    // Upgrade all from the new index.
    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("--all")
        .arg("--index-url")
        .arg(new_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Ignoring tool directory `tools/tool backup` with an invalid package name; move it outside the tool directory, or remove it if no longer needed
    Updated babel v2.6.0 -> v2.14.0
     - babel==2.6.0
     + babel==2.14.0
     - pytz==2018.5
    Installed 1 executable: pybabel
    Updated python-dotenv v0.10.2.post2 -> v1.0.1
     - python-dotenv==0.10.2.post2
     + python-dotenv==1.0.1
    Installed 1 executable: dotenv
    ");
    assert_eq!(context.read(backup), "unrelated backup\n");

    Ok(())
}

/// With `tool-install-locks`, the installed versions are used as resolver preferences, which must
/// not prevent the upgrade from selecting newer versions.
#[test]
fn tool_upgrade_all_tool_install_locks() -> Result<()> {
    let old_index = old_tool_index();
    let new_index = new_tool_index();

    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install `babel` from the old index.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("babel")
        .arg("--index-url")
        .arg(old_index.index_url())
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + babel==2.6.0
     + pytz==2018.5
    Installed 1 executable: pybabel
    ");

    // Upgrade all from the new index.
    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("--all")
        .arg("--index-url")
        .arg(new_index.index_url())
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Updated babel v2.6.0 -> v2.14.0
     - babel==2.6.0
     + babel==2.14.0
     - pytz==2018.5
    Installed 1 executable: pybabel
    ");
    let lock = uv_lock::Lock::from_canonical_toml(&context.read("tools/babel/uv.lock"))?;
    let [package] = lock.packages() else {
        anyhow::bail!(
            "expected one package in the upgraded tool lock: {:?}",
            lock.packages()
        );
    };
    assert_eq!(package.name().as_str(), "babel");
    assert_eq!(package.version(), Some(&"2.14.0".parse()?));
    Ok(())
}

#[test]
fn tool_upgrade_not_stop_if_upgrade_fails() -> anyhow::Result<()> {
    let old_index = old_tool_index();
    let new_index = new_tool_index();

    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    // Install `python-dotenv` from the old index.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("python-dotenv")
        .arg("--index-url")
        .arg(old_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + python-dotenv==0.10.2.post2
    Installed 1 executable: dotenv
    ");

    // Install `babel` from the old index.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("babel")
        .arg("--index-url")
        .arg(old_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + babel==2.6.0
     + pytz==2018.5
    Installed 1 executable: pybabel
    ");

    // `babel` sorts before `python-dotenv`, so its failure must not abort the later upgrade.
    tool_dir
        .child("babel")
        .child("uv-receipt.toml")
        .write_str("Invalid receipt")?;

    // Upgrade all from the new index.
    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("--all")
        .arg("--index-url")
        .arg(new_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Updated python-dotenv v0.10.2.post2 -> v1.0.1
     - python-dotenv==0.10.2.post2
     + python-dotenv==1.0.1
    Installed 1 executable: dotenv
    error: Failed to upgrade babel
      cause: `babel` is missing a valid receipt; run `uv tool install --force babel` to reinstall
    ");

    Ok(())
}

#[test]
fn tool_upgrade_settings() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install `black` with `lowest-direct`.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black>=23")
        .arg("--resolution=lowest-direct")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==23.1.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    // Upgrade `black`. This should be a no-op, since the resolution is set to `lowest-direct`.
    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("black")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Nothing to upgrade
    ");

    // Upgrade `black`, but override the resolution.
    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("black")
        .arg("--resolution=highest")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Updated black v23.1.0 -> v24.3.0
     - black==23.1.0
     + black==24.3.0
    Installed 2 executables: black, blackd
    ");
}

#[test]
fn tool_upgrade_no_binary_package_env_var() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black>=23")
        .arg("--resolution=lowest-direct")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + black==23.1.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");

    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("black")
        .arg("--resolution=highest")
        .env(EnvVars::UV_NO_BINARY_PACKAGE, "iniconfig")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Updated black v23.1.0 -> v24.3.0
     - black==23.1.0
     + black==24.3.0
    Installed 2 executables: black, blackd
    ");

    let receipt: toml::Value =
        toml::from_str(&context.read("tools/black/uv-receipt.toml")).unwrap();
    assert_snapshot!(
        receipt["tool"]["options"]["no-binary-package"].to_string(),
        @r#"["iniconfig"]"#
    );
}

#[test]
fn tool_upgrade_respect_constraints() {
    let old_index = old_tool_index();
    let new_index = new_tool_index();

    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install `babel` from the old index.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("babel<2.10")
        .arg("--index-url")
        .arg(old_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + babel==2.6.0
     + pytz==2018.5
    Installed 1 executable: pybabel
    ");

    // Upgrade `babel` from the new index. It should be updated, but not beyond the constraint.
    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("babel")
        .arg("--index-url")
        .arg(new_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Updated babel v2.6.0 -> v2.9.1
     - babel==2.6.0
     + babel==2.9.1
     - pytz==2018.5
     + pytz==2024.1
    Installed 1 executable: pybabel
    ");
}

#[test]
fn tool_upgrade_constraint() {
    let old_index = old_tool_index();
    let new_index = new_tool_index();

    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install `babel` from the old index.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("babel")
        .arg("--index-url")
        .arg(old_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + babel==2.6.0
     + pytz==2018.5
    Installed 1 executable: pybabel
    ");

    // Upgrade `babel`, but apply a constraint inline.
    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("babel<2.12.0")
        .arg("--index-url")
        .arg(new_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Updated babel v2.6.0 -> v2.11.0
     - babel==2.6.0
     + babel==2.11.0
     - pytz==2018.5
     + pytz==2024.1
    Installed 1 executable: pybabel
    ");

    // Upgrade `babel`, but apply a constraint via `--upgrade-package`.
    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("babel")
        .arg("--index-url")
        .arg(new_index.index_url())
        .arg("--upgrade-package")
        .arg("babel<2.14.0")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: `--upgrade-package` is enabled by default on `uv tool upgrade`
    Updated babel v2.11.0 -> v2.13.1
     - babel==2.11.0
     + babel==2.13.1
     - pytz==2024.1
     + setuptools==69.2.0
    Installed 1 executable: pybabel
    ");

    // Upgrade `babel` without a constraint.
    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("babel")
        .arg("--index-url")
        .arg(new_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Updated babel v2.13.1 -> v2.14.0
     - babel==2.13.1
     + babel==2.14.0
     - setuptools==69.2.0
    Installed 1 executable: pybabel
    ");

    // Passing `--upgrade` explicitly should warn.
    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("babel")
        .arg("--index-url")
        .arg(new_index.index_url())
        .arg("--upgrade")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: `--upgrade` is enabled by default on `uv tool upgrade`
    Nothing to upgrade
    ");
}

/// Upgrade an explicitly installed `--with` dependency while the tool remains pinned.
#[test]
fn tool_upgrade_with() {
    let old_index = old_tool_index();
    let new_index = new_tool_index();

    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // `python-dotenv` has no dependencies; `pytz` is added solely through `--with`.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("python-dotenv==0.10.2.post2")
        .arg("--with")
        .arg("pytz")
        .arg("--index-url")
        .arg(old_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + python-dotenv==0.10.2.post2
     + pytz==2018.5
    Installed 1 executable: dotenv
    ");

    // The receipt should retain `pytz` and allow it to upgrade independently of the tool.
    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("python-dotenv")
        .arg("--index-url")
        .arg(new_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Modified python-dotenv environment
     - pytz==2018.5
     + pytz==2024.1

    hint: `python-dotenv` is pinned to `0.10.2.post2` (installed with an exact version pin); reinstall with `uv tool install python-dotenv@latest` to upgrade to a new version.
    ");
}

#[test]
fn tool_upgrade_python() {
    let old_index = old_tool_index();

    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    uv_snapshot!(context.filters(), context.tool_install()
    .arg("babel==2.6.0")
    .arg("--index-url")
    .arg(old_index.index_url())
    .arg("--python").arg("3.11")
    .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + babel==2.6.0
     + pytz==2018.5
    Installed 1 executable: pybabel
    ");
    assert_tool_python_version(&context, "babel", "3.11");

    uv_snapshot!(
        context.filters(),
        context.tool_upgrade().arg("babel")
        .arg("--python").arg("3.12")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + babel==2.6.0
     + pytz==2018.5
    Installed 1 executable: pybabel
    Upgraded tool environment for `babel` to Python 3.12
    "
    );

    assert_tool_python_version(&context, "babel", "3.12");
}

#[test]
fn tool_upgrade_python_with_all() {
    let old_index = old_tool_index();

    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    uv_snapshot!(context.filters(), context.tool_install()
    .arg("babel==2.6.0")
    .arg("--index-url")
    .arg(old_index.index_url())
    .arg("--python").arg("3.11")
    .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + babel==2.6.0
     + pytz==2018.5
    Installed 1 executable: pybabel
    ");

    uv_snapshot!(context.filters(), context.tool_install()
    .arg("python-dotenv")
    .arg("--index-url")
    .arg(old_index.index_url())
    .arg("--python").arg("3.11")
    .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + python-dotenv==0.10.2.post2
    Installed 1 executable: dotenv
    ");
    assert_tool_python_version(&context, "babel", "3.11");
    assert_tool_python_version(&context, "python-dotenv", "3.11");

    uv_snapshot!(
        context.filters(),
        context.tool_upgrade().arg("--all")
        .arg("--python").arg("3.12")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + babel==2.6.0
     + pytz==2018.5
    Installed 1 executable: pybabel
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + python-dotenv==0.10.2.post2
    Installed 1 executable: dotenv
    Upgraded tool environments for `babel` and `python-dotenv` to Python 3.12
    "
    );

    assert_tool_python_version(&context, "babel", "3.12");
    assert_tool_python_version(&context, "python-dotenv", "3.12");
}

/// Upgrade a tool together with any additional entrypoints from other
/// packages.
#[test]
fn test_tool_upgrade_additional_entrypoints() {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install `babel` entrypoint, and all additional ones from `black` too.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("--python")
        .arg("3.11")
        .arg("--with-executables-from")
        .arg("black")
        .arg("babel==2.14.0")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + babel==2.14.0
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables from `black`: black, blackd
    Installed 1 executable: pybabel
    ");

    // Upgrade python, and make sure that all the entrypoints above get
    // re-installed.
    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("--python")
        .arg("3.12")
        .arg("babel")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + babel==2.14.0
     + black==24.3.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables from `black`: black, blackd
    Installed 1 executable: pybabel
    Upgraded tool environment for `babel` to Python 3.12
    ");
}

/// Upgrade a tool with an excluded dependency.
///
/// Compare with `tool_upgrade_respect_constraints`, which shows `pytz` being
/// upgraded alongside `babel`. Here, `pytz` is excluded, so it should remain
/// absent after the upgrade.
#[test]
fn tool_upgrade_excludes() {
    let old_index = old_tool_index();
    let new_index = new_tool_index();

    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    let excludes_txt = context.temp_dir.child("excludes.txt");
    excludes_txt.write_str("pytz").unwrap();

    // Install `babel` from the old index.
    // `pytz` is excluded, so it won't be installed despite being a dependency.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("babel<2.10")
        .arg("--excludes")
        .arg("excludes.txt")
        .arg("--index-url")
        .arg(old_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + babel==2.6.0
    Installed 1 executable: pybabel
    ");

    // Upgrade `babel` from the new index. Babel should be updated (within the `<2.10`
    // constraint), but `pytz` should remain excluded.
    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("babel")
        .arg("--index-url")
        .arg(new_index.index_url())
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Updated babel v2.6.0 -> v2.9.1
     - babel==2.6.0
     + babel==2.9.1
    Installed 1 executable: pybabel
    ");
}

/// Reuse the configured index username when the tool receipt has stripped its credentials.
///
/// See: <https://github.com/astral-sh/uv/issues/21216>
#[tokio::test]
async fn tool_upgrade_index_url_keyring_auth() -> Result<()> {
    let keyring_context = uv_test::test_context!("3.12");
    keyring_context
        .pip_install()
        .arg(
            keyring_context
                .workspace_root
                .join("test")
                .join("packages")
                .join("keyring_test_plugin"),
        )
        .assert()
        .success();

    let proxy = crate::pypi_proxy::start().await;
    let context = uv_test::test_context!("3.12")
        .with_exclude_newer("2025-01-18T00:00:00Z")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let path = std::env::join_paths([venv_bin_path(&keyring_context.venv), bin_dir.to_path_buf()])?;
    let credentials = format!(
        r#"{{"{host}": {{"public": "heron"}}}}"#,
        host = proxy.host_port()
    );

    let uv_toml = context.temp_dir.child("uv.toml");
    uv_toml.write_str(&format!(
        indoc! {r#"
            index-url = "{}"
            keyring-provider = "subprocess"
        "#},
        proxy.username_url("public", "/basic-auth/simple/")
    ))?;

    context
        .tool_install()
        .arg("executable-application")
        .arg("--index-url")
        .arg(proxy.username_url("public", "/basic-auth/simple"))
        .arg("--config-file")
        .arg(uv_toml.as_os_str())
        .env_remove(EnvVars::UV_DEFAULT_INDEX)
        .env(EnvVars::KEYRING_TEST_CREDENTIALS, &credentials)
        .env(EnvVars::PATH, &path)
        .assert()
        .success();

    let receipt = context.read("tools/executable-application/uv-receipt.toml");
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(receipt, @r#"
        [tool]
        requirements = [{ name = "executable-application" }]
        entrypoints = [
            { name = "app", install-path = "[TEMP_DIR]/bin/app", from = "executable-application" },
        ]

        [tool.options]
        index-url = "http://[LOCALHOST]/basic-auth/simple"
        keyring-provider = "subprocess"
        exclude-newer = "2025-01-18T00:00:00Z"
        "#);
    });

    uv_snapshot!(context.filters(), context.tool_upgrade()
        .arg("executable-application")
        .arg("--config-file")
        .arg(uv_toml.as_os_str())
        .env_remove(EnvVars::UV_DEFAULT_INDEX)
        .env(EnvVars::KEYRING_TEST_CREDENTIALS, &credentials)
        .env(EnvVars::PATH, &path), @"
    exit_code: 0 (success)
    ----- stderr -----
    Keyring request for public@http://[LOCALHOST]/basic-auth/simple/
    Keyring request for public@[LOCALHOST]
    Nothing to upgrade
    ");

    Ok(())
}

#[test]
fn tool_upgrade_writes_preview_lock() {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");

    context
        .tool_install()
        .arg("simple-launcher")
        .arg("--no-index")
        .arg("--find-links")
        .arg(context.workspace_root.join("test/links"))
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    let lock_path = tool_dir.child("simple-launcher").child("uv.lock");
    lock_path.assert(predicate::path::missing());
    context
        .tool_upgrade()
        .arg("simple-launcher")
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    insta::with_settings!({ filters => context.filters() }, {
        assert_snapshot!(context.read("tools/simple-launcher/uv.lock"), @r#"
        version = 1
        revision = 5
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        requirements = [{ name = "simple-launcher" }]
        constraints = [{ name = "simple-launcher" }]

        [[package]]
        name = "simple-launcher"
        version = "0.1.0"
        source = { registry = "[WORKSPACE]/test/links" }
        wheels = [
            { path = "[WORKSPACE]/test/links/simple_launcher-0.1.0-py3-none-any.whl" },
        ]
        "#);
    });
}

/// Ensure that a lock-backed upgrade recreates the environment for a requested Python version.
#[test]
fn tool_upgrade_lock_uses_requested_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let local_package = context.temp_dir.child("simple-launcher");
    local_package.create_dir_all()?;
    local_package.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "simple-launcher"
        version = "1.0.0"
        requires-python = ">=3.11"

        [project.scripts]
        simple-launcher = "simple_launcher:main"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;
    let local_package_src = local_package.child("src").child("simple_launcher");
    local_package_src.create_dir_all()?;
    local_package_src
        .child("__init__.py")
        .write_str("def main(): pass\n")?;

    context
        .tool_install()
        .arg(local_package.path())
        .arg("--python")
        .arg("3.11")
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();
    assert_tool_python_version(&context, "simple-launcher", "3.11");

    context
        .tool_upgrade()
        .arg("simple-launcher")
        .arg("--python")
        .arg("3.12")
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    assert_tool_python_version(&context, "simple-launcher", "3.12");

    Ok(())
}

fn assert_tool_python_version(context: &TestContext, tool: &str, version: &str) {
    let interpreter = venv_bin_path(context.temp_dir.child("tools").child(tool))
        .join(format!("python{}", std::env::consts::EXE_SUFFIX));
    context
        .external_command(interpreter)
        .args([
            "-I",
            "-B",
            "-c",
            "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')",
        ])
        .assert()
        .success()
        .stdout(predicate::eq(version).trim());
}

/// An index containing earlier synthetic tool releases and their dependencies.
fn old_tool_index() -> PackseServer {
    let scenario = toml::from_str::<Scenario>(indoc! {r#"
        name = "old-tool-index"

        [root]

        [expected]
        satisfiable = true

        [packages.babel.versions."2.6.0"]
        requires_python = ">=3.11"
        sdist = false
        requires = ["pytz"]
        entry_points = ["pybabel"]

        [packages.pytz.versions."2018.5"]
        requires_python = ">=3.11"
        sdist = false

        [packages.python-dotenv.versions."0.10.2.post2"]
        requires_python = ">=3.11"
        sdist = false
        entry_points = ["dotenv"]
    "#})
    .expect("old tool scenario should parse");
    PackseServer::from_scenario(&scenario)
}

/// Later synthetic releases change dependencies to exercise upgrade and removal behavior.
fn new_tool_index() -> PackseServer {
    let scenario = toml::from_str::<Scenario>(indoc! {r#"
        name = "new-tool-index"

        [root]

        [expected]
        satisfiable = true

        [packages.babel.versions."2.6.0"]
        requires_python = ">=3.11"
        sdist = false
        requires = ["pytz"]
        entry_points = ["pybabel"]

        [packages.babel.versions."2.9.1"]
        requires_python = ">=3.11"
        sdist = false
        requires = ["pytz"]
        entry_points = ["pybabel"]

        [packages.babel.versions."2.11.0"]
        requires_python = ">=3.11"
        sdist = false
        requires = ["pytz"]
        entry_points = ["pybabel"]

        [packages.babel.versions."2.13.1"]
        requires_python = ">=3.11"
        sdist = false
        requires = ["setuptools"]
        entry_points = ["pybabel"]

        [packages.babel.versions."2.14.0"]
        requires_python = ">=3.11"
        sdist = false
        entry_points = ["pybabel"]

        [packages.pytz.versions."2024.1"]
        requires_python = ">=3.11"
        sdist = false

        [packages.setuptools.versions."69.2.0"]
        requires_python = ">=3.11"
        sdist = false

        [packages.python-dotenv.versions."0.10.2.post2"]
        requires_python = ">=3.11"
        sdist = false
        entry_points = ["dotenv"]

        [packages.python-dotenv.versions."1.0.1"]
        requires_python = ">=3.11"
        sdist = false
        entry_points = ["dotenv"]
    "#})
    .expect("new tool scenario should parse");
    PackseServer::from_scenario(&scenario)
}
