use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::PathChild;
use fs_err as fs;
use insta::assert_snapshot;
use uv_static::EnvVars;
use uv_test::uv_snapshot;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[test]
fn tool_list() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-list.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_filtered_exe_suffix()
        .with_filter((r#"index-url = ".*"\n"#, ""))
        .with_tool_dirs(); // Install `list-tool`
    context
        .tool_install()
        .arg("list-tool==1.0.0")
        .assert()
        .success();
    uv_snapshot!(
        context.filters(),
        context.tool_list(),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    list-tool v1.0.0
    - list-tool
    - list-tool-helper
    "
    );
}

#[test]
fn tool_list_paths() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-list.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_filtered_exe_suffix()
        .with_filter((r#"index-url = ".*"\n"#, ""))
        .with_tool_dirs(); // Install `list-tool`
    context
        .tool_install()
        .arg("list-tool==1.0.0")
        .assert()
        .success();
    uv_snapshot!(
        context.filters(),
        context.tool_list().arg("--show-paths"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    list-tool v1.0.0 ([TEMP_DIR]/tools/list-tool)
    - list-tool ([TEMP_DIR]/bin/list-tool)
    - list-tool-helper ([TEMP_DIR]/bin/list-tool-helper)
    "
    );
}

#[cfg(windows)]
#[test]
fn tool_list_paths_windows() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-list.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_filtered_exe_suffix()
        .with_filter((r#"index-url = ".*"\n"#, ""));
    let context = context
        .clear_filters()
        .with_filtered_windows_temp_dir()
        .with_tool_dirs();

    // Install `list-tool`
    context
        .tool_install()
        .arg("list-tool==1.0.0")
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
    let _server = uv_test::packse::PackseServer::new("packages/tool-list.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_filtered_exe_suffix()
        .with_filter((r#"index-url = ".*"\n"#, ""))
        .with_tool_dirs();
    uv_snapshot!(
        context.filters(),
        context.tool_list(),
        @"
    exit_code: 0 (success)
    ----- stderr -----
    No tools installed
    "
    );
}

#[test]
fn tool_list_outdated_empty() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-list.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_filtered_exe_suffix()
        .with_filter((r#"index-url = ".*"\n"#, ""))
        .with_tool_dirs(); // With no tools installed, `--outdated` should produce the same output as the base case.
    uv_snapshot!(
        context.filters(),
        context.tool_list().arg("--outdated"),
        @"
    exit_code: 0 (success)
    ----- stderr -----
    No tools installed
    "
    );
}

#[test]
fn tool_list_outdated() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-list.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_filtered_exe_suffix()
        .with_filter((r#"index-url = ".*"\n"#, ""))
        .with_tool_dirs(); // Install an older version of `list-tool`.
    context
        .tool_install()
        .arg("list-tool==1.0.0")
        .assert()
        .success(); // With `--outdated`, the installed (older) version should be listed with the latest version.
    uv_snapshot!(
        context.filters(),
        context.tool_list().arg("--outdated"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    list-tool v1.0.0 [latest: 2.0.0]
    - list-tool
    - list-tool-helper
    "
    );
}

#[tokio::test]
async fn tool_list_outdated_respects_configured_index() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_packse_index("packages/tool-run.toml")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    context
        .tool_install()
        .arg("format-tool==24.2.0")
        .assert()
        .success();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/simple/format-tool/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{
                "meta": { "api-version": "1.1" },
                "name": "format-tool",
                "files": [{
                    "filename": "format-tool-99.0.0-py3-none-any.whl",
                    "url": "format-tool-99.0.0-py3-none-any.whl",
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
    .arg(context.temp_dir.child("uv.toml").as_os_str()), @"exit_code: 0 (success)");

    Ok(())
}

#[test]
fn tool_list_outdated_respects_exclude_newer() {
    let context = uv_test::test_context!("3.12")
        .with_packse_index("packages/tool-run.toml")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // Install `format-tool` with a persisted `exclude-newer` cutoff.
    context
        .tool_install()
        .arg("format-tool")
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
        .with_packse_index("packages/tool-run.toml")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // Install `format-tool` with a relative `exclude-newer` cutoff that initially resolves to 2024-03-01.
    context
        .tool_install()
        .arg("format-tool")
        .arg("--exclude-newer")
        .arg("3 weeks")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, "2024-03-22T00:00:00Z")
        .assert()
        .success();

    // Recompute the stored span at a later time so `format-tool` is considered outdated.
    uv_snapshot!(context.filters(), context.tool_list()
    .arg("--outdated")
    .env_remove(EnvVars::UV_EXCLUDE_NEWER)
    .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, "2024-04-15T00:00:00Z"), @"
    exit_code: 0 (success)
    ----- stdout -----
    format-tool v24.2.0 [latest: 24.3.0]
    - format-tool
    - format-tool-daemon
    ");
}

#[test]
fn tool_list_outdated_cli_exclude_newer() {
    let context = uv_test::test_context!("3.12")
        .with_packse_index("packages/tool-run.toml")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // Install an older version of `format-tool`.
    context
        .tool_install()
        .arg("format-tool==24.2.0")
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
    let _server = uv_test::packse::PackseServer::new("packages/tool-list.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_filtered_exe_suffix()
        .with_filter((r#"index-url = ".*"\n"#, ""))
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");

    // Install `list-tool`.
    context
        .tool_install()
        .arg("list-tool==1.0.0")
        .assert()
        .success();
    fs_err::remove_file(tool_dir.join("list-tool").join("uv-receipt.toml")).unwrap();
    uv_snapshot!(
        context.filters(),
        context.tool_list(),
        @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Ignoring malformed tool `list-tool` (run `uv tool uninstall list-tool` to remove)
    "
    );
}

#[test]
fn tool_list_bad_environment() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/tool-list.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_filtered_exe_suffix()
        .with_filter((r#"index-url = ".*"\n"#, ""));
    let context = context
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");

    // Install `list-tool`
    context
        .tool_install()
        .arg("list-tool==1.0.0")
        .assert()
        .success();

    // Install `list-other`
    context
        .tool_install()
        .arg("list-other==0.3.4")
        .assert()
        .success();

    let venv_path = uv_test::venv_bin_path(tool_dir.path().join("list-tool"));
    // Remove the python interpreter for list-tool
    fs::remove_dir_all(venv_path.clone())?;

    uv_snapshot!(
        context.filters(),
        context
            .tool_list()

            ,
        @"
    exit_code: 0 (success)
    ----- stdout -----
    list-other v0.3.4
    - list-other

    ----- stderr -----
    warning: Invalid environment at `tools/list-tool`: missing Python executable at `tools/list-tool/[BIN]/[PYTHON]` (run `uv tool install list-tool --reinstall` to reinstall)
    "
    );

    Ok(())
}

#[test]
fn tool_list_deprecated() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/tool-list.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_filtered_exe_suffix()
        .with_filter((r#"index-url = ".*"\n"#, ""))
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools"); // Install `list-tool`
    context
        .tool_install()
        .arg("list-tool==1.0.0")
        .assert()
        .success();

    // Ensure that we have a modern tool receipt.
    let receipt = fs_err::read_to_string(tool_dir.join("list-tool").join("uv-receipt.toml"))?;
    insta::with_settings!({filters => context.filters()}, {
        assert_snapshot!(
            receipt,
            @r#"
    [tool]
    requirements = [{ name = "list-tool", specifier = "==1.0.0" }]
    entrypoints = [
        { name = "list-tool", install-path = "[TEMP_DIR]/bin/list-tool", from = "list-tool" },
        { name = "list-tool-helper", install-path = "[TEMP_DIR]/bin/list-tool-helper", from = "list-tool" },
    ]

    [tool.options]
    exclude-newer = "2024-03-25T00:00:00Z"
    "#);
    });

    // Replace with a legacy receipt.
    fs::write(
        tool_dir.join("list-tool").join("uv-receipt.toml"),
        r#"
        [tool]
        requirements = ["list-tool==1.0.0"]
        entrypoints = [
            { name = "list-tool", install-path = "[TEMP_DIR]/bin/list-tool", from = "list-tool" },
            { name = "list-tool-helper", install-path = "[TEMP_DIR]/bin/list-tool-helper", from = "list-tool" },
        ]
        "#,
    )?; // Ensure that we can still list the tool.
    uv_snapshot!(
        context.filters(),
        context.tool_list(),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    list-tool v1.0.0
    - list-tool
    - list-tool-helper
    "
    ); // Replace with an invalid receipt.
    fs::write(
        tool_dir.join("list-tool").join("uv-receipt.toml"),
        r#"
        [tool]
        requirements = ["list-tool<>1.0.0"]
        entrypoints = [
            { name = "list-tool", install-path = "[TEMP_DIR]/bin/list-tool", from = "list-tool" },
            { name = "list-tool-helper", install-path = "[TEMP_DIR]/bin/list-tool-helper", from = "list-tool" },
        ]
        "#,
    )?; // Ensure that listing fails.
    uv_snapshot!(
        context.filters(),
        context.tool_list(),
        @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Ignoring malformed tool `list-tool` (run `uv tool uninstall list-tool` to remove)
    "
    );
    Ok(())
}

#[test]
fn tool_list_show_version_specifiers() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-list.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_filtered_exe_suffix()
        .with_filter((r#"index-url = ".*"\n"#, ""))
        .with_tool_dirs(); // Install `list-tool` with a version specifier
    context
        .tool_install()
        .arg("list-tool<2.0.0")
        .assert()
        .success(); // Install `list-flags`
    context.tool_install().arg("list-flags").assert().success();
    uv_snapshot!(
        context.filters(),
        context.tool_list().arg("--show-version-specifiers"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    list-flags v3.0.2
    - list-flags
    list-tool v1.0.0 [required: <2.0.0]
    - list-tool
    - list-tool-helper
    "
    ); // with paths
    uv_snapshot!(
        context.filters(),
        context
            .tool_list()
            .arg("--show-version-specifiers")
            .arg("--show-paths"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    list-flags v3.0.2 ([TEMP_DIR]/tools/list-flags)
    - list-flags ([TEMP_DIR]/bin/list-flags)
    list-tool v1.0.0 [required: <2.0.0] ([TEMP_DIR]/tools/list-tool)
    - list-tool ([TEMP_DIR]/bin/list-tool)
    - list-tool-helper ([TEMP_DIR]/bin/list-tool-helper)
    "
    );
}

#[test]
fn tool_list_show_with() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-list.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_filtered_exe_suffix()
        .with_filter((r#"index-url = ".*"\n"#, ""))
        .with_tool_dirs(); // Install `list-tool` without additional requirements
    context
        .tool_install()
        .arg("list-tool==1.0.0")
        .assert()
        .success(); // Install `list-flags` with additional requirements
    context
        .tool_install()
        .arg("list-flags")
        .arg("--with")
        .arg("list-dependency")
        .arg("--with")
        .arg("list-tool==1.0.0")
        .assert()
        .success(); // Install `list-other` with version specifier and additional requirements
    context
        .tool_install()
        .arg("list-other==0.3.4")
        .arg("--with")
        .arg("list-dependency")
        .assert()
        .success(); // Test with --show-with
    uv_snapshot!(
        context.filters(),
        context.tool_list().arg("--show-with"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    list-flags v3.0.2 [with: list-dependency, list-tool==1.0.0]
    - list-flags
    list-other v0.3.4 [with: list-dependency]
    - list-other
    list-tool v1.0.0
    - list-tool
    - list-tool-helper
    "
    ); // Test with both --show-with and --show-paths
    uv_snapshot!(
        context.filters(),
        context.tool_list().arg("--show-with").arg("--show-paths"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    list-flags v3.0.2 [with: list-dependency, list-tool==1.0.0] ([TEMP_DIR]/tools/list-flags)
    - list-flags ([TEMP_DIR]/bin/list-flags)
    list-other v0.3.4 [with: list-dependency] ([TEMP_DIR]/tools/list-other)
    - list-other ([TEMP_DIR]/bin/list-other)
    list-tool v1.0.0 ([TEMP_DIR]/tools/list-tool)
    - list-tool ([TEMP_DIR]/bin/list-tool)
    - list-tool-helper ([TEMP_DIR]/bin/list-tool-helper)
    "
    ); // Test with both --show-with and --show-version-specifiers
    uv_snapshot!(
        context.filters(),
        context
            .tool_list()
            .arg("--show-with")
            .arg("--show-version-specifiers"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    list-flags v3.0.2 [with: list-dependency, list-tool==1.0.0]
    - list-flags
    list-other v0.3.4 [required: ==0.3.4] [with: list-dependency]
    - list-other
    list-tool v1.0.0 [required: ==1.0.0]
    - list-tool
    - list-tool-helper
    "
    ); // Test with all flags
    uv_snapshot!(
        context.filters(),
        context
            .tool_list()
            .arg("--show-with")
            .arg("--show-version-specifiers")
            .arg("--show-paths"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    list-flags v3.0.2 [with: list-dependency, list-tool==1.0.0] ([TEMP_DIR]/tools/list-flags)
    - list-flags ([TEMP_DIR]/bin/list-flags)
    list-other v0.3.4 [required: ==0.3.4] [with: list-dependency] ([TEMP_DIR]/tools/list-other)
    - list-other ([TEMP_DIR]/bin/list-other)
    list-tool v1.0.0 [required: ==1.0.0] ([TEMP_DIR]/tools/list-tool)
    - list-tool ([TEMP_DIR]/bin/list-tool)
    - list-tool-helper ([TEMP_DIR]/bin/list-tool-helper)
    "
    );
}

#[test]
fn tool_list_show_extras() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-list.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_filtered_exe_suffix()
        .with_filter((r#"index-url = ".*"\n"#, ""))
        .with_tool_dirs(); // Install `list-tool` without extras
    context
        .tool_install()
        .arg("list-tool==1.0.0")
        .assert()
        .success(); // Install `list-flags` with extras and additional requirements
    context
        .tool_install()
        .arg("list-flags[async,dotenv]")
        .arg("--with")
        .arg("list-dependency")
        .assert()
        .success(); // Test with --show-extras only
    uv_snapshot!(
        context.filters(),
        context.tool_list().arg("--show-extras"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    list-flags v3.0.2 [extras: async, dotenv]
    - list-flags
    list-tool v1.0.0
    - list-tool
    - list-tool-helper
    "
    ); // Test with both --show-extras and --show-with
    uv_snapshot!(
        context.filters(),
        context.tool_list().arg("--show-extras").arg("--show-with"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    list-flags v3.0.2 [extras: async, dotenv] [with: list-dependency]
    - list-flags
    list-tool v1.0.0
    - list-tool
    - list-tool-helper
    "
    ); // Test with --show-extras and --show-paths
    uv_snapshot!(
        context.filters(),
        context.tool_list().arg("--show-extras").arg("--show-paths"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    list-flags v3.0.2 [extras: async, dotenv] ([TEMP_DIR]/tools/list-flags)
    - list-flags ([TEMP_DIR]/bin/list-flags)
    list-tool v1.0.0 ([TEMP_DIR]/tools/list-tool)
    - list-tool ([TEMP_DIR]/bin/list-tool)
    - list-tool-helper ([TEMP_DIR]/bin/list-tool-helper)
    "
    ); // Test with --show-extras and --show-version-specifiers
    uv_snapshot!(
        context.filters(),
        context
            .tool_list()
            .arg("--show-extras")
            .arg("--show-version-specifiers"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    list-flags v3.0.2 [extras: async, dotenv]
    - list-flags
    list-tool v1.0.0 [required: ==1.0.0]
    - list-tool
    - list-tool-helper
    "
    ); // Test with all flags including --show-extras
    uv_snapshot!(
        context.filters(),
        context
            .tool_list()
            .arg("--show-extras")
            .arg("--show-with")
            .arg("--show-version-specifiers")
            .arg("--show-paths"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    list-flags v3.0.2 [extras: async, dotenv] [with: list-dependency] ([TEMP_DIR]/tools/list-flags)
    - list-flags ([TEMP_DIR]/bin/list-flags)
    list-tool v1.0.0 [required: ==1.0.0] ([TEMP_DIR]/tools/list-tool)
    - list-tool ([TEMP_DIR]/bin/list-tool)
    - list-tool-helper ([TEMP_DIR]/bin/list-tool-helper)
    "
    );
}

#[test]
fn tool_list_show_python() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-list.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_filtered_exe_suffix()
        .with_filter((r#"index-url = ".*"\n"#, ""))
        .with_tool_dirs(); // Install `list-tool` with python 3.12
    context
        .tool_install()
        .arg("list-tool==1.0.0")
        .assert()
        .success(); // Test with --show-python
    uv_snapshot!(
        context.filters(),
        context.tool_list().arg("--show-python"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    list-tool v1.0.0 [CPython 3.12.[X]]
    - list-tool
    - list-tool-helper
    "
    );
}

#[test]
fn tool_list_show_all() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-list.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_filtered_exe_suffix()
        .with_filter((r#"index-url = ".*"\n"#, ""))
        .with_tool_dirs(); // Install `list-tool` without extras
    context
        .tool_install()
        .arg("list-tool==1.0.0")
        .assert()
        .success(); // Install `list-flags` with extras and additional requirements
    context
        .tool_install()
        .arg("list-flags[async,dotenv]")
        .arg("--with")
        .arg("list-dependency")
        .assert()
        .success(); // Test with all flags
    uv_snapshot!(
        context.filters(),
        context
            .tool_list()
            .arg("--show-extras")
            .arg("--show-with")
            .arg("--show-version-specifiers")
            .arg("--show-paths")
            .arg("--show-python"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    list-flags v3.0.2 [extras: async, dotenv] [with: list-dependency] [CPython 3.12.[X]] ([TEMP_DIR]/tools/list-flags)
    - list-flags ([TEMP_DIR]/bin/list-flags)
    list-tool v1.0.0 [required: ==1.0.0] [CPython 3.12.[X]] ([TEMP_DIR]/tools/list-tool)
    - list-tool ([TEMP_DIR]/bin/list-tool)
    - list-tool-helper ([TEMP_DIR]/bin/list-tool-helper)
    "
    );
}
