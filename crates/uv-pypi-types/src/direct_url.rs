use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};
use thiserror::Error;
use uv_git_types::GitUrl;
use uv_redacted::policy::Stripped;
use uv_redacted::{SafeUrl, SafeUrlError};

use crate::{HashAlgorithm, Hashes};

/// Metadata for a distribution that was installed via a direct URL.
///
/// See: <https://packaging.python.org/en/latest/specifications/direct-url-data-structure/>
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case", untagged)]
pub enum DirectUrl {
    /// The direct URL is a local directory. For example:
    /// ```json
    /// {"url": "file:///home/user/project", "dir_info": {}}
    /// ```
    LocalDirectory {
        url: SafeUrl<Stripped>,
        dir_info: DirInfo,
        #[serde(skip_serializing_if = "Option::is_none")]
        subdirectory: Option<Box<Path>>,
    },
    /// The direct URL is a path to an archive. For example:
    /// ```json
    /// {"archive_info": {"hash": "sha256=75909db2664838d015e3d9139004ee16711748a52c8f336b52882266540215d8", "hashes": {"sha256": "75909db2664838d015e3d9139004ee16711748a52c8f336b52882266540215d8"}}, "url": "https://files.pythonhosted.org/packages/b8/8b/31273bf66016be6ad22bb7345c37ff350276cfd46e389a0c2ac5da9d9073/wheel-0.41.2-py3-none-any.whl"}
    /// ```
    ArchiveUrl {
        /// The archive URL without a subdirectory fragment.
        url: SafeUrl<Stripped>,
        archive_info: ArchiveInfo,
        #[serde(skip_serializing_if = "Option::is_none")]
        subdirectory: Option<Box<Path>>,
    },
    /// The direct URL is path to a VCS repository. For example:
    /// ```json
    /// {"url": "https://github.com/pallets/flask.git", "vcs_info": {"commit_id": "8d9519df093864ff90ca446d4af2dc8facd3c542", "vcs": "git", "git_lfs": true }}
    /// ```
    VcsUrl {
        #[serde(flatten)]
        url: VcsUrl,
        #[serde(skip_serializing_if = "Option::is_none")]
        subdirectory: Option<Box<Path>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<PathBuf>,
    },
}

/// The repository URL and VCS information for a `direct_url.json` source.
///
/// The URL's VCS prefix matches the kind in [`VcsInfo`], for example `git+https://...`.
/// Serialized metadata omits the prefix and sensitive userinfo, while retaining the complete query.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VcsUrl {
    url: SafeUrl<Stripped>,
    vcs_info: VcsInfo,
}

impl VcsUrl {
    /// Parse a transport URL from `direct_url.json` with its VCS information.
    ///
    /// The VCS kind is added as a prefix before parsing, for example `https://...` becomes
    /// `git+https://...` for Git.
    ///
    /// # Errors
    ///
    /// Returns an error if the prefixed URL is invalid or has ambiguous credentials.
    fn parse(url: &str, vcs_info: VcsInfo) -> Result<Self, SafeUrlError> {
        Ok(Self {
            url: SafeUrl::parse(&format!("{}+{url}", vcs_info.vcs))?.with_policy(Stripped),
            vcs_info,
        })
    }

    /// Return the repository URL with its VCS prefix.
    pub fn url(&self) -> &SafeUrl<Stripped> {
        &self.url
    }

    /// Return the VCS metadata, including its kind and any revision.
    pub fn vcs_info(&self) -> &VcsInfo {
        &self.vcs_info
    }
}

impl From<&GitUrl> for VcsUrl {
    /// Build the metadata URL from the repository URL with a `git+` prefix; revisions and derived
    /// metadata are stored separately.
    fn from(value: &GitUrl) -> Self {
        // A `GitUrl` may contain a repository URL accepted without the ambiguous-credential check.
        // Reapplying the check when adding `git+` would reject those URLs.
        let url = SafeUrl::parse_relaxed(&format!("git+{}", value.url().as_str()))
            .expect("Git URL is invalid")
            .with_policy(Stripped);
        Self {
            url,
            vcs_info: VcsInfo {
                vcs: VcsKind::Git,
                commit_id: value.precise().as_ref().map(ToString::to_string),
                requested_revision: value.reference().as_str().map(ToString::to_string),
                git_lfs: value.lfs().enabled().then_some(true),
            },
        }
    }
}

