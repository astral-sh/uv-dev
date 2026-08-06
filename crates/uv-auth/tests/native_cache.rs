#![cfg(any(target_os = "macos", target_os = "windows"))]

use std::sync::Arc;

use uv_auth::{AuthBackend, AuthMiddleware, Credentials, CredentialsCache};
use uv_preview::{MaybePreviewFeature, Preview, PreviewFeature};
use uv_redacted::DisplaySafeUrl;
use wiremock::matchers::{basic_auth, method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn native_credentials_are_cached_by_service_path() -> Result<(), Box<dyn std::error::Error>> {
    let preview =
        Preview::from_feature_names([&MaybePreviewFeature::Known(PreviewFeature::NativeAuth)]);
    let provider = match AuthBackend::from_settings(preview).await? {
        AuthBackend::System(provider) => provider,
        AuthBackend::TextStore(..) => {
            return Err(std::io::Error::other("expected native authentication backend").into());
        }
    };

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/root"))
        .and(basic_auth("root-user", "root-password"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("/root/private.*"))
        .and(basic_auth("private-user", "private-password"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let root = DisplaySafeUrl::parse(&format!("{}/root", server.uri()))?;
    let private = DisplaySafeUrl::parse(&format!("{}/root/private", server.uri()))?;
    provider
        .store(
            &root,
            &Credentials::basic(
                Some("root-user".to_string()),
                Some("root-password".to_string()),
            ),
        )
        .await?;
    provider
        .store(
            &private,
            &Credentials::basic(
                Some("private-user".to_string()),
                Some("private-password".to_string()),
            ),
        )
        .await?;

    let result = async {
        let cache = Arc::new(CredentialsCache::new());
        let first_client = reqwest_middleware::ClientBuilder::new(reqwest::Client::new())
            .with(
                AuthMiddleware::new()
                    .with_cache_arc(cache.clone())
                    .with_preview(preview),
            )
            .build();
        let second_client = reqwest_middleware::ClientBuilder::new(reqwest::Client::new())
            .with(
                AuthMiddleware::new()
                    .with_cache_arc(cache)
                    .with_preview(preview),
            )
            .build();

        assert_eq!(first_client.get(root.as_str()).send().await?.status(), 200);

        // A second request must use the complete realm snapshot, not reload the keyring or reuse
        // the broader root credential.
        provider.remove(&root, "root-user").await?;
        provider.remove(&private, "private-user").await?;

        assert_eq!(
            second_client
                .get(format!("{private}/package"))
                .send()
                .await?
                .status(),
            200,
            "the cached realm must retain the more-specific credential"
        );

        let requests = server
            .received_requests()
            .await
            .ok_or_else(|| std::io::Error::other("mock server did not record requests"))?;
        assert_eq!(
            requests.len(),
            3,
            "only the first request should require an authentication challenge"
        );

        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;

    let _ = provider.remove(&root, "root-user").await;
    let _ = provider.remove(&private, "private-user").await;

    result
}
