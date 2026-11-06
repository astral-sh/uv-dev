use tokio::sync::Semaphore;
use tracing::debug;

use uv_client::{MetadataFormat, RegistryClient};
use uv_distribution_filename::DistFilename;
use uv_distribution_types::{
    File, IndexCapabilities, IndexEntryFilename, IndexLocations, IndexMetadataRef, IndexUrl,
    RequiresPython,
};
use uv_normalize::PackageName;
use uv_platform_tags::Tags;
use uv_resolver::{ExcludeNewer, Prerelease, PrereleaseMode};
use uv_warnings::warn_user_once;

/// A client to fetch the latest version of a package from an index.
///
/// The returned distribution is guaranteed to be compatible with the provided tags and Python
/// requirement (if specified).
#[derive(Debug, Clone)]
pub struct LatestClient<'env> {
    pub client: &'env RegistryClient,
    pub capabilities: &'env IndexCapabilities,
    pub prerelease: &'env Prerelease,
    pub exclude_newer: &'env ExcludeNewer,
    pub index_locations: &'env IndexLocations,
    pub tags: Option<&'env Tags>,
    pub requires_python: Option<&'env RequiresPython>,
}

struct LatestCandidate {
    filename: Option<DistFilename>,
    prerelease: PrereleaseMode,
    tags: Option<Tags>,
    requires_python: Option<RequiresPython>,
}

impl LatestCandidate {
    fn consider(
        &mut self,
        candidate: DistFilename,
        file: &File,
        exclude_newer: Option<&jiff::Timestamp>,
    ) {
        // Respect any exclude-newer cutoffs that were provided.
        if let Some(exclude_newer) = exclude_newer {
            match file.upload_time_utc_ms.as_ref() {
                Some(&upload_time) if upload_time >= exclude_newer.as_millisecond() => {
                    return;
                }
                None => {
                    warn_user_once!(
                        "`{}` is missing an upload date, but user provided: {}",
                        file.filename,
                        exclude_newer
                    );
                }
                _ => {}
            }
        }

        // Unless explicitly allowed, skip pre-release artifacts.
        if !candidate.version().is_stable() && !matches!(self.prerelease, PrereleaseMode::Allow) {
            return;
        }

        // Avoid yanked or otherwise withdrawn files.
        if file
            .yanked
            .as_ref()
            .is_some_and(|yanked| yanked.is_yanked())
        {
            return;
        }

        // Enforce the interpreter's `Requires-Python` constraints.
        if let Some(requires_python) = &self.requires_python
            && file
                .requires_python
                .as_ref()
                .is_some_and(|file_requires_python| {
                    !requires_python.is_contained_by(file_requires_python)
                })
        {
            return;
        }

        // Skip wheels that aren't compatible with the current platform.
        // TODO(konsti): Check variant compatibility when looking for updates.
        if let DistFilename::WheelFilename(filename) = &candidate
            && (filename.variant().is_some()
                || self
                    .tags
                    .as_ref()
                    .is_some_and(|tags| !filename.compatibility(tags).is_compatible()))
        {
            return;
        }

        // Prefer higher versions, and prefer wheels over sdists at parity.
        if self.filename.as_ref().is_none_or(|current| {
            candidate.version() > current.version()
                || (candidate.version() == current.version()
                    && matches!(candidate, DistFilename::WheelFilename(_))
                    && matches!(current, DistFilename::SourceDistFilename(_)))
        }) {
            self.filename = Some(candidate);
        }
    }
}

impl LatestClient<'_> {
    fn effective_exclude_newer(
        &self,
        package: &PackageName,
        index: &IndexUrl,
    ) -> Option<jiff::Timestamp> {
        self.exclude_newer.exclude_newer_package_for_index(
            package,
            self.index_locations.exclude_newer_for(index, package),
        )
    }

    /// Find the latest version of a package from an index.
    pub async fn find_latest(
        &self,
        package: &PackageName,
        index: Option<&IndexUrl>,
        download_concurrency: &Semaphore,
    ) -> Result<Option<DistFilename>, uv_client::Error> {
        debug!("Fetching latest version of: `{package}`");

        let mut latest = LatestCandidate {
            filename: None,
            prerelease: self.prerelease.mode(package),
            tags: self.tags.cloned(),
            requires_python: self.requires_python.cloned(),
        };

        let archives = match self
            .client
            .simple_detail(
                package,
                index.map(IndexMetadataRef::from),
                self.capabilities,
                download_concurrency,
            )
            .await
        {
            Ok(archives) => archives,
            Err(err)
                if matches!(
                    err.kind(),
                    uv_client::ErrorKind::RemotePackageNotFound(_)
                        | uv_client::ErrorKind::NoIndex(_)
                        | uv_client::ErrorKind::Offline(_)
                ) =>
            {
                Vec::new()
            }
            Err(err) => return Err(err),
        };

        for (index, archive) in archives {
            let exclude_newer = self.effective_exclude_newer(package, index);

            match archive {
                MetadataFormat::Simple(archive) => {
                    let package = package.clone();
                    latest = self
                        .client
                        .fold_version_files(archive, latest, move |latest, files| {
                            for (filename, file) in files.all(&package) {
                                latest.consider(filename, &file, exclude_newer.as_ref());
                            }
                        })
                        .await?;
                }
                MetadataFormat::Flat(entries) => {
                    for entry in entries {
                        let (filename, file, _) = entry.into_parts();
                        let filename = match filename {
                            IndexEntryFilename::DistFilename(filename) => filename,
                            IndexEntryFilename::VariantJson(_) => continue,
                        };
                        latest.consider(filename, &file, exclude_newer.as_ref());
                    }
                }
            }
        }

        if index.is_none() {
            for entry in self
                .client
                .find_links_entries(package, download_concurrency)
                .await?
            {
                let (filename, file, index) = entry.into_parts();
                let filename = match filename {
                    IndexEntryFilename::DistFilename(filename) => filename,
                    IndexEntryFilename::VariantJson(_) => continue,
                };
                let exclude_newer = self.effective_exclude_newer(package, &index);
                latest.consider(filename, &file, exclude_newer.as_ref());
            }
        }

        Ok(latest.filename)
    }
}
