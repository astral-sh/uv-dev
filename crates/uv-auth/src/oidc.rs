//! `OpenID` Connect discovery and OAuth 2.0 device authorization (RFC 8628).

use std::time::{Duration, Instant};

use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::debug;
use url::Url;

const DEFAULT_CLIENT_ID: &str = "uv";
const DEFAULT_SCOPE: &str = "openid";
const DEFAULT_POLL_INTERVAL: u64 = 5;

/// The OAuth metadata and refresh token associated with persisted bearer credentials.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct OidcSession {
    /// The authorization server that issued these credentials.
    pub issuer: Url,

    /// The endpoint that exchanges refresh tokens for access tokens.
    pub token_endpoint: Url,

    /// The public OAuth client identifier used for the authorization flow.
    pub client_id: String,

    /// The space-separated scopes requested during device authorization.
    pub scope: String,

    /// The refresh token returned by the authorization server.
    pub refresh_token: String,
}

impl std::fmt::Debug for OidcSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OidcSession")
            .field("issuer", &self.issuer)
            .field("token_endpoint", &self.token_endpoint)
            .field("client_id", &self.client_id)
            .field("scope", &self.scope)
            .field("refresh_token", &"****")
            .finish()
    }
}

/// `OpenID` Connect configuration for a package index.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct OidcConfig {
    /// The `OpenID` Connect issuer to use for endpoint discovery.
    #[serde(default)]
    pub issuer: Option<Url>,

    /// The public OAuth client identifier.
    #[serde(default)]
    pub client_id: Option<String>,

    /// The space-separated OAuth scopes to request.
    #[serde(default)]
    pub scope: Option<String>,
}

/// The discovery metadata needed to perform device authorization.
#[derive(Debug, Deserialize)]
pub struct OidcDiscoveryDocument {
    /// The issuer advertised by the authorization server.
    issuer: String,

    /// The endpoint that issues device and user codes.
    device_authorization_endpoint: String,

    /// The endpoint that exchanges an authorized device code for a token.
    token_endpoint: String,
}

/// The response to a device authorization request.
#[derive(Deserialize)]
pub struct DeviceAuthorizationResponse {
    /// The code used when polling the token endpoint.
    pub device_code: String,

    /// The code the user enters to authorize the request.
    pub user_code: String,

    /// The URL at which the user authorizes the request.
    pub verification_uri: String,

    /// A verification URL that already includes the user code, if available.
    #[serde(default)]
    pub verification_uri_complete: Option<String>,

    /// The number of seconds before the device code expires.
    pub expires_in: u64,

    /// The minimum number of seconds between token requests.
    #[serde(default = "default_poll_interval")]
    pub interval: u64,
}

/// The response to a device token request.
#[derive(Deserialize)]
pub struct DeviceTokenResponse {
    /// The access token returned after successful authorization.
    #[serde(default)]
    pub access_token: Option<String>,

    /// The authentication scheme associated with the access token.
    #[serde(default)]
    pub token_type: Option<String>,

    /// The number of seconds before the access token expires.
    #[serde(default)]
    pub expires_in: Option<u64>,

    /// The refresh token returned when the requested scopes permit offline access.
    #[serde(default)]
    pub refresh_token: Option<String>,

    /// The OAuth error returned while authorization is incomplete or unsuccessful.
    #[serde(default)]
    pub error: Option<String>,

    /// An optional human-readable description of the OAuth error.
    #[serde(default)]
    pub error_description: Option<String>,
}

/// A PKCE S256 challenge and its corresponding verifier.
pub struct PkceChallenge {
    /// The secret verifier sent to the token endpoint.
    code_verifier: String,

    /// The SHA-256 challenge sent to the device authorization endpoint.
    code_challenge: String,
}

/// An error returned while discovering or performing device authorization.
#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    #[error("Device authorization request failed: {0}")]
    DeviceAuthorizationFailed(String),

    #[error("Device authorization timed out")]
    TokenExpired,

    #[error("Authorization denied by user")]
    AccessDenied,

    #[error(transparent)]
    Request(#[from] reqwest::Error),
}

fn default_poll_interval() -> u64 {
    DEFAULT_POLL_INTERVAL
}

/// Generate an S256 PKCE challenge and a cryptographically random verifier.
pub fn generate_pkce() -> PkceChallenge {
    let mut random_bytes = [0; 32];
    rand::rng().fill_bytes(&mut random_bytes);

    let code_verifier = BASE64_URL_SAFE_NO_PAD.encode(random_bytes);
    let code_challenge = BASE64_URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()));

    PkceChallenge {
        code_verifier,
        code_challenge,
    }
}

