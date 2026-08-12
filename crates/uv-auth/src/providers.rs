use std::borrow::Cow;
use std::error::Error as _;
use std::ffi::OsStr;
use std::future::Future;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use http::header::AUTHORIZATION;
use jiff::Timestamp;
use reqsign::aws::DefaultSigner as AwsDefaultSigner;
use reqsign::azure::DefaultSigner as AzureDefaultSigner;
use reqsign::google::Credential as GoogleCredential;
use reqsign::google::DefaultSigner as GoogleDefaultSigner;
use reqsign::{Context, ProvideCredential};
use serde::Deserialize;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::debug;
use url::{ParseError, Url};

use uv_preview::{Preview, PreviewFeature};
use uv_static::EnvVars;
use uv_warnings::warn_user_once;

use crate::Credentials;
use crate::credentials::Token;
use crate::index::is_path_prefix;
use crate::realm::{Realm, RealmRef};

/// The username expected by Google Artifact Registry when using an `OAuth2` access token.
const GOOGLE_ARTIFACT_REGISTRY_USERNAME: &str = "oauth2accesstoken";

/// The hostname suffix used by Google Artifact Registry's Python package repositories.
const GOOGLE_ARTIFACT_REGISTRY_PYTHON_HOST_SUFFIX: &str = "-python.pkg.dev";

/// The environment variable containing the path to explicit Google Application Default
/// Credentials.
const GOOGLE_APPLICATION_CREDENTIALS: &str = "GOOGLE_APPLICATION_CREDENTIALS";

/// The environment variable containing the path to the Google Cloud SDK configuration directory.
const GOOGLE_CLOUD_SDK_CONFIG: &str = "CLOUDSDK_CONFIG";

/// Refresh managed registry credentials periodically, since access tokens are short-lived.
const REGISTRY_CREDENTIAL_CACHE_DURATION: Duration = Duration::from_mins(1);

/// Refresh active `gcloud` credentials before an in-flight request can outlive its access token.
const GOOGLE_ARTIFACT_REGISTRY_TOKEN_REFRESH_BUFFER: Duration = Duration::from_secs(10);

/// Avoid waiting indefinitely for Application Default Credentials from the metadata server.
const GOOGLE_ARTIFACT_REGISTRY_ADC_TIMEOUT: Duration = Duration::from_secs(10);

/// Avoid waiting indefinitely for credentials from an external command.
const REGISTRY_CREDENTIAL_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

/// The Google Cloud SDK launcher available on the current platform.
const GOOGLE_CLOUD_SDK_EXECUTABLE: &str = if cfg!(windows) {
    "gcloud.cmd"
} else {
    "gcloud"
};

/// The Microsoft Entra application ID for Azure DevOps, including Azure Artifacts.
const AZURE_DEVOPS_RESOURCE: &str = "499b84ac-1321-427f-aa17-267ca6975798";

/// Refresh Azure Artifacts credentials before an in-flight token can expire.
const AZURE_ARTIFACTS_REFRESH_BUFFER: Duration = Duration::from_secs(30);

/// The Azure CLI launcher available on the current platform.
const AZURE_CLI_EXECUTABLE: &str = if cfg!(windows) { "az.cmd" } else { "az" };

/// How widely credentials for a managed registry may be cached.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RegistryCredentialScope {
    /// Credentials can be reused across the registry host.
    Realm,
    /// Credentials must remain scoped to the matching registry URL.
    UrlOnly,
}

/// A built-in provider for credentials used by a Python package registry.
#[derive(Clone, Debug)]
pub enum RegistryAuthProvider {
    /// Google Artifact Registry authentication.
    Google(ArtifactRegistryProvider),
    /// Azure Artifacts authentication.
    Azure(AzureArtifactsProvider),
}

impl RegistryAuthProvider {
    /// Returns the built-in authentication provider for a registry URL, if one is available.
    pub fn for_url(url: &Url) -> Option<Self> {
        RegistryAuthProviders::default().provider_for(url)
    }

    /// Returns whether credentials are available for the registry.
    pub async fn has_credentials_for(&self, url: &Url) -> bool {
        self.credentials_for(url).await.is_some()
    }

    pub(crate) async fn credentials_for(&self, url: &Url) -> Option<Credentials> {
        match self {
            Self::Google(provider) => provider.credentials_for(url).await,
            Self::Azure(provider) => provider.credentials_for(url).await,
        }
    }

    pub(crate) fn supports_username(&self, username: Option<&str>) -> bool {
        match self {
            Self::Google(_) => ArtifactRegistryProvider::supports_username(username),
            Self::Azure(_) => username.is_none(),
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Google(_) => "Google Artifact Registry",
            Self::Azure(_) => "Azure Artifacts",
        }
    }

    pub(crate) fn cache_scope(&self) -> RegistryCredentialScope {
        match self {
            Self::Google(_) => RegistryCredentialScope::Realm,
            Self::Azure(_) => RegistryCredentialScope::Realm,
        }
    }
}

impl From<ArtifactRegistryProvider> for RegistryAuthProvider {
    fn from(provider: ArtifactRegistryProvider) -> Self {
        Self::Google(provider)
    }
}

impl From<AzureArtifactsProvider> for RegistryAuthProvider {
    fn from(provider: AzureArtifactsProvider) -> Self {
        Self::Azure(provider)
    }
}

/// Built-in registry providers shared by an authentication middleware.
#[derive(Clone, Debug, Default)]
pub(crate) struct RegistryAuthProviders {
    artifact_registry: ArtifactRegistryProvider,
    azure_artifacts: AzureArtifactsProvider,
}

impl RegistryAuthProviders {
    pub(crate) fn provider_for(&self, url: &Url) -> Option<RegistryAuthProvider> {
        if ArtifactRegistryProvider::is_artifact_registry(url) {
            Some(RegistryAuthProvider::from(self.artifact_registry.clone()))
        } else if AzureArtifactsProvider::is_azure_artifacts(url) {
            Some(RegistryAuthProvider::from(self.azure_artifacts.clone()))
        } else {
            None
        }
    }

    #[cfg(test)]
    pub(crate) fn set_artifact_registry_provider(&mut self, provider: ArtifactRegistryProvider) {
        self.artifact_registry = provider;
    }

    #[cfg(test)]
    pub(crate) fn set_azure_artifacts_provider(&mut self, provider: AzureArtifactsProvider) {
        self.azure_artifacts = provider;
    }
}

