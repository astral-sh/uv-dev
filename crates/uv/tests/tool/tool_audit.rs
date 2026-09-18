use std::process::{Command, Output};
use std::sync::LazyLock;

use anyhow::{Context, Result, anyhow};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use indoc::indoc;
use insta::assert_json_snapshot;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_static::EnvVars;
use uv_test::json_schema::JsonSchema;
use uv_test::jsonl::{JsonlOutput, JsonlResultExpectation};
use uv_test::{TestContext, uv_snapshot};

static TOOL_AUDIT_SCHEMA: LazyLock<std::result::Result<JsonSchema, String>> = LazyLock::new(|| {
    JsonSchema::new(include_str!(
        "../../../../docs/reference/internals/tool-audit.schema.json"
    ))
    .map_err(|error| error.to_string())
});

static TOOL_AUDIT_JSONL_SCHEMA: LazyLock<std::result::Result<JsonSchema, String>> =
    LazyLock::new(|| {
        JsonSchema::new(include_str!(
            "../../../../docs/reference/internals/tool-audit-jsonl.schema.json"
        ))
        .map_err(|error| error.to_string())
    });

fn parse_tool_audit_report(contents: &[u8]) -> Result<Value> {
    TOOL_AUDIT_SCHEMA
        .as_ref()
        .map_err(|error| anyhow!("invalid tool audit schema: {error}"))?
        .parse(contents)
        .context("tool audit schema mismatch")
}

fn parse_tool_audit_jsonl(
    output: &Output,
    expectation: JsonlResultExpectation,
) -> Result<JsonlOutput> {
    let schema = TOOL_AUDIT_JSONL_SCHEMA
        .as_ref()
        .map_err(|error| anyhow!("invalid JSONL tool audit schema: {error}"))?;
    JsonlOutput::parse(schema, output, expectation)
}

fn parse_tool_audit_jsonl_report(output: &Output) -> Result<(Vec<Value>, Value)> {
    let parsed = parse_tool_audit_jsonl(output, JsonlResultExpectation::Required)?;
    let mut result = parsed.result.context("missing JSONL tool audit result")?;
    result
        .as_object_mut()
        .context("expected an object-valued tool audit result")?
        .remove("type");
    Ok((
        parsed.progress,
        parse_tool_audit_report(&serde_json::to_vec(&result)?)?,
    ))
}

/// A test context for auditing tools installed into isolated directories.
struct AuditTestContext {
    /// The shared filesystem, cache, environment, and virtual environment for the `uv` invocation.
    inner: TestContext,
    /// The root directory for installed tools.
    tool_dir: ChildPath,
}

impl AuditTestContext {
    /// Create an audit test context with the required preview features enabled.
    fn new(python_version: &str) -> Self {
        let inner = uv_test::test_context!(python_version)
            .with_tool_dirs()
            .with_env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks");
        let tool_dir = inner.temp_dir.child("tools");

        Self { inner, tool_dir }
    }

    /// Add a custom snapshot filter.
    fn with_filter(mut self, filter: (impl Into<String>, impl Into<String>)) -> Self {
        self.inner = self.inner.with_filter(filter);
        self
    }

    fn filters(&self) -> Vec<(&str, &str)> {
        self.inner.filters()
    }

    fn report(&self, format: &str, service_url: &str) -> Command {
        let mut command = self.inner.tool_audit();
        command
            .env_remove(EnvVars::RUST_LOG)
            .env(
                EnvVars::UV_PREVIEW_FEATURES,
                "audit,tool-install-locks,json-output,jsonl",
            )
            .args(["--output-format", format, "--service-url", service_url]);
        command
    }

    /// Install a tool from the test links, optionally with a lockfile.
    fn install_tool(&self, name: &str, locked: bool) {
        let links = self.inner.workspace_root.join("test/links");

        let mut command = self.inner.tool_install();
        command
            .arg(name)
            .arg("--no-index")
            .arg("--find-links")
            .arg(links);
        if !locked {
            command.env_remove(EnvVars::UV_PREVIEW_FEATURES);
        }
        command.assert().success();
    }
}

