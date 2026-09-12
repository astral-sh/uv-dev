use anyhow::{Context, Result};
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
    exit_code: 1 (failure)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "status": "stale",
      "action": "check",
      "dry_run": false,
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
      "status": "fresh",
      "action": "check",
      "dry_run": false
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
      "action": "check",
      "dry_run": false,
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

    // An ordinary JSON lock updates the file instead of implicitly checking it.
    uv_snapshot!(context.filters(), context.lock().args([
        "--output-format", "json", "--preview-features", "json-output", "--offline",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "status": "fresh",
      "action": "update",
      "dry_run": false,
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
    Updated project v0.1.0 -> v0.2.0
    "#);
    assert_ne!(context.read("uv.lock"), lock);
    context
        .lock()
        .args(["--check", "--offline"])
        .assert()
        .success();
    Ok(())
}

#[test]
fn lock_json_actions() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let manifest = context.temp_dir.child("pyproject.toml");
    manifest.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;

    uv_snapshot!(context.filters(), context.lock().args([
        "--dry-run", "--output-format", "json", "--preview-features", "json-output", "--offline",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "status": "stale",
      "action": "create",
      "dry_run": true,
      "reason": {
        "code": "missing_lockfile"
      }
    }

    ----- stderr -----
    Resolved 1 package in [TIME]
    Add project v0.1.0
    "#);
    assert!(!context.temp_dir.child("uv.lock").exists());

    uv_snapshot!(context.filters(), context.lock().args([
        "--output-format", "json", "--preview-features", "json-output", "--offline",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "status": "fresh",
      "action": "create",
      "dry_run": false,
      "reason": {
        "code": "missing_lockfile"
      }
    }

    ----- stderr -----
    Resolved 1 package in [TIME]
    "#);
    let lock = context.read("uv.lock");
    uv_snapshot!(context.filters(), context.lock().args([
        "--output-format", "json", "--preview-features", "json-output", "--offline",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "status": "fresh",
      "action": "check",
      "dry_run": false
    }

    ----- stderr -----
    Resolved 1 package in [TIME]
    "#);
    assert_eq!(context.read("uv.lock"), lock);

    uv_snapshot!(context.filters(), context.lock().args([
        "--dry-run", "--output-format", "json", "--preview-features", "json-output", "--offline",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "status": "fresh",
      "action": "check",
      "dry_run": true
    }

    ----- stderr -----
    Resolved 1 package in [TIME]
    No lockfile changes detected
    "#);
    assert_eq!(context.read("uv.lock"), lock);

    manifest.write_str(&context.read("pyproject.toml").replace("0.1.0", "0.2.0"))?;
    uv_snapshot!(context.filters(), context.lock().args([
        "--dry-run", "--output-format", "json", "--preview-features", "json-output", "--offline",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "status": "stale",
      "action": "update",
      "dry_run": true,
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
    Update project v0.1.0 -> v0.2.0
    "#);
    assert_eq!(context.read("uv.lock"), lock);

    // Existence-only checks must not claim that the stale lock is fresh.
    uv_snapshot!(context.filters(), context.lock().args([
        "--check-exists", "--output-format", "json", "--preview-features", "json-output", "--offline",
    ]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "status": "not_checked",
      "action": "use",
      "dry_run": false
    }

    ----- stderr -----
    warning: The lockfile at `uv.lock` was only checked for validity, not whether it is up-to-date, because `--check-exists` was provided; use `--check` instead
    "#);
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
      "action": "check",
      "dry_run": false,
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
    exit_code: 2 (failure)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "status": "indeterminate",
      "action": "check",
      "dry_run": false,
      "error": {
        "code": "offline_cache_miss",
        "package": "a",
        "message": "Required data is not available in the cache"
      }
    }

    ----- stderr -----
    error: Failed to download `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`
      cause: Network connectivity is disabled, but the requested data wasn't found in the cache for: `http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`
    "#);

    // A subsequent metadata failure must not erase a proven requirement mismatch.
    member.write_str(&fs_err::read_to_string(&member)?.replace(">=0.1.0", ">=1.0.0"))?;
    uv_snapshot!(context.filters(), context.lock().args([
        "--check", "--output-format", "json", "--preview-features", "json-output",
        "--offline", "--no-cache",
    ]), @r#"
    exit_code: 2 (failure)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "status": "stale",
      "action": "check",
      "dry_run": false,
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
        "message": "Required data is not available in the cache"
      }
    }

    ----- stderr -----
    error: Failed to download `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`
      cause: Network connectivity is disabled, but the requested data wasn't found in the cache for: `http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`
    "#);
    assert_eq!(context.read("uv.lock"), lock);

    // Failed updates retain the same mismatch and original error without claiming a write.
    uv_snapshot!(context.filters(), context.lock().args([
        "--output-format", "json", "--preview-features", "json-output", "--offline", "--no-cache",
    ]), @r#"
    exit_code: 2 (failure)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "status": "stale",
      "dry_run": false,
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
        "message": "Required data is not available in the cache"
      }
    }

    ----- stderr -----
    error: Failed to download `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`
      cause: Network connectivity is disabled, but the requested data wasn't found in the cache for: `http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`
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
      "action": "check",
      "dry_run": false,
      "error": {
        "code": "authentication",
        "package": "a",
        "http_status": 401,
        "message": "Authentication failed"
      }
    }

    ----- stderr -----
    error: Failed to generate package metadata for `a==1.0.0 @ direct+http://[LOCALHOST]/a-1.0.0-py3-none-any.whl`
      cause: Failed to fetch: `http://[LOCALHOST]/a-1.0.0-py3-none-any.whl`
      cause: HTTP status client error (401 Unauthorized) for url (http://[LOCALHOST]/a-1.0.0-py3-none-any.whl)
    "#);
    assert_eq!(context.read("uv.lock"), lock);
    Ok(())
}

#[tokio::test]
async fn lock_json_failed_create() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let unauthorized = MockServer::start().await;
    Mock::given(path("/a-1.0.0-py3-none-any.whl"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&unauthorized)
        .await;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["a @ {wheel_url}"]
    "#, wheel_url = format!("{}/a-1.0.0-py3-none-any.whl", unauthorized.uri())})?;

    uv_snapshot!(context.filters(), context.lock().args([
        "--output-format", "json", "--preview-features", "json-output", "--no-cache",
    ]), @r#"
    exit_code: 2 (failure)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "status": "stale",
      "dry_run": false,
      "reason": {
        "code": "missing_lockfile"
      },
      "error": {
        "code": "authentication",
        "package": "a",
        "http_status": 401,
        "message": "Authentication failed"
      }
    }

    ----- stderr -----
    error: Failed to download `a @ http://[LOCALHOST]/a-1.0.0-py3-none-any.whl`
      cause: Failed to fetch: `http://[LOCALHOST]/a-1.0.0-py3-none-any.whl`
      cause: HTTP status client error (401 Unauthorized) for url (http://[LOCALHOST]/a-1.0.0-py3-none-any.whl)
    "#);
    assert!(!context.temp_dir.child("uv.lock").exists());
    Ok(())
}

#[test]
fn lock_json_omits_unparsed_dependency_group_values() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [dependency-groups]
        dev = ["a @ https://probe:lock-group-secret-canary@example.invalid/a.whl ; invalid_marker_name == 'value'"]
    "#})?;

    let output = context
        .lock()
        .args([
            "--output-format",
            "json",
            "--preview-features",
            "json-output",
            "--offline",
        ])
        .assert()
        .code(2);
    let report: Value = serde_json::from_slice(&output.get_output().stdout)?;
    insta::assert_json_snapshot!(report, {".path" => "[TEMP_DIR]/uv.lock"}, @r#"
    {
      "dry_run": false,
      "error": {
        "code": "evaluation_failed",
        "message": "Lock operation failed"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "schema": {
        "version": "preview"
      },
      "status": "indeterminate"
    }
    "#);
    assert!(!context.temp_dir.child("uv.lock").exists());
    Ok(())
}

#[test]
fn lock_json_omits_invalid_registry_values() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("simple/single-package.toml");
    let index = server.index_url();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["a==1.0.0"]
    "#})?;
    context
        .lock()
        .args(["--default-index", &index])
        .assert()
        .success();

    // Keep the lockfile valid TOML so validation, rather than parsing, rejects the source.
    let original = context.read("uv.lock");
    let parsed: toml::Value = toml::from_str(&original)?;
    let registry = parsed
        .get("package")
        .and_then(toml::Value::as_array)
        .context("lockfile has no packages")?
        .iter()
        .find(|package| package.get("name").and_then(toml::Value::as_str) == Some("a"))
        .and_then(|package| package.get("source"))
        .and_then(|source| source.get("registry"))
        .and_then(toml::Value::as_str)
        .context("package a has no registry source")?;
    let needle = format!("registry = {}", serde_json::to_string(registry)?);
    assert_eq!(original.matches(&needle).count(), 1);
    let invalid = original.replacen(
        &needle,
        r#"registry = "https://probe:lock-registry-secret-canary@[invalid-host/simple""#,
        1,
    );
    toml::from_str::<toml::Value>(&invalid)?;

    for dry_run in [true, false] {
        context.temp_dir.child("uv.lock").write_str(&invalid)?;
        let mut command = context.lock();
        command.args([
            "--default-index",
            &index,
            "--output-format",
            "json",
            "--preview-features",
            "json-output",
            "--offline",
        ]);
        if dry_run {
            command.arg("--dry-run");
        }
        let output = command.assert().success();
        let stdout = &output.get_output().stdout;
        let report: Value = serde_json::from_slice(stdout)?;
        insta::assert_json_snapshot!(report["validation_error"], @r#"
        {
          "code": "evaluation_failed",
          "message": "Lock operation failed"
        }
        "#);
        assert!(!String::from_utf8_lossy(stdout).contains("lock-registry-secret-canary"));
        assert_eq!(report["status"], if dry_run { "stale" } else { "fresh" });
        assert_eq!(report["dry_run"], dry_run);
        if dry_run {
            assert_eq!(context.read("uv.lock"), invalid);
        } else {
            assert_ne!(context.read("uv.lock"), invalid);
        }
    }
    Ok(())
}
