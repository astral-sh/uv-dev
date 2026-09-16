#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use anyhow::{Context, Result};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};
use insta::{allow_duplicates, assert_json_snapshot};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[cfg(windows)]
use uv_fs::Simplified;
use uv_resolver::no_solution_capture::{CaptureMetadata, CaptureOperation, CaptureScope};
use uv_static::EnvVars;
#[cfg(unix)]
use uv_test::ReadOnlyDirectoryGuard;
use uv_test::packse::{PackseServer, scenario::Scenario};
use uv_test::{TestContext, apply_filters, uv_snapshot};

const REQUEST: &str = "0123456789abcdef0123456789abcdef";

fn write_document(context: &TestContext, scope: CaptureScope, requirements: &[&str]) -> Result<()> {
    let requirements = serde_json::to_string(requirements)?;
    match scope {
        CaptureScope::Workspace => {
            context
                .temp_dir
                .child("pyproject.toml")
                .write_str(&formatdoc! {r#"
                    [project]
                    name = "capture-project"
                    version = "0.1.0"
                    requires-python = ">=3.12,<3.15"
                    dependencies = {requirements}
                "#})?;
        }
        CaptureScope::Script => {
            context
                .temp_dir
                .child("script.py")
                .write_str(&formatdoc! {r#"
                    # /// script
                    # requires-python = ">=3.12,<3.15"
                    # dependencies = {requirements}
                    # ///
                "#})?;
        }
    }
    Ok(())
}

fn lock_path(context: &TestContext, scope: CaptureScope) -> PathBuf {
    context.temp_dir.join(match scope {
        CaptureScope::Workspace => "uv.lock",
        CaptureScope::Script => "script.py.lock",
    })
}

fn lock_command(
    context: &TestContext,
    index: &str,
    scope: CaptureScope,
    metadata: CaptureMetadata,
    operation: CaptureOperation,
) -> Command {
    let mut command = context.lock();
    command
        .arg("--no-config")
        .arg("--index-url")
        .arg(index)
        .arg("--no-build")
        .arg("--python")
        .arg("3.12")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env_remove(EnvVars::UV_INTERNAL__RESOLVER_CAPTURE)
        .env_remove(EnvVars::UV_INTERNAL__RESOLVER_CAPTURE_REQUEST);
    match scope {
        CaptureScope::Workspace => {}
        CaptureScope::Script => {
            command.arg("--script").arg("script.py");
        }
    }
    match metadata {
        CaptureMetadata::Standard => {}
        CaptureMetadata::WithoutMetadata => {
            command
                .arg("--preview-features")
                .arg("lock-without-metadata");
        }
    }
    match operation {
        CaptureOperation::Write => {}
        CaptureOperation::DryRun => {
            command.arg("--dry-run");
        }
        CaptureOperation::Locked => {
            command.arg("--locked");
        }
        CaptureOperation::Frozen => {
            command.arg("--frozen");
        }
    }
    command
}

fn evidence_directory(context: &TestContext) -> Result<tempfile::TempDir> {
    Ok(tempfile::Builder::new()
        .prefix("resolver-evidence-")
        .tempdir_in(context.root.path())?)
}

fn with_capture(mut command: Command, destination: &Path) -> Command {
    command
        .env(EnvVars::UV_INTERNAL__RESOLVER_CAPTURE, destination)
        .env(EnvVars::UV_INTERNAL__RESOLVER_CAPTURE_REQUEST, REQUEST);
    command
}

fn run_captured(command: Command, destination: &Path) -> Result<(u32, Output)> {
    let child = with_capture(command, destination)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let producer_pid = child.id();
    Ok((producer_pid, child.wait_with_output()?))
}

fn assert_same_output(context: &TestContext, expected: &Output, actual: &Output) {
    assert_eq!(expected.status, actual.status);
    for (expected, actual) in [
        (&expected.stdout, &actual.stdout),
        (&expected.stderr, &actual.stderr),
    ] {
        assert_eq!(
            apply_filters(
                String::from_utf8_lossy(expected).into_owned(),
                context.filters()
            ),
            apply_filters(
                String::from_utf8_lossy(actual).into_owned(),
                context.filters()
            )
        );
    }
}

fn read_capture(
    destination: &Path,
    producer_pid: u32,
    scope: CaptureScope,
    metadata: CaptureMetadata,
    operation: CaptureOperation,
) -> Result<Value> {
    let file = fs_err::metadata(destination)
        .with_context(|| format!("missing capture at {}", destination.display()))?;
    assert!(file.is_file());
    assert!(file.len() <= 8 * 1024 * 1024);
    #[cfg(unix)]
    assert_eq!(file.permissions().mode() & 0o777, 0o600);
    let bytes = fs_err::read(destination)?;
    let capture: Value = serde_json::from_slice(&bytes)?;
    assert_eq!(capture["schema"], 1);
    assert_eq!(capture["request"], REQUEST);
    assert_eq!(capture["producer_pid"], producer_pid);
    assert_eq!(capture["command"], "lock");
    assert_eq!(capture["scope"], json!(scope));
    assert_eq!(capture["metadata"], json!(metadata));
    assert_eq!(capture["operation"], json!(operation));
    assert_eq!(capture["terminal"], "no_solution");
    assert_eq!(capture["status"], "complete");
    assert!(capture.get("reason").is_none());
    assert!(
        capture["limits"]["json_bytes"]
            .as_u64()
            .is_some_and(|limit| bytes.len() as u64 <= limit)
    );
    assert!(
        capture["graph"]["nodes"]
            .as_array()
            .is_some_and(|nodes| !nodes.is_empty())
    );
    Ok(capture)
}

/// The direct command determines the request identity, scope, operation, and lock representation.
#[test]
fn resolver_capture_direct_lock_modes() -> Result<()> {
    let server = PackseServer::from_scenario_without_build_dependencies(&Scenario::empty());
    allow_duplicates! {
        for scope in [CaptureScope::Workspace, CaptureScope::Script] {
            for metadata in [CaptureMetadata::Standard, CaptureMetadata::WithoutMetadata] {
                for operation in [
                    CaptureOperation::Write,
                    CaptureOperation::DryRun,
                    CaptureOperation::Locked,
                ] {
                    let context = uv_test::test_context!("3.12");
                    let directory = evidence_directory(&context)?;
                    let destination = directory.path().join("capture.json");
                    if operation == CaptureOperation::Locked {
                        write_document(&context, scope, &[])?;
                        lock_command(&context, &server.index_url(), scope, metadata, CaptureOperation::Write)
                            .assert()
                            .success();
                    }
                    write_document(&context, scope, &["missing"])?;
                    let previous_lock = fs_err::read(lock_path(&context, scope)).ok();
                    let command = || lock_command(&context, &server.index_url(), scope, metadata, operation);
                    let ordinary = match scope {
                        CaptureScope::Workspace => uv_snapshot!(context.filters(), command(), @"
                        exit_code: 1 (failure)
                        ----- stderr -----
                        error: No solution found when resolving dependencies
                          cause: Because missing was not found in the package registry and your project depends on missing, we can conclude that your project's requirements are unsatisfiable.
                        "),
                        CaptureScope::Script => uv_snapshot!(context.filters(), command(), @"
                        exit_code: 1 (failure)
                        ----- stderr -----
                        error: No solution found when resolving dependencies
                          cause: Because missing was not found in the package registry and you require missing, we can conclude that your requirements are unsatisfiable.
                        "),
                    };
                    assert!(!destination.exists());
                    let (producer_pid, captured) = run_captured(command(), &destination)?;
                    assert_same_output(&context, &ordinary, &captured);
                    let capture = read_capture(&destination, producer_pid, scope, metadata, operation)?;
                    assert_eq!(capture["graph"]["index_authentication"], json!({
                        "unauthorized": false,
                        "forbidden": false,
                    }));
                    assert_eq!(fs_err::read(lock_path(&context, scope)).ok(), previous_lock);
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    }?;
    Ok(())
}

/// A recovered failure must not create evidence for a command that eventually succeeds.
#[test]
fn resolver_capture_discards_recovered_failures() -> Result<()> {
    allow_duplicates! {
        for scenario in [
            "fork/non-local-fork-marker-unreachable.toml",
            "fork/non-local-fork-marker-eager.toml",
        ] {
            let server = PackseServer::new(scenario);
            let context = uv_test::test_context!("3.12");
            write_document(&context, CaptureScope::Workspace, &["a; sys_platform == 'win32'"])?;
            let directory = evidence_directory(&context)?;
            let destination = directory.path().join("capture.json");
            let mut command = lock_command(
                &context,
                &server.index_url(),
                CaptureScope::Workspace,
                CaptureMetadata::Standard,
                CaptureOperation::Write,
            );
            command.arg("--fork-strategy").arg("fewest");
            assert!(!lock_path(&context, CaptureScope::Workspace).exists());
            uv_snapshot!(context.filters(), with_capture(command, &destination), @"
            exit_code: 0 (success)
            ----- stderr -----
            Resolved 2 packages in [TIME]
            ");
            assert!(lock_path(&context, CaptureScope::Workspace).exists());
            assert!(!destination.exists());
        }
        Ok::<(), anyhow::Error>(())
    }?;
    Ok(())
}

/// A retry that still fails publishes the final error, not an earlier failed branch.
#[test]
fn resolver_capture_uses_final_retried_no_solution() -> Result<()> {
    let server = PackseServer::new("fork/non-local-fork-marker-eager-unsatisfiable.toml");
    let context = uv_test::test_context!("3.12");
    write_document(
        &context,
        CaptureScope::Workspace,
        &["a; sys_platform == 'win32'"],
    )?;
    let directory = evidence_directory(&context)?;
    let destination = directory.path().join("capture.json");
    let command = || {
        let mut command = lock_command(
            &context,
            &server.index_url(),
            CaptureScope::Workspace,
            CaptureMetadata::Standard,
            CaptureOperation::Write,
        );
        command.arg("--fork-strategy").arg("fewest");
        command
    };
    let (producer_pid, captured) = run_captured(command(), &destination)?;
    let ordinary = uv_snapshot!(context.filters(), command(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies for split (markers: python_full_version >= '3.14' and sys_platform == 'win32')
      cause: Because there are no versions of absent{python_full_version >= '3.14' and sys_platform == 'win32'} and all versions of a depend on absent{python_full_version >= '3.14' and sys_platform == 'win32'}, we can conclude that all versions of a cannot be used.
             And because your project depends on a{sys_platform == 'win32'}, we can conclude that your project's requirements are unsatisfiable.

    hint: While the active Python version is 3.12, the resolution failed for other Python versions supported by your project. Consider limiting your project's supported Python versions using `requires-python`.
    ");
    assert_same_output(&context, &ordinary, &captured);
    let capture = read_capture(
        &destination,
        producer_pid,
        CaptureScope::Workspace,
        CaptureMetadata::Standard,
        CaptureOperation::Write,
    )?;
    assert!(
        capture["graph"]["observations"]
            .as_array()
            .is_some_and(|observations| observations.iter().any(|value| value["name"] == "absent"))
    );
    assert!(!lock_path(&context, CaptureScope::Workspace).exists());
    Ok(())
}

/// Reading a frozen lock, or failing before resolution, cannot publish a solver failure.
#[test]
fn resolver_capture_requires_resolution() -> Result<()> {
    let server = PackseServer::from_scenario_without_build_dependencies(&Scenario::empty());
    for scope in [CaptureScope::Workspace, CaptureScope::Script] {
        for operation in [CaptureOperation::Locked, CaptureOperation::Frozen] {
            let context = uv_test::test_context!("3.12");
            write_document(&context, scope, &["missing"])?;
            let directory = evidence_directory(&context)?;
            let destination = directory.path().join("capture.json");
            let command = || {
                lock_command(
                    &context,
                    &server.index_url(),
                    scope,
                    CaptureMetadata::Standard,
                    operation,
                )
            };
            let ordinary = command().output()?;
            assert_eq!(ordinary.status.code(), Some(1), "{ordinary:?}");
            let (_, captured) = run_captured(command(), &destination)?;
            assert_same_output(&context, &ordinary, &captured);
            assert!(!destination.exists());
        }

        let context = uv_test::test_context!("3.12");
        write_document(&context, scope, &[])?;
        lock_command(
            &context,
            &server.index_url(),
            scope,
            CaptureMetadata::Standard,
            CaptureOperation::Write,
        )
        .assert()
        .success();
        let previous_lock = fs_err::read(lock_path(&context, scope))?;
        write_document(&context, scope, &["missing"])?;
        let directory = evidence_directory(&context)?;
        let destination = directory.path().join("capture.json");
        let command = lock_command(
            &context,
            &server.index_url(),
            scope,
            CaptureMetadata::Standard,
            CaptureOperation::Frozen,
        );
        let (_, output) = run_captured(command, &destination)?;
        output.assert().success();
        assert_eq!(fs_err::read(lock_path(&context, scope))?, previous_lock);
        assert!(!destination.exists());
    }
    Ok(())
}

/// Other resolver consumers do not receive the direct lock capability.
#[test]
fn resolver_capture_is_not_enabled_for_sync() -> Result<()> {
    let server = PackseServer::from_scenario_without_build_dependencies(&Scenario::empty());
    let context = uv_test::test_context!("3.12");
    write_document(&context, CaptureScope::Workspace, &["missing"])?;
    let directory = evidence_directory(&context)?;
    let destination = directory.path().join("capture.json");
    let mut command = context.sync();
    command
        .arg("--no-config")
        .arg("--index-url")
        .arg(server.index_url())
        .arg("--no-build")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER);
    uv_snapshot!(context.filters(), with_capture(command, &destination), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because missing was not found in the package registry and your project depends on missing, we can conclude that your project's requirements are unsatisfiable.
    ");
    assert!(!destination.exists());
    Ok(())
}

/// A build-system failure may contain a no-solution error without being the direct operation.
#[test]
fn resolver_capture_rejects_nested_build_no_solution() -> Result<()> {
    let server = PackseServer::from_scenario_without_build_dependencies(&Scenario::empty());
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "capture-project"
        version = "0.1.0"
        requires-python = ">=3.12,<3.15"
        dependencies = ["broken"]

        [tool.uv.sources]
        broken = { path = "broken" }
    "#})?;
    context.temp_dir.child("broken").create_dir_all()?;
    context
        .temp_dir
        .child("broken/pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "broken"
        dynamic = ["version"]

        [build-system]
        requires = ["missing-build-backend==1"]
        build-backend = "missing_backend"
    "#})?;
    let directory = evidence_directory(&context)?;
    let destination = directory.path().join("capture.json");
    let mut command = context.lock();
    command
        .arg("--no-config")
        .arg("--index-url")
        .arg(server.index_url())
        .env_remove(EnvVars::UV_EXCLUDE_NEWER);
    uv_snapshot!(context.filters(), with_capture(command, &destination), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to build `broken @ file://[TEMP_DIR]/broken`
      cause: Failed to resolve requirements from `build-system.requires`
      cause: No solution found when resolving: `missing-build-backend==1`
      cause: Because missing-build-backend was not found in the package registry and you require missing-build-backend==1, we can conclude that your requirements are unsatisfiable.
    ");
    assert!(!destination.exists());
    Ok(())
}

/// Startup removes both request inputs before invoking Python or another uv process.
#[test]
fn resolver_capture_is_not_inherited() -> Result<()> {
    let server = PackseServer::from_scenario_without_build_dependencies(&Scenario::empty());
    let context = uv_test::test_context!("3.12");
    write_document(&context, CaptureScope::Workspace, &["missing"])?;
    let directory = evidence_directory(&context)?;
    let destination = directory.path().join("capture.json");
    let mut command = context.run();
    command
        .arg("--no-project")
        .arg("--no-sync")
        .arg("--")
        .arg("python")
        .arg("-c")
        .arg(indoc! {r#"
            import json
            import os
            import subprocess
            import sys

            nested = subprocess.run([
                os.environ["UV"], "lock", "--no-config", "--cache-dir", sys.argv[1],
                "--python", sys.executable, "--index-url", sys.argv[2], "--no-build",
            ], capture_output=True, text=True)
            print(json.dumps({
                "destination_present": "UV_INTERNAL__RESOLVER_CAPTURE" in os.environ,
                "request_present": "UV_INTERNAL__RESOLVER_CAPTURE_REQUEST" in os.environ,
                "nested_exit": nested.returncode,
                "nested_no_solution": "No solution found when resolving dependencies" in nested.stderr,
            }, sort_keys=True))
        "#})
        .arg(context.cache_dir.path())
        .arg(server.index_url())
        .env_remove(EnvVars::UV_EXCLUDE_NEWER);
    let (_, output) = run_captured(command, &destination)?;
    assert!(output.status.success(), "{output:?}");
    assert_json_snapshot!(serde_json::from_slice::<Value>(&output.stdout)?, @r#"
    {
      "destination_present": false,
      "nested_exit": 1,
      "nested_no_solution": true,
      "request_present": false
    }
    "#);
    assert!(!destination.exists());
    Ok(())
}

/// Publication is create-only and cannot change the selected command's output or status.
#[test]
fn resolver_capture_never_clobbers_existing_entries() -> Result<()> {
    let server = PackseServer::from_scenario_without_build_dependencies(&Scenario::empty());
    let context = uv_test::test_context!("3.12");
    write_document(&context, CaptureScope::Workspace, &["missing"])?;
    let directory = evidence_directory(&context)?;
    let existing_file = directory.path().join("existing.json");
    fs_err::write(&existing_file, b"retained prior evidence\n")?;
    let existing_directory = directory.path().join("directory.json");
    fs_err::create_dir(&existing_directory)?;
    let missing_parent = directory.path().join("missing/capture.json");
    let command = || {
        lock_command(
            &context,
            &server.index_url(),
            CaptureScope::Workspace,
            CaptureMetadata::Standard,
            CaptureOperation::Write,
        )
    };
    let ordinary = command().output()?;
    assert_eq!(ordinary.status.code(), Some(1), "{ordinary:?}");
    for destination in [&existing_file, &existing_directory, &missing_parent] {
        let (_, captured) = run_captured(command(), destination)?;
        assert_same_output(&context, &ordinary, &captured);
    }
    assert_eq!(fs_err::read(&existing_file)?, b"retained prior evidence\n");
    assert!(existing_directory.is_dir());
    assert_eq!(fs_err::read_dir(&existing_directory)?.count(), 0);
    assert!(!directory.path().join("missing").exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn resolver_capture_publication_io_failure_keeps_original_error() -> Result<()> {
    let server = PackseServer::from_scenario_without_build_dependencies(&Scenario::empty());
    let context = uv_test::test_context!("3.12");
    write_document(&context, CaptureScope::Workspace, &["missing"])?;
    let directory = evidence_directory(&context)?;
    let dangling = directory.path().join("dangling.json");
    let absent = directory.path().join("absent.json");
    fs_err::os::unix::fs::symlink(&absent, &dangling)?;
    let command = || {
        lock_command(
            &context,
            &server.index_url(),
            CaptureScope::Workspace,
            CaptureMetadata::Standard,
            CaptureOperation::Write,
        )
    };
    let ordinary = command().output()?;
    assert_eq!(ordinary.status.code(), Some(1), "{ordinary:?}");
    let (_, captured) = run_captured(command(), &dangling)?;
    assert_same_output(&context, &ordinary, &captured);
    assert_eq!(fs_err::read_link(&dangling)?, absent);
    assert!(!absent.exists());

    let unwritable = directory.path().join("read-only");
    fs_err::create_dir(&unwritable)?;
    let destination = unwritable.join("capture.json");
    let guard = ReadOnlyDirectoryGuard::new(unwritable)?;
    let (_, captured) = run_captured(command(), &destination)?;
    assert_same_output(&context, &ordinary, &captured);
    assert!(!destination.exists());
    drop(guard);
    Ok(())
}

#[cfg(windows)]
#[test]
fn resolver_capture_windows_long_path_is_create_only() -> Result<()> {
    let server = PackseServer::from_scenario_without_build_dependencies(&Scenario::empty());
    let context = uv_test::test_context!("3.12");
    write_document(&context, CaptureScope::Workspace, &["missing"])?;
    let directory = evidence_directory(&context)?;
    let mut parent = directory.path().simplified().to_path_buf();
    while parent.as_os_str().len() <= 280 {
        parent.push("capture-directory");
    }
    fs_err::create_dir_all(&parent)?;
    let destination = parent.join("capture.json");
    let command = || {
        lock_command(
            &context,
            &server.index_url(),
            CaptureScope::Workspace,
            CaptureMetadata::Standard,
            CaptureOperation::Write,
        )
    };
    let (producer_pid, first) = run_captured(command(), &destination)?;
    assert_eq!(first.status.code(), Some(1), "{first:?}");
    read_capture(
        &destination,
        producer_pid,
        CaptureScope::Workspace,
        CaptureMetadata::Standard,
        CaptureOperation::Write,
    )?;
    let original = fs_err::read(&destination)?;
    let (_, second) = run_captured(command(), &destination)?;
    assert_same_output(&context, &first, &second);
    assert_eq!(fs_err::read(&destination)?, original);
    Ok(())
}

/// Package-not-found observations alone do not distinguish an authentication failure from a 404.
#[tokio::test]
async fn resolver_capture_records_global_index_authentication() -> Result<()> {
    let mut observations = Vec::new();
    for (status, ignored, unauthorized, forbidden) in [
        (401, None, true, false),
        (403, None, false, true),
        (404, None, false, false),
        (401, Some(401), false, false),
        (403, Some(403), false, false),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/simple/missing/"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let context = uv_test::test_context!("3.12").with_http_retries("0");
        let ignored = ignored.map(|status| format!("ignore-error-codes = [{status}]"));
        context
            .temp_dir
            .child("pyproject.toml")
            .write_str(&formatdoc! {r#"
            [project]
            name = "capture-project"
            version = "0.1.0"
            requires-python = ">=3.12,<3.15"
            dependencies = ["missing"]

            [[tool.uv.index]]
            name = "capture-registry"
            url = "{}/simple/"
            default = true
            {}
        "#, server.uri(), ignored.as_deref().unwrap_or("")})?;
        let directory = evidence_directory(&context)?;
        let destination = directory.path().join("capture.json");
        let command = || {
            let mut command = context.lock();
            command
                .arg("--index-strategy")
                .arg("first-index")
                .arg("--no-build")
                .env_remove(EnvVars::UV_EXCLUDE_NEWER);
            command
        };
        let (producer_pid, captured) = run_captured(command(), &destination)?;
        assert_eq!(captured.status.code(), Some(1), "{captured:?}");
        let capture = read_capture(
            &destination,
            producer_pid,
            CaptureScope::Workspace,
            CaptureMetadata::Standard,
            CaptureOperation::Write,
        )?;
        assert_eq!(
            capture["graph"]["index_authentication"],
            json!({
                "unauthorized": unauthorized,
                "forbidden": forbidden,
            })
        );
        let observation = capture["graph"]["observations"]
            .as_array()
            .and_then(|observations| observations.iter().find(|value| value["name"] == "missing"))
            .context("missing registry observation")?;
        assert_eq!(observation["listing"], "not_found");
        assert_eq!(observation["unavailable"]["kind"], "package_not_found");
        assert!(!fs_err::read_to_string(&destination)?.contains(&server.uri()));
        let ordinary = command().output()?;
        assert_same_output(&context, &ordinary, &captured);
        assert!(server.received_requests().await.is_some_and(|requests| {
            requests
                .iter()
                .any(|request| request.url.path() == "/simple/missing/")
        }));
        observations.push(json!({
            "http_status": status,
            "ignored": ignored.is_some(),
            "authentication": capture["graph"]["index_authentication"],
            "listing": observation["listing"],
            "unavailable": observation["unavailable"]["kind"],
        }));
    }
    assert_json_snapshot!(observations, @r#"
    [
      {
        "authentication": {
          "forbidden": false,
          "unauthorized": true
        },
        "http_status": 401,
        "ignored": false,
        "listing": "not_found",
        "unavailable": "package_not_found"
      },
      {
        "authentication": {
          "forbidden": true,
          "unauthorized": false
        },
        "http_status": 403,
        "ignored": false,
        "listing": "not_found",
        "unavailable": "package_not_found"
      },
      {
        "authentication": {
          "forbidden": false,
          "unauthorized": false
        },
        "http_status": 404,
        "ignored": false,
        "listing": "not_found",
        "unavailable": "package_not_found"
      },
      {
        "authentication": {
          "forbidden": false,
          "unauthorized": false
        },
        "http_status": 401,
        "ignored": true,
        "listing": "not_found",
        "unavailable": "package_not_found"
      },
      {
        "authentication": {
          "forbidden": false,
          "unauthorized": false
        },
        "http_status": 403,
        "ignored": true,
        "listing": "not_found",
        "unavailable": "package_not_found"
      }
    ]
    "#);
    Ok(())
}
