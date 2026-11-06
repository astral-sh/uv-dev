//! Check URL-bearing core metadata fields before uploading a distribution.

use std::fmt::{self, Display};
use std::path::Path;
use std::str::FromStr;

use astral_mail_headers::Message;

use uv_pep508::{Pep508Url, Requirement, VersionOrUrl};
use uv_pypi_types::MetadataError;
use uv_redacted::{SafeUrl, SafeUrlError};

use crate::PublishPrepareError;

/// Parse the URLs stored in metadata literally, without expanding environment variables or
/// interpreting unsupported URL schemes as local paths.
#[derive(Debug)]
struct MetadataUrl(SafeUrl);

impl Display for MetadataUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl Pep508Url for MetadataUrl {
    type Err = SafeUrlError;

    fn parse_url(url: &str, _working_dir: Option<&Path>) -> Result<Self, Self::Err> {
        SafeUrl::parse(url).map(Self)
    }

    fn displayable_with_credentials(&self) -> impl Display {
        self.0.displayable_with_credentials()
    }
}

pub(crate) fn validate(contents: &[u8]) -> Result<(), PublishPrepareError> {
    let message = Message::parse(contents);

    // Inspect every occurrence before `Metadata23` discards repeated singleton fields or project
    // URLs with duplicate labels. The original metadata is included in the uploaded archive.
    for field in ["Requires-Dist", "Project-URL", "Home-page", "Download-URL"] {
        for header in message.all(field) {
            let value = header.decoded_value().map_err(MetadataError::from)?;
            let value = value.trim();
            if value.is_empty() || value == "UNKNOWN" {
                continue;
            }

            let url = match field {
                "Requires-Dist" => {
                    // Version constraints do not contain URLs and can use legacy syntax that is
                    // otherwise accepted by the package index.
                    if !value.contains('@') {
                        continue;
                    }
                    let requirement = Requirement::<MetadataUrl>::from_str(value)
                        .map_err(|_| PublishPrepareError::InvalidMetadataUrl(field))?;
                    let Some(VersionOrUrl::Url(url)) = requirement.version_or_url else {
                        continue;
                    };
                    url.0
                }
                "Project-URL" => {
                    let Some((_, url)) = value.split_once(',') else {
                        continue;
                    };
                    parse_url(url.trim(), field)?
                }
                _ => parse_url(value, field)?,
            };
            if url.has_credentials() {
                return Err(PublishPrepareError::MetadataCredentials(field));
            }
        }
    }
    Ok(())
}

fn parse_url(value: &str, field: &'static str) -> Result<SafeUrl, PublishPrepareError> {
    // Parse failures can also contain credentials, so only report the field name.
    SafeUrl::parse(value).map_err(|_| PublishPrepareError::InvalidMetadataUrl(field))
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use uv_pep508::{Pep508Error, Requirement};

    use super::{MetadataUrl, validate};
    use crate::PublishPrepareError;

    #[test]
    fn metadata_urls_are_literal() -> Result<(), Pep508Error<MetadataUrl>> {
        let requirement: Requirement<MetadataUrl> =
            "dependency @ https://example.com/file.whl?path=${PATH}".parse()?;
        assert_snapshot!(requirement, @"dependency @ https://example.com/file.whl?path=${PATH}");
        Ok(())
    }

    #[test]
    fn metadata_url_fields() {
        let results = [
            "Requires-Dist: dependency @ https://user:password@example.com/file.whl",
            "Requires-Dist: dependency @ ftp://user:password@example.com/file.whl",
            "Requires-Dist: dependency[extra] @ https://example.com/file.whl?sig=signature ; python_version < '2'",
            "Project-URL: Source, https://token@example.com/repo",
            "Home-page: https://user:password@example.com/",
            "Download-URL: https://example.com/file.whl?X-Amz-Signature=signature",
            "Home-page: https://example.com/\nHome-page: https://token@example.com/",
            "Project-URL: Source, https://token@example.com/\nProject-URL: Source, https://example.com/",
            "rEqUiReS-dIsT: dependency @\n https://example.com/file.whl?%73ig=signature",
        ]
        .map(|metadata| validate(metadata.as_bytes()).expect_err("Credentials must be rejected"));
        assert_snapshot!(results.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"), @"
        The `Requires-Dist` metadata field contains URL credentials. Remove the credentials or use `--unsafe-allow-credentials` to publish them
        The `Requires-Dist` metadata field contains URL credentials. Remove the credentials or use `--unsafe-allow-credentials` to publish them
        The `Requires-Dist` metadata field contains URL credentials. Remove the credentials or use `--unsafe-allow-credentials` to publish them
        The `Project-URL` metadata field contains URL credentials. Remove the credentials or use `--unsafe-allow-credentials` to publish them
        The `Home-page` metadata field contains URL credentials. Remove the credentials or use `--unsafe-allow-credentials` to publish them
        The `Download-URL` metadata field contains URL credentials. Remove the credentials or use `--unsafe-allow-credentials` to publish them
        The `Home-page` metadata field contains URL credentials. Remove the credentials or use `--unsafe-allow-credentials` to publish them
        The `Project-URL` metadata field contains URL credentials. Remove the credentials or use `--unsafe-allow-credentials` to publish them
        The `Requires-Dist` metadata field contains URL credentials. Remove the credentials or use `--unsafe-allow-credentials` to publish them
        ");
    }

    #[test]
    fn metadata_without_credentials() -> Result<(), PublishPrepareError> {
        validate(
            b"Requires-Dist: dependency >=1.*\n\
              Requires-Dist: dependency[extra] @ https://example.com/file.whl?download=1 ; python_version > '3'\n\
              Requires-Dist: dependency @ git+ssh://git@example.com/repo.git@main\n\
              Requires-Dist: dependency @ ftp://example.com/file.whl?download=1\n\
              Project-URL: Source, https://example.com/repo\n\
              Home-page: UNKNOWN\n\
              Download-URL: https://example.com/file.whl#sha256=abc\n",
        )
    }

    #[test]
    fn invalid_urls_do_not_expose_credentials() {
        let results = [
            "Requires-Dist: dependency @ https://user/name:password@example.com/file.whl",
            "Project-URL: Source, https://user:password@example.com:invalid/",
        ]
        .map(|metadata| validate(metadata.as_bytes()).expect_err("Invalid URLs must be rejected"));
        assert_snapshot!(results.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"), @"
        Cannot check the `Requires-Dist` metadata field for URL credentials because it is invalid. Correct the field or use `--unsafe-allow-credentials` to skip this check
        Cannot check the `Project-URL` metadata field for URL credentials because it is invalid. Correct the field or use `--unsafe-allow-credentials` to skip this check
        ");
    }
}
