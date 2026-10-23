use std::borrow::Cow;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uv_platform::Platform;

use crate::{
    ImplementationName, LenientImplementationName, PythonInstallationKey, PythonVariant,
    PythonVersion,
};

/// The identifying fields of an entry in the Python download catalog.
///
/// The catalog uses platform spellings such as `darwin` and `riscv64` that differ from
/// installation keys.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PythonDownloadKey {
    name: String,
    arch: DownloadArchitecture,
    os: String,
    libc: String,
    major: u8,
    minor: u8,
    patch: u8,
    prerelease: Option<String>,
    variant: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct DownloadArchitecture {
    family: String,
    variant: Option<String>,
}

#[derive(Debug, Error)]
pub enum PythonDownloadKeyError {
    #[error(transparent)]
    Platform(#[from] uv_platform::Error),
    #[error("Unknown Python variant `{0}`")]
    Variant(String),
    #[error("{0}")]
    Version(String),
}

impl PythonDownloadKey {
    /// Convert catalog fields to the key used to select and install a Python download.
    pub fn installation_key(&self) -> Result<PythonInstallationKey, PythonDownloadKeyError> {
        let implementation = match self.name.as_str() {
            "cpython" => LenientImplementationName::Known(ImplementationName::CPython),
            "pypy" => LenientImplementationName::Known(ImplementationName::PyPy),
            "graalpy" => LenientImplementationName::Known(ImplementationName::GraalPy),
            _ => LenientImplementationName::Unknown(self.name.clone()),
        };

        let family = match self.arch.family.as_str() {
            "armv5tel" => "armv5te",
            // The catalog's `riscv64` builds include the common `gc` instruction set.
            "riscv64" => "riscv64gc",
            family => family,
        };
        let arch = if let Some(variant) = &self.arch.variant {
            Cow::Owned(format!("{family}_{variant}"))
        } else {
            Cow::Borrowed(family)
        };
        let platform = Platform::from_parts(&self.os, &arch, &self.libc)?;
        let variant = self
            .variant
            .as_deref()
            .map_or(Ok(PythonVariant::default()), |variant| {
                PythonVariant::from_str(variant)
                    .map_err(|()| PythonDownloadKeyError::Variant(variant.to_owned()))
            })?;
        let version = PythonVersion::from_str(&format!(
            "{}.{}.{}{}",
            self.major,
            self.minor,
            self.patch,
            self.prerelease.as_deref().unwrap_or_default(),
        ))
        .map_err(PythonDownloadKeyError::Version)?;

        Ok(PythonInstallationKey::new_from_version(
            implementation,
            &version,
            platform,
            variant,
        ))
    }
}
