//! Characterization of archive URL upgrades and reinstalls.

use anyhow::Result;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use indoc::indoc;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use uv_test::{TestContext, uv_snapshot};

const WHEEL_FILENAME: &str = "archive_url-1.0.0-py3-none-any.whl";
const DIST_INFO: &str = "archive_url-1.0.0.dist-info";

fn metadata_contents(summary: &str) -> String {
    format!("Metadata-Version: 2.3\nName: archive-url\nVersion: 1.0.0\nSummary: {summary}\n")
}

/// Generate a wheel containing only distribution metadata, without importable package code.
async fn metadata_wheel(summary: &str) -> Result<Vec<u8>> {
    let metadata = metadata_contents(summary);
    let wheel = indoc! {"
        Wheel-Version: 1.0
        Generator: uv-test
        Root-Is-Purelib: true
        Tag: py3-none-any
    "};
    let record = format!("{DIST_INFO}/METADATA,,\n{DIST_INFO}/WHEEL,,\n{DIST_INFO}/RECORD,,\n");
    let mut writer = ZipFileWriter::new(Vec::new());
    for (filename, contents) in [
        ("METADATA", metadata.as_str()),
        ("WHEEL", wheel),
        ("RECORD", record.as_str()),
    ] {
        let entry = ZipEntryBuilder::new(
            format!("{DIST_INFO}/{filename}").into(),
            Compression::Stored,
        );
        writer.write_entry_whole(entry, contents.as_bytes()).await?;
    }
    Ok(writer.close().await?)
}

fn assert_installed_metadata(context: &TestContext, summary: &str, url: &str) -> Result<()> {
    let dist_info = context.site_packages().join(DIST_INFO);
    assert_eq!(
        fs_err::read_to_string(dist_info.join("METADATA"))?,
        metadata_contents(summary),
    );
    let direct_url: serde_json::Value =
        serde_json::from_str(&fs_err::read_to_string(dist_info.join("direct_url.json"))?)?;
    assert_eq!(direct_url["url"], url);
    Ok(())
}

#[tokio::test]
async fn archive_url_reinstall() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let first_url = format!("{}/first/{WHEEL_FILENAME}", server.uri());
    let second_url = format!("{}/second/{WHEEL_FILENAME}", server.uri());

    for (directory, summary) in [("first", "first archive"), ("second", "second archive")] {
        let response = ResponseTemplate::new(200)
            .insert_header("Cache-Control", "public, max-age=31536000, immutable")
            .set_body_bytes(metadata_wheel(summary).await?);
        Mock::given(method("HEAD"))
            .and(path(format!("/{directory}/{WHEEL_FILENAME}")))
            .respond_with(response.clone())
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/{directory}/{WHEEL_FILENAME}")))
            .respond_with(response)
            .expect(1..)
            .mount(&server)
            .await;
    }

    uv_snapshot!(context.filters(), context.pip_install()
        .args(["--no-index", "--no-python-downloads"])
        .arg(format!("archive-url @ {first_url}")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + archive-url==1.0.0 (from http://[LOCALHOST]/first/archive_url-1.0.0-py3-none-any.whl)
    ");
    assert_installed_metadata(&context, "first archive", &first_url)?;
    let first_request_count = server
        .received_requests()
        .await
        .expect("request recording is enabled")
        .len();

    // An unchanged URL is satisfied by the installed distribution.
    uv_snapshot!(context.filters(), context.pip_install()
        .args(["--no-index", "--no-python-downloads"])
        .arg(format!("archive-url @ {first_url}")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");
    assert_installed_metadata(&context, "first archive", &first_url)?;

    // Upgrading resolves the direct URL again without reinstalling the same distribution.
    uv_snapshot!(context.filters(), context.pip_install()
        .args(["--no-index", "--no-python-downloads", "--upgrade"])
        .arg(format!("archive-url @ {first_url}")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    ");
    assert_installed_metadata(&context, "first archive", &first_url)?;

    // Reinstallation is explicit even when the URL and version are unchanged.
    uv_snapshot!(context.filters(), context.pip_install()
        .args(["--no-index", "--no-python-downloads", "--reinstall"])
        .arg(format!("archive-url @ {first_url}")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ archive-url==1.0.0 (from http://[LOCALHOST]/first/archive_url-1.0.0-py3-none-any.whl)
    ");
    assert_installed_metadata(&context, "first archive", &first_url)?;
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("request recording is enabled")
            .len(),
        first_request_count,
        "The cacheable first archive should be reused",
    );

    // A different URL replaces the installed source even when the version is unchanged.
    uv_snapshot!(context.filters(), context.pip_install()
        .args(["--no-index", "--no-python-downloads"])
        .arg(format!("archive-url @ {second_url}")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - archive-url==1.0.0 (from http://[LOCALHOST]/first/archive_url-1.0.0-py3-none-any.whl)
     + archive-url==1.0.0 (from http://[LOCALHOST]/second/archive_url-1.0.0-py3-none-any.whl)
    ");
    assert_installed_metadata(&context, "second archive", &second_url)?;
    let second_request_count = server
        .received_requests()
        .await
        .expect("request recording is enabled")
        .len();

    uv_snapshot!(context.filters(), context.pip_install()
        .args(["--no-index", "--no-python-downloads", "--upgrade"])
        .arg(format!("archive-url @ {first_url}")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - archive-url==1.0.0 (from http://[LOCALHOST]/second/archive_url-1.0.0-py3-none-any.whl)
     + archive-url==1.0.0 (from http://[LOCALHOST]/first/archive_url-1.0.0-py3-none-any.whl)
    ");
    assert_installed_metadata(&context, "first archive", &first_url)?;

    uv_snapshot!(context.filters(), context.pip_install()
        .args(["--no-index", "--no-python-downloads", "--reinstall"])
        .arg(format!("archive-url @ {second_url}")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - archive-url==1.0.0 (from http://[LOCALHOST]/first/archive_url-1.0.0-py3-none-any.whl)
     + archive-url==1.0.0 (from http://[LOCALHOST]/second/archive_url-1.0.0-py3-none-any.whl)
    ");
    assert_installed_metadata(&context, "second archive", &second_url)?;
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("request recording is enabled")
            .len(),
        second_request_count,
        "Both cacheable archives should be reused",
    );

    Ok(())
}
