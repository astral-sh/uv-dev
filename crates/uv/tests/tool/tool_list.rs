use std::collections::BTreeMap;
use std::convert::Infallible;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::PathChild;
use bytes::Bytes;
use fs_err as fs;
use http::header::{CONTENT_TYPE, USER_AGENT};
use http_body_util::Full;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use insta::assert_snapshot;
use serde_json::json;
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_static::EnvVars;
use uv_test::packse::generate_wheel;
use uv_test::uv_snapshot;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

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
fn tool_list_outdated_reuses_connections_per_interpreter() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"])
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let connections = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?;
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server = std::thread::spawn({
        let connections = Arc::clone(&connections);
        let requests = Arc::clone(&requests);
        move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test server runtime");
            runtime.block_on(async move {
                let listener =
                    tokio::net::TcpListener::from_std(listener).expect("test server listener");
                let serve =
                    async {
                        while let Ok((stream, _)) = listener.accept().await {
                            let connection = connections.fetch_add(1, Ordering::SeqCst);
                            let requests = Arc::clone(&requests);
                            tokio::spawn(async move {
                                let _ = hyper_util::server::conn::auto::Builder::new(
                                    TokioExecutor::new(),
                                )
                                .serve_connection(
                                    TokioIo::new(stream),
                                    service_fn(
                                        move |request: hyper::Request<hyper::body::Incoming>| {
                                            let requests = Arc::clone(&requests);
                                            async move {
                                                let path = request.uri().path().to_owned();
                                                let user_agent = request
                                                    .headers()
                                                    .get(USER_AGENT)
                                                    .and_then(|value| value.to_str().ok())
                                                    .unwrap_or_default()
                                                    .to_owned();
                                                requests
                                                    .lock()
                                                    .expect("request record mutex")
                                                    .push((path.clone(), connection, user_agent));
                                                let name = path
                                                    .trim_end_matches('/')
                                                    .rsplit('/')
                                                    .next()
                                                    .unwrap_or_default();
                                                let filename = format!(
                                                    "{}-2.0.0-py3-none-any.whl",
                                                    name.replace('-', "_")
                                                );
                                                let body = json!({
                                                    "meta": { "api-version": "1.1" },
                                                    "name": name,
                                                    "files": [{
                                                        "filename": filename,
                                                        "url": filename,
                                                        "hashes": {},
                                                        "upload-time": "2024-03-24T00:00:00Z"
                                                    }]
                                                });
                                                Ok::<_, Infallible>(
                                                    hyper::Response::builder()
                                                        .header(
                                                            CONTENT_TYPE,
                                                            "application/vnd.pypi.simple.v1+json",
                                                        )
                                                        .body(Full::new(Bytes::from(
                                                            body.to_string(),
                                                        )))
                                                        .expect("valid Simple API response"),
                                                )
                                            }
                                        },
                                    ),
                                )
                                .await;
                            });
                        }
                    };
                tokio::select! {
                    () = serve => {}
                    _ = shutdown_rx => {}
                }
            });
        }
    });

    for (name, python, index) in [
        ("tool-a", "3.12", "first"),
        ("tool-b", "3.12", "second"),
        ("tool-c", "3.13", "third"),
    ] {
        let (filename, wheel) = generate_wheel(
            &PackageName::from_str(name)?,
            &Version::from_str("1.0.0")?,
            &[],
            &BTreeMap::new(),
            None,
            "py3-none-any",
            &[name.to_owned()],
        );
        let path = context.temp_dir.child(filename);
        fs::write(&path, wheel)?;
        context
            .tool_install()
            .arg(path.as_os_str())
            .arg("--python")
            .arg(python)
            .arg("--default-index")
            .arg(format!("http://{address}/{index}"))
            .assert()
            .success();
    }

    uv_snapshot!(context.filters(), context.tool_list()
        .arg("--outdated")
        .env(EnvVars::UV_CONCURRENT_DOWNLOADS, "1"), @"
    exit_code: 0 (success)
    ----- stdout -----
    tool-a v1.0.0 [latest: 2.0.0]
    - tool-a
    tool-b v1.0.0 [latest: 2.0.0]
    - tool-b
    tool-c v1.0.0 [latest: 2.0.0]
    - tool-c
    ");

    drop(shutdown_tx);
    server.join().expect("test server thread");
    let requests = requests.lock().expect("request record mutex");
    assert_eq!(requests.len(), 3);
    assert_eq!(connections.load(Ordering::SeqCst), 2);
    assert_eq!(requests[0].0, "/first/tool-a/");
    assert_eq!(requests[1].0, "/second/tool-b/");
    assert_eq!(requests[2].0, "/third/tool-c/");
    assert_eq!(requests[0].1, requests[1].1);
    assert_ne!(requests[0].1, requests[2].1);
    for (request, version) in requests.iter().zip(["3.12.", "3.12.", "3.13."]) {
        let (_, linehaul) = request.2.split_once(' ').expect("LineHaul user agent");
        let linehaul: serde_json::Value = serde_json::from_str(linehaul)?;
        assert!(
            linehaul["python"]
                .as_str()
                .is_some_and(|value| value.starts_with(version))
        );
    }
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
        .env(
            EnvVars::UV_INTERNAL__TEST_CURRENT_TIMESTAMP,
            "2024-03-22T00:00:00Z",
        )
        .assert()
        .success();

    // Recompute the stored span at a later time so `black` is considered outdated.
    uv_snapshot!(context.filters(), context.tool_list()
    .arg("--outdated")
    .env_remove(EnvVars::UV_EXCLUDE_NEWER)
    .env(EnvVars::UV_INTERNAL__TEST_CURRENT_TIMESTAMP, "2024-04-15T00:00:00Z"), @"
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
        assert_snapshot!(context.read("tools/black/uv-receipt.toml"), @r#"
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
