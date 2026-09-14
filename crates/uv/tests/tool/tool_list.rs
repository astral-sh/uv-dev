use std::sync::LazyLock;

use anyhow::{Context, Result};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::PathChild;
use fs_err as fs;
use insta::assert_snapshot;
use serde_json::Value;
use uv_static::EnvVars;
use uv_test::json_schema::JsonSchema;
use uv_test::uv_snapshot;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

static TOOL_LIST_SCHEMA: LazyLock<std::result::Result<JsonSchema, String>> = LazyLock::new(|| {
    JsonSchema::new(include_str!(
        "../../../../docs/reference/internals/tool-list.schema.json"
    ))
    .map_err(|error| error.to_string())
});

static JSONL_PROGRESS_SCHEMA: LazyLock<std::result::Result<JsonSchema, String>> =
    LazyLock::new(|| {
        JsonSchema::new(include_str!(
            "../../../../docs/reference/internals/jsonl-progress.schema.json"
        ))
        .map_err(|error| error.to_string())
    });

fn parse_tool_list(contents: &[u8]) -> Result<Value> {
    TOOL_LIST_SCHEMA
        .as_ref()
        .map_err(|error| anyhow::anyhow!("invalid tool-list schema: {error}"))?
        .parse(contents)
        .context("tool-list schema mismatch")
}

fn parse_tool_list_jsonl(contents: &[u8]) -> Result<(Vec<Value>, Value)> {
    anyhow::ensure!(
        contents.ends_with(b"\n"),
        "incomplete JSONL tool-list record"
    );
    let mut events = std::str::from_utf8(contents)?
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<serde_json::Result<Vec<_>>>()?;
    let mut report = events
        .pop()
        .context("missing final JSONL tool-list report")?;
    let progress_schema = JSONL_PROGRESS_SCHEMA
        .as_ref()
        .map_err(|error| anyhow::anyhow!("invalid JSONL progress schema: {error}"))?;
    for event in &events {
        progress_schema
            .parse(&serde_json::to_vec(event)?)
            .context("JSONL progress schema mismatch")?;
    }
    let event_type = report
        .as_object_mut()
        .context("JSONL tool-list report is not an object")?
        .remove("type");
    anyhow::ensure!(
        event_type == Some(Value::String("result".to_owned())),
        "final JSONL event is not a result"
    );
    let report = parse_tool_list(&serde_json::to_vec(&report)?)?;
    Ok((events, report))
}

fn tool_list_jsonl(context: &uv_test::TestContext) -> std::process::Command {
    let mut command = context.tool_list();
    command.args(["--output-format", "jsonl", "--preview-features", "jsonl"]);
    command
}

fn install_local_jsonl_tool(context: &uv_test::TestContext) {
    context
        .tool_install()
        .arg(
            context
                .workspace_root
                .join("test/links/simple_launcher-0.1.0-py3-none-any.whl"),
        )
        .arg("--offline")
        .assert()
        .success();
}

macro_rules! tool_list_json_snapshot {
    ($($args:tt)*) => {{
        let output = uv_snapshot!($($args)*);
        if output.status.success() && !output.stdout.is_empty() {
            let result = parse_tool_list(&output.stdout);
            assert!(result.is_ok(), "tool-list schema mismatch: {result:?}");
        }
        output
    }};
}

#[test]
fn tool_list_schema_rejects_invalid_output() -> Result<()> {
    let report = serde_json::json!({
        "schema": {"version": "preview"},
        "tools": [{
            "name": "example",
            "version": "1.0",
            "latest_version": null,
            "path": "/tools/example",
            "python": {
                "path": "/tools/example/bin/python",
                "version": "3.12.14",
                "implementation": "cpython",
                "key": "cpython-3.12.14-linux-x86_64-gnu"
            },
            "commands": [{"name": "example", "path": "/bin/example"}],
            "extras": [],
            "version_specifiers": "",
            "with": []
        }]
    });
    parse_tool_list(&serde_json::to_vec(&report)?)?;

    let mut invalid = report.clone();
    invalid["schema"]["version"] = serde_json::json!(1);
    assert!(parse_tool_list(&serde_json::to_vec(&invalid)?).is_err());

    let mut invalid = report.clone();
    invalid["tools"][0]["latest_version"] = serde_json::json!(1);
    assert!(parse_tool_list(&serde_json::to_vec(&invalid)?).is_err());

    let mut invalid = report.clone();
    invalid["tools"][0]
        .as_object_mut()
        .unwrap()
        .remove("latest_version");
    assert!(parse_tool_list(&serde_json::to_vec(&invalid)?).is_err());

    let mut invalid = report.clone();
    invalid["tools"][0]["python"]["key"] = serde_json::json!(1);
    assert!(parse_tool_list(&serde_json::to_vec(&invalid)?).is_err());

    let mut invalid = report;
    invalid["tools"][0]["commands"][0]
        .as_object_mut()
        .unwrap()
        .remove("path");
    assert!(parse_tool_list(&serde_json::to_vec(&invalid)?).is_err());

    Ok(())
}

