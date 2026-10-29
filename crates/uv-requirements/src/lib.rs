pub use crate::extras::*;
pub use crate::lookahead::*;
pub use crate::script::{
    ScriptRequirementsError, script_extra_build_requires, script_specification,
};
pub use crate::source_tree::*;
pub use crate::sources::*;
pub use crate::specification::*;
pub use crate::unnamed::*;
pub use uv_configuration::RequirementsInput;

use uv_distribution::check_supported_wheel_variant;
use uv_distribution_types::{Dist, DistErrorKind, Requirement};

mod extras;
mod lookahead;
mod script;
mod source_tree;
mod sources;
mod specification;
mod unnamed;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} `{1}`")]
    Dist(
        DistErrorKind,
        Box<Dist>,
        #[source] Box<uv_distribution::Error>,
    ),

    #[error(transparent)]
    Distribution(#[from] Box<uv_distribution::Error>),

    #[error(transparent)]
    DistributionTypes(#[from] uv_distribution_types::Error),

    #[error(transparent)]
    HashStrategy(#[from] uv_types::HashStrategyError),

    #[error(transparent)]
    FlatIndex(#[from] Box<uv_client::FlatIndexError>),

    #[error(transparent)]
    WheelFilename(#[from] uv_distribution_filename::WheelFilenameError),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    /// Return whether this is an expected user-facing failure.
    pub fn is_user_failure(&self) -> bool {
        match self {
            Self::Dist(_, _, error) | Self::Distribution(error) => error.is_user_failure(),
            Self::FlatIndex(error) => error.is_user_failure(),
            Self::DistributionTypes(_) | Self::HashStrategy(_) | Self::WheelFilename(_) => true,
            Self::Io(error) => matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::InvalidInput
            ),
        }
    }

    /// Create an [`Error`] from a distribution error.
    fn from_dist(dist: Dist, err: uv_distribution::Error) -> Self {
        Self::Dist(
            DistErrorKind::from_dist(&dist, &err),
            Box::new(dist),
            Box::new(err),
        )
    }
}

/// Convert a [`Requirement`] into a [`Dist`], if it is a direct URL.
pub(crate) fn required_dist(requirement: &Requirement) -> Result<Option<Dist>, Error> {
    let Some(url) = requirement.source.to_verbatim_parsed_url() else {
        return Ok(None);
    };
    let dist = Dist::from_url(requirement.name.clone(), url)?;

    // A direct requirement has no index from which to resolve a wheel variant.
    if let Dist::Built(built) = &dist
        && let Err(err) = check_supported_wheel_variant(built)
    {
        return Err(Error::from_dist(dist, err));
    }
    Ok(Some(dist))
}
