#![cfg(all(
    feature = "native-auth",
    any(target_os = "macos", target_os = "windows")
))]

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

#[tokio::test]
async fn migrated_native_credentials_are_cached_by_realm() -> Result<(), Box<dyn std::error::Error>>
{
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
        .and(path_regex("/private.*"))
        .and(basic_auth("legacy-user", "legacy-password"))
        .respond_with(ResponseTemplate::new(200))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .with_priority(2)
        .mount(&server)
        .await;

    let realm = DisplaySafeUrl::parse(&server.uri())?;
    let private = DisplaySafeUrl::parse(&format!("{}/private", server.uri()))?;
    // An HTTP scheme-qualified host entry can migrate to its exact realm. Exact request-URL
    // entries remain legacy entries because their intended broader scope is unknown.
    let legacy = uv_keyring::Entry::new(&format!("uv:{}", server.uri()), "legacy-user")?;
    let other = MockServer::start().await;
    Mock::given(method("GET"))
        .and(basic_auth("legacy-user", "legacy-password"))
        .respond_with(ResponseTemplate::new(200))
        .with_priority(1)
        .mount(&other)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .with_priority(2)
        .mount(&other)
        .await;
    legacy.set_password("legacy-password").await?;
    let result = async {
        let cache = Arc::new(CredentialsCache::new());
        let first_client = reqwest_middleware::ClientBuilder::new(reqwest::Client::new())
            .with(
                AuthMiddleware::new()
                    .with_cache_arc(Arc::clone(&cache))
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
        let mut request = private.clone();
        request
            .set_username("legacy-user")
            .map_err(|()| std::io::Error::other("invalid username"))?;
        assert_eq!(
            first_client.get(request.as_str()).send().await?.status(),
            200
        );

        // Removing the migrated entry makes any subsequent native lookup fail. Shared clients
        // must retain the migrated service scope in their cached realm snapshot.
        provider.remove(&realm, "legacy-user").await?;
        request.set_path("/private/package");
        assert_eq!(
            second_client.get(request.as_str()).send().await?.status(),
            200
        );
        assert_eq!(second_client.get(other.uri()).send().await?.status(), 401);
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    let _ = provider.remove(&realm, "legacy-user").await;
    let _ = legacy.delete_credential().await;
    result
}