async fn mount_clean_service(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/querybatch"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{ "vulns": [] }]
        })))
        .mount(server)
        .await;
}

async fn mount_vulnerable_service(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/querybatch"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{ "vulns": [{ "id": "PYSEC-2026-0001" }] }]
        })))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v1/vulns/PYSEC-2026-0001"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "PYSEC-2026-0001",
            "modified": "2026-01-01T00:00:00Z",
            "summary": "A test vulnerability in simple-launcher",
            "affected": [{
                "package": {
                    "ecosystem": "PyPI",
                    "name": "simple-launcher"
                },
                "ranges": [{
                    "type": "ECOSYSTEM",
                    "events": [
                        { "introduced": "0" },
                        { "fixed": "0.2.0" }
                    ]
                }]
            }],
            "references": [{
                "type": "ADVISORY",
                "url": "https://example.com/advisory/PYSEC-2026-0001"
            }]
        })))
        .mount(server)
        .await;
}

#[test]
fn tool_audit_requires_selection() {
    let context = AuditTestContext::new("3.12");

    uv_snapshot!(context.filters(), context.inner.tool_audit(), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the following required arguments were not provided:
      <NAME>...

    Usage: uv tool audit --cache-dir [CACHE_DIR] <NAME>...

    For more information, try '--help'.
    ");

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all")
        .arg("simple-launcher"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--all' cannot be used with '<NAME>...'

    Usage: uv tool audit --cache-dir [CACHE_DIR] --all <NAME>...

    For more information, try '--help'.
    ");
}

#[test]
fn tool_audit_preview_features() {
    let context = AuditTestContext::new("3.12");

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all")
        .env_remove(EnvVars::UV_PREVIEW_FEATURES), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: `uv tool audit` is experimental and may change without warning. Pass `--preview-features audit,tool-install-locks` to disable this warning.
    No tools installed
    ");

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all")
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: `uv tool audit` is experimental and may change without warning. Pass `--preview-features tool-install-locks` to disable this warning.
    No tools installed
    ");

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all")
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: `uv tool audit` is experimental and may change without warning. Pass `--preview-features audit` to disable this warning.
    No tools installed
    ");

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all")
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks"), @"
    exit_code: 0 (success)
    ----- stderr -----
    No tools installed
    ");
}

#[test]
fn tool_audit_unknown_tool() {
    let context = AuditTestContext::new("3.12");

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("simple-launcher"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `simple-launcher` is not installed; run `uv tool install simple-launcher` to install
    ");
}

#[test]
fn tool_audit_missing_lockfile() {
    let context = AuditTestContext::new("3.12");
    context.install_tool("simple-launcher", false);

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Skipping tool `simple-launcher` because it does not have a lockfile; reinstall it with `--preview-features tool-install-locks` to audit it
    No auditable tools installed
    ");

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("simple-launcher"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Tool `simple-launcher` does not have a lockfile; reinstall it with `--preview-features tool-install-locks` to audit it
    ");
}

#[test]
fn tool_audit_invalid_receipt() -> Result<()> {
    let context = AuditTestContext::new("3.12");
    context.install_tool("simple-launcher", true);
    fs_err::write(
        context
            .tool_dir
            .join("simple-launcher")
            .join("uv-receipt.toml"),
        "not valid toml",
    )?;

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Ignoring malformed tool `simple-launcher` (run `uv tool uninstall simple-launcher` to remove)
    No auditable tools installed
    ");

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("simple-launcher"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Tool `simple-launcher` has an invalid receipt: Failed to read `uv-receipt.toml` at [TEMP_DIR]/tools/simple-launcher/uv-receipt.toml
    ");

    Ok(())
}

#[test]
fn tool_audit_invalid_lockfile() -> Result<()> {
    let context = AuditTestContext::new("3.12");
    context.install_tool("simple-launcher", true);
    fs_err::write(
        context.tool_dir.join("simple-launcher").join("uv.lock"),
        "not valid toml",
    )?;

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Skipping tool `simple-launcher` because its lockfile at `tools/simple-launcher/uv.lock` is invalid: TOML parse error at line 1, column 5
      |
    1 | not valid toml
      |     ^
    key with no value, expected `=`

    No auditable tools installed
    ");

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("simple-launcher"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse the lockfile for tool `simple-launcher` at `tools/simple-launcher/uv.lock`: TOML parse error at line 1, column 5
      |
    1 | not valid toml
      |     ^
    key with no value, expected `=`
    ");

    Ok(())
}

#[test]
fn tool_audit_unsupported_lockfile_version() -> Result<()> {
    let context = AuditTestContext::new("3.12");
    context.install_tool("simple-launcher", true);

    let lock_path = context.tool_dir.join("simple-launcher").join("uv.lock");
    let contents = fs_err::read_to_string(&lock_path)?;
    fs_err::write(
        &lock_path,
        contents.replacen("version = 1\n", "version = 2\n", 1),
    )?;

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Skipping tool `simple-launcher` because its lockfile at `tools/simple-launcher/uv.lock` uses an unsupported schema version (v2, but only v1 is supported)
    No auditable tools installed
    ");

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("simple-launcher"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The lockfile for tool `simple-launcher` at `tools/simple-launcher/uv.lock` uses an unsupported schema version (v2, but only v1 is supported)
    ");

    Ok(())
}

#[test]
fn tool_audit_unparsable_unsupported_lockfile_version() -> Result<()> {
    let context = AuditTestContext::new("3.12");
    context.install_tool("simple-launcher", true);

    let lock_path = context.tool_dir.join("simple-launcher").join("uv.lock");
    let contents = fs_err::read_to_string(&lock_path)?
        .replacen("version = 1\n", "version = 2\n", 1)
        .replacen("version = \"0.1.0\"\n", "version = false\n", 1);
    fs_err::write(&lock_path, contents)?;

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("simple-launcher"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The lockfile for tool `simple-launcher` at `tools/simple-launcher/uv.lock` uses an unsupported schema version (v2, but only v1 is supported)
    ");

    Ok(())
}

#[tokio::test]
async fn tool_audit_one_tool() {
    let context = AuditTestContext::new("3.12");
    context.install_tool("simple-launcher", true);

    let server = MockServer::start().await;
    mount_clean_service(&server).await;

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("simple-launcher")
        .arg("--service-url")
        .arg(server.uri()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Auditing `simple-launcher`
    Found no known vulnerabilities and no adverse project statuses in 1 package
    ");
}

#[tokio::test]
async fn tool_audit_all_tools() {
    let context = AuditTestContext::new("3.13");
    context.install_tool("simple-launcher", true);
    context.install_tool("basic-app", true);

    let server = MockServer::start().await;
    mount_clean_service(&server).await;

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all")
        .arg("--service-url")
        .arg(server.uri()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Auditing `basic-app`
    Found no known vulnerabilities and no adverse project statuses in 1 package
    Auditing `simple-launcher`
    Found no known vulnerabilities and no adverse project statuses in 1 package
    ");
}

#[tokio::test]
async fn tool_audit_multiple_tools() {
    let context = AuditTestContext::new("3.13");
    context.install_tool("simple-launcher", true);
    context.install_tool("basic-app", true);

    let server = MockServer::start().await;
    mount_clean_service(&server).await;

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("simple-launcher")
        .arg("basic-app")
        .arg("--service-url")
        .arg(server.uri()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Auditing `basic-app`
    Found no known vulnerabilities and no adverse project statuses in 1 package
    Auditing `simple-launcher`
    Found no known vulnerabilities and no adverse project statuses in 1 package
    ");
}

#[tokio::test]
async fn tool_audit_mixed_lockfiles() {
    let context = AuditTestContext::new("3.13");
    context.install_tool("simple-launcher", true);
    context.install_tool("basic-app", false);

    let server = MockServer::start().await;
    mount_clean_service(&server).await;

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all")
        .arg("--service-url")
        .arg(server.uri()), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Skipping tool `basic-app` because it does not have a lockfile; reinstall it with `--preview-features tool-install-locks` to audit it
    Auditing `simple-launcher`
    Found no known vulnerabilities and no adverse project statuses in 1 package
    ");
}

#[tokio::test]
async fn tool_audit_shared_dependencies() {
    let context = AuditTestContext::new("3.13");
    let links = context.inner.workspace_root.join("test/links");

    context
        .inner
        .tool_install()
        .arg("simple-launcher")
        .arg("--with")
        .arg("basic-app")
        .arg("--no-index")
        .arg("--find-links")
        .arg(links)
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        .assert()
        .success();
    context.install_tool("basic-app", true);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/querybatch"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{ "vulns": [] }, { "vulns": [] }]
        })))
        .mount(&server)
        .await;

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all")
        .arg("--service-url")
        .arg(server.uri()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Auditing `basic-app`
    Found no known vulnerabilities and no adverse project statuses in 1 package
    Auditing `simple-launcher`
    Found no known vulnerabilities and no adverse project statuses in 2 packages
    ");
}

#[tokio::test]
async fn tool_audit_vulnerability() {
    let context = AuditTestContext::new("3.12");
    context.install_tool("simple-launcher", true);

    let server = MockServer::start().await;
    mount_vulnerable_service(&server).await;

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("simple-launcher")
        .arg("--service-url")
        .arg(server.uri()), @"
    exit_code: 1 (failure)
    ----- stdout -----
    Tool `simple-launcher`:

    Vulnerabilities:

    simple-launcher 0.1.0 has 1 known vulnerability:

    - PYSEC-2026-0001: A test vulnerability in simple-launcher

      Fixed in: 0.2.0

      Advisory information: https://example.com/advisory/PYSEC-2026-0001


    ----- stderr -----
    Auditing `simple-launcher`
    Found 1 known vulnerability and no adverse project statuses in 1 package
    ");
}

#[tokio::test]
async fn tool_audit_ignore() {
    let context = AuditTestContext::new("3.12");
    context.install_tool("simple-launcher", true);

    let server = MockServer::start().await;
    mount_vulnerable_service(&server).await;

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all")
        .arg("--ignore")
        .arg("PYSEC-2026-0001")
        .arg("--ignore")
        .arg("CVE-DOES-NOT-EXIST")
        .arg("--service-url")
        .arg(server.uri()), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Ignored vulnerability `CVE-DOES-NOT-EXIST` does not match any vulnerability in the selected tools
    Auditing `simple-launcher`
    Found no known vulnerabilities and no adverse project statuses in 1 package
    ");
}

#[tokio::test]
async fn tool_audit_configured_ignore() -> Result<()> {
    let context = AuditTestContext::new("3.12");
    let config = context.inner.temp_dir.child("uv.toml");
    context.install_tool("simple-launcher", true);
    config.write_str(indoc! {r#"
        [audit]
        ignore = ["PYSEC-2026-0001"]
    "#})?;

    let server = MockServer::start().await;
    mount_vulnerable_service(&server).await;

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all")
        .arg("--config-file")
        .arg(config.path())
        .arg("--service-url")
        .arg(server.uri()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Auditing `simple-launcher`
    Found no known vulnerabilities and no adverse project statuses in 1 package
    ");

    Ok(())
}

#[tokio::test]
async fn tool_audit_json() -> Result<()> {
    let context = AuditTestContext::new("3.12");
    context.install_tool("simple-launcher", true);

    let server = MockServer::start().await;
    mount_clean_service(&server).await;

    let output = uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all")
        .arg("--output-format")
        .arg("json")
        .arg("--service-url")
        .arg(server.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks,json-output"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "tools": [
        {
          "name": "simple-launcher",
          "summary": {
            "audited_packages": 1,
            "vulnerabilities": 0,
            "adverse_statuses": 0
          },
          "vulnerabilities": [],
          "adverse_statuses": []
        }
      ]
    }
    "#);
    let expected = parse_tool_audit_report(&output.stdout)?;
    assert_eq!(expected["tools"][0]["name"], "simple-launcher");
    for mode in ["-q", "--no-progress"] {
        let output = context
            .report("json", &server.uri())
            .args(["--all", mode])
            .output()?;
        assert!(output.status.success());
        assert_eq!(parse_tool_audit_report(&output.stdout)?, expected);
    }
    let silent = context
        .report("json", &server.uri())
        .args(["--all", "-qq"])
        .output()?;
    assert!(silent.status.success());
    assert!(silent.stdout.is_empty());
    Ok(())
}

#[tokio::test]
async fn tool_audit_jsonl() -> Result<()> {
    let context = AuditTestContext::new("3.12");
    context.install_tool("simple-launcher", true);

    let server = MockServer::start().await;
    mount_clean_service(&server).await;

    let output = uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all")
        .arg("--output-format")
        .arg("jsonl")
        .arg("--service-url")
        .arg(server.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks,jsonl"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {"type":"progress","phase":"audit","status":"started","total":1}
    {"type":"progress","phase":"audit","status":"updated","name":"simple-launcher","version":"0.1.0","completed":1,"total":1}
    {"type":"progress","phase":"audit","status":"completed","completed":1,"total":1}
    {"type":"result","schema":{"version":"preview"},"tools":[{"name":"simple-launcher","summary":{"audited_packages":1,"vulnerabilities":0,"adverse_statuses":0},"vulnerabilities":[],"adverse_statuses":[]}]}
    "#
    );
    let (progress, expected) = parse_tool_audit_jsonl_report(&output)?;
    assert_eq!(progress.len(), 3);
    for mode in ["-q", "--no-progress"] {
        let output = context
            .report("jsonl", &server.uri())
            .args(["--all", mode])
            .output()?;
        assert!(output.status.success());
        let (progress, report) = parse_tool_audit_jsonl_report(&output)?;
        assert!(progress.is_empty());
        assert_eq!(report, expected);
    }
    let silent = context
        .report("jsonl", &server.uri())
        .args(["--all", "-qq"])
        .output()?;
    assert!(silent.status.success());
    assert!(silent.stdout.is_empty());
    parse_tool_audit_jsonl(&silent, JsonlResultExpectation::Forbidden)?;
    Ok(())
}

#[tokio::test]
async fn tool_audit_json_preview_warning() -> Result<()> {
    let context = AuditTestContext::new("3.12");
    context.install_tool("simple-launcher", true);

    let server = MockServer::start().await;
    mount_clean_service(&server).await;

    let output = uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all")
        .arg("--output-format")
        .arg("json")
        .arg("--service-url")
        .arg(server.uri()), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "tools": [
        {
          "name": "simple-launcher",
          "summary": {
            "audited_packages": 1,
            "vulnerabilities": 0,
            "adverse_statuses": 0
          },
          "vulnerabilities": [],
          "adverse_statuses": []
        }
      ]
    }

    ----- stderr -----
    warning: The `--output-format json` option is experimental and the schema may change without warning. Pass `--preview-features json-output` to disable this warning.
    "#);
    parse_tool_audit_report(&output.stdout)?;
    Ok(())
}

#[tokio::test]
async fn tool_audit_json_all_tools() -> Result<()> {
    let context = AuditTestContext::new("3.13");
    context.install_tool("simple-launcher", true);
    context.install_tool("basic-app", true);

    let server = MockServer::start().await;
    mount_clean_service(&server).await;

    let output = uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all")
        .arg("--output-format")
        .arg("json")
        .arg("--service-url")
        .arg(server.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks,json-output"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "tools": [
        {
          "name": "basic-app",
          "summary": {
            "audited_packages": 1,
            "vulnerabilities": 0,
            "adverse_statuses": 0
          },
          "vulnerabilities": [],
          "adverse_statuses": []
        },
        {
          "name": "simple-launcher",
          "summary": {
            "audited_packages": 1,
            "vulnerabilities": 0,
            "adverse_statuses": 0
          },
          "vulnerabilities": [],
          "adverse_statuses": []
        }
      ]
    }
    "#);
    let expected = parse_tool_audit_report(&output.stdout)?;
    assert_eq!(expected["tools"][0]["name"], "basic-app");
    assert_eq!(expected["tools"][1]["name"], "simple-launcher");
    let output = context
        .report("jsonl", &server.uri())
        .args(["--all", "--no-progress"])
        .output()?;
    assert!(output.status.success());
    let (progress, report) = parse_tool_audit_jsonl_report(&output)?;
    assert!(progress.is_empty());
    assert_eq!(report, expected);
    Ok(())
}

#[tokio::test]
async fn tool_audit_json_vulnerability() -> Result<()> {
    let context = AuditTestContext::new("3.12");
    context.install_tool("simple-launcher", true);

    let server = MockServer::start().await;
    mount_vulnerable_service(&server).await;

    let output = uv_snapshot!(context.filters(), context
        .report("json", &server.uri())
        .arg("simple-launcher"), @r#"
    exit_code: 1 (failure)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "tools": [
        {
          "name": "simple-launcher",
          "summary": {
            "audited_packages": 1,
            "vulnerabilities": 1,
            "adverse_statuses": 0
          },
          "vulnerabilities": [
            {
              "dependency": {
                "name": "simple-launcher",
                "version": "0.1.0"
              },
              "id": "PYSEC-2026-0001",
              "display_id": "PYSEC-2026-0001",
              "aliases": [],
              "summary": "A test vulnerability in simple-launcher",
              "description": null,
              "link": "https://example.com/advisory/PYSEC-2026-0001",
              "fix_versions": [
                "0.2.0"
              ],
              "published": null,
              "modified": "2026-01-01T00:00:00Z"
            }
          ],
          "adverse_statuses": []
        }
      ]
    }
    "#);
    let expected = parse_tool_audit_report(&output.stdout)?;
    assert_eq!(expected["tools"][0]["summary"]["vulnerabilities"], 1);
    for format in ["json", "jsonl"] {
        for mode in [None, Some("-q"), Some("--no-progress")] {
            let mut command = context.report(format, &server.uri());
            command.arg("simple-launcher");
            if let Some(mode) = mode {
                command.arg(mode);
            }
            let output = command.output()?;
            assert_eq!(output.status.code(), Some(1));
            if format == "jsonl" {
                let (progress, report) = parse_tool_audit_jsonl_report(&output)?;
                if mode.is_some() {
                    assert!(progress.is_empty());
                }
                assert_eq!(report, expected);
            } else {
                assert_eq!(parse_tool_audit_report(&output.stdout)?, expected);
            }
        }
        let silent = context
            .report(format, &server.uri())
            .args(["simple-launcher", "-qq"])
            .output()?;
        assert_eq!(silent.status.code(), Some(1));
        assert!(silent.stdout.is_empty());
        if format == "jsonl" {
            parse_tool_audit_jsonl(&silent, JsonlResultExpectation::Forbidden)?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn tool_audit_json_empty_and_setup_failure() -> Result<()> {
    let context = AuditTestContext::new("3.12");
    let server = MockServer::start().await;

    let output = uv_snapshot!(context.filters(), context
        .report("json", &server.uri())
        .arg("--all"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "tools": []
    }
    "#);
    let expected = parse_tool_audit_report(&output.stdout)?;
    assert_eq!(expected["tools"], json!([]));

    for format in ["json", "jsonl"] {
        let output = context
            .report(format, &server.uri())
            .arg("--all")
            .output()?;
        assert!(output.status.success());
        if format == "jsonl" {
            let (progress, report) = parse_tool_audit_jsonl_report(&output)?;
            assert!(progress.is_empty());
            assert_eq!(report, expected);
        } else {
            assert_eq!(parse_tool_audit_report(&output.stdout)?, expected);
        }
        let output = context
            .report(format, &server.uri())
            .arg("simple-launcher")
            .output()?;
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        if format == "jsonl" {
            let parsed = parse_tool_audit_jsonl(&output, JsonlResultExpectation::Forbidden)?;
            assert!(parsed.progress.is_empty());
        }
    }
    assert!(
        server
            .received_requests()
            .await
            .is_some_and(|requests| requests.is_empty())
    );

    context.install_tool("simple-launcher", false);
    for format in ["json", "jsonl"] {
        let output = context
            .report(format, &server.uri())
            .args(["--all", "--quiet"])
            .output()?;
        assert!(output.status.success());
        if format == "jsonl" {
            let (progress, report) = parse_tool_audit_jsonl_report(&output)?;
            assert!(progress.is_empty());
            assert_eq!(report, expected);
        } else {
            assert_eq!(parse_tool_audit_report(&output.stdout)?, expected);
        }
        let output = context
            .report(format, &server.uri())
            .arg("simple-launcher")
            .output()?;
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        if format == "jsonl" {
            let parsed = parse_tool_audit_jsonl(&output, JsonlResultExpectation::Forbidden)?;
            assert!(parsed.progress.is_empty());
        }
    }
    assert!(
        server
            .received_requests()
            .await
            .is_some_and(|requests| requests.is_empty())
    );
    Ok(())
}

#[tokio::test]
async fn tool_audit_sarif() {
    let context = AuditTestContext::new("3.12").with_filter((uv_version::version(), "[VERSION]"));
    context.install_tool("simple-launcher", true);

    let server = MockServer::start().await;
    mount_clean_service(&server).await;

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all")
        .arg("--output-format")
        .arg("sarif")
        .arg("--service-url")
        .arg(server.uri()), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "$schema": "https://docs.oasis-open.org/sarif/sarif/v2.1.0/os/schemas/sarif-schema-2.1.0.json",
      "runs": [
        {
          "automationDetails": {
            "id": "uv/tool-audit/simple-launcher"
          },
          "invocations": [
            {
              "executionSuccessful": true
            }
          ],
          "results": [],
          "tool": {
            "driver": {
              "downloadUri": "https://github.com/astral-sh/uv",
              "informationUri": "https://pypi.org/project/uv/",
              "name": "uv",
              "semanticVersion": "[VERSION]",
              "version": "[VERSION]"
            }
          }
        }
      ],
      "version": "2.1.0"
    }
    "#);
}

#[test]
fn tool_audit_sarif_no_auditable_tools() {
    let context = AuditTestContext::new("3.12");

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all")
        .arg("--output-format")
        .arg("sarif"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "$schema": "https://docs.oasis-open.org/sarif/sarif/v2.1.0/os/schemas/sarif-schema-2.1.0.json",
      "runs": [],
      "version": "2.1.0"
    }
    "#);

    context.install_tool("simple-launcher", false);

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("--all")
        .arg("--output-format")
        .arg("sarif"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "$schema": "https://docs.oasis-open.org/sarif/sarif/v2.1.0/os/schemas/sarif-schema-2.1.0.json",
      "runs": [],
      "version": "2.1.0"
    }

    ----- stderr -----
    warning: Skipping tool `simple-launcher` because it does not have a lockfile; reinstall it with `--preview-features tool-install-locks` to audit it
    "#);
}

#[tokio::test]
async fn tool_audit_sarif_all_tools() -> Result<()> {
    let context = AuditTestContext::new("3.13");
    context.install_tool("simple-launcher", true);
    context.install_tool("basic-app", true);

    let server = MockServer::start().await;
    mount_clean_service(&server).await;

    let output = context
        .inner
        .tool_audit()
        .arg("--all")
        .arg("--output-format")
        .arg("sarif")
        .arg("--service-url")
        .arg(server.uri())
        .output()?;
    assert_eq!(output.status.code(), Some(0));

    let report: Value = serde_json::from_slice(&output.stdout)?;
    let runs = report["runs"].as_array().map(|runs| {
        runs.iter()
            .map(|run| {
                json!({
                    "automation_id": run["automationDetails"]["id"],
                    "results": run["results"],
                })
            })
            .collect::<Vec<_>>()
    });
    assert_json_snapshot!(runs, @r#"
    [
      {
        "automation_id": "uv/tool-audit/basic-app",
        "results": []
      },
      {
        "automation_id": "uv/tool-audit/simple-launcher",
        "results": []
      }
    ]
    "#);

    Ok(())
}

#[tokio::test]
async fn tool_audit_sarif_vulnerability_location() -> Result<()> {
    let context = AuditTestContext::new("3.12");
    context.install_tool("simple-launcher", true);

    let server = MockServer::start().await;
    mount_vulnerable_service(&server).await;

    let output = context
        .inner
        .tool_audit()
        .arg("simple-launcher")
        .arg("--output-format")
        .arg("sarif")
        .arg("--service-url")
        .arg(server.uri())
        .output()?;
    assert_eq!(output.status.code(), Some(1));

    let report: Value = serde_json::from_slice(&output.stdout)?;
    assert_json_snapshot!(json!({
        "automation_id": report["runs"][0]["automationDetails"]["id"],
        "artifact": report["runs"][0]["results"][0]["locations"][0]["physicalLocation"]
            ["artifactLocation"]["uri"],
        "rule": report["runs"][0]["results"][0]["ruleId"],
        "runs": report["runs"].as_array().map(Vec::len),
    }), @r#"
    {
      "artifact": "temp/tools/simple-launcher/uv.lock",
      "automation_id": "uv/tool-audit/simple-launcher",
      "rule": "PYSEC-2026-0001",
      "runs": 1
    }
    "#);

    Ok(())
}

#[tokio::test]
async fn tool_audit_persisted_index_and_project_status() -> Result<()> {
    let context = AuditTestContext::new("3.12");
    let server = MockServer::start().await;
    let wheel_filename = "simple_launcher-0.1.0-py3-none-any.whl";
    let wheel = fs_err::read(
        context
            .inner
            .workspace_root
            .join("test/links")
            .join(wheel_filename),
    )?;

    let simple_index = json!({
        "meta": { "api-version": "1.1" },
        "name": "simple-launcher",
        "project-status": {
            "status": "archived",
            "reason": "no-longer-maintained"
        },
        "files": [{
            "filename": wheel_filename,
            "url": format!("{}/files/{wheel_filename}", server.uri()),
            "hashes": {
                "sha256": "5327e0bb67cdb46800999de6dcf034bf0a5335702883494af0d8b7f6ca48cee4"
            },
            "core-metadata": true,
            "upload-time": "2024-03-24T00:00:00Z"
        }]
    });
    Mock::given(method("GET"))
        .and(path("/simple/simple-launcher/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            simple_index.to_string(),
            "application/vnd.pypi.simple.v1+json",
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/files/{wheel_filename}.metadata")))
        .respond_with(ResponseTemplate::new(200).set_body_string(indoc! {"
            Metadata-Version: 2.1
            Name: simple-launcher
            Version: 0.1.0
            Requires-Python: >=3.8
        "}))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/files/{wheel_filename}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(wheel))
        .mount(&server)
        .await;
    mount_clean_service(&server).await;

    context
        .inner
        .tool_install()
        .arg("simple-launcher")
        .arg("--index-url")
        .arg(format!("{}/simple", server.uri()))
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.inner.tool_audit()
        .arg("simple-launcher")
        .arg("--service-url")
        .arg(server.uri()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Tool `simple-launcher`:

    Adverse statuses:

    - simple-launcher is archived: no-longer-maintained

    ----- stderr -----
    Auditing `simple-launcher`
    Found no known vulnerabilities and 1 adverse project status in 1 package
    ");

    Ok(())
}
