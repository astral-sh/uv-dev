//! Provenance and reuse of direct URL installations.

use anyhow::Result;
use assert_fs::prelude::*;
use serde_json::{Value, json};
use url::Url;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{basic_auth, path, query_param},
};

use uv_test::packse::generate_wheel;
use uv_test::uv_snapshot;

/// Send credentials and signed query parameters to the server, while recording a reusable URL
/// without authority credentials in the installed distribution's provenance.
#[tokio::test]
async fn direct_url_json_query() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let (filename, wheel) = generate_wheel(
        &"six".parse()?,
        &"1.17.0".parse()?,
        &[],
        &Default::default(),
        None,
        "py2.py3-none-any",
        &[],
    );
    let server = MockServer::start().await;
    Mock::given(path(format!("/{filename}")))
        .and(basic_auth("username", "password"))
        .and(query_param("st", "2026-09-15T16:34:14Z"))
        .and(query_param("sig", "abc+def="))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(wheel))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(path(format!("/{filename}")))
        .respond_with(ResponseTemplate::new(401).insert_header("WWW-Authenticate", "Basic"))
        .mount(&server)
        .await;

    let mut url = Url::parse(&format!(
        "{}/{filename}?st=2026-09-15T16:34:14Z&sig=abc%2Bdef%3D",
        server.uri()
    ))?;
    let provenance_url = url.to_string();
    url.set_username("username")
        .expect("HTTP URL accepts credentials");
    url.set_password(Some("password"))
        .expect("HTTP URL accepts credentials");
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&format!("six @ {url}"))?;

    uv_snapshot!(context.filters(), context.pip_install()
        .args(["-r", "requirements.txt", "--no-index", "--strict"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + six==1.17.0 (from http://username:****@[LOCALHOST]/six-1.17.0-py2.py3-none-any.whl?st=2026-09-15T16%3A34%3A14Z&sig=****)
    ");
    context.assert_installed("six", "1.17.0");
    let provenance: Value = serde_json::from_reader(fs_err::File::open(
        context
            .site_packages()
            .join("six-1.17.0.dist-info/direct_url.json"),
    )?)?;
    assert_eq!(
        provenance,
        json!({ "url": provenance_url, "archive_info": {} })
    );

    uv_snapshot!(context.pip_install()
        .args(["-r", "requirements.txt", "--no-index", "--dry-run", "--strict"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    Would make no changes
    ");

    uv_snapshot!(context.pip_install()
        .args(["-r", "requirements.txt", "--no-index", "--strict"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");
    Ok(())
}
