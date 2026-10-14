use anyhow::Result;
use assert_fs::prelude::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_test::uv_snapshot;

/// Applying the inactive requirement would force the older available version.
#[test]
fn select_matching_requirement_marker() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let links = context.temp_dir.child("links");
    links.create_dir_all()?;
    for version in ["1.0.0", "2.0.0"] {
        let filename = format!("ok-{version}-py3-none-any.whl");
        fs_err::copy(
            context.workspace_root.join("test/links").join(&filename),
            links.child(filename),
        )?;
    }
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("ok ; python_version >= '3.12'\nok==1.0.0 ; python_version < '3.12'\n")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--no-index", "--find-links"])
        .arg(links.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==2.0.0
    ");

    context.assert_installed("ok", "2.0.0");
    Ok(())
}

/// A space after the semicolon distinguishes a marker from a direct URL suffix.
#[tokio::test]
async fn semicolon_after_direct_url() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filter((r"\^+", "[UNDERLINE]"));
    let server = MockServer::start().await;
    let filename = "ok-1.0.0-py3-none-any.whl";
    Mock::given(method("GET"))
        .and(path(format!("/{filename}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(fs_err::read(
            context.workspace_root.join("test/links").join(filename),
        )?))
        .mount(&server)
        .await;
    let url = format!("{}/{filename}", server.uri());
    let requirements = context.temp_dir.child("requirements.txt");

    requirements.write_str(&format!("ok @ {url};python_version > '3.10'"))?;
    uv_snapshot!(context.filters(), context.pip_sync().arg("requirements.txt"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Couldn't parse requirement in `requirements.txt` at position 0
      cause: Expected direct URL (`http://[LOCALHOST]/ok-1.0.0-py3-none-any.whl;python_version%20%3E%20'3.10'`) to end in a supported file extension: `.whl`, `.tar.gz`, `.zip`, `.tar.bz2`, `.tar.lz`, `.tar.lzma`, `.tar.xz`, `.tar.zst`, `.tar`, `.tbz`, `.tgz`, `.tlz`, or `.txz`
             ok @ http://[LOCALHOST]/ok-1.0.0-py3-none-any.whl;python_version > '3.10'
                  [UNDERLINE]
    ");
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "malformed requirement should be rejected before fetching the wheel"
    );

    requirements.write_str(&format!("ok @ {url}; python_version > '3.10'"))?;
    uv_snapshot!(context.filters(), context.pip_sync().arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==1.0.0 (from http://[LOCALHOST]/ok-1.0.0-py3-none-any.whl)
    ");
    context.assert_installed("ok", "1.0.0");

    Ok(())
}
