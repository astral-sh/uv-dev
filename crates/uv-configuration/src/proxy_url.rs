#[cfg(feature = "schemars")]
use std::borrow::Cow;
use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use reqwest::Proxy;
use serde::{Deserialize, Deserializer, Serialize};
use url::Url;

use uv_redacted::DisplaySafeUrl;

/// A validated proxy URL.
///
/// This type validates that the [`Url`] is valid for a [`reqwest::Proxy`] on construction.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProxyUrl(DisplaySafeUrl);

/// Mapping to [`reqwest::proxy::Intercept`] kinds which are not public API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProxyUrlKind {
    Http,
    Https,
}

impl ProxyUrl {
    /// Returns a reference to the underlying URL.
    fn as_url(&self) -> &DisplaySafeUrl {
        &self.0
    }

    /// Constructs a [`reqwest::Proxy`] from this [`ProxyUrl`] for the given [`ProxyUrlKind`].
    pub fn as_proxy(&self, kind: ProxyUrlKind) -> Proxy {
        // The same conversion was validated on construction.
        match kind {
            ProxyUrlKind::Http => Proxy::http(self.0.as_str())
                .expect("Constructing a proxy from a url should never fail"),
            ProxyUrlKind::Https => Proxy::https(self.0.as_str())
                .expect("Constructing a proxy from a url should never fail"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProxyUrlError {
    #[error("invalid proxy URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error(
        "invalid proxy URL scheme `{scheme}` in `{url}`: expected http, https, socks5, or socks5h"
    )]
    InvalidScheme { scheme: String, url: DisplaySafeUrl },
}

/// Returns true if the input likely has no scheme (no "://" present).
fn lacks_scheme(s: &str) -> bool {
    !s.contains("://")
}

impl FromStr for ProxyUrl {
    type Err = ProxyUrlError;

    /// Parses a proxy URL from a string, assuming `http://` if no scheme is present.
    ///
    /// This matches reqwest's and curl's behavior.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        fn try_with_http_scheme(s: &str) -> Result<ProxyUrl, ProxyUrlError> {
            let with_scheme = format!("http://{s}");
            let url = Url::parse(&with_scheme)?;
            ProxyUrl::try_from(url)
        }

        match Url::parse(s) {
            Ok(url) => match Self::try_from(url) {
                Ok(proxy) => Ok(proxy),
                Err(ProxyUrlError::InvalidScheme { .. }) if lacks_scheme(s) => {
                    try_with_http_scheme(s)
                }
                Err(e) => Err(e),
            },
            Err(url::ParseError::RelativeUrlWithoutBase) => try_with_http_scheme(s),
            Err(err) => Err(ProxyUrlError::InvalidUrl(err)),
        }
    }
}

impl TryFrom<Url> for ProxyUrl {
    type Error = ProxyUrlError;

    fn try_from(url: Url) -> Result<Self, Self::Error> {
        let url = DisplaySafeUrl::from_url(url);
        match url.scheme() {
            "http" | "https" | "socks5" | "socks5h" => {
                Proxy::http(url.as_str()).map_err(|_| url::ParseError::EmptyHost)?;
                Ok(Self(url))
            }
            scheme => Err(ProxyUrlError::InvalidScheme {
                scheme: scheme.to_string(),
                url,
            }),
        }
    }
}

impl Display for ProxyUrl {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl<'de> Deserialize<'de> for ProxyUrl {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_str(&s).map_err(serde::de::Error::custom)
    }
}

