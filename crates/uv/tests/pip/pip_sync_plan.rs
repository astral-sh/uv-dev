//! Reporting and applying synchronization plans.

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_static::EnvVars;
use uv_test::packse::{PackseServer, scenario::Scenario};
use uv_test::uv_snapshot;

fn setup_index() -> Result<PackseServer> {
    let scenario = toml::from_str::<Scenario>(indoc! {r#"
        name = "sync-plan"
        [root]
        [expected]
        satisfiable = true
        [packages.idna.versions."3.6"]
        [packages.iniconfig.versions."2.0.0"]
    "#})?;
    Ok(PackseServer::from_scenario(&scenario))
}

#[test]
fn json() -> Result<()> {
    let index = setup_index()?;
    let context =
        uv_test::test_context!("3.12").with_env(EnvVars::UV_DEFAULT_INDEX, index.index_url());
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("iniconfig==2.0.0")?;

    context.pip_install().arg("idna==3.6").assert().success();

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--check")
        .arg("--output-format=json")
        .arg("--quiet"), @r#"
    exit_code: 1 (failure)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "changes": [
        {
          "name": "idna",
          "version": "3.6",
          "action": "uninstalled"
        },
        {
          "name": "iniconfig",
          "version": "2.0.0",
          "action": "installed"
        }
      ],
      "dry_run": true
    }
    "#);

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--dry-run")
        .arg("--output-format=json"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "changes": [
        {
          "name": "idna",
          "version": "3.6",
          "action": "uninstalled"
        },
        {
          "name": "iniconfig",
          "version": "2.0.0",
          "action": "installed"
        }
      ],
      "dry_run": true
    }

    ----- stderr -----
    Resolved 1 package in [TIME]
    Would download 1 package
    Would uninstall 1 package
    Would install 1 package
     - idna==3.6
     + iniconfig==2.0.0
    "#);

    context
        .pip_freeze()
        .assert()
        .success()
        .stdout("idna==3.6\n");

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--output-format=json")
        .arg("--quiet"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "changes": [
        {
          "name": "idna",
          "version": "3.6",
          "action": "uninstalled"
        },
        {
          "name": "iniconfig",
          "version": "2.0.0",
          "action": "installed"
        }
      ],
      "dry_run": false
    }
    "#);

    context
        .pip_freeze()
        .assert()
        .success()
        .stdout("iniconfig==2.0.0\n");

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--output-format=json")
        .arg("--quiet"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "changes": [],
      "dry_run": false
    }
    "#);

    requirements_txt.write_str("")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--output-format=json"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "changes": [],
      "dry_run": false
    }

    ----- stderr -----
    warning: Requirements file `requirements.txt` does not contain any dependencies
    No requirements found (hint: use `--allow-empty-requirements` to clear the environment)
    "#);

    Ok(())
}

#[test]
fn check_sync() -> Result<()> {
    let index = setup_index()?;
    let context =
        uv_test::test_context!("3.12").with_env(EnvVars::UV_DEFAULT_INDEX, index.index_url());
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("iniconfig==2.0.0")?;

    uv_snapshot!(context.pip_sync().arg("requirements.txt").arg("--check"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Would download 1 package
    Would install 1 package
     + iniconfig==2.0.0
    ");

    // Checking must leave the environment unchanged.
    uv_snapshot!(context.pip_sync().arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    uv_snapshot!(context.pip_sync().arg("requirements.txt").arg("--check").arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    Would make no changes
    ");

    // Sync also checks for packages that would be removed.
    requirements_txt.write_str("")?;
    uv_snapshot!(context.pip_sync().arg("requirements.txt").arg("--allow-empty-requirements").arg("--check"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    warning: Requirements file `requirements.txt` does not contain any dependencies
    Resolved in [TIME]
    Would uninstall 1 package
     - iniconfig==2.0.0
    ");

    context.assert_command("import iniconfig").success();

    Ok(())
}