impl Serialize for VcsUrl {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Strip credentials before removing the VCS prefix to retain the passwordless
        // `git` username for `git+https` URLs.
        let url = self.url.without_credentials();
        let mut state = serializer.serialize_struct("VcsUrl", 2)?;
        state.serialize_field("url", &url.as_str()[self.vcs_info.vcs.as_str().len() + 1..])?;
        state.serialize_field("vcs_info", &self.vcs_info)?;
        state.end()
    }
}

/// A failure to read the metadata schema or parse its URL.
#[derive(Debug, Error)]
pub enum DirectUrlError {
    /// The JSON cannot be read or does not match the direct URL metadata schema.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// The metadata matches the schema, but URL parsing failed.
    #[error(transparent)]
    Url(#[from] SafeUrlError),
}

/// Read `direct_url.json` and return its decoded [`DirectUrl`] and reconstructed [`SafeUrl`].
///
/// The URL can include a VCS prefix and revision, an archive hash, or a subdirectory fragment
/// derived from the metadata.
///
/// # Errors
///
/// Returns [`DirectUrlError::Json`] if reading fails, the JSON is malformed, or the metadata does
/// not match the expected schema. Returns [`DirectUrlError::Url`] if the metadata matches the
/// schema but its URL cannot be parsed.
///
/// Installed metadata from other tools may contain URLs that cannot be parsed, such as SCP-style
/// Git addresses. Distinguishing URL errors lets callers ignore an unusable origin without
/// ignoring malformed metadata or read errors.
pub fn read_direct_url_json(reader: impl Read) -> Result<(DirectUrl, SafeUrl), DirectUrlError> {
    let metadata: unparsed::DirectUrl = serde_json::from_reader(reader)?;
    let metadata = match metadata {
        unparsed::DirectUrl::LocalDirectory {
            url,
            dir_info,
            subdirectory,
        } => DirectUrl::LocalDirectory {
            url: SafeUrl::parse(&url)?.with_policy(Stripped),
            dir_info,
            subdirectory,
        },
        unparsed::DirectUrl::ArchiveUrl {
            url,
            archive_info,
            subdirectory,
        } => DirectUrl::ArchiveUrl {
            url: SafeUrl::parse(&url)?.with_policy(Stripped),
            archive_info,
            subdirectory,
        },
        unparsed::DirectUrl::VcsUrl {
            url,
            vcs_info,
            subdirectory,
            path,
        } => DirectUrl::VcsUrl {
            url: VcsUrl::parse(&url, vcs_info)?,
            subdirectory,
            path,
        },
    };
    let url = SafeUrl::from(&metadata);
    Ok((metadata, url))
}

mod unparsed {
    use std::fmt;
    use std::path::{Path, PathBuf};

    use serde::de::{IgnoredAny, MapAccess, Visitor};
    use serde::{Deserialize, Deserializer};

    use super::{ArchiveInfo, DirInfo, VcsInfo};

    /// Direct URL metadata with an unparsed URL, so schema and URL errors can be handled separately.
    pub(super) enum DirectUrl {
        LocalDirectory {
            url: String,
            dir_info: DirInfo,
            subdirectory: Option<Box<Path>>,
        },
        ArchiveUrl {
            url: String,
            archive_info: ArchiveInfo,
            subdirectory: Option<Box<Path>>,
        },
        VcsUrl {
            url: String,
            vcs_info: VcsInfo,
            subdirectory: Option<Box<Path>>,
            path: Option<PathBuf>,
        },
    }

    impl<'de> Deserialize<'de> for DirectUrl {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            #[derive(Deserialize)]
            #[serde(field_identifier, rename_all = "snake_case")]
            enum Field {
                Url,
                Subdirectory,
                DirInfo,
                ArchiveInfo,
                VcsInfo,
                Path,
                #[serde(other)]
                Other,
            }

            // A malformed nested field can belong to an unused alternative. Keep its parsing
            // failure local so another variant can still match. Buffering these fields also
            // keeps duplicate keys available to the derived struct deserializers.
            #[derive(Deserialize)]
            #[serde(untagged)]
            enum Parsed<T> {
                Valid(T),
                Invalid(IgnoredAny),
            }

            enum Value<T> {
                Missing,
                Present(Parsed<T>),
                Duplicate,
            }

            impl<T> Value<T> {
                fn read<'de, A>(&mut self, map: &mut A) -> Result<(), A::Error>
                where
                    A: MapAccess<'de>,
                    T: Deserialize<'de>,
                {
                    let value = map.next_value()?;
                    *self = match self {
                        Self::Missing => Self::Present(value),
                        Self::Present(_) | Self::Duplicate => Self::Duplicate,
                    };
                    Ok(())
                }

