use std::path::Path;
use std::str::FromStr;

use uv_distribution_types::{Index, IndexName, IndexSourceError, IndexUrl, Origin};
use uv_pep508::VerbatimUrl;
use uv_preview::PreviewFeature;
use uv_warnings::warn_user_once;

use crate::Error;

/// An unresolved index passed by the user by its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedIndex {
    name: IndexName,
    default: bool,
}

impl UnresolvedIndex {
    /// Resolve an index name against the effective filesystem configuration.
    fn resolve(self, indexes: &[Index], preview_enabled: bool) -> Result<Index, Error> {
        let Self { name, default } = self;
        let path_exists = Path::new(name.as_ref()).exists();

        // Outside preview, an existing path retains its current interpretation.
        if preview_enabled || !path_exists {
            if let Some(index) = indexes
                .iter()
                .find(|index| index.name.as_ref() == Some(&name))
            {
                if !preview_enabled {
                    warn_user_once!(
                        "Referencing an index by name is experimental and may change without warning. Pass `--preview-features {}` to disable this warning.",
                        PreviewFeature::IndexByName
                    );
                }

                let mut index = index.clone();
                // Keep relative paths anchored to their configuration file without marking them
                // as absolute when CLI settings are rebased or written back to a project.
                if let IndexUrl::Path(url) = index.url()
                    && url.prefers_relative()
                {
                    index.url = IndexUrl::from(VerbatimUrl::from_url(index.raw_url().clone()));
                }

                return Ok(Index {
                    default,
                    explicit: false,
                    origin: Some(Origin::Cli),
                    ..index
                });
            }

            if preview_enabled && !path_exists {
                return Err(Error::UnknownIndex(name));
            }
        }

        Ok(Index {
            default,
            origin: Some(Origin::Cli),
            ..Index::from_str(name.as_ref())?
        })
    }
}

/// A potentially unresolved index.
#[expect(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexArg {
    /// A usable index with a URL.
    Resolved(Index),
    /// An unresolved index specification.
    Unresolved(UnresolvedIndex),
}

impl IndexArg {
    fn new(value: &str, default: bool) -> Result<Self, IndexSourceError> {
        if let Ok(name) = IndexName::from_str(value) {
            return Ok(Self::Unresolved(UnresolvedIndex { name, default }));
        }

        let index = Index::from_str(value)?;
        Ok(Self::Resolved(Index {
            default,
            origin: Some(Origin::Cli),
            ..index
        }))
    }

    /// Parse an index passed via `--index`.
    pub fn from_index(value: &str) -> Result<Self, IndexSourceError> {
        Self::new(value, false)
    }

    /// Parse an index passed via `--default-index`.
    pub fn from_default_index(value: &str) -> Result<Self, IndexSourceError> {
        Self::new(value, true)
    }

    /// Resolve the argument against indexes from the effective configuration.
    pub fn resolve(self, indexes: &[Index]) -> Result<Index, Error> {
        let index = match self {
            Self::Resolved(index) => index,
            Self::Unresolved(index) => {
                index.resolve(indexes, uv_preview::is_enabled(PreviewFeature::IndexByName))?
            }
        };

        index.url().warn_on_disambiguated_relative_path();

        Ok(index)
    }
}
