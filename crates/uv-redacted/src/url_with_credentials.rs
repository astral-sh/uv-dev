use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::DisplaySafeUrl;

/// A Serde wire representation that retains a URL's authentication.
///
/// Use this type only for wire formats that explicitly preserve credentials, such as direct
/// download locations in lockfiles and tool receipts. Ordinary [`DisplaySafeUrl`] serialization
/// removes sensitive userinfo and recognized credential query parameters. Display and debug output
/// remain redacted, and deserialization uses the usual human-input ambiguity checks.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schemars", schemars(transparent))]
#[serde(transparent)]
pub struct UrlWithCredentials(DisplaySafeUrl);

impl UrlWithCredentials {
    /// Return the URL for use in requests.
    pub fn as_url(&self) -> &DisplaySafeUrl {
        &self.0
    }

    /// Return the URL for use in requests.
    pub fn into_url(self) -> DisplaySafeUrl {
        self.0
    }
}

impl From<DisplaySafeUrl> for UrlWithCredentials {
    fn from(url: DisplaySafeUrl) -> Self {
        Self(url)
    }
}

impl Serialize for UrlWithCredentials {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        Url::serialize(&self.0, serializer)
    }
}

impl Display for UrlWithCredentials {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, formatter)
    }
}