                fn required(self) -> Result<T, ()> {
                    match self {
                        Self::Present(Parsed::Valid(value)) => Ok(value),
                        Self::Missing | Self::Present(Parsed::Invalid(_)) | Self::Duplicate => {
                            Err(())
                        }
                    }
                }
            }

            impl<T> Value<Option<T>> {
                fn optional(self) -> Result<Option<T>, ()> {
                    match self {
                        Self::Missing => Ok(None),
                        Self::Present(Parsed::Valid(value)) => Ok(value),
                        Self::Present(Parsed::Invalid(_)) | Self::Duplicate => Err(()),
                    }
                }
            }

            struct DirectUrlVisitor;

            impl<'de> Visitor<'de> for DirectUrlVisitor {
                type Value = DirectUrl;

                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("direct URL metadata")
                }

                // Scalar URLs can contain credentials or signed query parameters.
                fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E>
                where
                    E: serde::de::Error,
                {
                    Err(E::custom("invalid direct URL metadata"))
                }

                fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
                where
                    A: MapAccess<'de>,
                {
                    let mut url = None;
                    let mut subdirectory = None;
                    let mut dir_info = Value::Missing;
                    let mut archive_info = Value::Missing;
                    let mut vcs_info = Value::Missing;
                    let mut path = Value::Missing;
                    while let Some(field) = map.next_key()? {
                        match field {
                            Field::Url => {
                                if url.is_some() {
                                    return Err(serde::de::Error::duplicate_field("url"));
                                }
                                url = Some(map.next_value()?);
                            }
                            Field::Subdirectory => {
                                if subdirectory.is_some() {
                                    return Err(serde::de::Error::duplicate_field("subdirectory"));
                                }
                                subdirectory = Some(map.next_value()?);
                            }
                            Field::DirInfo => dir_info.read(&mut map)?,
                            Field::ArchiveInfo => archive_info.read(&mut map)?,
                            Field::VcsInfo => vcs_info.read(&mut map)?,
                            Field::Path => path.read(&mut map)?,
                            Field::Other => {
                                map.next_value::<IgnoredAny>()?;
                            }
                        }
                    }
                    let url = url.ok_or_else(|| serde::de::Error::missing_field("url"))?;
                    let subdirectory = subdirectory.flatten();

                    // The first valid variant wins, including when another variant's nested
                    // metadata is malformed or contains duplicate fields.
                    if let Ok(dir_info) = dir_info.required() {
                        return Ok(DirectUrl::LocalDirectory {
                            url,
                            dir_info,
                            subdirectory,
                        });
                    }
                    if let Ok(archive_info) = archive_info.required() {
                        return Ok(DirectUrl::ArchiveUrl {
                            url,
                            archive_info,
                            subdirectory,
                        });
                    }
                    if let (Ok(vcs_info), Ok(path)) = (vcs_info.required(), path.optional()) {
                        return Ok(DirectUrl::VcsUrl {
                            url,
                            vcs_info,
                            subdirectory,
                            path,
                        });
                    }
                    Err(serde::de::Error::custom("invalid direct URL metadata"))
                }
            }

            deserializer.deserialize_any(DirectUrlVisitor)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DirInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ArchiveInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) hashes: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct VcsInfo {
    pub vcs: VcsKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_lfs: Option<bool>, // Prefix lfs with VcsKind::Git per PEP 610
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VcsKind {
    Git,
    Hg,
    Bzr,
    Svn,
}

impl VcsKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::Hg => "hg",
            Self::Bzr => "bzr",
            Self::Svn => "svn",
        }
    }
}