/// A provider for authentication credentials for Google Artifact Registry.
#[derive(Clone, Debug)]
pub struct ArtifactRegistryProvider {
    signer: Option<GoogleDefaultSigner>,
    credentials: Arc<Mutex<Option<CachedRegistryCredentials>>>,
}

#[derive(Clone, Debug)]
struct CachedRegistryCredentials {
    credentials: Option<Credentials>,
    expires_at: Instant,
}

/// Fetch and cache both successful and unsuccessful managed registry credential lookups.
async fn cached_registry_credentials<F>(
    cache: &Mutex<Option<CachedRegistryCredentials>>,
    fetch: F,
) -> Option<Credentials>
where
    F: Future<Output = (Option<Credentials>, Duration)>,
{
    let mut cached_credentials = cache.lock().await;
    if let Some(credentials) = cached_credentials
        .as_ref()
        .filter(|credentials| credentials.expires_at > Instant::now())
    {
        return credentials.credentials.clone();
    }

    let (credentials, cache_duration) = fetch.await;
    *cached_credentials = Some(CachedRegistryCredentials {
        credentials: credentials.clone(),
        expires_at: Instant::now() + cache_duration,
    });
    credentials
}

/// Determine how long a token can be reused before its provider must refresh it.
fn registry_credential_cache_duration(
    expires_at: Timestamp,
    refresh_buffer: Duration,
) -> Option<Duration> {
    let now = Timestamp::now();
    if expires_at <= now {
        return None;
    }

    Some(
        expires_at
            .duration_since(now)
            .unsigned_abs()
            .saturating_sub(refresh_buffer)
            .min(REGISTRY_CREDENTIAL_CACHE_DURATION),
    )
}

/// Run an external registry credential helper without accepting interactive input.
async fn registry_credential_command_output(
    program: &OsStr,
    arguments: &[&str],
    command_name: &str,
) -> Option<Vec<u8>> {
    let mut command = Command::new(program);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(REGISTRY_CREDENTIAL_COMMAND_TIMEOUT, command.output())
        .await
        .inspect_err(|_| {
            debug!("Timed out retrieving credentials from {command_name}");
        })
        .ok()?
        .inspect_err(|err| {
            debug!("Failed to run {command_name}: {err}");
        })
        .ok()?;
    if !output.status.success() {
        debug!("{command_name} exited with status {}", output.status);
        return None;
    }

    Some(output.stdout)
}

#[derive(Debug, Deserialize)]
struct GcloudConfig {
    credential: Option<GcloudCredential>,
}

#[derive(Debug, Deserialize)]
struct GcloudCredential {
    access_token: Option<String>,
    token_expiry: Option<String>,
}

/// The shared Google Artifact Registry provider.
static GOOGLE_ARTIFACT_REGISTRY_PROVIDER: LazyLock<ArtifactRegistryProvider> =
    LazyLock::new(|| ArtifactRegistryProvider {
        signer: None,
        credentials: Arc::new(Mutex::new(None)),
    });

/// The shared Google Artifact Registry signer.
static GOOGLE_ARTIFACT_REGISTRY_SIGNER: LazyLock<GoogleDefaultSigner> = LazyLock::new(|| {
    reqsign::google::default_signer("artifactregistry.googleapis.com")
        .with_credential_provider(ArtifactRegistryCredentialProvider)
});

/// A Google Application Default Credentials provider that preserves the documented lookup order.
///
/// Unlike the default `reqsign` provider, this provider does not fall through to another identity
/// when a configured credentials file exists but cannot be loaded.
#[derive(Clone, Copy, Debug)]
struct ArtifactRegistryCredentialProvider;

impl ProvideCredential for ArtifactRegistryCredentialProvider {
    type Credential = GoogleCredential;

    async fn provide_credential(
        &self,
        context: &Context,
    ) -> reqsign::Result<Option<Self::Credential>> {
        if let Some(path) = context
            .env_var(GOOGLE_APPLICATION_CREDENTIALS)
            .filter(|path| !path.is_empty())
        {
            return reqsign::google::FileCredentialProvider::new(path)
                .provide_credential(context)
                .await;
        }

        if let Some(path) = google_cloud_sdk_adc_path(context) {
            match reqsign::google::FileCredentialProvider::new(path)
                .provide_credential(context)
                .await
            {
                Ok(credentials) => return Ok(credentials),
                Err(err) if error_is_not_found(&err) => {}
                Err(err) => return Err(err),
            }
        }

        reqsign::google::VmMetadataCredentialProvider::new()
            .provide_credential(context)
            .await
    }
}

fn google_cloud_sdk_adc_path(context: &Context) -> Option<String> {
    let config_dir = if let Some(path) = context
        .env_var(GOOGLE_CLOUD_SDK_CONFIG)
        .filter(|path| !path.is_empty())
    {
        PathBuf::from(path)
    } else if let Some(path) = cfg!(windows)
        .then(|| context.env_var(EnvVars::APPDATA))
        .flatten()
        .filter(|path| !path.is_empty())
    {
        PathBuf::from(path).join("gcloud")
    } else if let Some(path) = context
        .env_var(EnvVars::XDG_CONFIG_HOME)
        .filter(|path| !path.is_empty())
    {
        PathBuf::from(path).join("gcloud")
    } else {
        let path = context
            .env_var(EnvVars::HOME)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| context.home_dir())?;
        path.join(".config").join("gcloud")
    };

    Some(
        config_dir
            .join("application_default_credentials.json")
            .to_string_lossy()
            .into_owned(),
    )
}

fn error_is_not_found(err: &reqsign::Error) -> bool {
    let mut source = err.source();
    while let Some(err) = source {
        if err
            .downcast_ref::<std::io::Error>()
            .is_some_and(|err| err.kind() == std::io::ErrorKind::NotFound)
        {
            return true;
        }
        source = err.source();
    }
    false
}

impl Default for ArtifactRegistryProvider {
    fn default() -> Self {
        GOOGLE_ARTIFACT_REGISTRY_PROVIDER.clone()
    }
}

impl ArtifactRegistryProvider {
    /// Returns `true` if the URL is for Google Artifact Registry.
    fn is_artifact_registry(url: &Url) -> bool {
        url.scheme() == "https"
            && url
                .host_str()
                .is_some_and(|host| host.ends_with(GOOGLE_ARTIFACT_REGISTRY_PYTHON_HOST_SUFFIX))
    }