#[test]
fn tool_list() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // Install `black`
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stdout -----
    black v24.2.0
    - black
    - blackd
    ");
}

#[test]
fn tool_list_paths() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // Install `black`
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_list().arg("--show-paths"), @"
    exit_code: 0 (success)
    ----- stdout -----
    black v24.2.0 ([TEMP_DIR]/tools/black)
    - black ([TEMP_DIR]/bin/black)
    - blackd ([TEMP_DIR]/bin/blackd)
    ");
}

#[cfg(windows)]
#[test]
fn tool_list_paths_windows() {
    let context = uv_test::test_context!("3.12")
        .clear_filters()
        .with_filtered_windows_temp_dir()
        .with_tool_dirs();

    // Install `black`
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    uv_snapshot!(context.filters_without_standard_filters(), context.tool_list().arg("--show-paths"), @r###"
    exit_code: 0 (success)
    ----- stdout -----
    black v24.2.0 ([TEMP_DIR]\tools\black)
    - black ([TEMP_DIR]\bin\black.exe)
    - blackd ([TEMP_DIR]\bin\blackd.exe)
    "###);
}

#[test]
fn tool_list_empty() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stderr -----
    No tools installed
    ");
}

#[test]
fn tool_list_outdated_empty() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // With no tools installed, `--outdated` should produce the same output as the base case.
    uv_snapshot!(context.filters(), context.tool_list()
    .arg("--outdated"), @"
    exit_code: 0 (success)
    ----- stderr -----
    No tools installed
    ");
}

#[test]
fn tool_list_outdated() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // Install an older version of `black`.
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    // With `--outdated`, the installed (older) version should be listed with the latest version.
    uv_snapshot!(context.filters(), context.tool_list()
    .arg("--outdated"), @"
    exit_code: 0 (success)
    ----- stdout -----
    black v24.2.0 [latest: 24.3.0]
    - black
    - blackd
    ");
}

#[tokio::test]
async fn tool_list_outdated_respects_configured_index() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/simple/black/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{
                "meta": { "api-version": "1.1" },
                "name": "black",
                "files": [{
                    "filename": "black-99.0.0-py3-none-any.whl",
                    "url": "black-99.0.0-py3-none-any.whl",
                    "hashes": {},
                    "upload-time": "2024-03-24T00:00:00Z"
                }]
            }"#,
            "application/vnd.pypi.simple.v1+json",
        ))
        .expect(1)
        .mount(&server)
        .await;

    fs::write(
        context.temp_dir.child("uv.toml"),
        format!(
            "[[index]]\nname = \"ordinary\"\nurl = \"{}/simple\"\ndefault = true\n",
            server.uri()
        ),
    )?;

    uv_snapshot!(context.filters(), context.tool_list()
    .arg("--outdated")
    .arg("--config-file")
    .arg(context.temp_dir.child("uv.toml").as_os_str()), @"
    exit_code: 0 (success)
    ----- stdout -----
    black v24.2.0 [latest: 99.0.0]
    - black
    - blackd
    ");

    Ok(())
}

