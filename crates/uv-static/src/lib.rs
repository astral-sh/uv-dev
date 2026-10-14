pub use env_vars::*;
pub use known_stdlib::*;

mod env_vars;
mod known_stdlib;

use std::borrow::Cow;

use thiserror::Error;

/// The implementation used to read and write tar archives.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TarBackend {
    #[default]
    TarCodec,
    TokioTar,
}

impl TarBackend {
    /// Select the tar implementation, allowing users to enable the legacy backend.
    pub fn from_env() -> Self {
        if matches!(
            parse_boolish_environment_variable(EnvVars::UV_LEGACY_TAR_BACKEND),
            Ok(Some(true))
        ) {
            Self::TokioTar
        } else {
            Self::TarCodec
        }
    }
}

/// The base URL for the default Astral mirror.
const ASTRAL_MIRROR_BASE_URL: &str = "https://releases.astral.sh";

/// Read the user-configured Astral mirror URL from the environment, if set.
pub fn astral_mirror_url_from_env() -> Option<String> {
    std::env::var_os(EnvVars::UV_ASTRAL_MIRROR_URL).and_then(|url| {
        if url.as_os_str().is_empty() {
            None
        } else {
            Some(url.to_string_lossy().into_owned())
        }
    })
}

/// Return the effective Astral mirror base URL, using the default mirror when unset.
pub fn astral_mirror_base_url(astral_mirror_url: Option<&str>) -> Cow<'_, str> {
    custom_astral_mirror_url(astral_mirror_url)
        .map(|url| Cow::Borrowed(url.trim_end_matches('/')))
        .unwrap_or(Cow::Borrowed(ASTRAL_MIRROR_BASE_URL))
}

/// Return a user-configured Astral mirror URL, treating empty values as unset.
pub fn custom_astral_mirror_url(astral_mirror_url: Option<&str>) -> Option<&str> {
    astral_mirror_url.filter(|url| !url.is_empty())
}

#[derive(Debug, Error)]
#[error("Failed to parse environment variable `{name}` with invalid value `{value}`: {err}")]
pub struct InvalidEnvironmentVariable {
    pub name: String,
    pub value: String,
    pub err: String,
}

/// Parse a boolean environment variable.
///
/// Adapted from Clap's `BoolishValueParser` which is dual licensed under the MIT and Apache-2.0.
pub fn parse_boolish_environment_variable(
    name: &'static str,
) -> Result<Option<bool>, InvalidEnvironmentVariable> {
    let Some(value) = std::env::var_os(name) else {
        return Ok(None);
    };

    let Some(value) = value.to_str() else {
        return Err(InvalidEnvironmentVariable {
            name: name.to_string(),
            value: value.to_string_lossy().to_string(),
            err: "expected a valid UTF-8 string".to_string(),
        });
    };

    let value = match value.to_lowercase().as_str() {
        "y" | "yes" | "t" | "true" | "on" | "1" => true,
        "n" | "no" | "f" | "false" | "off" | "0" => false,
        _ => {
            return Err(InvalidEnvironmentVariable {
                name: name.to_string(),
                value: value.to_string(),
                err: "expected a boolish value".to_string(),
            });
        }
    };

    Ok(Some(value))
}
