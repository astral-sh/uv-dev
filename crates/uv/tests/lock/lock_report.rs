use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};
use serde_json::Value;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

use uv_static::EnvVars;
use uv_test::archive::write_tar_gz;
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

    manifest.write_str(
        &context
            .read("pyproject.toml")
            .replace("name = \"project\"", "name = \"renamed\""),
    )?;
    uv_snapshot!(context.filters(), context.lock().args([
        "--check-exists", "--output-format", "json", "--preview-features", "json-output", "--offline",
    ]), @r#"
    exit_code: 1 (failure)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/uv.lock",
      "status": "stale",
      "action": "use",
      "dry_run": false,
      "reason": {
        "code": "missing_root",
        "package": "renamed"
      }
    }

    ----- stderr -----
    error: The lockfile at `uv.lock` needs to be updated, but `--check-exists` was provided: Missing workspace member `renamed`.

    hint: To update the lockfile, run `uv lock`.
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
    exit_code: 1 (failure)
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
        "message": "Failed to download `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`",
        "causes": [
          "Network connectivity is disabled, but the requested data wasn't found in the cache: http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl"
        ]
      }
    }

    ----- stderr -----
    error: Failed to download `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`
      cause: Network connectivity is disabled, but the requested data wasn't found in the cache: http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl
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
      "action": "check",
      "dry_run": false,
      "reason": {
        "code": "package_requirements_changed",
        "package": "member-demo",
        "expected": [
          "a>=1"
        ],
        "actual": [
          "a>=0.1"
        ]
      },
      "error": {
        "code": "offline_cache_miss",
        "package": "a",
        "message": "Failed to download `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`",
        "causes": [
          "Network connectivity is disabled, but the requested data wasn't found in the cache: http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl"
        ]
      }
    }

    ----- stderr -----
    error: Failed to download `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`
      cause: Network connectivity is disabled, but the requested data wasn't found in the cache: http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl
    "#);
    assert_eq!(context.read("uv.lock"), lock);

    // Failed updates retain the same mismatch and original error without claiming a write.
    uv_snapshot!(context.filters(), context.lock().args([
        "--output-format", "json", "--preview-features", "json-output", "--offline", "--no-cache",
    ]), @r#"
    exit_code: 1 (failure)
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
          "a>=1"
        ],
        "actual": [
          "a>=0.1"
        ]
      },
      "error": {
        "code": "offline_cache_miss",
        "package": "a",
        "message": "Failed to download `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`",
        "causes": [
          "Network connectivity is disabled, but the requested data wasn't found in the cache: http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl"
        ]
      }
    }

    ----- stderr -----
    error: Failed to download `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`
      cause: Network connectivity is disabled, but the requested data wasn't found in the cache: http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl
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
      "validation_error": {
        "code": "authentication",
        "package": "a",
        "http_status": 401,
        "message": "Failed to generate package metadata for `a==1.0.0 @ direct+http://[LOCALHOST]/a-1.0.0-py3-none-any.whl`",
        "causes": [
          "Failed to fetch: http://[LOCALHOST]/a-1.0.0-py3-none-any.whl",
          "HTTP status client error (401 Unauthorized) for url (http://[LOCALHOST]/a-1.0.0-py3-none-any.whl)"
        ]
      },
      "error": {
        "code": "authentication",
        "package": "a",
        "http_status": 401,
        "message": "Failed to generate package metadata for `a==1.0.0 @ direct+http://[LOCALHOST]/a-1.0.0-py3-none-any.whl`",
        "causes": [
          "Failed to fetch: http://[LOCALHOST]/a-1.0.0-py3-none-any.whl",
          "HTTP status client error (401 Unauthorized) for url (http://[LOCALHOST]/a-1.0.0-py3-none-any.whl)"
        ]
      }
    }

    ----- stderr -----
    error: Failed to generate package metadata for `a==1.0.0 @ direct+http://[LOCALHOST]/a-1.0.0-py3-none-any.whl`
      cause: Failed to fetch: http://[LOCALHOST]/a-1.0.0-py3-none-any.whl
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
        "message": "Failed to download `a @ http://[LOCALHOST]/a-1.0.0-py3-none-any.whl`",
        "causes": [
          "Failed to fetch: http://[LOCALHOST]/a-1.0.0-py3-none-any.whl",
          "HTTP status client error (401 Unauthorized) for url (http://[LOCALHOST]/a-1.0.0-py3-none-any.whl)"
        ]
      }
    }

    ----- stderr -----
    error: Failed to download `a @ http://[LOCALHOST]/a-1.0.0-py3-none-any.whl`
      cause: Failed to fetch: http://[LOCALHOST]/a-1.0.0-py3-none-any.whl
      cause: HTTP status client error (401 Unauthorized) for url (http://[LOCALHOST]/a-1.0.0-py3-none-any.whl)
    "#);
    assert!(!context.temp_dir.child("uv.lock").exists());
    Ok(())
}

