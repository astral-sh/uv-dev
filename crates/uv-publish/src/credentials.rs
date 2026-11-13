//! Check URLs in core metadata before publishing the archive.

use astral_mail_headers::Message;

use uv_pep508::{Requirement, VersionOrUrl};
use uv_pypi_types::MetadataError;
use uv_redacted::SafeUrl;

use crate::PublishPrepareError;

pub(crate) fn validate(message: &Message<'_>) -> Result<(), PublishPrepareError> {
    // The archive retains every header, even if `Metadata23` drops a duplicate project URL label
    // or a repeated singleton field.
    for field in ["Requires-Dist", "Project-URL", "Home-page", "Download-URL"] {
        for header in message.all(field) {
            let value = header.decoded_value().map_err(MetadataError::from)?;
            let value = value.trim();
            if value.is_empty() || value == "UNKNOWN" {
                continue;
            }

            // Parser errors can contain the original credentials. Report only the field name.
            let invalid = || PublishPrepareError::InvalidMetadataUrl(field);
            let url = if field == "Requires-Dist" {
                // A direct reference has an `@`, and a bare absolute URL has a scheme.
                // Ignore marker text when deciding whether a legacy requirement needs parsing.
                let dependency = value
                    .split_once(';')
                    .map_or(value, |(dependency, _)| dependency);
                if !dependency.contains(['@', ':']) {
                    continue;
                }
                let requirement = value
                    .parse::<Requirement<SafeUrl>>()
                    .map_err(|_| invalid())?;
                let Some(VersionOrUrl::Url(url)) = requirement.version_or_url else {
                    continue;
                };
                url
            } else {
                let url = if field == "Project-URL" {
                    let (_, url) = value.split_once(',').ok_or_else(invalid)?;
                    url.trim()
                } else {
                    value
                };
                SafeUrl::parse(url).map_err(|_| invalid())?
            };

            if url.has_credentials() {
                return Err(PublishPrepareError::MetadataCredentials(field));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use astral_mail_headers::Message;
    use insta::assert_snapshot;
    use uv_pep508::Requirement;
    use uv_redacted::SafeUrl;

    use super::validate;
    use crate::PublishPrepareError;

    #[test]
    fn metadata_urls_are_literal() {
        let requirement: Requirement<SafeUrl> =
            "dependency @ ftp://example.com/file.whl?path=${PATH}"
                .parse()
                .expect("Valid direct reference");
        assert_snapshot!(requirement, @"dependency @ ftp://example.com/file.whl?path=${PATH}");
    }

    #[test]
    fn metadata_url_fields() {
        for (field, value) in [
            (
                "Requires-Dist",
                "dependency @ ftp://user:password@example.com/file.whl",
            ),
            (
                "Requires-Dist",
                "dependency @\n https://example.com/file.whl?%73ig=signature",
            ),
            ("Project-URL", "Source, https://token@example.com/repo"),
            (
                "Home-page",
                "https://example.com/\nHome-page: https://token@example.com/",
            ),
            (
                "Download-URL",
                "https://example.com/file.whl?X-Amz-Signature=signature",
            ),
        ] {
            let header = format!("{field}: {value}");
            assert_matches!(validate(&Message::parse(header.as_bytes())),
                Err(PublishPrepareError::MetadataCredentials(actual)) if actual == field);
        }
    }

    #[test]
    fn metadata_without_credentials() -> Result<(), PublishPrepareError> {
        validate(&Message::parse(
            b"Requires-Dist: dependency >=1.* ; platform_version == '12:34'\n\
              Requires-Dist: dependency @ git+ssh://git@example.com/repo.git@main\n\
              Project-URL: Source, https://example.com/repo\n\
              Home-page: UNKNOWN\n\
              Download-URL: https://example.com/file.whl?download=1#sha256=abc\n",
        ))
    }

    #[test]
    fn malformed_fields_do_not_expose_credentials() {
        for (field, value) in [
            ("Requires-Dist", "https://example.com/file.whl?sig=secret"),
            (
                "Requires-Dist",
                "dependency @ https://user/name:secret@example.com/file.whl",
            ),
            ("Project-URL", "https://token@example.com/repo"),
            (
                "Project-URL",
                "Source, https://user:secret@example.com:invalid/",
            ),
        ] {
            let header = format!("{field}: {value}");
            assert_matches!(validate(&Message::parse(header.as_bytes())),
                Err(PublishPrepareError::InvalidMetadataUrl(actual)) if actual == field);
        }
    }
}
