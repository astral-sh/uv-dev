use std::path::Path;

use uv_cache_info::CacheInfo;
use uv_distribution_filename::{VariantLabel, WheelFilename};
use uv_distribution_types::{
    BuildInfo, BuiltDist, CachedDist, Dist, Hashed, InstalledDist, InstalledDistKind,
    VariantsJsonContent,
};
use uv_metadata::{
    find_flat_dist_info, read_archive_wheel_variant, read_flat_wheel_metadata, read_wheel_variant,
};
use uv_preview::PreviewFeature;
use uv_pypi_types::{DirectUrl, HashDigest, HashDigests, ResolutionMetadata};

use crate::{Error, supported_variant_properties};

/// Allow variant wheels from an index or local path when the wheel variants preview is enabled.
///
/// Variant compatibility is checked separately during resolution and installation.
///
/// TODO: Support variant wheels from direct URLs and Git paths.
pub fn check_supported_wheel_variant(dist: &BuiltDist) -> Result<(), Error> {
    let filename = dist.wheel_filename();
    if enabled_variant_label(filename)?.is_some() {
        let supported_source = match dist {
            BuiltDist::Registry(_) | BuiltDist::Path(_) => true,
            BuiltDist::DirectUrl(_) | BuiltDist::GitPath(_) => false,
        };
        if !supported_source {
            return Err(Error::UnsupportedWheelVariant(Box::new(filename.clone())));
        }
    }
    Ok(())
}

/// Check that an extracted wheel's variant properties are supported by the target system.
pub fn check_wheel_variant_compatibility(
    filename: &WheelFilename,
    archive: &Path,
) -> Result<(), Error> {
    let Some(label) = enabled_variant_label(filename)? else {
        return Ok(());
    };

    let dist_info = find_flat_dist_info(filename, archive)
        .map_err(|err| Error::WheelMetadata(archive.to_path_buf(), Box::new(err)))?;
    let path = archive.join(format!("{dist_info}.dist-info/variant.json"));
    check_variant_file(label, &path)
}

/// Check that an installed local wheel's variant properties are supported by the target system.
pub fn check_installed_local_wheel_variant_compatibility(
    dist: &InstalledDist,
) -> Result<(), Error> {
    let InstalledDistKind::Url(installed) = &dist.kind else {
        return Ok(());
    };
    let DirectUrl::ArchiveUrl { url, .. } = installed.direct_url.as_ref() else {
        return Ok(());
    };
    let Ok(source_path) = url.to_file_path() else {
        return Ok(());
    };
    let Some(filename) = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.parse::<WheelFilename>().ok())
    else {
        return Ok(());
    };
    let Some(label) = filename.variant() else {
        return Ok(());
    };
    check_variant_file(label, &installed.path.join("variant.json"))
}

/// Check that a wheel archive's variant properties are supported by the target system.
pub fn check_archive_wheel_variant_compatibility(
    filename: &WheelFilename,
    archive: &Path,
) -> Result<(), Error> {
    let Some(label) = enabled_variant_label(filename)? else {
        return Ok(());
    };
    let metadata = read_archive_wheel_variant(filename, archive, label)
        .map_err(|err| Error::WheelMetadata(archive.to_path_buf(), Box::new(err)))?;
    check_variant_properties(label, &metadata)
}

fn enabled_variant_label(filename: &WheelFilename) -> Result<Option<&VariantLabel>, Error> {
    let Some(label) = filename.variant() else {
        return Ok(None);
    };
    if !uv_preview::is_enabled(PreviewFeature::WheelVariants) {
        return Err(Error::UnsupportedWheelVariant(Box::new(filename.clone())));
    }
    Ok(Some(label))
}

fn check_variant_file(label: &VariantLabel, path: &Path) -> Result<(), Error> {
    let metadata = read_wheel_variant(path, label)
        .map_err(|err| Error::WheelMetadata(path.to_path_buf(), Box::new(err)))?;
    check_variant_properties(label, &metadata)
}

fn check_variant_properties(
    label: &VariantLabel,
    metadata: &VariantsJsonContent,
) -> Result<(), Error> {
    if let Some(properties) =
        metadata.unsupported_variant_properties(label, &supported_variant_properties(metadata))
    {
        return Err(Error::IncompatibleWheelVariant {
            label: label.clone(),
            properties,
        });
    }
    Ok(())
}

/// A locally available wheel.
#[derive(Debug, Clone)]
pub struct LocalWheel {
    /// The remote distribution from which this wheel was downloaded.
    pub(crate) dist: Dist,
    /// The parsed filename.
    pub(crate) filename: WheelFilename,
    /// The canonicalized path in the cache directory to which the wheel was downloaded.
    /// Typically, a directory within the archive bucket.
    pub(crate) archive: Box<Path>,
    /// The cache info of the wheel.
    pub(crate) cache: CacheInfo,
    /// The build info, if available.
    pub(crate) build: Option<BuildInfo>,
    /// The computed hashes of the wheel.
    pub(crate) hashes: HashDigests,
}

impl LocalWheel {
    /// Return the [`Dist`] from which this wheel was downloaded.
    fn remote(&self) -> &Dist {
        &self.dist
    }

    /// Read the [`ResolutionMetadata`] from a wheel.
    pub(crate) fn metadata(&self) -> Result<ResolutionMetadata, Error> {
        read_flat_wheel_metadata(&self.filename, &self.archive)
            .map_err(|err| Error::WheelMetadata(self.archive.to_path_buf(), Box::new(err)))
    }
}

impl Hashed for LocalWheel {
    fn hashes(&self) -> &[HashDigest] {
        self.hashes.as_slice()
    }
}

/// Convert a [`LocalWheel`] into a [`CachedDist`].
impl From<LocalWheel> for CachedDist {
    fn from(wheel: LocalWheel) -> Self {
        Self::from_remote(
            wheel.dist,
            wheel.filename,
            wheel.hashes,
            wheel.cache,
            wheel.build,
            wheel.archive,
        )
    }
}

impl std::fmt::Display for LocalWheel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.remote())
    }
}