    /// Returns `true` if the username is compatible with Google Artifact Registry credentials.
    fn supports_username(username: Option<&str>) -> bool {
        username.is_none_or(|username| username == GOOGLE_ARTIFACT_REGISTRY_USERNAME)
    }

    /// Returns credentials for Google Artifact Registry, if available.
    ///
    /// This follows the lookup order of Google's `keyrings.google-artifactregistry-auth` package:
    /// Application Default Credentials are preferred, then active `gcloud` credentials.
    async fn credentials_for(&self, url: &Url) -> Option<Credentials> {
        if !Self::is_artifact_registry(url) {
            return None;
        }

        cached_registry_credentials(&self.credentials, async {
            let explicit_adc = std::env::var_os(GOOGLE_APPLICATION_CREDENTIALS)
                .is_some_and(|path| !path.is_empty());
            if let Some(credentials) = self.credentials_from_adc(url).await {
                debug!(
                    "Found Google Artifact Registry credentials from Application Default Credentials"
                );
                (Some(credentials), REGISTRY_CREDENTIAL_CACHE_DURATION)
            } else if explicit_adc {
                debug!(
                    "Skipping Google Artifact Registry credentials from gcloud because explicit Application Default Credentials are configured"
                );
                (None, REGISTRY_CREDENTIAL_CACHE_DURATION)
            } else if let Some((credentials, cache_duration)) = Self::credentials_from_gcloud().await
            {
                debug!("Found Google Artifact Registry credentials from gcloud");
                (Some(credentials), cache_duration)
            } else {
                debug!("No Google Artifact Registry credentials found");
                (None, REGISTRY_CREDENTIAL_CACHE_DURATION)
            }
        })
        .await
    }

    async fn credentials_from_adc(&self, url: &Url) -> Option<Credentials> {
        let request = http::Request::get(url.as_str())
            .body(())
            .inspect_err(|err| {
                debug!("Failed to build Google Artifact Registry credential request: {err}");
            })
            .ok()?;
        let (mut parts, ()) = request.into_parts();
        let Ok(result) = tokio::time::timeout(
            GOOGLE_ARTIFACT_REGISTRY_ADC_TIMEOUT,
            self.signer
                .as_ref()
                .unwrap_or(&GOOGLE_ARTIFACT_REGISTRY_SIGNER)
                .sign(&mut parts, None),
        )
        .await
        else {
            debug!("Timed out retrieving Google Artifact Registry Application Default Credentials");
            return None;
        };
        result
            .inspect_err(|err| {
                debug!(
                    "Failed to retrieve Google Artifact Registry Application Default Credentials: {err}"
                );
            })
            .ok()?;

        let token = parts
            .headers
            .get(AUTHORIZATION)?
            .to_str()
            .ok()?
            .strip_prefix("Bearer ")?;
        Self::credentials_from_token(token.to_string())
    }

    async fn credentials_from_gcloud() -> Option<(Credentials, Duration)> {
        Self::credentials_from_gcloud_command(OsStr::new(GOOGLE_CLOUD_SDK_EXECUTABLE)).await
    }

    async fn credentials_from_gcloud_command(program: &OsStr) -> Option<(Credentials, Duration)> {
        let output = registry_credential_command_output(
            program,
            &["config", "config-helper", "--format=json(credential)"],
            "gcloud config config-helper",
        )
        .await?;
        Self::credentials_from_gcloud_output(&output)
    }

    fn credentials_from_gcloud_output(output: &[u8]) -> Option<(Credentials, Duration)> {
        let config = serde_json::from_slice::<GcloudConfig>(output)
            .inspect_err(|err| {
                debug!("Failed to parse credentials from `gcloud config config-helper`: {err}");
            })
            .ok()?;
        let credential = config.credential?;
        let token_expiry = credential
            .token_expiry?
            .parse::<Timestamp>()
            .inspect_err(|err| {
                debug!("Failed to parse credentials from `gcloud config config-helper`: {err}");
            })
            .ok()?;
        let Some(cache_duration) = registry_credential_cache_duration(
            token_expiry,
            GOOGLE_ARTIFACT_REGISTRY_TOKEN_REFRESH_BUFFER,
        ) else {
            debug!("Ignoring expired Google Artifact Registry credentials");
            return None;
        };
        Some((
            Self::credentials_from_token(credential.access_token?)?,
            cache_duration,
        ))
    }

    fn credentials_from_token(token: String) -> Option<Credentials> {
        if token.trim().is_empty() {
            return None;
        }

        Some(Credentials::basic(
            Some(GOOGLE_ARTIFACT_REGISTRY_USERNAME.to_string()),
            Some(token),
        ))
    }

    #[cfg(test)]
    pub(crate) fn with_signer(signer: GoogleDefaultSigner) -> Self {
        Self {
            signer: Some(signer),
            credentials: Arc::new(Mutex::new(None)),
        }
    }

    #[cfg(test)]
    pub(crate) async fn cache_missing_credentials(&self) {
        *self.credentials.lock().await = Some(CachedRegistryCredentials {
            credentials: None,
            expires_at: Instant::now() + REGISTRY_CREDENTIAL_CACHE_DURATION,
        });
    }

    #[cfg(test)]
    pub(crate) async fn clear_cached_credentials(&self) {
        *self.credentials.lock().await = None;
    }
}

/// A provider for authentication credentials for Azure Artifacts.
#[derive(Clone, Debug)]
pub struct AzureArtifactsProvider {
    credentials: Arc<Mutex<Option<CachedRegistryCredentials>>>,
}

#[derive(Deserialize)]
struct AzureCliToken {
    #[serde(rename = "accessToken")]
    access_token: String,
    expires_on: i64,
    #[serde(rename = "tokenType")]
    token_type: Option<String>,
}

/// The shared Azure Artifacts provider.
static AZURE_ARTIFACTS_PROVIDER: LazyLock<AzureArtifactsProvider> =
    LazyLock::new(|| AzureArtifactsProvider {
        credentials: Arc::new(Mutex::new(None)),
    });

impl Default for AzureArtifactsProvider {
    fn default() -> Self {
        AZURE_ARTIFACTS_PROVIDER.clone()
    }
}