impl std::fmt::Display for VcsKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&DirectUrl> for SafeUrl {
    fn from(value: &DirectUrl) -> Self {
        match value {
            DirectUrl::LocalDirectory {
                url,
                subdirectory,
                dir_info: _,
            } => {
                let mut url = url.as_url().clone();
                if let Some(subdirectory) = subdirectory {
                    url.set_fragment(Some(&format!("subdirectory={}", subdirectory.display())));
                }
                url
            }
            DirectUrl::ArchiveUrl {
                url,
                subdirectory,
                archive_info,
            } => {
                let mut url = url.as_url().clone();
                let mut fragments = Vec::new();
                if let Some(subdirectory) = subdirectory {
                    fragments.push(format!("subdirectory={}", subdirectory.display()));
                }
                if let Some(hash) = archive_info
                    .hashes
                    .as_ref()
                    .and_then(|hashes| {
                        HashAlgorithm::preferred().find_map(|algorithm| {
                            hashes
                                .get(algorithm.as_str())
                                .map(|digest| format!("{algorithm}={digest}"))
                        })
                    })
                    .or_else(|| {
                        let hash = archive_info.hash.as_ref()?;
                        Hashes::parse_fragment(hash).is_ok().then(|| hash.clone())
                    })
                {
                    fragments.push(hash);
                }
                if !fragments.is_empty() {
                    url.set_fragment(Some(&fragments.join("&")));
                }
                url
            }
            DirectUrl::VcsUrl {
                url,
                subdirectory,
                path,
            } => {
                let vcs_info = url.vcs_info();
                let mut url = url.url().as_url().clone();
                if let Some(commit_id) = &vcs_info.commit_id {
                    let path = format!("{}@{commit_id}", url.path());
                    url.set_path(&path);
                } else if let Some(requested_revision) = &vcs_info.requested_revision {
                    let path = format!("{}@{requested_revision}", url.path());
                    url.set_path(&path);
                }
                let mut frags: Vec<String> = Vec::new();
                if let Some(subdirectory) = subdirectory {
                    frags.push(format!("subdirectory={}", subdirectory.display()));
                }
                // Displays nicely that lfs was used
                if let Some(true) = vcs_info.git_lfs {
                    frags.push("lfs=true".to_string());
                }
                if let Some(path) = path {
                    frags.push(format!("path={}", path.display()));
                }
                if !frags.is_empty() {
                    url.set_fragment(Some(&frags.join("&")));
                }
                url
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Read};

    use anyhow::Result;

    use super::{DirectUrl, DirectUrlError, VcsInfo, VcsKind, VcsUrl, read_direct_url_json};
    use uv_redacted::SafeUrl;

    #[test]
    fn direct_url_retains_read_error() -> Result<()> {
        struct FailedRead;

        impl Read for FailedRead {
            fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("metadata read failed"))
            }
        }

        let reader = br#"{"url":"https://example.com/repo","vcs_info":"#
            .as_slice()
            .chain(FailedRead);
        match read_direct_url_json(reader).unwrap_err() {
            DirectUrlError::Json(error) => assert!(error.is_io()),
            DirectUrlError::Url(error) => return Err(error.into()),
        }
        Ok(())
    }

    #[test]
    fn direct_url_scalar_metadata_error() -> Result<()> {
        let error = read_direct_url_json(
            br#""https://user:secret@example.com/project?token=secret""#.as_slice(),
        )
        .unwrap_err();
        match error {
            DirectUrlError::Json(error) => {
                insta::assert_snapshot!(error, @"invalid direct URL metadata at line 1 column 54");
            }
            DirectUrlError::Url(error) => return Err(error.into()),
        }
        Ok(())
    }

