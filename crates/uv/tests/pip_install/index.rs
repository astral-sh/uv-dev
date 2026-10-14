//! Package-index response handling.

use uv_test::packse::PackseServer;
use uv_test::uv_snapshot;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