impl AzureArtifactsProvider {
    /// Returns whether the URL identifies an Azure Artifacts Python package feed.
    pub fn is_azure_artifacts(url: &Url) -> bool {
        url.scheme() == "https"
            && url.host_str().is_some_and(|host| {
                host == "pkgs.dev.azure.com" || host.ends_with(".pkgs.visualstudio.com")
            })
            && url.path_segments().is_some_and(|mut segments| {
                segments.any(|segment| segment == "_packaging")
                    && segments.any(|segment| segment == "pypi")
            })
    }

    /// Returns credentials for Azure Artifacts, if available from the Azure CLI.
    pub(crate) async fn credentials_for(&self, url: &Url) -> Option<Credentials> {
        if !Self::is_azure_artifacts(url) {
            return None;
        }

        cached_registry_credentials(&self.credentials, async {
            if let Some((credentials, cache_duration)) = Self::credentials_from_cli().await {
                debug!("Found Azure Artifacts credentials from the Azure CLI");
                (Some(credentials), cache_duration)
            } else {
                debug!("No Azure Artifacts credentials found");
                (None, REGISTRY_CREDENTIAL_CACHE_DURATION)
            }
        })
        .await
    }

    /// Returns whether credentials are available for Azure Artifacts.
    pub async fn has_credentials_for(&self, url: &Url) -> bool {
        self.credentials_for(url).await.is_some()
    }

    async fn credentials_from_cli() -> Option<(Credentials, Duration)> {
        Self::credentials_from_cli_command(OsStr::new(AZURE_CLI_EXECUTABLE)).await
    }

    async fn credentials_from_cli_command(program: &OsStr) -> Option<(Credentials, Duration)> {
        let output = registry_credential_command_output(
            program,
            &[
                "account",
                "get-access-token",
                "--resource",
                AZURE_DEVOPS_RESOURCE,
                "--output",
                "json",
            ],
            "az account get-access-token",
        )
        .await?;
        Self::credentials_from_cli_output(&output)
    }

    fn credentials_from_cli_output(output: &[u8]) -> Option<(Credentials, Duration)> {
        let token = serde_json::from_slice::<AzureCliToken>(output)
            .inspect_err(|err| {
                debug!("Failed to parse credentials from the Azure CLI: {err}");
            })
            .ok()?;
        if token.access_token.trim().is_empty()
            || token
                .token_type
                .as_deref()
                .is_some_and(|token_type| !token_type.eq_ignore_ascii_case("bearer"))
        {
            debug!("Ignoring invalid Azure CLI access token");
            return None;
        }

        let expires_at = Timestamp::from_second(token.expires_on)
            .inspect_err(|err| {
                debug!("Failed to parse Azure CLI access token expiration: {err}");
            })
            .ok()?;
        let Some(cache_duration) =
            registry_credential_cache_duration(expires_at, AZURE_ARTIFACTS_REFRESH_BUFFER)
        else {
            debug!("Ignoring expired Azure CLI access token");
            return None;
        };
        if cache_duration.is_zero() {
            debug!("Ignoring Azure CLI access token that is about to expire");
            return None;
        }

        Some((
            Credentials::bearer(token.access_token.into_bytes()),
            cache_duration,
        ))
    }

    #[cfg(test)]
    pub(crate) fn with_cached_credentials(credentials: Option<Credentials>) -> Self {
        Self {
            credentials: Arc::new(Mutex::new(Some(CachedRegistryCredentials {
                credentials,
                expires_at: Instant::now() + REGISTRY_CREDENTIAL_CACHE_DURATION,
            }))),
        }
    }

    #[cfg(test)]
    pub(crate) async fn cache_credentials(&self, credentials: Option<Credentials>) {
        *self.credentials.lock().await = Some(CachedRegistryCredentials {
            credentials,
            expires_at: Instant::now() + REGISTRY_CREDENTIAL_CACHE_DURATION,
        });
    }
}

/// The [`Realm`] for the Hugging Face platform.
static HUGGING_FACE_REALM: LazyLock<Realm> = LazyLock::new(|| {
    let url = Url::parse("https://huggingface.co").expect("Failed to parse Hugging Face URL");
    Realm::from(&url)
});

/// The authentication token for the Hugging Face platform, if set.
static HUGGING_FACE_TOKEN: LazyLock<Option<Vec<u8>>> = LazyLock::new(|| {
    // Extract the Hugging Face token from the environment variable, if it exists.
    let hf_token = std::env::var(EnvVars::HF_TOKEN)
        .ok()
        .map(String::into_bytes)
        .filter(|token| !token.is_empty())?;

    if std::env::var_os(EnvVars::UV_NO_HF_TOKEN).is_some() {
        debug!("Ignoring Hugging Face token from environment due to `UV_NO_HF_TOKEN`");
        return None;
    }

    debug!("Found Hugging Face token in environment");
    Some(hf_token)
});

/// A provider for authentication credentials for the Hugging Face platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HuggingFaceProvider;

impl HuggingFaceProvider {
    /// Returns the credentials for the Hugging Face platform, if available.
    pub(crate) fn credentials_for(url: &Url) -> Option<Credentials> {
        if RealmRef::from(url) == *HUGGING_FACE_REALM {
            if let Some(token) = HUGGING_FACE_TOKEN.as_ref() {
                return Some(Credentials::Bearer {
                    token: Token::new(token.clone()),
                });
            }
        }
        None
    }
}

/// The [`Url`] for the S3 endpoint, if set.
static S3_ENDPOINT_URL: LazyLock<Result<Option<Url>, ParseError>> =
    LazyLock::new(|| endpoint_url(EnvVars::UV_S3_ENDPOINT_URL));

/// A provider for authentication credentials for S3 endpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct S3EndpointProvider;

