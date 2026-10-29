use thiserror::Error;
use uv_redacted::SafeUrl;

use crate::{
    CanonicalFileLocation, IndexLocations, IndexRequestPolicy, IndexUrl, ProxyIndexError,
    ToUrlError,
};

/// A registry artifact's fetch URL together with the route that governs its request and cache.
#[derive(Debug, Clone)]
pub struct RegistryArtifactRequest<'a> {
    url: SafeUrl,
    context: RegistryRequestContext<'a>,
}

/// The cache identity and request policy selected from one registry route.
///
/// A proxy can share its endpoint URL with another index declaration. Keep its policy with the
/// request instead of looking the policy up again by that endpoint URL.
#[derive(Debug, Clone, Copy)]
pub struct RegistryRequestContext<'a> {
    index: &'a IndexUrl,
    policy: &'a IndexRequestPolicy,
    is_proxy: bool,
}

impl<'a> RegistryArtifactRequest<'a> {
    /// Prepare a canonical artifact for the configured direct or proxy route.
    pub fn new(
        index: &'a IndexUrl,
        location: &CanonicalFileLocation,
        indexes: &'a IndexLocations,
    ) -> Result<Self, RegistryArtifactRequestError> {
        if let Some(proxy) = indexes.proxy_route_for(index) {
            Ok(Self {
                url: proxy.artifact_url_for_request(location)?,
                context: RegistryRequestContext {
                    index: proxy.effective_url(),
                    policy: proxy.request_policy(),
                    is_proxy: true,
                },
            })
        } else {
            Ok(Self {
                url: location.to_url()?,
                context: RegistryRequestContext {
                    index,
                    policy: indexes.request_policy(index),
                    is_proxy: false,
                },
            })
        }
    }

    /// The artifact URL used for HTTP requests.
    pub fn url(&self) -> &SafeUrl {
        &self.url
    }

    /// The route information to retain through metadata and download fallbacks.
    pub fn context(&self) -> RegistryRequestContext<'a> {
        self.context
    }

    /// Consume the prepared request and return its fetch URL.
    pub fn into_url(self) -> SafeUrl {
        self.url
    }
}

impl<'a> RegistryRequestContext<'a> {
    /// The physical index used for cache partitioning and endpoint capabilities.
    pub fn index(self) -> &'a IndexUrl {
        self.index
    }

    /// The request policy of the selected index declaration.
    pub fn policy(self) -> &'a IndexRequestPolicy {
        self.policy
    }

    /// Whether registry hashes must be enforced for a configured proxy.
    pub fn is_proxy(self) -> bool {
        self.is_proxy
    }
}

/// An artifact location could not be prepared for its registry route.
#[derive(Debug, Error)]
pub enum RegistryArtifactRequestError {
    #[error(transparent)]
    Proxy(#[from] ProxyIndexError),
    #[error(transparent)]
    InvalidUrl(#[from] ToUrlError),
}
