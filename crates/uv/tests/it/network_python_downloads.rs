use assert_fs::fixture::{ChildPath, FileWriteStr, PathChild};
use serde_json::json;
use uv_static::EnvVars;
use uv_test::{TestContext, uv_snapshot};

use super::network_fixtures::{
    http_error_server, io_error_server, streaming_server, time_out_response,
};

fn write_python_downloads_json(context: &TestContext, mock_server_uri: &str) -> ChildPath {
    let python_downloads_json = context.temp_dir.child("python_downloads.json");
    let interpreter = json!({
        "cpython-3.10.0-darwin-aarch64-none": {
            "arch": {
                "family": "aarch64",
                "variant": null
            },
            "libc": "none",
            "major": 3,
            "minor": 10,
            "name": "cpython",
            "os": "darwin",
            "patch": 0,
            "prerelease": "",
            "sha256": null,
            "url": format!("{mock_server_uri}/astral-sh/python-build-standalone/releases/download/20211017/cpython-3.10.0-aarch64-apple-darwin-pgo%2Blto-20211017T1616.tar.zst"),
            "variant": null
        }
    });
    python_downloads_json
        .write_str(&serde_json::to_string(&interpreter).unwrap())
        .unwrap();
    python_downloads_json
}

/// Check the Python install error message when the server returns HTTP status 500, a retryable
/// error.
#[tokio::test]
async fn python_install_http_500() {
    let context = uv_test::test_context_with_versions!(&[])
        .without_python_download_cache()
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_managed_python_dirs();

    let (_server_drop_guard, mock_server_uri) = http_error_server().await;

    let python_downloads_json = write_python_downloads_json(&context, &mock_server_uri);

    uv_snapshot!(context.filters(), context
        .python_install()
        .arg("cpython-3.10.0-darwin-aarch64-none")
        .arg("--python-downloads-json-url")
        .arg(python_downloads_json.path())
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to install cpython-3.10.0-[PLATFORM]
      cause: Request failed after 3 retries in [TIME]
      cause: Failed to download `http://[LOCALHOST]/astral-sh/python-build-standalone/releases/download/20211017/cpython-3.10.0-[PLATFORM]-pgo%2Blto-20211017T1616.tar.zst`
      cause: HTTP status server error (500 Internal Server Error) for url (http://[LOCALHOST]/astral-sh/python-build-standalone/releases/download/20211017/cpython-3.10.0-[PLATFORM]-pgo%2Blto-20211017T1616.tar.zst)
    ");
}

/// Check the Python install error message when the server returns a retryable IO error.
#[tokio::test]
async fn python_install_io_error() {
    let context = uv_test::test_context_with_versions!(&[])
        .without_python_download_cache()
        .with_filtered_python_keys()
        .with_filtered_exe_suffix()
        .with_managed_python_dirs();

    let (_server_drop_guard, mock_server_uri) = io_error_server().await;

    let python_downloads_json = write_python_downloads_json(&context, &mock_server_uri);

    uv_snapshot!(context.filters(), context
        .python_install()
        .arg("cpython-3.10.0-darwin-aarch64-none")
        .arg("--python-downloads-json-url")
        .arg(python_downloads_json.path())
        .env(EnvVars::UV_INTERNAL__TEST_NO_HTTP_RETRY_DELAY, "true"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to install cpython-3.10.0-[PLATFORM]
      cause: Request failed after 3 retries in [TIME]
      cause: Failed to download `http://[LOCALHOST]/astral-sh/python-build-standalone/releases/download/20211017/cpython-3.10.0-[PLATFORM]-pgo%2Blto-20211017T1616.tar.zst`
      cause: error sending request for url (http://[LOCALHOST]/astral-sh/python-build-standalone/releases/download/20211017/cpython-3.10.0-[PLATFORM]-pgo%2Blto-20211017T1616.tar.zst)
      cause: client error (SendRequest)
      cause: connection closed before message completed
    ");
}

#[tokio::test]
async fn retry_read_timeout_python_downloads_json() {
    let context = uv_test::test_context_with_versions!(&[]).with_fast_http_retry();

    let (server, _guard) = streaming_server(time_out_response);

    uv_snapshot!(context.filters(), context
        .python_list()
        .env_remove(EnvVars::UV_PYTHON_DOWNLOADS)
        .arg("--python-downloads-json-url")
        .arg(&server), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Error while fetching remote python downloads json from `http://[LOCALHOST]/`
      cause: Request failed after 1 retry in [TIME]
      cause: Failed to download `http://[LOCALHOST]/`
      cause: error decoding response body for url (http://[LOCALHOST]/)
      cause: request or response body error
      cause: operation timed out
    ");
}
