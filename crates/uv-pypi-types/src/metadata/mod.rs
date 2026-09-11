mod build_requires;
mod metadata10;
mod metadata23;
mod metadata_resolver;
mod pyproject_toml;
mod requires_dist;

use std::borrow::Cow;
use std::str::Utf8Error;

use astral_mail_headers::{DecodeError, Message};
use thiserror::Error;

use uv_normalize::InvalidNameError;
use uv_pep440::{VersionParseError, VersionSpecifiersParseError};
use uv_pep508::Pep508Error;

use crate::VerbatimParsedUrl;

pub use build_requires::BuildRequires;
pub use metadata_resolver::ResolutionMetadata;
pub use metadata10::Metadata10;
pub use metadata23::{Keywords, Metadata23, ProjectUrls};
pub use pyproject_toml::PyProjectToml;
pub use requires_dist::RequiresDist;

/// <https://github.com/PyO3/python-pkginfo-rs/blob/d719988323a0cfea86d4737116d7917f30e819e2/src/error.rs>
///
/// The error type
#[derive(Error, Debug)]
pub enum MetadataError {
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error("Invalid `pyproject.toml`")]
    InvalidPyprojectTomlSyntax(#[source] toml_edit::TomlError),
    #[error(transparent)]
    InvalidPyprojectTomlSchema(toml_edit::de::Error),
    #[error(
        "`pyproject.toml` is using the `[project]` table, but the required `project.name` field is not set"
    )]
    MissingName,
    #[error("Metadata field {0} not found")]
    FieldNotFound(&'static str),
    #[error("Invalid version: {0}")]
    Pep440VersionError(VersionParseError),
    #[error(transparent)]
    Pep440Error(#[from] VersionSpecifiersParseError),
    #[error(transparent)]
    Pep508Error(#[from] Box<Pep508Error<VerbatimParsedUrl>>),
    #[error(transparent)]
    InvalidName(#[from] InvalidNameError),
    #[error("Invalid `Metadata-Version` field: {0}")]
    InvalidMetadataVersion(String),
    #[error("`Import-Name` and `Import-Namespace` must not both contain `{0}`")]
    DuplicateImportName(String),
    #[error("Reading metadata from `PKG-INFO` requires Metadata 2.2 or later (found: {0})")]
    UnsupportedMetadataVersion(String),
    #[error("The following field was marked as dynamic: {0}")]
    DynamicField(&'static str),
    #[error(
        "The project uses Poetry's syntax to declare its dependencies, despite including a `project` table in `pyproject.toml`"
    )]
    PoetrySyntax,
    #[error("Failed to read `requires.txt` contents")]
    RequiresTxtContents(#[from] std::io::Error),
    #[error("The description is not valid utf-8")]
    DescriptionEncoding(#[source] Utf8Error),
}

impl From<Pep508Error<VerbatimParsedUrl>> for MetadataError {
    fn from(error: Pep508Error<VerbatimParsedUrl>) -> Self {
        Self::Pep508Error(Box::new(error))
    }
}

/// The headers of a distribution metadata file.
#[derive(Debug)]
struct Headers<'a> {
    message: Message<'a>,
}

impl<'a> Headers<'a> {
    /// Parse the headers from the given metadata file content.
    fn parse(content: &'a [u8]) -> Self {
        Self {
            message: Message::parse(content),
        }
    }

    /// Return the first value associated with the header with the given name.
    fn get_first_value(&self, name: &str) -> Result<Option<String>, DecodeError> {
        self.message
            .first(name)
            .map(|header| header.decoded_value().map(Cow::into_owned))
            .transpose()
            .map(|value| value.filter(|value| value != "UNKNOWN"))
    }

    /// Return all values associated with the header with the given name.
    fn get_all_values<'m>(
        &'m self,
        name: &'m str,
    ) -> impl Iterator<Item = Result<String, DecodeError>> + 'm {
        self.message
            .all(name)
            .map(|header| header.decoded_value().map(Cow::into_owned))
            .filter(|value| match value {
                Ok(value) => value != "UNKNOWN",
                Err(_error) => true,
            })
    }
}

/// Parse a `Metadata-Version` field into a (major, minor) tuple.
fn parse_version(metadata_version: &str) -> Result<(u8, u8), MetadataError> {
    let (major, minor) = metadata_version
        .split_once('.')
        .ok_or_else(|| MetadataError::InvalidMetadataVersion(metadata_version.to_string()))?;
    let major = major
        .parse::<u8>()
        .map_err(|_| MetadataError::InvalidMetadataVersion(metadata_version.to_string()))?;
    let minor = minor
        .parse::<u8>()
        .map_err(|_| MetadataError::InvalidMetadataVersion(metadata_version.to_string()))?;
    Ok((major, minor))
}

#[cfg(test)]
mod headers_tests {
    use super::{DecodeError, Headers, MetadataError, ResolutionMetadata};

    #[test]
    fn all_values_preserve_order_and_duplicates() -> Result<(), DecodeError> {
        let headers = Headers::parse(
            b"X-Value: first\r\nx-VALUE: UNKNOWN\r\nOther: ignored\r\nX-VALUE: second\r\nX-Value: first\r\nx-value: unknown\r\n\r\n",
        );
        let name = String::from("x-value");
        let mut values = headers.get_all_values(name.as_str());

        assert_eq!(values.next().transpose()?.as_deref(), Some("first"));
        assert_eq!(
            values.collect::<Result<Vec<_>, _>>()?,
            ["second", "first", "unknown"]
        );
        assert!(headers.get_all_values("missing").next().is_none());
        Ok(())
    }

    #[test]
    fn all_values_preserve_decoding() -> Result<(), DecodeError> {
        let headers = Headers::parse(
            b"X-Value: =?utf-8?Q?touch=C3=A9?=\r\nX-Value: folded\r\n\tvalue\r\nX-Value: touch\xc3\xa9\r\nX-Value: touch\xe9\r\nX-Value: =?utf-8?Q?UNKNOWN?=\r\n\r\n",
        );
        let values = headers
            .get_all_values("X-Value")
            .collect::<Result<Vec<_>, _>>()?;

        assert_eq!(values, ["touché", "folded value", "touché", "touch�"]);
        Ok(())
    }

    #[test]
    fn pkg_info_reports_first_dynamic_field() {
        let cases: [(&[u8], &str); 3] = [
            (
                b"Metadata-Version: 2.2\r\nName: example\r\nVersion: 1.0\r\nDynamic: UNKNOWN\r\nDynamic: Version\r\ndYnAmIc: Requires-Python\r\nDynamic: Requires-Dist\r\n\r\n",
                "Requires-Python",
            ),
            (
                b"Metadata-Version: 2.2\r\nName: example\r\nVersion: 1.0\r\nDynamic: Requires-Dist\r\nDynamic: Requires-Python\r\n\r\n",
                "Requires-Dist",
            ),
            (
                b"Metadata-Version: 2.2\r\nName: example\r\nVersion: 1.0\r\nDynamic: Provides-Extra\r\nDynamic: Requires-Dist\r\n\r\n",
                "Provides-Extra",
            ),
        ];

        for (content, expected) in cases {
            match ResolutionMetadata::parse_pkg_info(content).unwrap_err() {
                MetadataError::DynamicField(field) => assert_eq!(field, expected),
                other => panic!("expected the first dynamic field, got {other}"),
            }
        }
    }
}