#[tokio::test]
async fn lock_json_registry_authentication() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    Mock::given(path("/simple/a/"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["a"]
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args([
        "--output-format", "json", "--preview-features", "json-output", "--no-cache",
        "--default-index",
    ]).arg(format!("{}/simple", server.uri())), @r#"
    exit_code: 1 (failure)
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
        "http_status": 401,
        "message": "Because a was not found in the package registry and your project depends on a, we can conclude that your project's requirements are unsatisfiable.",
        "hints": [
          "An index URL (http://[LOCALHOST]/simple) could not be queried due to a lack of valid authentication credentials (401 Unauthorized)"
        ]
      }
    }

    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because a was not found in the package registry and your project depends on a, we can conclude that your project's requirements are unsatisfiable.

    hint: An index URL (http://[LOCALHOST]/simple) could not be queried due to a lack of valid authentication credentials (401 Unauthorized)
    "#);
    assert!(!context.temp_dir.child("uv.lock").exists());
    Ok(())
}

#[test]
fn lock_json_build_constraint_hash_change() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let manifest = context.temp_dir.child("pyproject.toml");
    manifest.write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        [tool.uv]
        package = false
        build-constraint-dependencies = [
            {{ requirement = "setuptools==70.0.0", hashes = ["sha256:{hash}"] }},
        ]
    "#, hash = "0".repeat(64)})?;
    context
        .lock()
        .args(["--offline", "--no-index"])
        .assert()
        .success();
    let locked = context.read("uv.lock");
    manifest.write_str(
        &context
            .read("pyproject.toml")
            .replace(&"0".repeat(64), &"1".repeat(64)),
    )?;
    uv_snapshot!(context.filters(), context.lock().args([
        "--check", "--offline", "--no-index",
        "--output-format", "json", "--preview-features", "json-output",
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
        "code": "build_constraints_changed",
        "expected": [
          "setuptools==70.0.0 --hash=sha256:1111111111111111111111111111111111111111111111111111111111111111"
        ],
        "actual": [
          "setuptools==70.0.0 --hash=sha256:0000000000000000000000000000000000000000000000000000000000000000"
        ]
      }
    }

    ----- stderr -----
    Resolved 1 package in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--check` was provided.

    hint: To update the lockfile, run `uv lock`.
    "#);
    assert_eq!(locked, context.read("uv.lock"));
    Ok(())
}

#[test]
fn lock_json_invalid_direct_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = context.temp_dir.child("a-1.0.0-py3-none-any.whl");
    wheel.write_binary(b"invalid wheel archive")?;
    let url = url::Url::from_file_path(wheel.path())
        .map_err(|()| anyhow::anyhow!("wheel path is not absolute"))?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["a @ {url}"]
        [tool.uv]
        package = false
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args([
        "--offline", "--no-index", "--output-format", "json", "--preview-features", "json-output",
    ]), @r#"
    exit_code: 1 (failure)
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
        "code": "metadata_unavailable",
        "package": "a",
        "message": "Failed to read `a @ file://[TEMP_DIR]/a-1.0.0-py3-none-any.whl`",
        "causes": [
          "Failed to extract archive: a-1.0.0-py3-none-any.whl",
          "Invalid zip file structure",
          "Encountered an unexpected header (actual: 0x61766e69, expected: 0x4034b50)."
        ]
      }
    }

    ----- stderr -----
    error: Failed to read `a @ file://[TEMP_DIR]/a-1.0.0-py3-none-any.whl`
      cause: Failed to extract archive: a-1.0.0-py3-none-any.whl
      cause: Invalid zip file structure
      cause: Encountered an unexpected header (actual: 0x61766e69, expected: 0x4034b50).
    "#);
    assert!(!context.temp_dir.child("uv.lock").exists());
    Ok(())
}