#[test]
fn tool_list_outdated_respects_exclude_newer() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // Install `black` with a persisted `exclude-newer` cutoff.
    context
        .tool_install()
        .arg("black")
        .arg("--exclude-newer")
        .arg("2024-03-25T00:00:00Z")
        .assert()
        .success();

    // `--outdated` should respect the stored tool settings and avoid flagging upgrades that
    // `uv tool upgrade` would intentionally skip.
    uv_snapshot!(context.filters(), context.tool_list()
    .arg("--outdated"), @"
    exit_code: 0 (success)
    ");
}

#[test]
fn tool_list_outdated_recomputes_relative_exclude_newer() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // Install `black` with a relative `exclude-newer` cutoff that initially resolves to 2024-03-01.
    context
        .tool_install()
        .arg("black")
        .arg("--exclude-newer")
        .arg("3 weeks")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, "2024-03-22T00:00:00Z")
        .assert()
        .success();

    // Recompute the stored span at a later time so `black` is considered outdated.
    uv_snapshot!(context.filters(), context.tool_list()
    .arg("--outdated")
    .env_remove(EnvVars::UV_EXCLUDE_NEWER)
    .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, "2024-04-15T00:00:00Z"), @"
    exit_code: 0 (success)
    ----- stdout -----
    black v24.2.0 [latest: 24.3.0]
    - black
    - blackd
    ");
}

#[test]
fn tool_list_outdated_cli_exclude_newer() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // Install an older version of `black`.
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    // `--exclude-newer` should filter out releases newer than the cutoff when determining the
    // latest available tool version.
    uv_snapshot!(context.filters(), context.tool_list()
    .arg("--outdated")
    .arg("--exclude-newer")
    .arg("2024-03-01T00:00:00Z"), @"
    exit_code: 0 (success)
    ");
}

#[test]
fn tool_list_missing_receipt() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");

    // Install `black`
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    fs_err::remove_file(tool_dir.join("black").join("uv-receipt.toml")).unwrap();

    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Ignoring malformed tool `black` (run `uv tool uninstall black` to remove)
    ");

    tool_list_json_snapshot!(context.filters(), context.tool_list()
    .args(["--output-format", "json", "--preview-features", "json-output"]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "tools": []
    }

    ----- stderr -----
    warning: Ignoring malformed tool `black` (run `uv tool uninstall black` to remove)
    "#);
}

#[test]
fn tool_list_bad_environment() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");

    // Install `black`
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    // Install `ruff`
    context.tool_install().arg("ruff==0.3.4").assert().success();

    let venv_path = uv_test::venv_bin_path(tool_dir.path().join("black"));
    // Remove the python interpreter for black
    fs::remove_dir_all(venv_path.clone())?;

    uv_snapshot!(
        context.filters(),
        context
            .tool_list()

            ,
        @"
    exit_code: 0 (success)
    ----- stdout -----
    ruff v0.3.4
    - ruff

    ----- stderr -----
    warning: Invalid environment at `tools/black`: missing Python executable at `tools/black/[BIN]/[PYTHON]` (run `uv tool install black --reinstall` to reinstall)
    "
    );

    Ok(())
}

#[test]
fn tool_list_deprecated() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");

    // Install `black`
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    // Ensure that we have a modern tool receipt.
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(fs_err::read_to_string(tool_dir.join("black").join("uv-receipt.toml")).unwrap(), @r#"
        [tool]
        requirements = [{ name = "black", specifier = "==24.2.0" }]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]

        [tool.options]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    // Replace with a legacy receipt.
    fs::write(
        tool_dir.join("black").join("uv-receipt.toml"),
        r#"
        [tool]
        requirements = ["black==24.2.0"]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]
        "#,
    )?;

    // Ensure that we can still list the tool.
    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stdout -----
    black v24.2.0
    - black
    - blackd
    ");

    // Replace with an invalid receipt.
    fs::write(
        tool_dir.join("black").join("uv-receipt.toml"),
        r#"
        [tool]
        requirements = ["black<>24.2.0"]
        entrypoints = [
            { name = "black", install-path = "[TEMP_DIR]/bin/black", from = "black" },
            { name = "blackd", install-path = "[TEMP_DIR]/bin/blackd", from = "black" },
        ]
        "#,
    )?;

    // Ensure that listing fails.
    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Ignoring malformed tool `black` (run `uv tool uninstall black` to remove)
    ");

    Ok(())
}

