use serde::{Deserialize, Deserializer};
#[cfg(feature = "schemars")]
use std::borrow::Cow;
use std::str::FromStr;
use url::Url;

/// A host specification (wildcard, or host, with optional scheme and/or port) for which
/// certificates are not verified when making HTTPS requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustedHost {
    Wildcard,
    Host {
        scheme: Option<String>,
        host: String,
        port: Option<u16>,
    },
}

impl TrustedHost {
    /// Returns `true` if the [`Url`] matches this trusted host.
    pub fn matches(&self, url: &Url) -> bool {
        match self {
            Self::Wildcard => true,
            Self::Host { scheme, host, port } => {
                if scheme.as_ref().is_some_and(|scheme| scheme != url.scheme()) {
                    return false;
                }

                if port.is_some_and(|port| url.port() != Some(port)) {
                    return false;
                }

                if Some(host.as_str()) != url.host_str() {
                    return false;
                }

                true
            }
        }
    }
}

impl<'de> Deserialize<'de> for TrustedHost {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Inner {
            scheme: Option<String>,
            host: String,
            port: Option<u16>,
        }

        serde_untagged::UntaggedEnumVisitor::new()
            .string(|string| Self::from_str(string).map_err(serde::de::Error::custom))
            .map(|map| {
                map.deserialize::<Inner>().map(|inner| Self::Host {
                    scheme: inner.scheme,
                    host: inner.host,
                    port: inner.port,
                })
            })
            .deserialize(deserializer)
    }
}

impl serde::Serialize for TrustedHost {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        let s = self.to_string();
        serializer.serialize_str(&s)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TrustedHostError {
    #[error("missing host for `--trusted-host`: `{0}`")]
    MissingHost(String),
    #[error("invalid port for `--trusted-host`: `{0}`")]
    InvalidPort(String),
}

impl FromStr for TrustedHost {
    type Err = TrustedHostError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "*" {
            return Ok(Self::Wildcard);
        }

        // Detect scheme.
        let (scheme, s) = if let Some(s) = s.strip_prefix("https://") {
            (Some("https".to_string()), s)
        } else if let Some(s) = s.strip_prefix("http://") {
            (Some("http".to_string()), s)
        } else {
            (None, s)
        };

        let mut parts = s.splitn(2, ':');

        // Detect host.
        let host = parts
            .next()
            .and_then(|host| host.split('/').next())
            .filter(|host| !host.is_empty())
            .map(ToString::to_string)
            .ok_or_else(|| TrustedHostError::MissingHost(s.to_string()))?;

        // Detect port.
        let port = parts
            .next()
            .map(str::parse)
            .transpose()
            .map_err(|_| TrustedHostError::InvalidPort(s.to_string()))?;

        Ok(Self::Host { scheme, host, port })
    }
}