impl S3EndpointProvider {
    /// Returns `true` if the URL matches the configured S3 endpoint.
    pub(crate) fn is_s3_endpoint(url: &Url, preview: Preview) -> Result<bool> {
        if let Some(s3_endpoint_url) = S3_ENDPOINT_URL
            .as_ref()
            .map_err(|error| *error)
            .with_context(|| format!("Invalid `{}`", EnvVars::UV_S3_ENDPOINT_URL))?
        {
            if !preview.is_enabled(PreviewFeature::S3Endpoint) {
                warn_user_once!(
                    "The `s3-endpoint` option is experimental and may change without warning. Pass `--preview-features {}` to disable this warning.",
                    PreviewFeature::S3Endpoint
                );
            }

            // Treat any URL under the endpoint path on the same domain or subdomain as available
            // for S3 signing.
            if is_endpoint_url(url, s3_endpoint_url) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Creates a new S3 signer with the configured region.
    ///
    /// This is potentially expensive as it may invoke credential helpers, so the result
    /// should be cached.
    pub(crate) fn create_signer() -> AwsDefaultSigner {
        // TODO(charlie): Can `reqsign` infer the region for us? Profiles, for example,
        // often have a region set already.
        let region = std::env::var(EnvVars::AWS_REGION)
            .map(Cow::Owned)
            .unwrap_or_else(|_| {
                std::env::var(EnvVars::AWS_DEFAULT_REGION)
                    .map(Cow::Owned)
                    .unwrap_or_else(|_| Cow::Borrowed("us-east-1"))
            });
        reqsign::aws::default_signer("s3", &region)
    }
}

/// The [`Url`] for the GCS endpoint, if set.
static GCS_ENDPOINT_URL: LazyLock<Result<Option<Url>, ParseError>> =
    LazyLock::new(|| endpoint_url(EnvVars::UV_GCS_ENDPOINT_URL));

/// A provider for authentication credentials for GCS endpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GcsEndpointProvider;

impl GcsEndpointProvider {
    /// Returns `true` if the URL matches the configured GCS endpoint.
    pub(crate) fn is_gcs_endpoint(url: &Url, preview: Preview) -> Result<bool> {
        if let Some(gcs_endpoint_url) = GCS_ENDPOINT_URL
            .as_ref()
            .map_err(|error| *error)
            .with_context(|| format!("Invalid `{}`", EnvVars::UV_GCS_ENDPOINT_URL))?
        {
            if !preview.is_enabled(PreviewFeature::GcsEndpoint) {
                warn_user_once!(
                    "The `gcs-endpoint` option is experimental and may change without warning. Pass `--preview-features {}` to disable this warning.",
                    PreviewFeature::GcsEndpoint
                );
            }

            // Treat any URL under the endpoint path on the same domain or subdomain as available
            // for GCS signing.
            if is_endpoint_url(url, gcs_endpoint_url) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Creates a new GCS signer.
    ///
    /// This is potentially expensive as it may invoke credential helpers, so the result
    /// should be cached.
    pub(crate) fn create_signer() -> GoogleDefaultSigner {
        reqsign::google::default_signer("storage.googleapis.com")
    }
}

/// The [`Url`] for the Azure endpoint, if set.
static AZURE_ENDPOINT_URL: LazyLock<Result<Option<Url>, ParseError>> =
    LazyLock::new(|| endpoint_url(EnvVars::UV_AZURE_ENDPOINT_URL));

/// A provider for authentication credentials for Azure endpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AzureEndpointProvider;

impl AzureEndpointProvider {
    /// Returns `true` if the URL matches the configured Azure endpoint.
    pub(crate) fn is_azure_endpoint(url: &Url, preview: Preview) -> Result<bool> {
        if let Some(azure_endpoint_url) = AZURE_ENDPOINT_URL
            .as_ref()
            .map_err(|error| *error)
            .with_context(|| format!("Invalid `{}`", EnvVars::UV_AZURE_ENDPOINT_URL))?
        {
            if !preview.is_enabled(PreviewFeature::AzureEndpoint) {
                warn_user_once!(
                    "The `azure-endpoint` option is experimental and may change without warning. Pass `--preview-features {}` to disable this warning.",
                    PreviewFeature::AzureEndpoint
                );
            }

            // Treat any URL under the endpoint path on the same domain or subdomain as available
            // for Azure signing.
            if is_endpoint_url(url, azure_endpoint_url) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Creates a new Azure signer using the default Azure credential chain.
    ///
    /// This is potentially expensive as it may invoke credential helpers, so the result
    /// should be cached.
    pub(crate) fn create_signer() -> AzureDefaultSigner {
        reqsign::azure::default_signer()
    }
}

/// Returns the configured endpoint [`Url`], if set and valid.
fn endpoint_url(env_var: &str) -> Result<Option<Url>, ParseError> {
    let Some(endpoint_url) = std::env::var(env_var).ok() else {
        return Ok(None);
    };
    Url::parse(&endpoint_url).map(Some)
}

/// Returns `true` if `url` is within the configured S3, GCS, or Azure-compatible endpoint URL.
///
/// The URL must be in the same realm, or a subdomain of the endpoint realm, and must be under the
/// endpoint path using complete path-segment prefix matching.
fn is_endpoint_url(url: &Url, endpoint_url: &Url) -> bool {
    let endpoint_realm = RealmRef::from(endpoint_url);
    let realm = RealmRef::from(url);
    if realm != endpoint_realm && !realm.is_subdomain_of(endpoint_realm) {
        return false;
    }

    is_path_prefix(endpoint_url.path(), url.path())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use reqsign::{FileRead, StaticEnv};

    use super::*;

    #[derive(Clone, Debug, Default)]
    struct TestFileRead {
        files: Arc<HashMap<String, Vec<u8>>>,
    }

    impl TestFileRead {
        fn new(files: HashMap<String, Vec<u8>>) -> Self {
            Self {
                files: Arc::new(files),
            }
        }
    }

    impl FileRead for TestFileRead {
        async fn file_read(&self, path: &str) -> reqsign::Result<Vec<u8>> {
            self.files.get(path).cloned().ok_or_else(|| {
                reqsign::Error::unexpected("test credential file not found").with_source(
                    std::io::Error::new(std::io::ErrorKind::NotFound, "test credential file"),
                )
            })
        }
    }

    fn service_account_credentials() -> Vec<u8> {
        br#"{
            "type": "service_account",
            "private_key": "-----BEGIN RSA PRIVATE KEY-----\ntest\n-----END RSA PRIVATE KEY-----",
            "client_email": "test@example.iam.gserviceaccount.com"
        }"#
        .to_vec()
    }

    fn cloud_sdk_credentials_path() -> String {
        PathBuf::from("/cloud-sdk")
            .join("application_default_credentials.json")
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn test_registry_auth_provider() {
        let registry = Url::parse("https://us-central1-python.pkg.dev/project/private/simple/")
            .expect("Google Artifact Registry URL should parse");
        let provider = RegistryAuthProvider::for_url(&registry)
            .expect("Google Artifact Registry should have a built-in provider");

        assert_eq!(provider.name(), "Google Artifact Registry");
        assert_eq!(provider.cache_scope(), RegistryCredentialScope::Realm);
        assert!(provider.supports_username(None));
        assert!(provider.supports_username(Some("oauth2accesstoken")));
        assert!(!provider.supports_username(Some("another-user")));

        let azure_registry = Url::parse(
            "https://pkgs.dev.azure.com/organization/project/_packaging/feed/pypi/simple/",
        )
        .expect("Azure Artifacts URL should parse");
        let provider = RegistryAuthProvider::for_url(&azure_registry)
            .expect("Azure Artifacts should have a built-in provider");

        assert_eq!(provider.name(), "Azure Artifacts");
        assert_eq!(provider.cache_scope(), RegistryCredentialScope::Realm);
        assert!(provider.supports_username(None));
        assert!(!provider.supports_username(Some("another-user")));

        let other_registry =
            Url::parse("https://example.com/simple/").expect("Other registry URL should parse");
        assert!(RegistryAuthProvider::for_url(&other_registry).is_none());
    }

    #[test]
    fn test_registry_credential_cache_duration() {
        let now = Timestamp::now();
        let expired = now
            .checked_sub(Duration::from_secs(1))
            .expect("Expired timestamp should fit");
        let expiring = now
            .checked_add(Duration::from_secs(5))
            .expect("Expiring timestamp should fit");
        let long_lived = now
            .checked_add(Duration::from_hours(1))
            .expect("Long-lived timestamp should fit");

        assert_eq!(
            registry_credential_cache_duration(expired, Duration::from_secs(10)),
            None
        );
        assert_eq!(
            registry_credential_cache_duration(expiring, Duration::from_secs(10)),
            Some(Duration::ZERO)
        );
        assert_eq!(
            registry_credential_cache_duration(long_lived, Duration::from_secs(10)),
            Some(REGISTRY_CREDENTIAL_CACHE_DURATION)
        );
    }

    #[tokio::test]
    async fn test_artifact_registry_credentials_from_adc() {
        let provider = ArtifactRegistryProvider::with_signer(
            reqsign::google::default_signer("artifactregistry.googleapis.com")
                .with_credential_provider(reqsign::google::TokenCredentialProvider::new(
                    "test-token",
                )),
        );

        assert_eq!(
            provider
                .credentials_for(
                    &Url::parse("https://us-central1-python.pkg.dev/project/index/simple").unwrap()
                )
                .await,
            Some(Credentials::basic(
                Some("oauth2accesstoken".to_string()),
                Some("test-token".to_string())
            ))
        );
    }

    #[tokio::test]
    async fn test_artifact_registry_credentials_ignores_other_hosts() {
        let provider = ArtifactRegistryProvider::with_signer(
            reqsign::google::default_signer("artifactregistry.googleapis.com")
                .with_credential_provider(reqsign::google::TokenCredentialProvider::new(
                    "test-token",
                )),
        );

        assert_eq!(
            provider
                .credentials_for(&Url::parse("https://python.pkg.dev.example.com/simple").unwrap())
                .await,
            None
        );
        assert_eq!(
            provider
                .credentials_for(
                    &Url::parse("https://us-central1-docker.pkg.dev/project/image").unwrap()
                )
                .await,
            None
        );
        assert_eq!(
            provider
                .credentials_for(
                    &Url::parse("https://us-central1-python.pkg.dev.evil.example/simple").unwrap()
                )
                .await,
            None
        );
        assert_eq!(
            provider
                .credentials_for(
                    &Url::parse("http://us-central1-python.pkg.dev/project/index/simple").unwrap()
                )
                .await,
            None
        );
    }

    #[tokio::test]
    async fn test_artifact_registry_credentials_caches_missing_credentials() {
        let provider = ArtifactRegistryProvider::with_signer(
            reqsign::google::default_signer("artifactregistry.googleapis.com")
                .with_credential_provider(reqsign::google::TokenCredentialProvider::new(
                    "test-token",
                )),
        );
        provider.cache_missing_credentials().await;

        assert_eq!(
            provider
                .credentials_for(
                    &Url::parse("https://us-central1-python.pkg.dev/project/index/simple").unwrap()
                )
                .await,
            None
        );
    }

    #[tokio::test]
    async fn test_artifact_registry_credentials_fail_closed_for_explicit_adc() {
        let context = Context::new()
            .with_env(StaticEnv {
                envs: HashMap::from([
                    (
                        GOOGLE_APPLICATION_CREDENTIALS.to_string(),
                        "/missing/credentials.json".to_string(),
                    ),
                    (
                        GOOGLE_CLOUD_SDK_CONFIG.to_string(),
                        "/cloud-sdk".to_string(),
                    ),
                ]),
                home_dir: None,
            })
            .with_file_read(TestFileRead::new(HashMap::from([(
                cloud_sdk_credentials_path(),
                service_account_credentials(),
            )])));

        assert!(
            ArtifactRegistryCredentialProvider
                .provide_credential(&context)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_artifact_registry_credentials_respect_cloud_sdk_config() {
        let context = Context::new()
            .with_env(StaticEnv {
                envs: HashMap::from([(
                    GOOGLE_CLOUD_SDK_CONFIG.to_string(),
                    "/cloud-sdk".to_string(),
                )]),
                home_dir: None,
            })
            .with_file_read(TestFileRead::new(HashMap::from([(
                cloud_sdk_credentials_path(),
                service_account_credentials(),
            )])));

        let credentials = ArtifactRegistryCredentialProvider
            .provide_credential(&context)
            .await
            .expect("Credentials should load")
            .expect("Credentials should exist");
        assert_eq!(
            credentials
                .service_account
                .expect("Credentials should contain service account")
                .client_email,
            "test@example.iam.gserviceaccount.com"
        );
    }

    #[test]
    fn test_artifact_registry_cloud_sdk_adc_paths() {
        let application_default_credentials = |config_dir: PathBuf| {
            config_dir
                .join("gcloud")
                .join("application_default_credentials.json")
                .to_string_lossy()
                .into_owned()
        };

        let explicit_cloud_sdk_config = Context::new().with_env(StaticEnv {
            envs: HashMap::from([
                (
                    GOOGLE_CLOUD_SDK_CONFIG.to_string(),
                    "/cloud-sdk".to_string(),
                ),
                (EnvVars::APPDATA.to_string(), "/app-data".to_string()),
                (EnvVars::XDG_CONFIG_HOME.to_string(), "/xdg".to_string()),
                (EnvVars::HOME.to_string(), "/home".to_string()),
            ]),
            home_dir: None,
        });
        assert_eq!(
            google_cloud_sdk_adc_path(&explicit_cloud_sdk_config),
            Some(cloud_sdk_credentials_path())
        );

        let platform_config = Context::new().with_env(StaticEnv {
            envs: HashMap::from([
                (EnvVars::APPDATA.to_string(), "/app-data".to_string()),
                (EnvVars::XDG_CONFIG_HOME.to_string(), "/xdg".to_string()),
                (EnvVars::HOME.to_string(), "/home".to_string()),
            ]),
            home_dir: None,
        });
        assert_eq!(
            google_cloud_sdk_adc_path(&platform_config),
            Some(application_default_credentials(PathBuf::from(
                if cfg!(windows) { "/app-data" } else { "/xdg" }
            )))
        );

        let home_directory = Context::new().with_env(StaticEnv {
            envs: HashMap::new(),
            home_dir: Some(PathBuf::from("/home")),
        });
        assert_eq!(
            google_cloud_sdk_adc_path(&home_directory),
            Some(application_default_credentials(
                PathBuf::from("/home").join(".config")
            ))
        );
    }

    #[tokio::test]
    async fn test_artifact_registry_credentials_fail_closed_for_cloud_sdk_adc() {
        let context = Context::new()
            .with_env(StaticEnv {
                envs: HashMap::from([(
                    GOOGLE_CLOUD_SDK_CONFIG.to_string(),
                    "/cloud-sdk".to_string(),
                )]),
                home_dir: None,
            })
            .with_file_read(TestFileRead::new(HashMap::from([(
                cloud_sdk_credentials_path(),
                br#"{"type":"not_a_google_credential"}"#.to_vec(),
            )])));

        assert!(
            ArtifactRegistryCredentialProvider
                .provide_credential(&context)
                .await
                .is_err(),
            "Invalid Cloud SDK application default credentials must not fall back to another identity"
        );
    }

    #[test]
    fn test_artifact_registry_credentials_supports_username() {
        assert!(ArtifactRegistryProvider::supports_username(None));
        assert!(ArtifactRegistryProvider::supports_username(Some(
            "oauth2accesstoken"
        )));
        assert!(!ArtifactRegistryProvider::supports_username(Some("user")));
    }

    #[test]
    fn test_artifact_registry_credentials_from_gcloud_output() {
        assert_eq!(
            ArtifactRegistryProvider::credentials_from_gcloud_output(
                br#"{"credential":{"access_token":"test-token","token_expiry":"2099-05-29T00:00:00Z"}}"#
            ),
            Some((
                Credentials::basic(
                    Some("oauth2accesstoken".to_string()),
                    Some("test-token".to_string())
                ),
                REGISTRY_CREDENTIAL_CACHE_DURATION
            ))
        );
        assert_eq!(
            ArtifactRegistryProvider::credentials_from_gcloud_output(
                br#"{"credential":{"access_token":"test-token"}}"#
            ),
            None
        );
        assert_eq!(
            ArtifactRegistryProvider::credentials_from_gcloud_output(
                br#"{"credential":{"access_token":"test-token","token_expiry":"2000-05-29T00:00:00Z"}}"#
            ),
            None
        );
        assert_eq!(
            ArtifactRegistryProvider::credentials_from_gcloud_output(
                br#"{"credential":{"access_token":"   ","token_expiry":"2099-05-29T00:00:00Z"}}"#
            ),
            None
        );
    }

    #[test]
    fn test_artifact_registry_credentials_refresh_before_gcloud_token_expiry() {
        let token_expiry = Timestamp::now()
            .checked_add(Duration::from_secs(20))
            .expect("Token expiry should fit in a timestamp");
        let output = serde_json::json!({
            "credential": {
                "access_token": "test-token",
                "token_expiry": token_expiry.to_string(),
            },
        });

        let (_, cache_duration) =
            ArtifactRegistryProvider::credentials_from_gcloud_output(output.to_string().as_bytes())
                .expect("Google Cloud SDK credentials should load");

        assert!(
            cache_duration < Duration::from_secs(15),
            "Credentials should be refreshed before the token expires, got {cache_duration:?}"
        );
    }

    #[test]
    fn test_artifact_registry_gcloud_launcher() {
        assert_eq!(
            GOOGLE_CLOUD_SDK_EXECUTABLE,
            if cfg!(windows) {
                "gcloud.cmd"
            } else {
                "gcloud"
            }
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn test_artifact_registry_credentials_from_windows_gcloud_launcher() {
        let directory = tempfile::tempdir().expect("Google Cloud SDK test directory should exist");
        let executable = directory.path().join("gcloud.cmd");
        fs_err::write(
            &executable,
            [
                "@echo off",
                r#"if not "%~1"=="config" exit /b 1"#,
                r#"if not "%~2"=="config-helper" exit /b 1"#,
                r#"if not "%~3"=="--format=json(credential)" exit /b 1"#,
                r#"echo {"credential":{"access_token":"test-token","token_expiry":"2099-05-29T00:00:00Z"}}"#,
            ]
            .join("\r\n"),
        )
        .expect("Google Cloud SDK test launcher should be written");

        assert_eq!(
            ArtifactRegistryProvider::credentials_from_gcloud_command(executable.as_os_str()).await,
            Some((
                Credentials::basic(
                    Some("oauth2accesstoken".to_string()),
                    Some("test-token".to_string())
                ),
                REGISTRY_CREDENTIAL_CACHE_DURATION
            ))
        );
    }

    #[test]
    fn test_azure_artifacts_host() {
        for url in [
            "https://pkgs.dev.azure.com/organization/project/_packaging/feed/pypi/simple/",
            "https://organization.pkgs.visualstudio.com/project/_packaging/feed/pypi/upload/",
        ] {
            assert!(
                AzureArtifactsProvider::is_azure_artifacts(&Url::parse(url).unwrap()),
                "Failed to match Azure Artifacts URL: {url}"
            );
        }

        for url in [
            "http://pkgs.dev.azure.com/organization/_packaging/feed/pypi/simple/",
            "https://pkgs.dev.azure.com.example.com/organization/_packaging/feed/pypi/simple/",
            "https://pkgs.visualstudio.com/organization/_packaging/feed/pypi/simple/",
            "https://pkgs.dev.azure.com/organization/_packaging/feed/npm/registry/",
            "https://pkgs.dev.azure.com/organization/project",
            "https://dev.azure.com/organization/project",
            "https://example.com/organization/_packaging/feed/pypi/simple/",
        ] {
            assert!(
                !AzureArtifactsProvider::is_azure_artifacts(&Url::parse(url).unwrap()),
                "Should not match non-Azure Artifacts URL: {url}"
            );
        }
    }

    #[tokio::test]
    async fn test_azure_artifacts_credentials_from_cache() {
        let credentials = Credentials::bearer(b"test-token".to_vec());
        let provider = AzureArtifactsProvider::with_cached_credentials(Some(credentials.clone()));

        assert_eq!(
            provider
                .credentials_for(
                    &Url::parse(
                        "https://pkgs.dev.azure.com/organization/project/_packaging/feed/pypi/simple/"
                    )
                    .unwrap()
                )
                .await,
            Some(credentials)
        );
        assert_eq!(
            provider
                .credentials_for(&Url::parse("https://example.com/simple/").unwrap())
                .await,
            None
        );
    }

    #[tokio::test]
    async fn test_azure_artifacts_credentials_caches_missing_credentials() {
        let provider = AzureArtifactsProvider::with_cached_credentials(None);

        assert_eq!(
            provider
                .credentials_for(
                    &Url::parse(
                        "https://pkgs.dev.azure.com/organization/project/_packaging/feed/pypi/simple/"
                    )
                    .unwrap()
                )
                .await,
            None
        );
    }

    #[test]
    fn test_azure_artifacts_credentials_from_cli_output() {
        assert_eq!(
            AzureArtifactsProvider::credentials_from_cli_output(
                br#"{"accessToken":"test-token","expires_on":4102444800,"tokenType":"Bearer"}"#
            ),
            Some((
                Credentials::bearer(b"test-token".to_vec()),
                REGISTRY_CREDENTIAL_CACHE_DURATION
            ))
        );

        for output in [
            br#"{"accessToken":"","expires_on":4102444800}"#.as_slice(),
            br#"{"accessToken":"   ","expires_on":4102444800}"#.as_slice(),
            br#"{"accessToken":"test-token","expires_on":946684800}"#.as_slice(),
            br#"{"accessToken":"test-token","expires_on":4102444800,"tokenType":"Basic"}"#
                .as_slice(),
            br#"{"accessToken":"test-token"}"#.as_slice(),
        ] {
            assert_eq!(
                AzureArtifactsProvider::credentials_from_cli_output(output),
                None
            );
        }
    }

    #[test]
    fn test_azure_artifacts_credentials_refresh_before_expiration() {
        let expires_on = Timestamp::now().as_second() + 15;
        let output = format!(
            r#"{{"accessToken":"test-token","expires_on":{expires_on},"tokenType":"Bearer"}}"#
        );

        assert_eq!(
            AzureArtifactsProvider::credentials_from_cli_output(output.as_bytes()),
            None
        );
    }

    #[test]
    fn test_azure_artifacts_cli_launcher() {
        assert_eq!(
            AZURE_CLI_EXECUTABLE,
            if cfg!(windows) { "az.cmd" } else { "az" }
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn test_azure_artifacts_credentials_from_windows_cli_launcher() {
        let directory = tempfile::tempdir().expect("Azure CLI test directory should exist");
        let executable = directory.path().join("az.cmd");
        fs_err::write(
            &executable,
            [
                "@echo off",
                r#"if not "%~1"=="account" exit /b 1"#,
                r#"if not "%~2"=="get-access-token" exit /b 1"#,
                r#"if not "%~3"=="--resource" exit /b 1"#,
                r#"if not "%~4"=="499b84ac-1321-427f-aa17-267ca6975798" exit /b 1"#,
                r#"if not "%~5"=="--output" exit /b 1"#,
                r#"if not "%~6"=="json" exit /b 1"#,
                r#"echo {"accessToken":"test-token","expires_on":4102444800,"tokenType":"Bearer"}"#,
            ]
            .join("\r\n"),
        )
        .expect("Azure CLI test launcher should be written");

        assert_eq!(
            AzureArtifactsProvider::credentials_from_cli_command(executable.as_os_str()).await,
            Some((
                Credentials::bearer(b"test-token".to_vec()),
                REGISTRY_CREDENTIAL_CACHE_DURATION
            ))
        );
    }

    #[test]
    fn test_endpoint_url_matches_path_prefix() {
        let endpoint_url = Url::parse("https://example.com/private").unwrap();

        for url in [
            "https://example.com/private",
            "https://example.com/private/",
            "https://example.com/private/packages/anyio.whl",
        ] {
            assert!(
                is_endpoint_url(&Url::parse(url).unwrap(), &endpoint_url),
                "Failed to match endpoint URL prefix: {url}"
            );
        }
    }

    #[test]
    fn test_endpoint_url_rejects_partial_path_segments() {
        let endpoint_url = Url::parse("https://example.com/private").unwrap();

        for url in [
            "https://example.com/public",
            "https://example.com/private-bucket",
            "https://example.com/privatebucket",
        ] {
            assert!(
                !is_endpoint_url(&Url::parse(url).unwrap(), &endpoint_url),
                "Should not match URL outside endpoint path: {url}"
            );
        }
    }

    #[test]
    fn test_endpoint_url_matches_subdomain_with_path_prefix() {
        let endpoint_url = Url::parse("https://example.com/private").unwrap();

        assert!(is_endpoint_url(
            &Url::parse("https://bucket.example.com/private/package.whl").unwrap(),
            &endpoint_url
        ));
        assert!(!is_endpoint_url(
            &Url::parse("https://bucket.example.com/public/package.whl").unwrap(),
            &endpoint_url
        ));
    }

    #[test]
    fn test_endpoint_url_root_path_matches_all_paths() {
        let endpoint_url = Url::parse("https://example.com").unwrap();

        for url in [
            "https://example.com/package.whl",
            "https://bucket.example.com/package.whl",
        ] {
            assert!(
                is_endpoint_url(&Url::parse(url).unwrap(), &endpoint_url),
                "Failed to match URL under endpoint root: {url}"
            );
        }
    }
}