#[test]
fn tool_list_show_version_specifiers() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // Install `black` with a version specifier
    context
        .tool_install()
        .arg("black<24.3.0")
        .assert()
        .success();

    // Install `flask`
    context.tool_install().arg("flask").assert().success();

    uv_snapshot!(context.filters(), context.tool_list().arg("--show-version-specifiers"), @"
    exit_code: 0 (success)
    ----- stdout -----
    black v24.2.0 [required: <24.3.0]
    - black
    - blackd
    flask v3.0.2
    - flask
    ");

    // with paths
    uv_snapshot!(context.filters(), context.tool_list().arg("--show-version-specifiers").arg("--show-paths"), @"
    exit_code: 0 (success)
    ----- stdout -----
    black v24.2.0 [required: <24.3.0] ([TEMP_DIR]/tools/black)
    - black ([TEMP_DIR]/bin/black)
    - blackd ([TEMP_DIR]/bin/blackd)
    flask v3.0.2 ([TEMP_DIR]/tools/flask)
    - flask ([TEMP_DIR]/bin/flask)
    ");
}

#[test]
fn tool_list_show_with() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // Install `black` without additional requirements
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    // Install `flask` with additional requirements
    context
        .tool_install()
        .arg("flask")
        .arg("--with")
        .arg("requests")
        .arg("--with")
        .arg("black==24.2.0")
        .assert()
        .success();

    // Install `ruff` with version specifier and additional requirements
    context
        .tool_install()
        .arg("ruff==0.3.4")
        .arg("--with")
        .arg("requests")
        .assert()
        .success();

    // Test with --show-with
    uv_snapshot!(context.filters(), context.tool_list().arg("--show-with"), @"
    exit_code: 0 (success)
    ----- stdout -----
    black v24.2.0
    - black
    - blackd
    flask v3.0.2 [with: requests, black==24.2.0]
    - flask
    ruff v0.3.4 [with: requests]
    - ruff
    ");

    // Test with both --show-with and --show-paths
    uv_snapshot!(context.filters(), context.tool_list().arg("--show-with").arg("--show-paths"), @"
    exit_code: 0 (success)
    ----- stdout -----
    black v24.2.0 ([TEMP_DIR]/tools/black)
    - black ([TEMP_DIR]/bin/black)
    - blackd ([TEMP_DIR]/bin/blackd)
    flask v3.0.2 [with: requests, black==24.2.0] ([TEMP_DIR]/tools/flask)
    - flask ([TEMP_DIR]/bin/flask)
    ruff v0.3.4 [with: requests] ([TEMP_DIR]/tools/ruff)
    - ruff ([TEMP_DIR]/bin/ruff)
    ");

    // Test with both --show-with and --show-version-specifiers
    uv_snapshot!(context.filters(), context.tool_list().arg("--show-with").arg("--show-version-specifiers"), @"
    exit_code: 0 (success)
    ----- stdout -----
    black v24.2.0 [required: ==24.2.0]
    - black
    - blackd
    flask v3.0.2 [with: requests, black==24.2.0]
    - flask
    ruff v0.3.4 [required: ==0.3.4] [with: requests]
    - ruff
    ");

    // Test with all flags
    uv_snapshot!(context.filters(), context.tool_list()
    .arg("--show-with")
    .arg("--show-version-specifiers")
    .arg("--show-paths"), @"
    exit_code: 0 (success)
    ----- stdout -----
    black v24.2.0 [required: ==24.2.0] ([TEMP_DIR]/tools/black)
    - black ([TEMP_DIR]/bin/black)
    - blackd ([TEMP_DIR]/bin/blackd)
    flask v3.0.2 [with: requests, black==24.2.0] ([TEMP_DIR]/tools/flask)
    - flask ([TEMP_DIR]/bin/flask)
    ruff v0.3.4 [required: ==0.3.4] [with: requests] ([TEMP_DIR]/tools/ruff)
    - ruff ([TEMP_DIR]/bin/ruff)
    ");
}

#[test]
fn tool_list_show_extras() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // Install `black` without extras
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    // Install `flask` with extras and additional requirements
    context
        .tool_install()
        .arg("flask[async,dotenv]")
        .arg("--with")
        .arg("requests")
        .assert()
        .success();

    // Test with --show-extras only
    uv_snapshot!(context.filters(), context.tool_list().arg("--show-extras"), @"
    exit_code: 0 (success)
    ----- stdout -----
    black v24.2.0
    - black
    - blackd
    flask v3.0.2 [extras: async, dotenv]
    - flask
    ");

    // Test with both --show-extras and --show-with
    uv_snapshot!(context.filters(), context.tool_list().arg("--show-extras").arg("--show-with"), @"
    exit_code: 0 (success)
    ----- stdout -----
    black v24.2.0
    - black
    - blackd
    flask v3.0.2 [extras: async, dotenv] [with: requests]
    - flask
    ");

    // Test with --show-extras and --show-paths
    uv_snapshot!(context.filters(), context.tool_list().arg("--show-extras").arg("--show-paths"), @"
    exit_code: 0 (success)
    ----- stdout -----
    black v24.2.0 ([TEMP_DIR]/tools/black)
    - black ([TEMP_DIR]/bin/black)
    - blackd ([TEMP_DIR]/bin/blackd)
    flask v3.0.2 [extras: async, dotenv] ([TEMP_DIR]/tools/flask)
    - flask ([TEMP_DIR]/bin/flask)
    ");

    // Test with --show-extras and --show-version-specifiers
    uv_snapshot!(context.filters(), context.tool_list().arg("--show-extras").arg("--show-version-specifiers"), @"
    exit_code: 0 (success)
    ----- stdout -----
    black v24.2.0 [required: ==24.2.0]
    - black
    - blackd
    flask v3.0.2 [extras: async, dotenv]
    - flask
    ");

    // Test with all flags including --show-extras
    uv_snapshot!(context.filters(), context.tool_list()
    .arg("--show-extras")
    .arg("--show-with")
    .arg("--show-version-specifiers")
    .arg("--show-paths"), @"
    exit_code: 0 (success)
    ----- stdout -----
    black v24.2.0 [required: ==24.2.0] ([TEMP_DIR]/tools/black)
    - black ([TEMP_DIR]/bin/black)
    - blackd ([TEMP_DIR]/bin/blackd)
    flask v3.0.2 [extras: async, dotenv] [with: requests] ([TEMP_DIR]/tools/flask)
    - flask ([TEMP_DIR]/bin/flask)
    ");
}

#[test]
fn tool_list_show_python() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // Install `black` with python 3.12
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    // Test with --show-python
    uv_snapshot!(context.filters(), context.tool_list().arg("--show-python"), @"
    exit_code: 0 (success)
    ----- stdout -----
    black v24.2.0 [CPython 3.12.[X]]
    - black
    - blackd
    ");
}

#[test]
fn tool_list_show_all() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // Install `black` without extras
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    // Install `flask` with extras and additional requirements
    context
        .tool_install()
        .arg("flask[async,dotenv]")
        .arg("--with")
        .arg("requests")
        .assert()
        .success();

    // Test with all flags
    uv_snapshot!(context.filters(), context.tool_list()
    .arg("--show-extras")
    .arg("--show-with")
    .arg("--show-version-specifiers")
    .arg("--show-paths")
    .arg("--show-python"), @"
    exit_code: 0 (success)
    ----- stdout -----
    black v24.2.0 [required: ==24.2.0] [CPython 3.12.[X]] ([TEMP_DIR]/tools/black)
    - black ([TEMP_DIR]/bin/black)
    - blackd ([TEMP_DIR]/bin/blackd)
    flask v3.0.2 [extras: async, dotenv] [with: requests] [CPython 3.12.[X]] ([TEMP_DIR]/tools/flask)
    - flask ([TEMP_DIR]/bin/flask)
    ");
}

#[test]
fn tool_list_empty_json() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    tool_list_json_snapshot!(context.filters(), context.tool_list()
    .arg("--output-format=json"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "tools": []
    }

    ----- stderr -----
    warning: The `--output-format json` option is experimental and the schema may change without warning. Pass `--preview-features json-output` to disable this warning.
    "#);
}

#[test]
fn tool_list_outdated_empty_json() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // With no tools installed, `--outdated` should produce the same output as the base case.
    tool_list_json_snapshot!(context.filters(), context.tool_list()
    .args(["--preview-features", "json-output"])
    .arg("--output-format=json")
    .arg("--outdated"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "tools": []
    }
    "#);
}

#[test]
fn tool_list_initialized_empty_json() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    fs::create_dir_all(context.temp_dir.child("tools"))?;

    tool_list_json_snapshot!(context.filters(), context.tool_list()
    .args(["--output-format", "json", "--preview-features", "json-output"]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "tools": []
    }
    "#);

    Ok(())
}

#[test]
fn tool_list_json_quiet() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_tool_dirs();
    let list = || {
        let mut command = context.tool_list();
        command.args([
            "--offline",
            "--no-python-downloads",
            "--preview-features",
            "json-output",
        ]);
        command
    };
    let check = || -> Result<()> {
        let default = list().args(["--output-format", "json"]).assert().success();
        let quiet = list()
            .args(["--output-format", "json", "-q"])
            .assert()
            .success();
        assert_eq!(default.get_output().stdout, quiet.get_output().stdout);
        assert_eq!(
            parse_tool_list(&quiet.get_output().stdout)?,
            serde_json::json!({"schema": {"version": "preview"}, "tools": []}),
        );
        list()
            .args(["--output-format", "json", "-qq"])
            .assert()
            .success()
            .stdout("");
        list()
            .args(["--output-format", "text", "-q"])
            .assert()
            .success()
            .stdout("");
        Ok(())
    };

    check()?;
    fs::create_dir_all(context.temp_dir.child("tools"))?;
    check()?;
    Ok(())
}

#[test]
fn tool_list_jsonl_empty_modes() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_tool_dirs();
    for initialized in [false, true] {
        if initialized {
            fs::create_dir_all(context.temp_dir.child("tools"))?;
        }
        for arguments in [
            &[][..],
            &["--no-progress"][..],
            &["--quiet"][..],
            &["--outdated"][..],
        ] {
            let output = tool_list_jsonl(&context)
                .arg("--offline")
                .args(arguments)
                .assert()
                .success();
            let (progress, report) = parse_tool_list_jsonl(&output.get_output().stdout)?;
            assert!(progress.is_empty());
            assert_eq!(
                report,
                serde_json::json!({"schema": {"version": "preview"}, "tools": []})
            );
        }
        tool_list_jsonl(&context)
            .args(["--offline", "-qq"])
            .assert()
            .success()
            .stdout("");
    }
    Ok(())
}

#[test]
fn tool_list_jsonl_installed() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    install_local_jsonl_tool(&context);
    let json = context
        .tool_list()
        .args([
            "--offline",
            "--output-format",
            "json",
            "--preview-features",
            "json-output",
        ])
        .assert()
        .success();
    let expected = parse_tool_list(&json.get_output().stdout)?;
    assert_eq!(expected["tools"][0]["name"], "simple-launcher");
    assert_eq!(expected["tools"][0]["version"], "0.1.0");
    assert_eq!(
        expected["tools"][0]["commands"][0]["name"],
        "simple_launcher"
    );

    for arguments in [
        &[][..],
        &["--no-progress"][..],
        &["--quiet"][..],
        &[
            "--show-paths",
            "--show-version-specifiers",
            "--show-with",
            "--show-extras",
            "--show-python",
        ][..],
    ] {
        let output = tool_list_jsonl(&context)
            .arg("--offline")
            .args(arguments)
            .assert()
            .success();
        let (progress, report) = parse_tool_list_jsonl(&output.get_output().stdout)?;
        assert!(progress.is_empty());
        assert_eq!(report, expected);
    }
    tool_list_jsonl(&context)
        .args(["--offline", "-qq"])
        .assert()
        .success()
        .stdout("");
    Ok(())
}

#[tokio::test]
async fn tool_list_jsonl_outdated_progress() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    install_local_jsonl_tool(&context);
    let server = MockServer::start().await;
    let config = context.temp_dir.child("uv.toml");
    fs::write(
        &config,
        format!(
            "[[index]]\nname = \"ordinary\"\nurl = \"{}/simple\"\ndefault = true\n",
            server.uri()
        ),
    )?;
    Mock::given(method("GET"))
        .and(path("/simple/simple-launcher/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{
                "meta": { "api-version": "1.1" },
                "name": "simple-launcher",
                "files": [{
                    "filename": "simple_launcher-0.2.0-py3-none-any.whl",
                    "url": "simple_launcher-0.2.0-py3-none-any.whl",
                    "hashes": {},
                    "upload-time": "2024-03-24T00:00:00Z"
                }]
            }"#,
            "application/vnd.pypi.simple.v1+json",
        ))
        .mount(&server)
        .await;

    let list = || {
        let mut command = context.tool_list();
        command
            .args(["--outdated", "--config-file"])
            .arg(config.as_os_str());
        command
    };
    let json = list()
        .args([
            "--output-format",
            "json",
            "--preview-features",
            "json-output",
        ])
        .assert()
        .success();
    let expected = parse_tool_list(&json.get_output().stdout)?;
    assert_eq!(expected["tools"][0]["latest_version"], "0.2.0");
    let output = list()
        .args(["--output-format", "jsonl", "--preview-features", "jsonl"])
        .assert()
        .success();
    let (progress, report) = parse_tool_list_jsonl(&output.get_output().stdout)?;
    assert_eq!(report, expected);
    assert_eq!(
        Value::Array(progress),
        serde_json::json!([
            {"type":"progress","phase":"latest_version","status":"started","total":1},
            {"type":"progress","phase":"latest_version","status":"updated","name":"simple-launcher","version":"0.2.0","completed":1,"total":1},
            {"type":"progress","phase":"latest_version","status":"completed","completed":1,"total":1}
        ])
    );

    for argument in ["--no-progress", "--quiet"] {
        let output = list()
            .args([
                "--output-format",
                "jsonl",
                "--preview-features",
                "jsonl",
                argument,
            ])
            .assert()
            .success();
        let (progress, report) = parse_tool_list_jsonl(&output.get_output().stdout)?;
        assert!(progress.is_empty());
        assert_eq!(report, expected);
    }

    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/simple/simple-launcher/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"meta":{"api-version":"1.1"},"name":"simple-launcher","files":[]}"#,
            "application/vnd.pypi.simple.v1+json",
        ))
        .mount(&server)
        .await;
    let output = list()
        .args(["--output-format", "jsonl", "--preview-features", "jsonl"])
        .assert()
        .success();
    let (progress, report) = parse_tool_list_jsonl(&output.get_output().stdout)?;
    assert_eq!(report["tools"], serde_json::json!([]));
    assert_eq!(
        Value::Array(progress),
        serde_json::json!([
            {"type":"progress","phase":"latest_version","status":"started","total":1},
            {"type":"progress","phase":"latest_version","status":"updated","completed":1,"total":1},
            {"type":"progress","phase":"latest_version","status":"completed","completed":1,"total":1}
        ])
    );
    Ok(())
}