impl std::fmt::Display for TrustedHost {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Wildcard => {
                write!(f, "*")?;
            }
            Self::Host { scheme, host, port } => {
                if let Some(scheme) = &scheme {
                    write!(f, "{scheme}://{host}")?;
                } else {
                    write!(f, "{host}")?;
                }

                if let Some(port) = port {
                    write!(f, ":{port}")?;
                }
            }
        }

        Ok(())
    }
}

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for TrustedHost {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("TrustedHost")
    }

    fn json_schema(_generator: &mut schemars::generate::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "A host or host-port pair."
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{TrustedHost, TrustedHostError};

    #[test]
    fn reject_empty_trusted_host_strings() {
        for (input, error_input) in [
            ("", ""),
            ("http://", ""),
            ("https://", ""),
            ("/path", "/path"),
            (":443", ":443"),
            ("https://:443", ":443"),
            ("https:///path", "/path"),
            (":invalid", ":invalid"),
            ("https://:invalid", ":invalid"),
            ("/path:invalid", "/path:invalid"),
        ] {
            let expected = format!("missing host for `--trusted-host`: `{error_input}`");
            let error = input.parse::<TrustedHost>().unwrap_err();
            assert!(matches!(&error, TrustedHostError::MissingHost(value) if value == error_input));
            assert_eq!(error.to_string(), expected);

            let error =
                serde_json::from_value::<TrustedHost>(serde_json::json!(input)).unwrap_err();
            assert_eq!(error.to_string(), expected);

            let json = serde_json::to_string(input).unwrap();
            let error = serde_json::from_str::<TrustedHost>(&json).unwrap_err();
            assert!(
                error.to_string().starts_with(&expected),
                "{input:?}: {error}"
            );
        }
    }

    #[test]
    fn preserve_trusted_host_parse_errors() {
        for (input, error_input) in [
            ("example.com:invalid", "example.com:invalid"),
            ("https://example.com:invalid", "example.com:invalid"),
            ("example.com:65536", "example.com:65536"),
            ("example.com:", "example.com:"),
        ] {
            let error = input.parse::<TrustedHost>().unwrap_err();
            assert!(matches!(&error, TrustedHostError::InvalidPort(value) if value == error_input));
            assert_eq!(
                error.to_string(),
                format!("invalid port for `--trusted-host`: `{error_input}`")
            );
        }

        for (input, scheme, host, port, display) in [
            ("example.com", None, "example.com", None, "example.com"),
            (
                "example.com:8080",
                None,
                "example.com",
                Some(8080),
                "example.com:8080",
            ),
            (
                "http://example.com",
                Some("http"),
                "example.com",
                None,
                "http://example.com",
            ),
            (
                "https://example.com",
                Some("https"),
                "example.com",
                None,
                "https://example.com",
            ),
            (
                "https://example.com/hello/world",
                Some("https"),
                "example.com",
                None,
                "https://example.com",
            ),
            (
                "https://example.com:8443",
                Some("https"),
                "example.com",
                Some(8443),
                "https://example.com:8443",
            ),
            (" ", None, " ", None, " "),
        ] {
            let parsed = input.parse::<TrustedHost>().unwrap();
            assert_eq!(
                parsed,
                TrustedHost::Host {
                    scheme: scheme.map(str::to_string),
                    host: host.to_string(),
                    port,
                }
            );
            assert_eq!(parsed.to_string(), display);
            let json = serde_json::to_string(input).unwrap();
            assert_eq!(serde_json::from_str::<TrustedHost>(&json).unwrap(), parsed);
            assert_eq!(
                serde_json::to_value(&parsed).unwrap(),
                serde_json::json!(display)
            );
        }

        let wildcard = "*".parse::<TrustedHost>().unwrap();
        assert_eq!(wildcard, TrustedHost::Wildcard);
        assert!(wildcard.matches(&url::Url::parse("https://example.com").unwrap()));
        let host = "https://example.com:8443".parse::<TrustedHost>().unwrap();
        assert!(host.matches(&url::Url::parse("https://example.com:8443/path").unwrap()));
        assert!(!host.matches(&url::Url::parse("https://example.com:8444/path").unwrap()));
        assert!(!host.matches(&url::Url::parse("http://example.com:8443/path").unwrap()));
    }

    #[test]
    fn preserve_trusted_host_serde_forms() {
        for (value, expected, display) in [
            (
                serde_json::json!({"host": ""}),
                TrustedHost::Host {
                    scheme: None,
                    host: String::new(),
                    port: None,
                },
                "",
            ),
            (
                serde_json::json!({"scheme": "https", "host": "", "port": 443}),
                TrustedHost::Host {
                    scheme: Some("https".to_string()),
                    host: String::new(),
                    port: Some(443),
                },
                "https://:443",
            ),
            (
                serde_json::json!({"scheme": "https", "host": "example.com", "port": 8443}),
                TrustedHost::Host {
                    scheme: Some("https".to_string()),
                    host: "example.com".to_string(),
                    port: Some(8443),
                },
                "https://example.com:8443",
            ),
        ] {
            let json = serde_json::to_string(&value).unwrap();
            let parsed = serde_json::from_str::<TrustedHost>(&json).unwrap();
            assert_eq!(parsed, expected);
            assert_eq!(parsed.to_string(), display);
            assert_eq!(
                serde_json::to_value(&parsed).unwrap(),
                serde_json::json!(display)
            );
        }
    }

    #[test]
    fn parse() {
        assert_eq!(
            "*".parse::<super::TrustedHost>().unwrap(),
            super::TrustedHost::Wildcard
        );

        assert_eq!(
            "example.com".parse::<super::TrustedHost>().unwrap(),
            super::TrustedHost::Host {
                scheme: None,
                host: "example.com".to_string(),
                port: None
            }
        );

        assert_eq!(
            "example.com:8080".parse::<super::TrustedHost>().unwrap(),
            super::TrustedHost::Host {
                scheme: None,
                host: "example.com".to_string(),
                port: Some(8080)
            }
        );

        assert_eq!(
            "https://example.com".parse::<super::TrustedHost>().unwrap(),
            super::TrustedHost::Host {
                scheme: Some("https".to_string()),
                host: "example.com".to_string(),
                port: None
            }
        );

        assert_eq!(
            "https://example.com/hello/world"
                .parse::<super::TrustedHost>()
                .unwrap(),
            super::TrustedHost::Host {
                scheme: Some("https".to_string()),
                host: "example.com".to_string(),
                port: None
            }
        );
    }
}