/// Discover device authorization endpoints, if the issuer supports them.
pub async fn discover(
    client: &reqwest::Client,
    issuer: &Url,
) -> Result<Option<OidcDiscoveryDocument>, OidcError> {
    let mut discovery_url = issuer.clone();
    if !discovery_url.path().ends_with('/') {
        discovery_url.set_path(&format!("{}/", discovery_url.path()));
    }

    let Ok(discovery_url) = discovery_url.join(".well-known/openid-configuration") else {
        return Ok(None);
    };

    debug!("Attempting OpenID Connect discovery at {discovery_url}");

    let response = match client.get(discovery_url.clone()).send().await {
        Ok(response) => response,
        Err(error) => {
            debug!("OpenID Connect discovery at {discovery_url} failed: {error}");
            return Ok(None);
        }
    };

    if !response.status().is_success() {
        debug!(
            "OpenID Connect discovery at {discovery_url} returned {}",
            response.status()
        );
        return Ok(None);
    }

    match response.json::<OidcDiscoveryDocument>().await {
        Ok(document) => {
            debug!("Discovered OpenID Connect issuer {}", document.issuer);
            Ok(Some(document))
        }
        Err(error) => {
            debug!("Invalid OpenID Connect discovery metadata at {discovery_url}: {error}");
            Ok(None)
        }
    }
}

fn resolve_endpoint(issuer: &Url, endpoint: &str) -> Result<Url, OidcError> {
    issuer.join(endpoint).map_err(|error| {
        OidcError::DeviceAuthorizationFailed(format!("Invalid endpoint URL: {error}"))
    })
}

/// Return the session associated with a successful device authorization, if refresh is supported.
pub fn session(
    issuer: &Url,
    discovery: &OidcDiscoveryDocument,
    client_id: Option<&str>,
    scope: Option<&str>,
    refresh_token: Option<String>,
) -> Result<Option<OidcSession>, OidcError> {
    let Some(refresh_token) = refresh_token else {
        return Ok(None);
    };

    Ok(Some(OidcSession {
        issuer: issuer.clone(),
        token_endpoint: resolve_endpoint(issuer, &discovery.token_endpoint)?,
        client_id: client_id.unwrap_or(DEFAULT_CLIENT_ID).to_string(),
        scope: scope.unwrap_or(DEFAULT_SCOPE).to_string(),
        refresh_token,
    }))
}

/// Request a device code and user verification instructions.
pub async fn device_authorize(
    client: &reqwest::Client,
    issuer: &Url,
    discovery: &OidcDiscoveryDocument,
    challenge: &PkceChallenge,
    client_id: Option<&str>,
    scope: Option<&str>,
) -> Result<DeviceAuthorizationResponse, OidcError> {
    let endpoint = resolve_endpoint(issuer, &discovery.device_authorization_endpoint)?;
    let client_id = client_id.unwrap_or(DEFAULT_CLIENT_ID);
    let scope = scope.unwrap_or(DEFAULT_SCOPE);

    let response = client
        .post(endpoint)
        .form(&[
            ("client_id", client_id),
            ("scope", scope),
            ("code_challenge", &challenge.code_challenge),
            ("code_challenge_method", "S256"),
        ])
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(OidcError::DeviceAuthorizationFailed(format!(
            "status {status}: {body}"
        )));
    }

    response.json().await.map_err(|error| {
        OidcError::DeviceAuthorizationFailed(format!(
            "Failed to parse device authorization response: {error}"
        ))
    })
}

/// Poll for an access token until authorization succeeds, fails, or expires.
pub async fn poll_for_token(
    client: &reqwest::Client,
    issuer: &Url,
    discovery: &OidcDiscoveryDocument,
    authorization: &DeviceAuthorizationResponse,
    challenge: &PkceChallenge,
    client_id: Option<&str>,
) -> Result<DeviceTokenResponse, OidcError> {
    let endpoint = resolve_endpoint(issuer, &discovery.token_endpoint)?;
    let client_id = client_id.unwrap_or(DEFAULT_CLIENT_ID);
    let mut interval = authorization.interval;
    let deadline = Instant::now() + Duration::from_secs(authorization.expires_in);

    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;

        if Instant::now() >= deadline {
            return Err(OidcError::TokenExpired);
        }

        let response = client
            .post(endpoint.clone())
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", &authorization.device_code),
                ("client_id", client_id),
                ("code_verifier", &challenge.code_verifier),
            ])
            .send()
            .await?;

        // RFC 8628 uses unsuccessful HTTP status codes for pending authorization.
        let response = response
            .json::<DeviceTokenResponse>()
            .await
            .map_err(|error| {
                OidcError::DeviceAuthorizationFailed(format!(
                    "Failed to parse token response: {error}"
                ))
            })?;

        match response.error.as_deref() {
            Some("authorization_pending") => {
                debug!("Device authorization is still pending");
            }
            Some("slow_down") => {
                interval += 5;
                debug!("Increasing the device authorization polling interval to {interval}s");
            }
            Some("expired_token") => return Err(OidcError::TokenExpired),
            Some("access_denied") => return Err(OidcError::AccessDenied),
            Some(error) => {
                let description = response
                    .error_description
                    .unwrap_or_else(|| error.to_string());
                return Err(OidcError::DeviceAuthorizationFailed(description));
            }
            None => return Ok(response),
        }
    }
}
