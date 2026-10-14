use std::cmp::Reverse;
use std::sync::Arc;

use futures::{FutureExt, TryStreamExt, stream::FuturesUnordered};
use tracing::{debug, instrument};

use uv_configuration::BuildOptions;
use uv_distribution::DistributionDatabase;
use uv_distribution_filename::WheelFilename;
use uv_distribution_types::{
    BuildableSource, CachedDist, DerivationChain, Dist, DistErrorKind, DistributionId, Hashed,
    Identifier, Name, RemoteSource, Resolution,
};
use uv_normalize::PackageName;
use uv_platform_tags::Tags;
use uv_redacted::DisplaySafeUrl;
use uv_types::{BuildContext, HashStrategy, InFlight};

/// Prepare distributions for installation.
///
/// Downloads, builds, and unzips a set of distributions.
pub struct Preparer<'a, Context: BuildContext> {
    tags: &'a Tags,
    hashes: &'a HashStrategy,
    build_options: &'a BuildOptions,
    database: DistributionDatabase<'a, Context>,
    reporter: Option<Arc<dyn Reporter>>,
}

impl<'a, Context: BuildContext> Preparer<'a, Context> {
    pub fn new(
        tags: &'a Tags,
        hashes: &'a HashStrategy,
        build_options: &'a BuildOptions,
        database: DistributionDatabase<'a, Context>,
    ) -> Self {
        Self {
            tags,
            hashes,
            build_options,
            database,
            reporter: None,
        }
    }

    /// Set the [`Reporter`] to use for operations.
    #[must_use]
    pub fn with_reporter(self, reporter: Arc<dyn Reporter>) -> Self {
        Self {
            database: self
                .database
                .with_reporter(reporter.clone().into_distribution_reporter()),
            reporter: Some(reporter),
            ..self
        }
    }

    /// Download, build, and unzip a set of distributions.
    #[instrument(skip_all, fields(total = distributions.len()))]
    pub async fn prepare(
        &self,
        mut distributions: Vec<Arc<Dist>>,
        in_flight: &InFlight,
        resolution: &Resolution,
    ) -> Result<Vec<CachedDist>, Error> {
        // Sort the distributions by size.
        distributions
            .sort_unstable_by_key(|distribution| Reverse(distribution.size().unwrap_or(u64::MAX)));

        let wheels = distributions
            .into_iter()
            .map(async |dist| {
                let wheel = self
                    .get_wheel(Arc::unwrap_or_clone(dist), in_flight, resolution)
                    .boxed_local()
                    .await?;
                if let Some(reporter) = self.reporter.as_ref() {
                    reporter.on_progress(&wheel);
                }
                Ok::<CachedDist, Error>(wheel)
            })
            .collect::<FuturesUnordered<_>>()
            .try_collect()
            .await?;

        if let Some(reporter) = self.reporter.as_ref() {
            reporter.on_complete();
        }

        Ok(wheels)
    }

    /// Download, build, and unzip a single wheel.
    #[instrument(skip_all, fields(name = % dist, size = ? dist.size(), url = dist.file().map(| file | file.url.to_string()).unwrap_or_default()))]
    async fn get_wheel(
        &self,
        dist: Dist,
        in_flight: &InFlight,
        resolution: &Resolution,
    ) -> Result<CachedDist, Error> {
        // Validate that the distribution is compatible with the build options.
        match dist {
            Dist::Built(ref dist) => {
                if self.build_options.no_binary_package(dist.name()) {
                    return Err(Error::NoBinary(dist.name().clone()));
                }
            }
            Dist::Source(ref dist) => {
                if self.build_options.no_build_package(dist.name()) {
                    if dist.is_editable() || dist.is_first_party() {
                        debug!(
                            "Allowing build for first-party or editable source distribution: {dist}"
                        );
                    } else {
                        return Err(Error::NoBuild(dist.name().clone()));
                    }
                }
            }
        }

        let id = dist.distribution_id();
        let download = match in_flight.downloads.register_or_wait(&id).await {
            Some(result) => {
                let cached = result.map_err(Error::Thread)?;
                validate_in_flight_wheel(&dist, cached.filename())
                    .map_err(|err| Error::from_dist(dist, err, resolution))?;
                return Ok(cached);
            }
            None => PendingDownload::new(in_flight, id),
        };

        let policy = self.hashes.archive_policy(&dist);
        let result = self
            .database
            .get_or_build_wheel(&dist, self.tags, policy)
            .boxed_local()
            .await
            .and_then(|wheel| {
                if wheel.satisfies(policy) {
                    Ok(CachedDist::from(wheel))
                } else {
                    Err(uv_distribution::Error::hash_mismatch(
                        dist.to_string(),
                        policy.digests(),
                        wheel.hashes(),
                    ))
                }
            })
            .map_err(|err| Error::from_dist(dist, err, resolution));

        download.complete(result.as_ref().cloned().map_err(ToString::to_string));
        result
    }
}

/// Publish a result for a registered download, including when its owner is cancelled.
struct PendingDownload<'a> {
    in_flight: &'a InFlight,
    id: Option<DistributionId>,
}

impl<'a> PendingDownload<'a> {
    fn new(in_flight: &'a InFlight, id: DistributionId) -> Self {
        Self {
            in_flight,
            id: Some(id),
        }
    }

    fn complete(mut self, result: Result<CachedDist, String>) {
        let id = self.id.take().expect("download has not been completed");
        self.in_flight.downloads.done(id, result);
    }
}

impl Drop for PendingDownload<'_> {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            // Waiters require a terminal result; removing a pending registration cannot wake them.
            self.in_flight.downloads.done(
                id,
                Err("Distribution preparation was cancelled".to_string()),
            );
        }
    }
}