    #[test]
    fn direct_url_variant_precedence() -> Result<()> {
        let (metadata, _) = read_direct_url_json(
            br#"{"url":"file:///project","vcs_info":{"vcs":false},"archive_info":{},"dir_info":{},"path":42}"#.as_slice(),
        )?;
        insta::assert_json_snapshot!(metadata, @r#"
        {
          "url": "file:///project",
          "dir_info": {}
        }
        "#);
        Ok(())
    }

    #[test]
    fn direct_url_falls_back_after_duplicate_nested_field() -> Result<()> {
        let (metadata, _) = read_direct_url_json(
            br#"{"url":"https://example.com/project.zip","dir_info":{"editable":true,"editable":false},"archive_info":{}}"#.as_slice(),
        )?;
        insta::assert_json_snapshot!(metadata, @r#"
        {
          "url": "https://example.com/project.zip",
          "archive_info": {}
        }
        "#);
        Ok(())
    }

    #[test]
    fn direct_url_falls_back_after_duplicate_variant_field() -> Result<()> {
        let (metadata, _) = read_direct_url_json(
            br#"{"url":"https://example.com/project.zip","dir_info":{},"dir_info":{},"archive_info":{}}"#.as_slice(),
        )?;
        insta::assert_json_snapshot!(metadata, @r#"
        {
          "url": "https://example.com/project.zip",
          "archive_info": {}
        }
        "#);
        Ok(())
    }

    #[test]
    fn direct_url_rejects_sequence_representation() {
        assert!(read_direct_url_json(br#"["file:///project",{},null]"#.as_slice()).is_err());
    }

    #[test]
    fn archive_serialization_strips_userinfo_and_retains_query() -> Result<()> {
        let (metadata, _) = read_direct_url_json(
            br#"{"url":"https://user:password@example.com/project.whl?sig=signature&download=1","archive_info":{}}"#.as_slice(),
        )?;

        assert_eq!(SafeUrl::from(&metadata).password(), Some("password"));
        insta::assert_json_snapshot!(metadata, @r#"
        {
          "url": "https://example.com/project.whl?sig=signature&download=1",
          "archive_info": {}
        }
        "#);
        insta::assert_snapshot!(format!("{metadata:?}"), @r#"ArchiveUrl { url: SafeUrl { scheme: "https", cannot_be_a_base: false, username: "user", password: Some("****"), host: Some(Domain("example.com")), port: None, path: "/project.whl", query: Some("sig=****&download=1"), fragment: None }, archive_info: ArchiveInfo { hash: None, hashes: None }, subdirectory: None }"#);
        Ok(())
    }

    #[test]
    fn vcs_serialization_strips_userinfo_and_retains_query() -> Result<()> {
        let (metadata, _) = read_direct_url_json(
            br#"{"url":"https://user:password@example.com/project.git?sig=signature&download=1","vcs_info":{"vcs":"git","commit_id":"abcdef","requested_revision":"main","git_lfs":true},"subdirectory":"src","path":"dist/project.whl"}"#.as_slice(),
        )?;

        assert_eq!(SafeUrl::from(&metadata).password(), Some("password"));
        assert_eq!(
            SafeUrl::from(&metadata).as_str(),
            "git+https://user:password@example.com/project.git@abcdef?sig=signature&download=1#subdirectory=src&lfs=true&path=dist/project.whl"
        );
        insta::assert_json_snapshot!(metadata, @r#"
        {
          "url": "https://example.com/project.git?sig=signature&download=1",
          "vcs_info": {
            "vcs": "git",
            "commit_id": "abcdef",
            "requested_revision": "main",
            "git_lfs": true
          },
          "subdirectory": "src",
          "path": "dist/project.whl"
        }
        "#);
        Ok(())
    }

    #[test]
    fn vcs_serialization_retains_passwordless_git_username() -> Result<()> {
        let (metadata, _) = read_direct_url_json(
            br#"{"url":"https://git@example.com/project.git","vcs_info":{"vcs":"git"}}"#.as_slice(),
        )?;
        insta::assert_json_snapshot!(metadata, @r#"
        {
          "url": "https://git@example.com/project.git",
          "vcs_info": {
            "vcs": "git"
          }
        }
        "#);
        Ok(())
    }

    #[test]
    fn vcs_url_round_trip_retains_kind() -> Result<()> {
        let metadata = DirectUrl::VcsUrl {
            url: VcsUrl::parse(
                "https://example.com/repo",
                VcsInfo {
                    vcs: VcsKind::Hg,
                    commit_id: Some("abcdef".to_string()),
                    requested_revision: Some("default".to_string()),
                    git_lfs: None,
                },
            )?,
            subdirectory: None,
            path: None,
        };
        insta::assert_json_snapshot!(metadata, @r#"
        {
          "url": "https://example.com/repo",
          "vcs_info": {
            "vcs": "hg",
            "commit_id": "abcdef",
            "requested_revision": "default"
          }
        }
        "#);

        let json = serde_json::to_vec(&metadata)?;
        let (reconstructed, url) = read_direct_url_json(json.as_slice())?;
        assert_eq!(reconstructed, metadata);
        assert_eq!(url.as_str(), "hg+https://example.com/repo@abcdef");
        Ok(())
    }

    #[test]
    fn vcs_url_uses_prefixed_parsing_rules() -> Result<()> {
        for (transport, expected) in [
            (
                "http://EXAMPLE.com:80/repo",
                "git+http://EXAMPLE.com:80/repo",
            ),
            ("https:example.com", "git+https:example.com"),
            (":foo", "git+:foo"),
        ] {
            let json = serde_json::json!({"url": transport, "vcs_info": {"vcs": "git"}});
            let (metadata, _) = read_direct_url_json(json.to_string().as_bytes())?;
            assert_eq!(SafeUrl::from(&metadata).as_str(), expected);
            assert_eq!(serde_json::to_value(metadata)?, json);
        }
        Ok(())
    }
}