#[tokio::test]
async fn tool_list_jsonl_index_failure() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    install_local_jsonl_tool(&context);
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/simple/simple-launcher/"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let config = context.temp_dir.child("uv.toml");
    let index = server
        .uri()
        .replace("http://", "http://user:tool-list-jsonl-canary@");
    fs::write(
        &config,
        format!("[[index]]\nurl = \"{index}/simple\"\ndefault = true\n"),
    )?;
    let output = tool_list_jsonl(&context)
        .args(["--outdated", "--config-file"])
        .arg(config.as_os_str())
        .env(EnvVars::UV_HTTP_RETRIES, "0")
        .assert()
        .code(2);
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(!stdout.contains("tool-list-jsonl-canary"));
    let events = stdout
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<serde_json::Result<Vec<_>>>()?;
    assert_eq!(
        Value::Array(events),
        serde_json::json!([
            {"type":"progress","phase":"latest_version","status":"started","total":1}
        ])
    );
    assert!(String::from_utf8_lossy(&output.get_output().stderr).contains("500"));
    Ok(())
}

#[test]
fn tool_list_jsonl_preview_warning() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_tool_dirs();
    let unacknowledged = context
        .tool_list()
        .args(["--offline", "--output-format", "jsonl"])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&unacknowledged.get_output().stderr);
    assert_eq!(
        stderr
            .matches("The JSONL output format is experimental")
            .count(),
        1
    );
    assert!(!stderr.contains("The `--output-format json` option is experimental"));
    let acknowledged = tool_list_jsonl(&context)
        .arg("--offline")
        .assert()
        .success();
    assert_eq!(
        parse_tool_list_jsonl(&unacknowledged.get_output().stdout)?,
        parse_tool_list_jsonl(&acknowledged.get_output().stdout)?
    );
    assert!(
        !String::from_utf8_lossy(&acknowledged.get_output().stderr)
            .contains("The JSONL output format is experimental")
    );
    Ok(())
}