/// Validate that a wheel shared by an in-flight download matches the requested distribution.
///
/// The wheel matches the distribution that started the download, but other requests can share the
/// same in-flight entry, such as requirements using the same local path with different names.
fn validate_in_flight_wheel(
    dist: &Dist,
    filename: &WheelFilename,
) -> Result<(), uv_distribution::Error> {
    if *dist.name() != filename.name {
        return Err(uv_distribution::Error::WheelMetadataNameMismatch {
            given: dist.name().clone(),
            metadata: filename.name.clone(),
        });
    }
    if let Some(version) = dist.version()
        && !filename.matches_version(version)
    {
        return Err(uv_distribution::Error::WheelMetadataVersionMismatch {
            given: version.clone(),
            metadata: filename.version.clone(),
        });
    }
    Ok(())
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Building source distributions is disabled, but attempted to build `{0}`")]
    NoBuild(PackageName),
    #[error("Using pre-built wheels is disabled, but attempted to use `{0}`")]
    NoBinary(PackageName),
    #[error("{0} `{1}`")]
    Dist(
        DistErrorKind,
        Box<Dist>,
        DerivationChain,
        #[source] Box<uv_distribution::Error>,
    ),
    #[error("Cyclic build dependency detected for `{0}`")]
    CyclicBuildDependency(PackageName),
    #[error("Unzip failed in another thread: {0}")]
    Thread(String),
}

impl Error {
    /// Return whether this is an expected user-facing failure.
    pub fn is_user_failure(&self) -> bool {
        match self {
            Self::NoBuild(_) | Self::NoBinary(_) | Self::CyclicBuildDependency(_) => true,
            Self::Dist(_, _, _, error) => error.is_user_failure(),
            Self::Thread(_) => false,
        }
    }

    /// Create an [`Error`] from a distribution error.
    fn from_dist(dist: Dist, err: uv_distribution::Error, resolution: &Resolution) -> Self {
        let chain =
            DerivationChain::from_resolution(resolution, (&dist).into()).unwrap_or_default();
        Self::Dist(
            DistErrorKind::from_dist(&dist, &err),
            Box::new(dist),
            chain,
            Box::new(err),
        )
    }
}

pub trait Reporter: Send + Sync {
    /// Callback to invoke when a wheel is unzipped. This implies that the wheel was downloaded and,
    /// if necessary, built.
    fn on_progress(&self, dist: &CachedDist);

    /// Callback to invoke when the operation is complete.
    fn on_complete(&self);

    /// Callback to invoke when a download is kicked off.
    fn on_download_start(&self, name: &PackageName, size: Option<u64>) -> usize;

    /// Callback to invoke when a download makes progress (i.e. some number of bytes are
    /// downloaded).
    fn on_download_progress(&self, index: usize, bytes: u64);

    /// Callback to invoke when a download is complete.
    fn on_download_complete(&self, name: &PackageName, index: usize);

    /// Callback to invoke when a source distribution build is kicked off.
    fn on_build_start(&self, source: &BuildableSource) -> usize;

    /// Callback to invoke when a source distribution build is complete.
    fn on_build_complete(&self, source: &BuildableSource, id: usize);

    /// Callback to invoke when a repository checkout begins.
    fn on_checkout_start(&self, url: &DisplaySafeUrl, rev: &str) -> usize;

    /// Callback to invoke when a repository checkout completes.
    fn on_checkout_complete(&self, url: &DisplaySafeUrl, rev: &str, index: usize);
}

impl dyn Reporter {
    /// Converts this reporter to a [`uv_distribution::Reporter`].
    fn into_distribution_reporter(self: Arc<dyn Reporter>) -> Arc<dyn uv_distribution::Reporter> {
        Arc::new(Facade { reporter: self })
    }
}

/// A facade for converting from [`Reporter`] to [`uv_distribution::Reporter`].
struct Facade {
    reporter: Arc<dyn Reporter>,
}

impl uv_distribution::Reporter for Facade {
    fn on_build_start(&self, source: &BuildableSource) -> usize {
        self.reporter.on_build_start(source)
    }

    fn on_build_complete(&self, source: &BuildableSource, id: usize) {
        self.reporter.on_build_complete(source, id);
    }

    fn on_checkout_start(&self, url: &DisplaySafeUrl, rev: &str) -> usize {
        self.reporter.on_checkout_start(url, rev)
    }

    fn on_checkout_complete(&self, url: &DisplaySafeUrl, rev: &str, index: usize) {
        self.reporter.on_checkout_complete(url, rev, index);
    }

    fn on_download_start(&self, name: &PackageName, size: Option<u64>) -> usize {
        self.reporter.on_download_start(name, size)
    }

    fn on_download_progress(&self, index: usize, inc: u64) {
        self.reporter.on_download_progress(index, inc);
    }

    fn on_download_complete(&self, name: &PackageName, index: usize) {
        self.reporter.on_download_complete(name, index);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use uv_distribution_types::DistributionId;
    use uv_types::InFlight;

    use super::PendingDownload;

    #[tokio::test]
    async fn cancelled_download_wakes_waiters() {
        let in_flight = InFlight::default();
        let id = DistributionId::PathBuf("example.whl".into());
        assert!(in_flight.downloads.register_or_wait(&id).await.is_none());
        let download = PendingDownload::new(&in_flight, id.clone());

        let (registration, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(biased; in_flight.downloads.register_or_wait(&id), async {
                drop(download);
            })
        })
        .await
        .expect("cancelled download left a waiter blocked");

        assert!(matches!(
            registration,
            Some(Err(message))
                if message == "Distribution preparation was cancelled"
        ));
    }
}
