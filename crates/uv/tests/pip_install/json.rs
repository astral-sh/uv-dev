use assert_cmd::prelude::*;

use uv_test::packse::PackseServer;
use uv_test::uv_snapshot;

#[test]
fn json_check() {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("simple/dependency-groups.toml");

    uv_snapshot!(context.pip_install()
        .arg("iniconfig==2.0.0")
        .arg("--default-index")
        .arg(server.index_url())
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
          "name": "iniconfig",
          "version": "2.0.0",
          "action": "installed"
        }
      ],
      "dry_run": true
    }
    "#);

    context.pip_freeze().assert().success().stdout("");
}

#[test]
fn json_dry_run() {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("simple/dependency-groups.toml");

    uv_snapshot!(context.pip_install()
        .arg("iniconfig==2.0.0")
        .arg("--default-index")
        .arg(server.index_url())
        .arg("--dry-run")
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
          "name": "iniconfig",
          "version": "2.0.0",
          "action": "installed"
        }
      ],
      "dry_run": true
    }
    "#);

    context.pip_freeze().assert().success().stdout("");
}

#[test]
fn json_install() {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("simple/dependency-groups.toml");

    uv_snapshot!(context.pip_install()
        .arg("iniconfig==2.0.0")
        .arg("--default-index")
        .arg(server.index_url())
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

    // An already satisfied requirement returns before resolution.
    uv_snapshot!(context.pip_install()
        .arg("iniconfig==2.0.0")
        .arg("--default-index")
        .arg(server.index_url())
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
}
