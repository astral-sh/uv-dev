#![cfg(all(feature = "native-auth", target_os = "windows"))]

use std::time::{SystemTime, UNIX_EPOCH};

use uv_auth::{AuthBackend, Credentials};
use uv_keyring::windows::WinCredential;
use uv_preview::{MaybePreviewFeature, Preview, PreviewFeature};
use uv_redacted::DisplaySafeUrl;

#[tokio::test]
async fn native_store_enumerates_many_credentials_in_one_realm()
-> Result<(), Box<dyn std::error::Error>> {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let preview =
        Preview::from_feature_names([&MaybePreviewFeature::Known(PreviewFeature::NativeAuth)]);
    let provider = match AuthBackend::from_settings(preview).await? {
        AuthBackend::System(provider) => provider,
        AuthBackend::TextStore(..) => {
            return Err(std::io::Error::other("expected native authentication backend").into());
        }
    };
    let entries = (0..16)
        .map(|index| {
            let url = DisplaySafeUrl::parse(&format!(
                "https://native-auth-{unique}.example.invalid/credential-{index}"
            ))?;
            let credentials = Credentials::basic(
                Some(format!("user-{index}")),
                Some(format!("{index:02}{}", "x".repeat(1_000))),
            );
            Ok::<_, uv_redacted::DisplaySafeUrlError>((url, credentials))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let result = async {
        for (url, credentials) in &entries {
            provider.store(url, credentials).await?;
        }
        for (url, credentials) in &entries {
            if provider.fetch(url, credentials.username()).await? != Some(credentials.clone()) {
                return Err(std::io::Error::other("unexpected stored credentials").into());
            }
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    for (url, credentials) in &entries {
        if let Some(username) = credentials.username() {
            let _ = provider.remove(url, username).await;
        }
    }
    result
}

#[tokio::test]
async fn native_store_distinguishes_account_name_collisions()
-> Result<(), Box<dyn std::error::Error>> {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let preview =
        Preview::from_feature_names([&MaybePreviewFeature::Known(PreviewFeature::NativeAuth)]);
    let provider = match AuthBackend::from_settings(preview).await? {
        AuthBackend::System(provider) => provider,
        AuthBackend::TextStore(..) => {
            return Err(std::io::Error::other("expected native authentication backend").into());
        }
    };
    let url = DisplaySafeUrl::parse(&format!(
        "https://native-auth-case-{unique}.example.invalid/credentials"
    ))?;
    let first = Credentials::basic(Some("aa".to_owned()), Some("first".to_owned()));
    let second = Credentials::basic(Some("aG".to_owned()), Some("second".to_owned()));
    let result = async {
        provider.store(&url, &first).await?;
        provider.store(&url, &second).await?;
        if provider.fetch(&url, Some("aa")).await? != Some(first) {
            return Err(std::io::Error::other("unexpected stored credentials").into());
        }
        if provider.fetch(&url, Some("aG")).await? != Some(second) {
            return Err(std::io::Error::other("unexpected stored credentials").into());
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    let _ = provider.remove(&url, "aa").await;
    let _ = provider.remove(&url, "aG").await;
    result
}

#[tokio::test]
async fn native_store_distinguishes_signed_url_identities() -> Result<(), Box<dyn std::error::Error>>
{
    let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let preview =
        Preview::from_feature_names([&MaybePreviewFeature::Known(PreviewFeature::NativeAuth)]);
    let provider = match AuthBackend::from_settings(preview).await? {
        AuthBackend::System(provider) => provider,
        AuthBackend::TextStore(..) => {
            return Err(std::io::Error::other("expected native authentication backend").into());
        }
    };
    let first_url = DisplaySafeUrl::parse(&format!(
        "https://native-auth-signed-{unique}.example.invalid/signed?X-Amz-Signature=one"
    ))?;
    let second_url = DisplaySafeUrl::parse(&format!(
        "https://native-auth-signed-{unique}.example.invalid/signed?X-Amz-Signature=two"
    ))?;
    let first = Credentials::basic(Some("signed".to_owned()), Some("first".to_owned()));
    let second = Credentials::basic(Some("signed".to_owned()), Some("second".to_owned()));
    let result = async {
        provider.store(&first_url, &first).await?;
        provider.store(&second_url, &second).await?;
        if provider.fetch(&first_url, Some("signed")).await? != Some(first) {
            return Err(std::io::Error::other("unexpected stored credentials").into());
        }
        if provider.fetch(&second_url, Some("signed")).await? != Some(second) {
            return Err(std::io::Error::other("unexpected stored credentials").into());
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    let _ = provider.remove(&first_url, "signed").await;
    let _ = provider.remove(&second_url, "signed").await;
    result
}

#[tokio::test]
async fn native_store_ignores_corrupt_realm_entries() -> Result<(), Box<dyn std::error::Error>> {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let preview =
        Preview::from_feature_names([&MaybePreviewFeature::Known(PreviewFeature::NativeAuth)]);
    let provider = match AuthBackend::from_settings(preview).await? {
        AuthBackend::System(provider) => provider,
        AuthBackend::TextStore(..) => {
            return Err(std::io::Error::other("expected native authentication backend").into());
        }
    };
    let valid_url = DisplaySafeUrl::parse(&format!(
        "https://native-auth-corrupt-{unique}.example.invalid/valid"
    ))?;
    let corrupt_url = DisplaySafeUrl::parse(&format!(
        "https://native-auth-corrupt-{unique}.example.invalid/corrupt"
    ))?;
    let valid = Credentials::basic(Some("valid".to_owned()), Some("valid-password".to_owned()));
    let corrupt = Credentials::basic(
        Some("corrupt".to_owned()),
        Some("corrupt-password".to_owned()),
    );
    let result = async {
        provider.store(&valid_url, &valid).await?;
        provider.store(&corrupt_url, &corrupt).await?;
        let target_prefix = format!("uv:https://native-auth-corrupt-{unique}.example.invalid:");
        let credential = WinCredential::enumerate(&target_prefix)
            .await?
            .into_iter()
            .find(|credential| {
                serde_json::from_slice::<serde_json::Value>(credential.secret())
                    .ok()
                    .and_then(|value| {
                        value
                            .get("service")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned)
                    })
                    .as_deref()
                    == Some(corrupt_url.as_str())
            })
            .ok_or_else(|| std::io::Error::other("failed to enumerate test credential"))?;
        let entry =
            uv_keyring::Entry::new_with_credential(Box::new(credential.credential().clone()));
        entry.set_secret(b"not JSON").await?;
        let fetched = provider.fetch(&valid_url, Some("valid")).await;
        let _ = entry.delete_credential().await;
        if fetched? != Some(valid) {
            return Err(std::io::Error::other("corrupt entry hid valid credentials").into());
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    let _ = provider.remove(&valid_url, "valid").await;
    let _ = provider.remove(&corrupt_url, "corrupt").await;
    result
}

#[tokio::test]
async fn native_store_removes_credentials() -> Result<(), Box<dyn std::error::Error>> {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let preview =
        Preview::from_feature_names([&MaybePreviewFeature::Known(PreviewFeature::NativeAuth)]);
    let provider = match AuthBackend::from_settings(preview).await? {
        AuthBackend::System(provider) => provider,
        AuthBackend::TextStore(..) => {
            return Err(std::io::Error::other("expected native authentication backend").into());
        }
    };
    let url = DisplaySafeUrl::parse(&format!(
        "https://native-auth-remove-{unique}.example.invalid/credentials"
    ))?;
    let credentials = Credentials::basic(Some("user".to_owned()), Some("password".to_owned()));
    let result = async {
        provider.store(&url, &credentials).await?;
        if provider.fetch(&url, Some("user")).await? != Some(credentials) {
            return Err(std::io::Error::other("unexpected stored credentials").into());
        }
        provider.remove(&url, "user").await?;
        if provider.fetch(&url, Some("user")).await?.is_some() {
            return Err(std::io::Error::other("removed credential was still returned").into());
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    let _ = provider.remove(&url, "user").await;
    result
}
