use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};
use serde_json::Value;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

use uv_test::packse::PackseServer;
use uv_test::uv_snapshot;

#[test]
fn lock_check_json_freshness() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let manifest = context.temp_dir.child("pyproject.toml");
    manifest.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;

    let output = uv_snapshot!(context.filters(), context.lock().args([
        "--check", "--output-format", "json", "--preview-features", "json-output", "--offline",
    ]), @r#"
    exit_code: 2 (failure)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "status": "stale",
      "reason": {
        "code": "missing_lockfile"
      }
    }

    ----- stderr -----
    error: Unable to find lockfile at `uv.lock`, but `--check` was provided. To create a lockfile, run `uv lock` or `uv sync` without the flag.
    "#);
    serde_json::from_slice::<Value>(&output.stdout)?;
    assert!(!context.temp_dir.child("uv.lock").exists());

    context.lock().arg("--offline").assert().success();
    let lock = context.read("uv.lock");
    uv_snapshot!(context.filters(), context.lock().args([
        "--check", "--output-format", "json", "--preview-features", "json-output", "--offline",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "status": "fresh"
    }

    ----- stderr -----
    Resolved 1 package in [TIME]
    "#);

    manifest.write_str(&context.read("pyproject.toml").replace("0.1.0", "0.2.0"))?;
    uv_snapshot!(context.filters(), context.lock().args([
        "--check", "--output-format", "json", "--preview-features", "json-output", "--offline",
    ]), @r#"
    exit_code: 1 (failure)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "status": "stale",
      "reason": {
        "code": "version_changed",
        "package": "project",
        "expected": [
          "0.2.0"
        ],
        "actual": [
          "0.1.0"
        ]
      }
    }

    ----- stderr -----
    Resolved 1 package in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--check` was provided.

    hint: To update the lockfile, run `uv lock`.
    "#);
    assert_eq!(context.read("uv.lock"), lock);

    // JSON output must not turn an ordinary lock update into a check implicitly.
    uv_snapshot!(context.filters(), context.lock().args([
        "--output-format", "json", "--offline",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `--output-format json` requires `--check` or `--locked`
    ");
    assert_eq!(context.read("uv.lock"), lock);
    Ok(())
}

#[test]
fn lock_check_json_cutoff() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;
    context.lock().arg("--offline").assert().success();
    let lock = context.read("uv.lock");
    uv_snapshot!(context.filters(), context.lock().args([
        "--check", "--output-format", "json", "--preview-features", "json-output",
        "--exclude-newer", "2024-01-01T00:00:00Z", "--offline",
    ]), @r#"
    exit_code: 1 (failure)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "status": "stale",
      "reason": {
        "code": "exclude_newer_changed",
        "message": "change of exclude newer timestamp from `2024-03-25T00:00:00Z` to `2024-01-01T00:00:00Z`"
      }
    }

    ----- stderr -----
    Resolving despite existing lockfile due to change of exclude newer timestamp from `2024-03-25T00:00:00Z` to `2024-01-01T00:00:00Z`
    Resolved 1 package in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--check` was provided.

    hint: To update the lockfile, run `uv lock`.
    "#);
    assert_eq!(context.read("uv.lock"), lock);
    Ok(())
}

#[test]
fn lock_check_json_offline_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("simple/single-package.toml");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "workspace-demo"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["a @ {wheel_url}"]

        [tool.uv.workspace]
        members = ["member-demo"]
    "#, wheel_url = server.file_url("a-1.0.0-py3-none-any.whl")})?;
    let member = context.temp_dir.child("member-demo/pyproject.toml");
    member.write_str(indoc! {r#"
        [project]
        name = "member-demo"
        version = "0.1.0"
        dependencies = ["a>=0.1.0"]
    "#})?;
    context.lock().assert().success();
    let lock = context.read("uv.lock");

    // A requested upgrade is not evidence that the existing lock is stale.
    uv_snapshot!(context.filters(), context.lock().args([
        "--check", "--output-format", "json", "--preview-features", "json-output",
        "--upgrade-package", "a", "--offline", "--no-cache",
    ]), @r#"
    exit_code: 1 (failure)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "status": "indeterminate",
      "error": {
        "code": "offline_cache_miss",
        "package": "a",
        "message": "Failed to download `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`",
        "causes": [
          "Network connectivity is disabled, but the requested data wasn't found in the cache for: `http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`"
        ]
      }
    }

    ----- stderr -----
      × Failed to download `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`
      ╰─▶ Network connectivity is disabled, but the requested data wasn't found in the cache for: `http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`
    "#);

    // A subsequent metadata failure must not erase a proven requirement mismatch.
    member.write_str(&fs_err::read_to_string(&member)?.replace(">=0.1.0", ">=1.0.0"))?;
    uv_snapshot!(context.filters(), context.lock().args([
        "--check", "--output-format", "json", "--preview-features", "json-output",
        "--offline", "--no-cache",
    ]), @r#"
    exit_code: 1 (failure)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "status": "stale",
      "reason": {
        "code": "package_requirements_changed",
        "package": "member-demo",
        "expected": [
          "a>=1.0.0"
        ],
        "actual": [
          "a>=0.1.0"
        ]
      },
      "error": {
        "code": "offline_cache_miss",
        "package": "a",
        "message": "Failed to download `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`",
        "causes": [
          "Network connectivity is disabled, but the requested data wasn't found in the cache for: `http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`"
        ]
      }
    }

    ----- stderr -----
      × Failed to download `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`
      ╰─▶ Network connectivity is disabled, but the requested data wasn't found in the cache for: `http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`
    "#);
    assert_eq!(context.read("uv.lock"), lock);
    Ok(())
}

#[tokio::test]
async fn lock_check_json_authentication() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("simple/single-package.toml");
    let wheel_url = server.file_url("a-1.0.0-py3-none-any.whl");
    let manifest = context.temp_dir.child("pyproject.toml");
    manifest.write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["a @ {wheel_url}"]
    "#})?;
    context.lock().assert().success();

    let unauthorized = MockServer::start().await;
    Mock::given(path("/a-1.0.0-py3-none-any.whl"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&unauthorized)
        .await;
    let unauthorized_url = format!("{}/a-1.0.0-py3-none-any.whl", unauthorized.uri());
    manifest.write_str(
        &context
            .read("pyproject.toml")
            .replace(&wheel_url, &unauthorized_url),
    )?;
    let lock = context
        .read("uv.lock")
        .replace(&wheel_url, &unauthorized_url);
    context.temp_dir.child("uv.lock").write_str(&lock)?;

    uv_snapshot!(context.filters(), context.lock().args([
        "--check", "--output-format", "json", "--preview-features", "json-output",
        "--no-cache",
    ]), @r#"
    exit_code: 2 (failure)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "status": "indeterminate",
      "error": {
        "code": "authentication",
        "package": "a",
        "http_status": 401,
        "message": "Failed to generate package metadata for `a==1.0.0 @ direct+http://[LOCALHOST]/a-1.0.0-py3-none-any.whl`",
        "causes": [
          "Failed to fetch: `http://[LOCALHOST]/a-1.0.0-py3-none-any.whl`",
          "HTTP status client error (401 Unauthorized) for url (http://[LOCALHOST]/a-1.0.0-py3-none-any.whl)"
        ]
      }
    }

    ----- stderr -----
    error: Failed to generate package metadata for `a==1.0.0 @ direct+http://[LOCALHOST]/a-1.0.0-py3-none-any.whl`
      Caused by: Failed to fetch: `http://[LOCALHOST]/a-1.0.0-py3-none-any.whl`
      Caused by: HTTP status client error (401 Unauthorized) for url (http://[LOCALHOST]/a-1.0.0-py3-none-any.whl)
    "#);
    assert_eq!(context.read("uv.lock"), lock);
    Ok(())
}
