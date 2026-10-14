//! Remote inputs rejected before interpreter discovery in offline mode.

use uv_test::uv_snapshot;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn remote_file() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_string("ok==1.0.0\n"))
        .expect(0)
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn requirements_file_respects_offline_mode() {
    let server = remote_file().await;
    let context =
        uv_test::test_context_with_versions!(&[]).with_filter((server.uri(), "http://[LOCALHOST]"));
    uv_snapshot!(context.filters(), context.pip_install()
        .args(["--offline", "-r"])
        .arg(format!("{}/requirements.txt", server.uri())), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Network connectivity is disabled, but a remote requirements file was requested: http://[LOCALHOST]/requirements.txt
    ");
}

#[tokio::test]
async fn constraints_file_respects_offline_mode() {
    let server = remote_file().await;
    let context =
        uv_test::test_context_with_versions!(&[]).with_filter((server.uri(), "http://[LOCALHOST]"));
    uv_snapshot!(context.filters(), context.pip_install()
        .args(["--offline", "-c"])
        .arg(format!("{}/constraints.txt", server.uri()))
        .arg("ok==1.0.0"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Network connectivity is disabled, but a remote requirements file was requested: http://[LOCALHOST]/constraints.txt
    ");
}