/// Registry resolution reports an offline cache miss through the resolver's hint.
#[test]
fn lock_check_json_offline_registry() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("simple/single-package.toml");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["a"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock().arg("--index-url").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock().args([
        "--check", "--output-format", "json", "--preview-features", "json-output",
        "--offline", "--no-cache", "--upgrade-package", "a",
    ]).arg("--index-url").arg(server.index_url()), @r#"
    exit_code: 1 (failure)
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
        "message": "Because a was not found in the cache and your project depends on a, we can conclude that your project's requirements are unsatisfiable.",
        "hints": [
          "Packages were unavailable because the network was disabled. When the network is disabled, registry packages may only be read from the cache."
        ]
      }
    }

    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because a was not found in the cache and your project depends on a, we can conclude that your project's requirements are unsatisfiable.

    hint: Packages were unavailable because the network was disabled. When the network is disabled, registry packages may only be read from the cache.
    "#);
    Ok(())
}

/// Source-mode changes remain visible in both sides of a requirement mismatch.
#[test]
fn lock_check_json_script_editable_source() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["child"]
        # [tool.uv.sources]
        # child = { path = "child", editable = true }
        # ///
    "#})?;
    context
        .temp_dir
        .child("child/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"
        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--script", "script.py", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    let lock = context.read("script.py.lock");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["child"]
        # [tool.uv.sources]
        # child = { path = "child", editable = false }
        # ///
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args([
        "--script", "script.py", "--check", "--output-format", "json",
        "--preview-features", "json-output", "--offline",
    ]), @r#"
    exit_code: 1 (failure)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "path": "[TEMP_DIR]/script.py.lock",
      "status": "stale",
      "action": "check",
      "dry_run": false,
      "reason": {
        "code": "requirements_changed",
        "expected": [
          "child @ file://[TEMP_DIR]/child (editable: false) (virtual: false)"
        ],
        "actual": [
          "child @ file://[TEMP_DIR]/child (editable: true) (virtual: false)"
        ]
      }
    }

    ----- stderr -----
    Resolved 1 package in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--check` was provided.

    hint: To update the lockfile, run `uv lock`.
    "#);
    assert_eq!(context.read("script.py.lock"), lock);
    Ok(())
}

/// Structured errors retain marker-replacement hints from the shared diagnostic interface.
#[cfg(feature = "test-universal")]
#[test]
fn lock_json_overlapping_environment() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.8"
        [tool.uv]
        environments = ["platform_system != 'Windows'", "python_version > '3.10'"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock().args([
        "--output-format", "json", "--preview-features", "json-output", "--offline",
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
        "code": "evaluation_failed",
        "message": "Supported environments must be disjoint, but the following markers overlap: `sys_platform != 'win32'` and `python_full_version >= '3.11'`",
        "hints": [
          "replace `python_full_version >= '3.11'` with `python_full_version >= '3.11' and sys_platform == 'win32'`"
        ]
      }
    }

    ----- stderr -----
    error: Supported environments must be disjoint, but the following markers overlap: `sys_platform != 'win32'` and `python_full_version >= '3.11'`

    hint: replace `python_full_version >= '3.11'` with `python_full_version >= '3.11' and sys_platform == 'win32'`
    "#);
    Ok(())
}

#[test]
fn lock_json_invalid_registry_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["validation==2.0.0"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links"])
        .arg(context.workspace_root.join("test/links"))
        .args(["--output-format", "json", "--preview-features", "json-output"]), @r#"
    exit_code: 1 (failure)
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
        "code": "metadata_unavailable",
        "package": "validation",
        "message": "Because validation==2.0.0 has invalid metadata and your project depends on validation==2.0.0, we can conclude that your project's requirements are unsatisfiable.",
        "hints": [
          "Metadata for `validation` (v2.0.0) could not be parsed:/n  Failed to parse version: Unexpected end of version specifier, expected operator. Did you mean `==12`?:/n  12/n  ^^/n"
        ]
      }
    }

    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because validation==2.0.0 has invalid metadata and your project depends on validation==2.0.0, we can conclude that your project's requirements are unsatisfiable.

    hint: Metadata for `validation` (v2.0.0) could not be parsed:
      Failed to parse version: Unexpected end of version specifier, expected operator. Did you mean `==12`?:
      12
      ^^
    "#);
    Ok(())
}

#[test]
fn lock_json_invalid_registry_structure() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["validation==3.0.0"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links"])
        .arg(context.workspace_root.join("test/links"))
        .args(["--output-format", "json", "--preview-features", "json-output"]), @r#"
    exit_code: 1 (failure)
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
        "code": "metadata_unavailable",
        "package": "validation",
        "message": "Because validation==3.0.0 has an invalid package format and your project depends on validation==3.0.0, we can conclude that your project's requirements are unsatisfiable.",
        "hints": [
          "The structure of `validation` (v3.0.0) was invalid:/n  Multiple .dist-info directories found: validation-2.0.0, validation-3.0.0"
        ]
      }
    }

    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because validation==3.0.0 has an invalid package format and your project depends on validation==3.0.0, we can conclude that your project's requirements are unsatisfiable.

    hint: The structure of `validation` (v3.0.0) was invalid:
      Multiple .dist-info directories found: validation-2.0.0, validation-3.0.0
    "#);
    Ok(())
}

/// Ignored metadata HTTP errors become resolver hints rather than an error source chain.
#[tokio::test]
async fn lock_json_registry_metadata_authentication() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    Mock::given(path("/simple/a/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            format!(r#"<a href="{}/a-1.0.0-py3-none-any.whl" data-core-metadata="true" data-upload-time="2023-01-01T00:00:00Z">a-1.0.0-py3-none-any.whl</a>"#, server.uri()),
            "text/html",
        ))
        .mount(&server).await;
    Mock::given(path("/a-1.0.0-py3-none-any.whl.metadata"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["a"]
        [tool.uv]
        package = false
        [[tool.uv.index]]
        name = "metadata"
        url = "{}/simple"
        authenticate = "never"
        ignore-error-codes = [401]
        default = true
    "#, server.uri()})?;
    uv_snapshot!(context.filters(), context.lock().args([
        "--output-format", "json", "--preview-features", "json-output", "--no-cache",
    ]), @r#"
    exit_code: 1 (failure)
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
        "message": "Because a==1.0.0 could not be fetched from the network (`401 Unauthorized`) and only a==1.0.0 is available, we can conclude that all versions of a cannot be used./nAnd because your project depends on a, we can conclude that your project's requirements are unsatisfiable.",
        "hints": [
          "Metadata for `a` (v1.0.0) could not be fetched; the server returned: `401 Unauthorized`"
        ]
      }
    }

    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because a==1.0.0 could not be fetched from the network (`401 Unauthorized`) and only a==1.0.0 is available, we can conclude that all versions of a cannot be used.
             And because your project depends on a, we can conclude that your project's requirements are unsatisfiable.

    hint: Metadata for `a` (v1.0.0) could not be fetched; the server returned: `401 Unauthorized`
    "#);
    context
        .temp_dir
        .child("uv.lock")
        .assert(predicates::path::missing());
    Ok(())
}

/// A registry source build failure retains the package and metadata classification in JSON.
#[tokio::test]
async fn lock_json_registry_build_failure() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let mut source = Vec::new();
    write_tar_gz(
        &mut source,
        &[
            (
                "broken-1.0.0/pyproject.toml",
                indoc! {r#"
            [project]
            name = "broken"
            version = "1.0.0"
            dynamic = ["dependencies"]
            [build-system]
            requires = []
            build-backend = "backend"
            backend-path = ["."]
        "#},
            ),
            (
                "broken-1.0.0/backend.py",
                indoc! {r#"
            def get_requires_for_build_wheel(config_settings=None):
                raise SystemExit("metadata build is unavailable")
        "#},
            ),
        ],
    )?;
    Mock::given(path("/simple/broken/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "<a href=\"../../broken-1.0.0.tar.gz\">broken-1.0.0.tar.gz</a>",
            "text/html",
        ))
        .mount(&server)
        .await;
    Mock::given(path("/broken-1.0.0.tar.gz"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(source))
        .mount(&server)
        .await;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["broken"]
    "#})?;
    let output = uv_snapshot!(context.filters(), context.lock().args([
        "--output-format", "json", "--preview-features", "json-output", "--index-url",
    ]).arg(format!("{}/simple", server.uri())).env_remove(EnvVars::UV_EXCLUDE_NEWER), @r#"
    exit_code: 1 (failure)
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
        "code": "metadata_unavailable",
        "package": "broken",
        "message": "Failed to build `broken==1.0.0`",
        "causes": [
          "The build backend returned an error",
          "Call to `backend.get_requires_for_build_wheel` failed (exit status: 1)/n/n[stderr]/nmetadata build is unavailable/n"
        ],
        "hints": [
          "`broken` (v1.0.0) was included because `project` (v0.1.0) depends on `broken`",
          "Build failures usually indicate a problem with the package or the build environment"
        ]
      }
    }

    ----- stderr -----
    error: Failed to build `broken==1.0.0`
      cause: The build backend returned an error
      cause: Call to `backend.get_requires_for_build_wheel` failed (exit status: 1)

             [stderr]
             metadata build is unavailable

    hint: `broken` (v1.0.0) was included because `project` (v0.1.0) depends on `broken`

    hint: Build failures usually indicate a problem with the package or the build environment
    "#);
    serde_json::from_slice::<Value>(&output.stdout)?;
    Ok(())
}