impl Serialize for ProxyUrl {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        serializer.serialize_str(self.as_url().as_str())
    }
}

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for ProxyUrl {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("ProxyUrl")
    }

    fn json_schema(_generator: &mut schemars::generate::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "format": "uri",
            "description": "A proxy URL (e.g., `http://proxy.example.com:8080`)."
        })
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;

    #[test]
    fn parse_valid_proxy_urls() {
        // HTTP proxy
        let url = "http://proxy.example.com:8080".parse::<ProxyUrl>().unwrap();
        assert_eq!(url.to_string(), "http://proxy.example.com:8080/");

        // HTTPS proxy
        let url = "https://proxy.example.com:8080"
            .parse::<ProxyUrl>()
            .unwrap();
        assert_eq!(url.to_string(), "https://proxy.example.com:8080/");

        // SOCKS5 proxy (no trailing slash for socks URLs)
        let url = "socks5://proxy.example.com:1080"
            .parse::<ProxyUrl>()
            .unwrap();
        assert_eq!(url.to_string(), "socks5://proxy.example.com:1080");

        // SOCKS5H proxy
        let url = "socks5h://proxy.example.com:1080"
            .parse::<ProxyUrl>()
            .unwrap();
        assert_eq!(url.to_string(), "socks5h://proxy.example.com:1080");

        // Proxy with auth
        let url = "http://user:pass@proxy.example.com:8080"
            .parse::<ProxyUrl>()
            .unwrap();
        assert_eq!(
            Url::from(url.as_url().clone()).to_string(),
            "http://user:pass@proxy.example.com:8080/"
        );
    }

    #[test]
    fn parse_proxy_url_without_scheme() {
        // URL without a scheme (no "://") should default to http://
        // This matches curl and reqwest behavior
        let url = "proxy.example.com:8080".parse::<ProxyUrl>().unwrap();
        assert_eq!(url.to_string(), "http://proxy.example.com:8080/");

        // With auth but no scheme
        let url = "user:pass@proxy.example.com:8080"
            .parse::<ProxyUrl>()
            .unwrap();
        assert_eq!(
            Url::from(url.as_url().clone()).to_string(),
            "http://user:pass@proxy.example.com:8080/"
        );

        // Just hostname
        let url = "proxy.example.com".parse::<ProxyUrl>().unwrap();
        assert_eq!(url.to_string(), "http://proxy.example.com/");
    }

    #[test]
    fn parse_invalid_proxy_urls() {
        let result = "ftp://proxy.example.com:8080".parse::<ProxyUrl>();
        assert_matches!(result, Err(ProxyUrlError::InvalidScheme { .. }));
        insta::assert_snapshot!(
            result.unwrap_err().to_string(),
            @"invalid proxy URL scheme `ftp` in `ftp://proxy.example.com:8080/`: expected http, https, socks5, or socks5h"
        );

        // Invalid URL (spaces are not allowed)
        let result = "not a url".parse::<ProxyUrl>();
        assert_matches!(result, Err(ProxyUrlError::InvalidUrl(_)));
        insta::assert_snapshot!(
            result.unwrap_err().to_string(),
            @"invalid proxy URL: invalid international domain name"
        );

        // Empty string
        let result = "".parse::<ProxyUrl>();
        assert_matches!(result, Err(ProxyUrlError::InvalidUrl(_)));
        insta::assert_snapshot!(
            result.unwrap_err().to_string(),
            @"invalid proxy URL: empty host"
        );

        let result = "file:///path/to/file".parse::<ProxyUrl>();
        assert_matches!(result, Err(ProxyUrlError::InvalidScheme { .. }));
        insta::assert_snapshot!(
            result.unwrap_err().to_string(),
            @"invalid proxy URL scheme `file` in `file:///path/to/file`: expected http, https, socks5, or socks5h"
        );
    }

    #[test]
    fn deserialize_invalid_proxy_url() {
        let result: Result<ProxyUrl, _> = serde_json::from_str(r#""ftp://proxy.example.com:8080""#);
        insta::assert_snapshot!(
            result.unwrap_err().to_string(),
            @"invalid proxy URL scheme `ftp` in `ftp://proxy.example.com:8080/`: expected http, https, socks5, or socks5h"
        );

        let result: Result<ProxyUrl, _> = serde_json::from_str(r#""not a url""#);
        insta::assert_snapshot!(
            result.unwrap_err().to_string(),
            @"invalid proxy URL: invalid international domain name"
        );
    }

    #[test]
    fn reject_unconstructible_proxy_urls() {
        for input in [
            "socks5:!",
            "socks5h:!",
            "socks5:fixture-user:fixture-secret@proxy.invalid:!",
        ] {
            let url = Url::parse(input).unwrap();
            assert!(!url.has_host());
            assert!(Proxy::http(url.as_str()).is_err());
            assert!(Proxy::https(url.as_str()).is_err());

            for error in [
                ProxyUrl::try_from(url).unwrap_err(),
                ProxyUrl::from_str(input).unwrap_err(),
            ] {
                assert_matches!(
                    &error,
                    ProxyUrlError::InvalidUrl(url::ParseError::EmptyHost)
                );
                assert_eq!(error.to_string(), "invalid proxy URL: empty host");
            }
            assert_eq!(
                serde_json::from_value::<ProxyUrl>(serde_json::Value::from(input))
                    .unwrap_err()
                    .to_string(),
                "invalid proxy URL: empty host",
            );
        }
    }

    #[test]
    fn preserve_proxy_fallback_and_serialization() {
        for (input, stored) in [
            (
                "http://proxy.example.com:8080",
                "http://proxy.example.com:8080/",
            ),
            (
                "https://proxy.example.com:8080",
                "https://proxy.example.com:8080/",
            ),
            ("socks5://proxy.example.com", "socks5://proxy.example.com"),
            (
                "socks5h://proxy.example.com:1080",
                "socks5h://proxy.example.com:1080",
            ),
            ("proxy.example.com:8080", "http://proxy.example.com:8080/"),
            ("proxy.example.com", "http://proxy.example.com/"),
            ("socks5:1080", "socks5:1080"),
            ("socks5h:1080", "socks5h:1080"),
            ("http://[::1]:8080", "http://[::1]:8080/"),
            ("https://[2001:db8::1]:8443", "https://[2001:db8::1]:8443/"),
            ("socks5://[::1]:1080", "socks5://[::1]:1080"),
            ("socks5h://[2001:db8::1]", "socks5h://[2001:db8::1]"),
        ] {
            let proxy = ProxyUrl::from_str(input).unwrap();
            assert_eq!(proxy.to_string(), stored);
            assert_eq!(
                serde_json::to_value(&proxy).unwrap(),
                serde_json::Value::from(stored),
            );
            assert_eq!(
                ProxyUrl::try_from(Url::parse(stored).unwrap()).unwrap(),
                proxy
            );
            assert_eq!(
                serde_json::from_value::<ProxyUrl>(serde_json::Value::from(input)).unwrap(),
                proxy,
            );
            for kind in [ProxyUrlKind::Http, ProxyUrlKind::Https] {
                let _proxy = proxy.as_proxy(kind);
            }
        }

        let input = "http://fixture-user:fixture-secret@proxy.example.com:8080";
        let proxy = ProxyUrl::from_str(input).unwrap();
        assert_eq!(
            serde_json::to_value(&proxy).unwrap(),
            serde_json::Value::from("http://fixture-user:fixture-secret@proxy.example.com:8080/"),
        );
        assert!(!proxy.to_string().contains("fixture-secret"));
        assert!(!format!("{proxy:?}").contains("fixture-secret"));
        for kind in [ProxyUrlKind::Http, ProxyUrlKind::Https] {
            let _proxy = proxy.as_proxy(kind);
        }
    }

    #[test]
    fn proxy_validation_errors_are_url_free() {
        let input = "socks5:fixture-user:fixture-secret@proxy.invalid:!";
        for error in [
            ProxyUrl::try_from(Url::parse(input).unwrap()).unwrap_err(),
            ProxyUrl::from_str(input).unwrap_err(),
        ] {
            assert_eq!(error.to_string(), "invalid proxy URL: empty host");
            assert_eq!(format!("{error:?}"), "InvalidUrl(EmptyHost)");
            let source = std::error::Error::source(&error).unwrap();
            assert_eq!(source.to_string(), "empty host");
            assert_eq!(format!("{source:?}"), "EmptyHost");
            assert!(source.source().is_none());
        }
    }
}
