//! Package-index response handling.

use anyhow::Result;
use assert_fs::prelude::*;
use uv_test::packse::PackseServer;
use uv_test::uv_snapshot;
use wiremock::matchers::{basic_auth, method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Cache credentials from `uv.toml` for a username-only command-line index URL.
#[tokio::test]
async fn cache_uv_toml_credentials() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let filename = "ok-1.0.0-py3-none-any.whl";
    Mock::given(method("GET"))
        .and(path("/protected/simple/ok/"))
        .and(basic_auth("public", "heron"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            format!(r#"<a href="/protected/files/{filename}" data-upload-time="2024-01-01T00:00:00Z">{filename}</a>"#),
            "text/html",
        ))
        .with_priority(1)
        .expect(1..)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/protected/files/{filename}")))
        .and(basic_auth("public", "heron"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(fs_err::read(
            context.workspace_root.join("test/links").join(filename),
        )?))
        .with_priority(1)
        .expect(1..)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("^/protected/"))
        .respond_with(
            ResponseTemplate::new(401)
                .insert_header("WWW-Authenticate", r#"Basic realm="authenticated""#),
        )
        .mount(&server)
        .await;

    context.temp_dir.child("uv.toml").write_str(&format!(
        "[pip]\nextra-index-url = [\"http://public:heron@{}/protected/simple/\"]\n",
        server.address()
    ))?;
    uv_snapshot!(context.pip_install()
        .arg("ok")
        .arg("--index-url")
        .arg(format!("{}/empty/simple/", server.uri()))
        .arg("--extra-index-url")
        .arg(format!("http://public@{}/protected/simple/", server.address())), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==1.0.0
    ");
    context.assert_installed("ok", "1.0.0");
    Ok(())
}

/// Reject a redirect to a different package's index page instead of retrying indefinitely.
#[tokio::test]
async fn bogus_redirect() {
    let context = uv_test::test_context!("3.12");
    let index = PackseServer::new("simple/single-package.toml");
    let redirect = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/simple/b/"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", format!("{}a/", index.index_url())),
        )
        .expect(1)
        .mount(&redirect)
        .await;

    uv_snapshot!(context.pip_install()
        .arg("--default-index")
        .arg(format!("{}/simple/", redirect.uri()))
        .arg("b"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: The index returned metadata for the wrong package: expected distribution for b, got distribution for a
    ");
}
