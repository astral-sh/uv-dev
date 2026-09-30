use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fmt::Write as _;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use indoc::indoc;
use insta::assert_json_snapshot;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_static::EnvVars;
use uv_test::packse::generate_wheel;
use uv_test::{TestContext, uv_snapshot};

#[derive(Clone, Debug, Eq, PartialEq)]
struct AuditRequest {
    connection: usize,
    path: String,
    packages: Vec<String>,
}

struct AuditServer {
    address: SocketAddr,
    connections: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<AuditRequest>>>,
    registry_peak: Arc<AtomicUsize>,
    shutdown: tokio::sync::oneshot::Sender<()>,
    thread: std::thread::JoinHandle<()>,
}

impl AuditServer {
    fn start(responses: BTreeMap<String, (&'static str, Vec<u8>)>) -> Result<Self> {
        Self::start_with_delay(responses, Duration::ZERO)
    }

    fn start_with_delay(
        responses: BTreeMap<String, (&'static str, Vec<u8>)>,
        registry_delay: Duration,
    ) -> Result<Self> {
        let connections = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let registry_active = Arc::new(AtomicUsize::new(0));
        let registry_peak = Arc::new(AtomicUsize::new(0));
        let responses = Arc::new(responses);
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let thread = std::thread::spawn({
            let connections = Arc::clone(&connections);
            let requests = Arc::clone(&requests);
            let registry_peak = Arc::clone(&registry_peak);
            move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("test server runtime");
                runtime.block_on(async move {
                    let listener =
                        tokio::net::TcpListener::from_std(listener).expect("test server listener");
                    let serve = async {
                        while let Ok((stream, _)) = listener.accept().await {
                            let connection = connections.fetch_add(1, Ordering::SeqCst);
                            let requests = Arc::clone(&requests);
                            let responses = Arc::clone(&responses);
                            let registry_active = Arc::clone(&registry_active);
                            let registry_peak = Arc::clone(&registry_peak);
                            tokio::spawn(async move {
                                let _ = hyper_util::server::conn::auto::Builder::new(
                                    TokioExecutor::new(),
                                )
                                .serve_connection(
                                    TokioIo::new(stream),
                                    service_fn(
                                        move |request: hyper::Request<hyper::body::Incoming>| {
                                            let requests = Arc::clone(&requests);
                                            let responses = Arc::clone(&responses);
                                            let registry_active = Arc::clone(&registry_active);
                                            let registry_peak = Arc::clone(&registry_peak);
                                            async move {
                                                let method = request.method().clone();
                                                let path = request.uri().path().to_owned();
                                                let body = request
                                                    .into_body()
                                                    .collect()
                                                    .await
                                                    .expect("complete request body")
                                                    .to_bytes();
                                                let mut packages = Vec::new();
                                                let (status, content_type, body) = if method
                                                    == hyper::Method::POST
                                                    && path == "/v1/querybatch"
                                                {
                                                    let body: Value = serde_json::from_slice(&body)
                                                        .expect("valid OSV query");
                                                    let queries = body["queries"]
                                                        .as_array()
                                                        .expect("OSV query array");
                                                    packages = queries
                                                        .iter()
                                                        .map(|query| {
                                                            query["package"]["name"]
                                                                .as_str()
                                                                .expect("package name")
                                                                .to_owned()
                                                        })
                                                        .collect();
                                                    let body = json!({
                                                        "results": vec![json!({"vulns": []}); queries.len()]
                                                    });
                                                    (200, "application/json", body.to_string().into_bytes())
                                                } else if method == hyper::Method::GET
                                                    && let Some((content_type, body)) = responses.get(&path)
                                                {
                                                    if *content_type == "application/vnd.pypi.simple.v1+json" {
                                                        let active = registry_active.fetch_add(1, Ordering::SeqCst) + 1;
                                                        registry_peak.fetch_max(active, Ordering::SeqCst);
                                                        tokio::time::sleep(registry_delay).await;
                                                        registry_active.fetch_sub(1, Ordering::SeqCst);
                                                    }
                                                    (200, *content_type, body.clone())
                                                } else {
                                                    (404, "text/plain", Vec::new())
                                                };
                                                requests
                                                    .lock()
                                                    .expect("request record mutex")
                                                    .push(AuditRequest { connection, path, packages });
                                                Ok::<_, Infallible>(
                                                    hyper::Response::builder()
                                                        .status(status)
                                                        .header("content-type", content_type)
                                                        .header("cache-control", "no-store")
                                                        .body(Full::new(Bytes::from(body)))
                                                        .expect("valid audit response"),
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
        Ok(Self {
            address,
            connections,
            requests,
            registry_peak,
            shutdown,
            thread,
        })
    }

    fn stop(self) -> (usize, Vec<AuditRequest>) {
        drop(self.shutdown);
        self.thread.join().expect("test server thread");
        (
            self.connections.load(Ordering::SeqCst),
            self.requests.lock().expect("request record mutex").clone(),
        )
    }
}

fn install_tool(context: &TestContext, name: &str, locked: bool) {
    let links = context.workspace_root.join("test/links");

    let mut command = context.tool_install();
    command
        .arg(name)
        .arg("--no-index")
        .arg("--find-links")
        .arg(links);
    if locked {
        command.env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks");
    }
    command.assert().success();
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
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.filters(), context.tool_audit(), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the following required arguments were not provided:
      <NAME>...

    Usage: uv tool audit --cache-dir [CACHE_DIR] <NAME>...

    For more information, try '--help'.
    ");

    uv_snapshot!(context.filters(), context.tool_audit()
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
    let context = uv_test::test_context!("3.12").with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: `uv tool audit` is experimental and may change without warning. Pass `--preview-features audit,tool-install-locks` to disable this warning.
    No tools installed
    ");

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: `uv tool audit` is experimental and may change without warning. Pass `--preview-features tool-install-locks` to disable this warning.
    No tools installed
    ");

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: `uv tool audit` is experimental and may change without warning. Pass `--preview-features audit` to disable this warning.
    No tools installed
    ");

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    No tools installed
    ");
}

#[test]
fn tool_audit_unknown_tool() {
    let context = uv_test::test_context!("3.12").with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("simple-launcher")
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `simple-launcher` is not installed; run `uv tool install simple-launcher` to install
    ");
}

#[test]
fn tool_audit_missing_lockfile() {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    install_tool(&context, "simple-launcher", false);

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Skipping tool `simple-launcher` because it does not have a lockfile; reinstall it with `--preview-features tool-install-locks` to audit it
    No auditable tools installed
    ");

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("simple-launcher")
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Tool `simple-launcher` does not have a lockfile; reinstall it with `--preview-features tool-install-locks` to audit it
    ");
}

#[test]
fn tool_audit_invalid_receipt() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    install_tool(&context, "simple-launcher", true);
    fs_err::write(
        tool_dir.join("simple-launcher").join("uv-receipt.toml"),
        "not valid toml",
    )?;

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Ignoring malformed tool `simple-launcher` (run `uv tool uninstall simple-launcher` to remove)
    No auditable tools installed
    ");

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("simple-launcher")
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Tool `simple-launcher` has an invalid receipt: Failed to read `uv-receipt.toml` at [TEMP_DIR]/tools/simple-launcher/uv-receipt.toml
    ");

    Ok(())
}

#[test]
fn tool_audit_invalid_lockfile() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    install_tool(&context, "simple-launcher", true);
    fs_err::write(
        tool_dir.join("simple-launcher").join("uv.lock"),
        "not valid toml",
    )?;

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Skipping tool `simple-launcher` because its lockfile at `tools/simple-launcher/uv.lock` is invalid: TOML parse error at line 1, column 5
      |
    1 | not valid toml
      |     ^
    key with no value, expected `=`

    No auditable tools installed
    ");

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("simple-launcher")
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
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
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    install_tool(&context, "simple-launcher", true);

    let lock_path = tool_dir.join("simple-launcher").join("uv.lock");
    let contents = fs_err::read_to_string(&lock_path)?;
    fs_err::write(
        &lock_path,
        contents.replacen("version = 1\n", "version = 2\n", 1),
    )?;

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Skipping tool `simple-launcher` because its lockfile at `tools/simple-launcher/uv.lock` uses an unsupported schema version (v2, but only v1 is supported)
    No auditable tools installed
    ");

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("simple-launcher")
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The lockfile for tool `simple-launcher` at `tools/simple-launcher/uv.lock` uses an unsupported schema version (v2, but only v1 is supported)
    ");

    Ok(())
}

#[test]
fn tool_audit_unparsable_unsupported_lockfile_version() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    install_tool(&context, "simple-launcher", true);

    let lock_path = tool_dir.join("simple-launcher").join("uv.lock");
    let contents = fs_err::read_to_string(&lock_path)?
        .replacen("version = 1\n", "version = 2\n", 1)
        .replacen("version = \"0.1.0\"\n", "version = false\n", 1);
    fs_err::write(&lock_path, contents)?;

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("simple-launcher")
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The lockfile for tool `simple-launcher` at `tools/simple-launcher/uv.lock` uses an unsupported schema version (v2, but only v1 is supported)
    ");

    Ok(())
}

#[tokio::test]
async fn tool_audit_one_tool() {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    install_tool(&context, "simple-launcher", true);

    let server = MockServer::start().await;
    mount_clean_service(&server).await;

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("simple-launcher")
        .arg("--service-url")
        .arg(server.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    Auditing `simple-launcher`
    Found no known vulnerabilities and no adverse project statuses in 1 package
    ");
}

#[tokio::test]
async fn tool_audit_all_tools() {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    install_tool(&context, "simple-launcher", true);
    install_tool(&context, "basic-app", true);

    let server = MockServer::start().await;
    mount_clean_service(&server).await;

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        .arg("--service-url")
        .arg(server.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
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
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    install_tool(&context, "simple-launcher", true);
    install_tool(&context, "basic-app", true);

    let server = MockServer::start().await;
    mount_clean_service(&server).await;

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("simple-launcher")
        .arg("basic-app")
        .arg("--service-url")
        .arg(server.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    Auditing `basic-app`
    Found no known vulnerabilities and no adverse project statuses in 1 package
    Auditing `simple-launcher`
    Found no known vulnerabilities and no adverse project statuses in 1 package
    ");
}

#[test]
fn tool_audit_batches_osv_queries() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    install_tool(&context, "simple-launcher", true);
    install_tool(&context, "basic-app", true);

    let server = AuditServer::start(BTreeMap::new())?;

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        .arg("--service-url")
        .arg(format!("http://{}", server.address))
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    Auditing `basic-app`
    Found no known vulnerabilities and no adverse project statuses in 1 package
    Auditing `simple-launcher`
    Found no known vulnerabilities and no adverse project statuses in 1 package
    ");

    let (connections, requests) = server.stop();
    assert_eq!(connections, 1);
    assert_eq!(
        requests,
        [AuditRequest {
            connection: 0,
            path: "/v1/querybatch".to_owned(),
            packages: vec!["basic-app".to_owned(), "simple-launcher".to_owned()],
        }]
    );
    Ok(())
}

#[test]
fn tool_audit_reuses_registry_connections() -> Result<()> {
    for limit in [1_usize, 2] {
        let context = uv_test::test_context!("3.12").with_tool_dirs();
        let mut responses = BTreeMap::new();
        for (index, name, status) in [
            ("first", "audit-tool-a", "active"),
            ("second", "audit-tool-b", "archived"),
            ("first", "audit-tool-c", "active"),
        ] {
            let name = PackageName::from_str(name)?;
            let version = Version::from_str("1.0")?;
            let (filename, wheel) = generate_wheel(
                &name,
                &version,
                &[],
                &BTreeMap::new(),
                None,
                "py3-none-any",
                &[name.to_string()],
            );
            let metadata =
                format!("Metadata-Version: 2.3\nName: {name}\nVersion: {version}\n").into_bytes();
            let simple_index = json!({
                "meta": { "api-version": "1.1" },
                "name": name,
                "project-status": { "status": status },
                "files": [{
                    "filename": filename,
                    "url": format!("/files/{filename}"),
                    "hashes": { "sha256": hex::encode(Sha256::digest(&wheel)) },
                    "core-metadata": { "sha256": hex::encode(Sha256::digest(&metadata)) },
                    "upload-time": "2024-03-24T00:00:00Z"
                }]
            });
            responses.insert(
                format!("/{index}/{name}/"),
                (
                    "application/vnd.pypi.simple.v1+json",
                    simple_index.to_string().into_bytes(),
                ),
            );
            responses.insert(
                format!("/files/{filename}.metadata"),
                ("text/plain", metadata),
            );
            responses.insert(
                format!("/files/{filename}"),
                ("application/octet-stream", wheel),
            );
        }
        let server = AuditServer::start_with_delay(responses, Duration::from_millis(100))?;
        for (index, name) in [
            ("first", "audit-tool-a"),
            ("second", "audit-tool-b"),
            ("first", "audit-tool-c"),
        ] {
            context
                .tool_install()
                .arg(name)
                .arg("--default-index")
                .arg(format!("http://{}/{index}/", server.address))
                .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
                .assert()
                .success();
        }
        let initial_connections = server.connections.load(Ordering::SeqCst);
        server.registry_peak.store(0, Ordering::SeqCst);
        server
            .requests
            .lock()
            .expect("request record mutex")
            .clear();

        insta::allow_duplicates! {
        uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        .arg("--service-url")
        .arg(format!("http://{}", server.address))
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        .env(EnvVars::UV_CONCURRENT_DOWNLOADS, limit.to_string())
        , @"
    exit_code: 0 (success)
    ----- stdout -----
    Tool `audit-tool-b`:

    Adverse statuses:

    - audit-tool-b is archived

    ----- stderr -----
    Auditing `audit-tool-a`
    Found no known vulnerabilities and no adverse project statuses in 1 package
    Auditing `audit-tool-b`
    Found no known vulnerabilities and 1 adverse project status in 1 package
    Auditing `audit-tool-c`
    Found no known vulnerabilities and no adverse project statuses in 1 package
    ");
        }

        let peak = server.registry_peak.load(Ordering::SeqCst);
        let (connections, requests) = server.stop();
        assert_eq!(peak, limit);
        assert_eq!(connections - initial_connections, limit + 1);
        let registry_requests = requests
            .iter()
            .filter(|request| request.path != "/v1/querybatch")
            .collect::<Vec<_>>();
        assert_eq!(registry_requests.len(), 3);
        let first_connections = registry_requests
            .iter()
            .filter(|request| request.path.starts_with("/first/"))
            .map(|request| request.connection)
            .collect::<Vec<_>>();
        assert_eq!(first_connections.len(), 2);
        assert_eq!(first_connections[0] == first_connections[1], limit == 1);
        assert_eq!(registry_requests[2].path, "/second/audit-tool-b/");
        assert!(first_connections.contains(&registry_requests[2].connection));
        let osv_requests = requests
            .iter()
            .filter(|request| request.path == "/v1/querybatch")
            .collect::<Vec<_>>();
        assert_eq!(osv_requests.len(), 1);
        assert_eq!(
            osv_requests[0].packages,
            ["audit-tool-a", "audit-tool-b", "audit-tool-c"]
        );
        assert_ne!(osv_requests[0].connection, registry_requests[0].connection);
    }
    Ok(())
}

#[test]
fn tool_audit_reuses_project_statuses() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let mut responses = BTreeMap::new();
    let mut files: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for (name, version, shared_version) in [
        ("audit-shared", "1.0", None),
        ("audit-shared", "2.0", None),
        ("audit-tool-a", "1.0", Some("1.0")),
        ("audit-tool-b", "1.0", Some("2.0")),
        ("audit-tool-c", "1.0", Some("1.0")),
    ] {
        let requirements = shared_version
            .map(|version| format!("audit-shared=={version}").parse())
            .transpose()?
            .into_iter()
            .collect::<Vec<_>>();
        let (filename, wheel) = generate_wheel(
            &name.parse()?,
            &version.parse()?,
            &requirements,
            &BTreeMap::new(),
            None,
            "py3-none-any",
            &[name.to_owned()],
        );
        let mut metadata = format!("Metadata-Version: 2.3\nName: {name}\nVersion: {version}\n");
        if let Some(version) = shared_version {
            writeln!(metadata, "Requires-Dist: audit-shared=={version}")?;
        }
        let metadata = metadata.into_bytes();
        files.entry(name.to_owned()).or_default().push(json!({
            "filename": filename,
            "url": format!("/files/{filename}"),
            "hashes": { "sha256": hex::encode(Sha256::digest(&wheel)) },
            "core-metadata": { "sha256": hex::encode(Sha256::digest(&metadata)) },
            "upload-time": "2024-03-24T00:00:00Z"
        }));
        responses.insert(
            format!("/files/{filename}.metadata"),
            ("text/plain", metadata),
        );
        responses.insert(
            format!("/files/{filename}"),
            ("application/octet-stream", wheel),
        );
    }
    for index in ["first", "second"] {
        for (name, files) in &files {
            let status = match (index, name.as_str()) {
                ("first", "audit-shared") => "archived",
                ("second", "audit-shared") => "deprecated",
                _ => "active",
            };
            responses.insert(
                format!("/{index}/{name}/"),
                (
                    "application/vnd.pypi.simple.v1+json",
                    json!({
                        "meta": { "api-version": "1.1" }, "name": name,
                        "project-status": { "status": status }, "files": files,
                    })
                    .to_string()
                    .into_bytes(),
                ),
            );
        }
    }
    let server = AuditServer::start(responses)?;
    for (index, name) in [
        ("first", "audit-tool-a"),
        ("first", "audit-tool-b"),
        ("second", "audit-tool-c"),
    ] {
        context
            .tool_install()
            .arg(name)
            .arg("--default-index")
            .arg(format!("http://{}/{index}/", server.address))
            .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
            .assert()
            .success();
    }
    for limit in [1, 2] {
        server
            .requests
            .lock()
            .expect("request record mutex")
            .clear();
        let output = context
            .tool_audit()
            .arg("--all")
            .arg("--output-format")
            .arg("json")
            .arg("--service-url")
            .arg(format!("http://{}", server.address))
            .env(
                EnvVars::UV_PREVIEW_FEATURES,
                "audit,tool-install-locks,json-output",
            )
            .env(EnvVars::UV_CONCURRENT_DOWNLOADS, limit.to_string())
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let report: Value = serde_json::from_slice(&output)?;
        for (number, name, status) in [
            (0, "audit-tool-a", "archived"),
            (1, "audit-tool-b", "archived"),
            (2, "audit-tool-c", "deprecated"),
        ] {
            let tool = &report["tools"][number];
            assert_eq!(tool["name"], name);
            assert_eq!(tool["summary"]["audited_packages"], 2);
            assert_eq!(
                tool["adverse_statuses"],
                json!([{ "name": "audit-shared", "status": status, "reason": null }])
            );
        }
        let requests = server.requests.lock().expect("request record mutex");
        let paths = requests
            .iter()
            .filter(|request| request.path != "/v1/querybatch")
            .map(|request| request.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths.len(), 5);
        assert_eq!(
            paths
                .iter()
                .filter(|path| **path == "/first/audit-shared/")
                .count(),
            1
        );
        assert_eq!(
            paths
                .iter()
                .filter(|path| **path == "/second/audit-shared/")
                .count(),
            1
        );
    }
    server.stop();
    Ok(())
}

#[tokio::test]
async fn tool_audit_mixed_lockfiles() {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    install_tool(&context, "simple-launcher", true);
    install_tool(&context, "basic-app", false);

    let server = MockServer::start().await;
    mount_clean_service(&server).await;

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        .arg("--service-url")
        .arg(server.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Skipping tool `basic-app` because it does not have a lockfile; reinstall it with `--preview-features tool-install-locks` to audit it
    Auditing `simple-launcher`
    Found no known vulnerabilities and no adverse project statuses in 1 package
    ");
}

#[tokio::test]
async fn tool_audit_shared_dependencies() {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    let links = context.workspace_root.join("test/links");

    context
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
    install_tool(&context, "basic-app", true);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/querybatch"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{ "vulns": [] }, { "vulns": [] }]
        })))
        .mount(&server)
        .await;

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        .arg("--service-url")
        .arg(server.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    Auditing `basic-app`
    Found no known vulnerabilities and no adverse project statuses in 1 package
    Auditing `simple-launcher`
    Found no known vulnerabilities and no adverse project statuses in 2 packages
    ");
}

#[tokio::test]
async fn tool_audit_batches_shared_versions_and_pages() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    for version in ["1.0", "2.0"] {
        let (filename, wheel) = generate_wheel(
            &"audit-shared".parse()?,
            &version.parse()?,
            &[],
            &BTreeMap::new(),
            None,
            "py3-none-any",
            &[],
        );
        wheels.child(filename).write_binary(&wheel)?;
    }
    for (name, shared_version) in [
        ("audit-tool-a", "1.0"),
        ("audit-tool-b", "2.0"),
        ("audit-tool-c", "1.0"),
    ] {
        let (filename, wheel) = generate_wheel(
            &name.parse()?,
            &"1.0".parse()?,
            &[format!("audit-shared=={shared_version}").parse()?],
            &BTreeMap::new(),
            None,
            "py3-none-any",
            &[name.to_owned()],
        );
        wheels.child(filename).write_binary(&wheel)?;
        context
            .tool_install()
            .arg(name)
            .arg("--no-index")
            .arg("--find-links")
            .arg(wheels.path())
            .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
            .assert()
            .success();
    }

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/querybatch"))
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).expect("valid OSV query");
            let results = body["queries"]
                .as_array()
                .expect("OSV query array")
                .iter()
                .map(|query| {
                    if query["package"]["name"] != "audit-shared" {
                        json!({"vulns": []})
                    } else if query["version"] == "2.0" {
                        json!({"vulns": [{"id": "VULN-3"}]})
                    } else if query["page_token"] == "next" {
                        json!({"vulns": [{"id": "VULN-2"}]})
                    } else {
                        json!({"vulns": [{"id": "VULN-1"}], "next_page_token": "next"})
                    }
                })
                .collect::<Vec<_>>();
            ResponseTemplate::new(200).set_body_json(json!({"results": results}))
        })
        .mount(&server)
        .await;
    for id in ["VULN-1", "VULN-2", "VULN-3"] {
        Mock::given(method("GET"))
            .and(path(format!("/v1/vulns/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": id,
                "modified": "2026-01-01T00:00:00Z"
            })))
            .mount(&server)
            .await;
    }

    let output = context
        .tool_audit()
        .arg("--all")
        .arg("--output-format")
        .arg("json")
        .arg("--service-url")
        .arg(server.uri())
        .env(
            EnvVars::UV_PREVIEW_FEATURES,
            "audit,tool-install-locks,json-output",
        )
        .output()?;
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout)?;
    let findings = report["tools"].as_array().map(|tools| {
        tools
            .iter()
            .map(|tool| {
                json!([
                    tool["name"],
                    tool["vulnerabilities"].as_array().map(|vulnerabilities| {
                        vulnerabilities
                            .iter()
                            .map(|vulnerability| {
                                json!([
                                    vulnerability["dependency"]["name"],
                                    vulnerability["dependency"]["version"],
                                    vulnerability["id"]
                                ])
                            })
                            .collect::<Vec<_>>()
                    })
                ])
            })
            .collect::<Vec<_>>()
    });
    assert_json_snapshot!(findings, @r#"
    [
      [
        "audit-tool-a",
        [
          [
            "audit-shared",
            "1.0",
            "VULN-1"
          ],
          [
            "audit-shared",
            "1.0",
            "VULN-2"
          ]
        ]
      ],
      [
        "audit-tool-b",
        [
          [
            "audit-shared",
            "2.0",
            "VULN-3"
          ]
        ]
      ],
      [
        "audit-tool-c",
        [
          [
            "audit-shared",
            "1.0",
            "VULN-1"
          ],
          [
            "audit-shared",
            "1.0",
            "VULN-2"
          ]
        ]
      ]
    ]
    "#);

    let requests = server
        .received_requests()
        .await
        .expect("requests should be recorded");
    let mut queries = requests
        .iter()
        .filter(|request| request.method == "POST")
        .flat_map(|request| {
            let body: Value = serde_json::from_slice(&request.body).expect("valid OSV query");
            body["queries"]
                .as_array()
                .expect("OSV query array")
                .iter()
                .map(|query| {
                    (
                        query["package"]["name"]
                            .as_str()
                            .expect("package name")
                            .to_owned(),
                        query["version"]
                            .as_str()
                            .expect("package version")
                            .to_owned(),
                        query["page_token"].as_str().map(str::to_owned),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    queries.sort_unstable();
    assert_json_snapshot!(queries, @r#"
    [
      [
        "audit-shared",
        "1.0",
        null
      ],
      [
        "audit-shared",
        "1.0",
        "next"
      ],
      [
        "audit-shared",
        "2.0",
        null
      ],
      [
        "audit-tool-a",
        "1.0",
        null
      ],
      [
        "audit-tool-b",
        "1.0",
        null
      ],
      [
        "audit-tool-c",
        "1.0",
        null
      ]
    ]
    "#);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        2
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "GET")
            .count(),
        3
    );
    Ok(())
}

#[tokio::test]
async fn tool_audit_vulnerability() {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    install_tool(&context, "simple-launcher", true);

    let server = MockServer::start().await;
    mount_vulnerable_service(&server).await;

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("simple-launcher")
        .arg("--service-url")
        .arg(server.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
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
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    install_tool(&context, "simple-launcher", true);

    let server = MockServer::start().await;
    mount_vulnerable_service(&server).await;

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        .arg("--ignore")
        .arg("PYSEC-2026-0001")
        .arg("--ignore")
        .arg("CVE-DOES-NOT-EXIST")
        .arg("--service-url")
        .arg(server.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Ignored vulnerability `CVE-DOES-NOT-EXIST` does not match any vulnerability in the selected tools
    Auditing `simple-launcher`
    Found no known vulnerabilities and no adverse project statuses in 1 package
    ");
}

#[tokio::test]
async fn tool_audit_configured_ignore() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let config = context.temp_dir.child("uv.toml");
    install_tool(&context, "simple-launcher", true);
    config.write_str(indoc! {r#"
        [audit]
        ignore = ["PYSEC-2026-0001"]
    "#})?;

    let server = MockServer::start().await;
    mount_vulnerable_service(&server).await;

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        .arg("--config-file")
        .arg(config.path())
        .arg("--service-url")
        .arg(server.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    Auditing `simple-launcher`
    Found no known vulnerabilities and no adverse project statuses in 1 package
    ");

    Ok(())
}

#[tokio::test]
async fn tool_audit_json() {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    install_tool(&context, "simple-launcher", true);

    let server = MockServer::start().await;
    mount_clean_service(&server).await;

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        .arg("--output-format")
        .arg("json")
        .arg("--service-url")
        .arg(server.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks,json-output")
        , @r#"
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
}

#[tokio::test]
async fn tool_audit_json_preview_warning() {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    install_tool(&context, "simple-launcher", true);

    let server = MockServer::start().await;
    mount_clean_service(&server).await;

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        .arg("--output-format")
        .arg("json")
        .arg("--service-url")
        .arg(server.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @r#"
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
}

#[tokio::test]
async fn tool_audit_json_all_tools() {
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    install_tool(&context, "simple-launcher", true);
    install_tool(&context, "basic-app", true);

    let server = MockServer::start().await;
    mount_clean_service(&server).await;

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        .arg("--output-format")
        .arg("json")
        .arg("--service-url")
        .arg(server.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks,json-output")
        , @r#"
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
}

#[tokio::test]
async fn tool_audit_sarif() {
    let context = uv_test::test_context!("3.12")
        .with_filter((uv_version::version(), "[VERSION]"))
        .with_tool_dirs();
    install_tool(&context, "simple-launcher", true);

    let server = MockServer::start().await;
    mount_clean_service(&server).await;

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        .arg("--output-format")
        .arg("sarif")
        .arg("--service-url")
        .arg(server.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @r#"
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
    let context = uv_test::test_context!("3.12").with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        .arg("--output-format")
        .arg("sarif")
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "$schema": "https://docs.oasis-open.org/sarif/sarif/v2.1.0/os/schemas/sarif-schema-2.1.0.json",
      "runs": [],
      "version": "2.1.0"
    }
    "#);

    install_tool(&context, "simple-launcher", false);

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("--all")
        .arg("--output-format")
        .arg("sarif")
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @r#"
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
    let context = uv_test::test_context!("3.13").with_tool_dirs();
    install_tool(&context, "simple-launcher", true);
    install_tool(&context, "basic-app", true);

    let server = MockServer::start().await;
    mount_clean_service(&server).await;

    let output = context
        .tool_audit()
        .arg("--all")
        .arg("--output-format")
        .arg("sarif")
        .arg("--service-url")
        .arg(server.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
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
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    install_tool(&context, "simple-launcher", true);

    let server = MockServer::start().await;
    mount_vulnerable_service(&server).await;

    let output = context
        .tool_audit()
        .arg("simple-launcher")
        .arg("--output-format")
        .arg("sarif")
        .arg("--service-url")
        .arg(server.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
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
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let server = MockServer::start().await;
    let wheel_filename = "simple_launcher-0.1.0-py3-none-any.whl";
    let wheel = fs_err::read(
        context
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
        .tool_install()
        .arg("simple-launcher")
        .arg("--index-url")
        .arg(format!("{}/simple", server.uri()))
        .env(EnvVars::UV_PREVIEW_FEATURES, "tool-install-locks")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_audit()
        .arg("simple-launcher")
        .arg("--service-url")
        .arg(server.uri())
        .env(EnvVars::UV_PREVIEW_FEATURES, "audit,tool-install-locks")
        , @"
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
