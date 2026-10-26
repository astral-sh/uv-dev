//! Provenance and reuse of direct URL installations.

use anyhow::Result;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{basic_auth, path, query_param},
};

use uv_redacted::SafeUrl;
use uv_test::archive::generate_source_archive;
use uv_test::packse::WheelBuilder;
use uv_test::{TestContext, uv_snapshot};

fn assert_archive_provenance(context: &TestContext, dist_info: &str, url: &str) -> Result<()> {
    let provenance: Value = serde_json::from_reader(fs_err::File::open(
        context
            .site_packages()
            .join(dist_info)
            .join("direct_url.json"),
    )?)?;
    assert_eq!(provenance, json!({ "url": url, "archive_info": {} }));
    Ok(())
}

/// A same-version wheel at a new path must replace the installed payload and provenance.
#[test]
fn path_changes_with_same_name() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let one = context.temp_dir.child("one/ok-1.0.0-py3-none-any.whl");
    let two = context.temp_dir.child("two/ok-1.0.0-py3-none-any.whl");
    let name = "ok".parse()?;
    let version = "1.0.0".parse()?;
    for (wheel, payload) in [(&one, "one"), (&two, "two")] {
        let (_, bytes) = WheelBuilder::new(&name, &version)
            .files(&[("ok/origin.txt", payload)])
            .build();
        wheel.write_binary(&bytes)?;
    }
    let installed_payload = ChildPath::new(context.site_packages().join("ok/origin.txt"));

    uv_snapshot!(context.filters(), context.pip_install().arg(one.path()).arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==1.0.0 (from file://[TEMP_DIR]/one/ok-1.0.0-py3-none-any.whl)
    ");
    installed_payload.assert("one");
    assert_archive_provenance(
        &context,
        "ok-1.0.0.dist-info",
        SafeUrl::from_file_path(one.path())
            .expect("absolute wheel path")
            .as_str(),
    )?;

    uv_snapshot!(context.filters(), context.pip_install().arg(two.path()).arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - ok==1.0.0 (from file://[TEMP_DIR]/one/ok-1.0.0-py3-none-any.whl)
     + ok==1.0.0 (from file://[TEMP_DIR]/two/ok-1.0.0-py3-none-any.whl)
    ");
    installed_payload.assert("two");
    assert_archive_provenance(
        &context,
        "ok-1.0.0.dist-info",
        SafeUrl::from_file_path(two.path())
            .expect("absolute wheel path")
            .as_str(),
    )?;
    Ok(())
}

/// A wheel built from a source archive must record the archive URL as its provenance.
#[tokio::test]
async fn direct_url_json_direct_url() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let filename = "source_distribution-0.0.3.tar.gz";
    let archive =
        generate_source_archive(&"source-distribution".parse()?, &"0.0.3".parse()?, "", None)?;
    let server = MockServer::start().await;
    Mock::given(path(format!("/{filename}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(archive))
        .mount(&server)
        .await;
    let url = format!("{}/{filename}", server.uri());
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&format!("source-distribution @ {url}"))?;

    uv_snapshot!(context.filters(), context.pip_install()
        .args(["-r", "requirements.txt", "--no-index", "--strict"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.3 (from http://[LOCALHOST]/source_distribution-0.0.3.tar.gz)
    ");
    context.assert_installed("source_distribution", "0.0.3");
    assert_archive_provenance(&context, "source_distribution-0.0.3.dist-info", &url)?;
    Ok(())
}

/// Send credentials and signed query parameters to the server, while recording a reusable URL
/// without authority credentials in the installed distribution's provenance.
#[tokio::test]
async fn direct_url_json_query() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let (filename, wheel) = WheelBuilder::new(&"six".parse()?, &"1.17.0".parse()?)
        .tag("py2.py3-none-any")
        .build();
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

    let mut url = SafeUrl::parse(&format!(
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
    assert_archive_provenance(&context, "six-1.17.0.dist-info", &provenance_url)?;

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
