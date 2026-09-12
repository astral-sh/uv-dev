use http::Extensions;
use std::fmt::Debug;
use uv_auth::AzureEndpointProvider;
use uv_preview::Preview;
use uv_redacted::DisplaySafeUrl;

use reqwest::header::HeaderValue;
use reqwest::{Request, Response};
use reqwest_middleware::{Middleware, Next};

/// Reject accidental public-index access from local-scenario tests.
pub(crate) struct DenyPypiMiddleware;

#[derive(Debug, thiserror::Error)]
#[error("Live PyPI access is disabled for this test: `{url}`")]
pub(crate) struct DenyPypiError {
    url: DisplaySafeUrl,
}

impl DenyPypiMiddleware {
    pub(crate) fn check(url: &url::Url) -> Result<(), DenyPypiError> {
        if let Some(host) = url.host_str() {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            if [
                "pypi.org",
                "pythonhosted.org",
                "pypi.python.org",
                "pypi-proxy.fly.dev",
            ]
            .iter()
            .any(|domain| {
                host == *domain
                    || host
                        .strip_suffix(domain)
                        .is_some_and(|prefix| prefix.ends_with('.'))
            }) {
                return Err(DenyPypiError {
                    url: DisplaySafeUrl::from_url(url.clone()),
                });
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl Middleware for DenyPypiMiddleware {
    async fn handle(
        &self,
        request: Request,
        extensions: &mut Extensions,
        next: Next<'_>,
    ) -> reqwest_middleware::Result<Response> {
        Self::check(request.url())
            .map_err(|error| reqwest_middleware::Error::Middleware(error.into()))?;
        next.run(request, extensions).await
    }
}

pub(crate) struct AzureStorageMiddleware {
    pub(crate) preview: Preview,
}

#[async_trait::async_trait]
impl Middleware for AzureStorageMiddleware {
    async fn handle(
        &self,
        mut request: Request,
        extensions: &mut Extensions,
        next: Next<'_>,
    ) -> reqwest_middleware::Result<Response> {
        if AzureEndpointProvider::is_azure_endpoint(request.url(), self.preview)? {
            // Anonymous requests need 2019-12-12 or later to return a 401
            // bearer challenge instead of 409 when account-level public access
            // is disabled. Use the earliest such version supported by Azure
            // Stack Hub (2301 and later).
            //
            // https://learn.microsoft.com/en-us/rest/api/storageservices/authorize-with-azure-active-directory#bearer-challenge
            // https://learn.microsoft.com/en-us/azure-stack/user/azure-stack-acs-differences#api-version
            request
                .headers_mut()
                .entry("x-ms-version")
                .or_insert(HeaderValue::from_static("2020-10-02"));
        }

        next.run(request, extensions).await
    }
}

/// A custom error type for the offline middleware.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OfflineError {
    url: DisplaySafeUrl,
}

impl OfflineError {
    /// Returns the URL that caused the error.
    pub(crate) fn url(&self) -> &DisplaySafeUrl {
        &self.url
    }
}

impl std::fmt::Display for OfflineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Network connectivity is disabled, but the requested data wasn't found in the cache for: `{}`",
            self.url
        )
    }
}

impl std::error::Error for OfflineError {}

/// A middleware that always returns an error indicating that the client is offline.
pub(crate) struct OfflineMiddleware;

#[async_trait::async_trait]
impl Middleware for OfflineMiddleware {
    async fn handle(
        &self,
        req: Request,
        _extensions: &mut Extensions,
        _next: Next<'_>,
    ) -> reqwest_middleware::Result<Response> {
        Err(reqwest_middleware::Error::Middleware(
            OfflineError {
                url: DisplaySafeUrl::from_url(req.url().clone()),
            }
            .into(),
        ))
    }
}