#[test]
fn tool_list_outdated_json() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_keys()
        .with_filtered_python_names()
        .with_filter((r"(/tools/[^/]+)/(?:bin|Scripts)/", "$1/[BIN]/"))
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // Install an older version of `black`.
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    // With `--outdated`, the installed (older) version should be listed with the latest version.
    tool_list_json_snapshot!(context.filters(), context.tool_list()
    .args(["--preview-features", "json-output"])
    .arg("--output-format=json")
    .arg("--outdated"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "tools": [
        {
          "name": "black",
          "version": "24.2.0",
          "latest_version": "24.3.0",
          "path": "[TEMP_DIR]/tools/black",
          "python": {
            "path": "[TEMP_DIR]/tools/black/[BIN]/[PYTHON]",
            "version": "3.12.[X]",
            "implementation": "cpython",
            "key": "cpython-3.12.[X]-[PLATFORM]"
          },
          "commands": [
            {
              "name": "black",
              "path": "[TEMP_DIR]/bin/black"
            },
            {
              "name": "blackd",
              "path": "[TEMP_DIR]/bin/blackd"
            }
          ],
          "extras": [],
          "version_specifiers": "==24.2.0",
          "with": []
        }
      ]
    }
    "#);
}

#[test]
fn tool_list_json() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_keys()
        .with_filtered_python_names()
        .with_filter((r"(/tools/[^/]+)/(?:bin|Scripts)/", "$1/[BIN]/"))
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // Install `flask` with extras and additional requirements.
    context
        .tool_install()
        .arg("flask[async,dotenv]!=42,<69")
        .arg("--with")
        .arg("requests!=69")
        .arg("--with")
        .arg("black")
        .assert()
        .success();

    let report = tool_list_json_snapshot!(context.filters(), context.tool_list()
    .args(["--preview-features", "json-output"])
    .arg("--output-format=json"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "tools": [
        {
          "name": "flask",
          "version": "3.0.2",
          "latest_version": null,
          "path": "[TEMP_DIR]/tools/flask",
          "python": {
            "path": "[TEMP_DIR]/tools/flask/[BIN]/[PYTHON]",
            "version": "3.12.[X]",
            "implementation": "cpython",
            "key": "cpython-3.12.[X]-[PLATFORM]"
          },
          "commands": [
            {
              "name": "flask",
              "path": "[TEMP_DIR]/bin/flask"
            }
          ],
          "extras": [
            "async",
            "dotenv"
          ],
          "version_specifiers": "!=42, <69",
          "with": [
            "requests!=69",
            "black"
          ]
        }
      ]
    }
    "#);

    // The text-display flags do not remove information from the JSON report.
    let all_fields = context
        .tool_list()
        .args([
            "--output-format",
            "json",
            "--preview-features",
            "json-output",
            "--show-paths",
            "--show-version-specifiers",
            "--show-with",
            "--show-extras",
            "--show-python",
        ])
        .assert()
        .success();
    assert_eq!(
        parse_tool_list(&report.stdout)?,
        parse_tool_list(&all_fields.get_output().stdout)?,
    );

    let quiet = context
        .tool_list()
        .args([
            "--output-format",
            "json",
            "--preview-features",
            "json-output",
            "-q",
        ])
        .assert()
        .success();
    assert_eq!(report.stdout, quiet.get_output().stdout);
    context
        .tool_list()
        .args([
            "--output-format",
            "json",
            "--preview-features",
            "json-output",
            "-qq",
        ])
        .assert()
        .success()
        .stdout("");

    Ok(())
}
