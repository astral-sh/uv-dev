use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::{FutureExt, StreamExt};
use reqwest::Response;
use tokio::sync::{AcquireError, Semaphore};
use tokio::task::JoinError;
use tracing::{Instrument, Span, debug, info_span, warn};
use url::Url;

use uv_cache::{Cache, CacheBucket};
use uv_cache_key::cache_digest;
use uv_distribution_filename::DistFilename;
use uv_distribution_types::{File, FileLocation, IndexUrl, UrlString};
use uv_pypi_types::HashDigests;
use uv_redacted::DisplaySafeUrl;
use uv_small_str::SmallString;

use crate::cached_client::{CacheControl, CachedClientError};
use crate::html::SimpleDetailHTML;
use crate::{CachedClient, Connectivity, Error, ErrorKind, OwnedArchive, RetryState};

#[derive(Debug, thiserror::Error)]
pub enum FlatIndexError {
    #[error("Expected a file URL, but received: {0}")]
    NonFileUrl(DisplaySafeUrl),

    #[error("Failed to read `--find-links` directory: {0}")]
    FindLinksDirectory(PathBuf, #[source] FindLinksDirectoryError),

    #[error("Failed to read `--find-links` file: {0}")]
    FindLinksFile(PathBuf, #[source] Error),

    #[error("Failed to read `--find-links` URL: {0}")]
    FindLinksUrl(DisplaySafeUrl, #[source] Error),
}

impl FlatIndexError {
    /// Return whether this is an expected user-facing failure.
    pub fn is_user_failure(&self) -> bool {
        match self {
            Self::NonFileUrl(_) => true,
            Self::FindLinksFile(_, error) | Self::FindLinksUrl(_, error) => error.is_user_failure(),
            Self::FindLinksDirectory(..) => false,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FindLinksDirectoryError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    VerbatimUrl(#[from] uv_pep508::VerbatimUrlError),
    #[error(transparent)]
    Admission(#[from] AcquireError),
    #[error(transparent)]
    Worker(#[from] JoinError),
}

/// An entry in a `--find-links` index.
#[derive(Debug, Clone)]
pub struct FlatIndexEntry {
    filename: DistFilename,
    file: File,
    index: IndexUrl,
}

impl FlatIndexEntry {
    /// Return the distribution filename.
    pub fn filename(&self) -> &DistFilename {
        &self.filename
    }

    /// Convert the entry into its component parts.
    pub fn into_parts(self) -> (DistFilename, File, IndexUrl) {
        (self.filename, self.file, self.index)
    }
}

#[derive(Debug, Default, Clone)]
pub struct FlatIndexEntries {
    /// The list of `--find-links` entries.
    entries: Vec<FlatIndexEntry>,
    /// Whether any `--find-links` entries could not be resolved due to a lack of network
    /// connectivity.
    offline: bool,
}

impl FlatIndexEntries {
    /// Convert the entries into their component parts.
    pub fn into_parts(self) -> (Vec<FlatIndexEntry>, bool) {
        (self.entries, self.offline)
    }

    /// Create a [`FlatIndexEntries`] from a list of `--find-links` entries.
    fn from_entries(entries: Vec<FlatIndexEntry>) -> Self {
        Self {
            entries,
            offline: false,
        }
    }

    /// Create a [`FlatIndexEntries`] to represent an offline `--find-links` entry.
    fn offline() -> Self {
        Self {
            entries: Vec::new(),
            offline: true,
        }
    }

    /// Extend this list of `--find-links` entries with another list.
    fn extend(&mut self, other: Self) {
        self.entries.extend(other.entries);
        self.offline |= other.offline;
    }

    /// Return the number of `--find-links` entries.
    fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return `true` if there are no `--find-links` entries.
    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A client for reading distributions from `--find-links` entries (local directories or local and
/// remote HTML indexes).
#[derive(Debug, Clone)]
pub struct FlatIndexClient<'a> {
    client: &'a CachedClient,
    connectivity: Connectivity,
    cache: &'a Cache,
    directory_concurrency: Arc<Semaphore>,
}

impl<'a> FlatIndexClient<'a> {
    /// Create a new [`FlatIndexClient`].
    pub fn new(client: &'a CachedClient, connectivity: Connectivity, cache: &'a Cache) -> Self {
        Self::new_with_directory_concurrency(
            client,
            connectivity,
            cache,
            Arc::new(Semaphore::new(16)),
        )
    }

    /// Share directory admission across independently cached index lookups.
    pub(crate) fn new_with_directory_concurrency(
        client: &'a CachedClient,
        connectivity: Connectivity,
        cache: &'a Cache,
        directory_concurrency: Arc<Semaphore>,
    ) -> Self {
        Self {
            client,
            connectivity,
            cache,
            directory_concurrency,
        }
    }

    /// Read the directories and flat remote indexes from `--find-links`.
    pub async fn fetch_all(
        &self,
        indexes: impl Iterator<Item = &IndexUrl>,
    ) -> Result<FlatIndexEntries, FlatIndexError> {
        let mut fetches = futures::stream::iter(indexes)
            .map(async |index| {
                let entries = self.fetch_index(index).await?;
                if entries.is_empty() {
                    warn!("No packages found in `--find-links` entry: {}", index);
                } else {
                    debug!(
                        "Found {} package{} in `--find-links` entry: {}",
                        entries.len(),
                        if entries.len() == 1 { "" } else { "s" },
                        index
                    );
                }
                Ok::<FlatIndexEntries, FlatIndexError>(entries)
            })
            .buffered(16);

        let mut results = FlatIndexEntries::default();
        while let Some(entries) = fetches.next().await.transpose()? {
            results.extend(entries);
        }
        results
            .entries
            .sort_by(|a, b| a.filename.cmp(&b.filename).then(a.index.cmp(&b.index)));
        Ok(results)
    }

    /// Fetch a flat remote index from a `--find-links` URL.
    pub(crate) async fn fetch_index(
        &self,
        index: &IndexUrl,
    ) -> Result<FlatIndexEntries, FlatIndexError> {
        match index {
            IndexUrl::Path(url) => {
                let path = url
                    .to_file_path()
                    .map_err(|()| FlatIndexError::NonFileUrl(url.to_url()))?;
                match self
                    .read_local_index(path.clone(), index.clone())
                    .await
                    .map_err(|err| FlatIndexError::FindLinksDirectory(path.clone(), err))?
                {
                    Some(entries) => Ok(entries),
                    None => self
                        .read_from_file(&path, index)
                        .await
                        .map_err(|err| FlatIndexError::FindLinksFile(path.clone(), err)),
                }
            }
            IndexUrl::Pypi(url) | IndexUrl::Url(url) => self
                .read_from_url(url, index)
                .await
                .map_err(|err| FlatIndexError::FindLinksUrl(url.to_url(), err)),
        }
    }

    /// Read a flat remote index from a `--find-links` URL.
    async fn read_from_url(
        &self,
        url: &DisplaySafeUrl,
        flat_index: &IndexUrl,
    ) -> Result<FlatIndexEntries, Error> {
        let cache_entry = self.cache.entry(
            CacheBucket::FlatIndex,
            "html",
            format!("{}.msgpack", cache_digest(&url.to_string())),
        );
        let cache_control = match self.connectivity {
            Connectivity::Online => CacheControl::from(
                self.cache
                    .freshness(&cache_entry, None, None)
                    .map_err(ErrorKind::Io)?,
            ),
            Connectivity::Offline => CacheControl::AllowStale,
        };

        let flat_index_request = self
            .client
            .uncached()
            .for_host(url)
            .get(Url::from(url.clone()))
            .header("Accept-Encoding", "gzip")
            .header("Accept", "text/html")
            .build()
            .map_err(|err| {
                ErrorKind::from_reqwest(url.clone(), err, self.client.certificate_source())
            })?;
        let parse_simple_response = |response: Response, _: &mut RetryState| {
            async {
                // Use the response URL, rather than the request URL, as the base for relative URLs.
                // This ensures that we handle redirects and other URL transformations correctly.
                let url = DisplaySafeUrl::from_url(response.url().clone());

                let text = response.text().await.map_err(|err| {
                    ErrorKind::from_reqwest(url.clone(), err, self.client.certificate_source())
                })?;
                let unarchived = Self::parse_html(&text, &url)
                    .map_err(|err| Error::from_html_err(err, url.clone()))?;
                OwnedArchive::from_unarchived(&unarchived)
            }
            .boxed_local()
            .instrument(info_span!("parse_flat_index_html", url = % url))
        };
        let response = self
            .client
            .get_cacheable_with_retry(
                flat_index_request,
                &cache_entry,
                cache_control,
                parse_simple_response,
            )
            .await;
        match response {
            Ok(files) => {
                let files = files.iter().map(|file| {
                    rkyv::deserialize::<File, rkyv::rancor::Error>(file)
                        .expect("archived version always deserializes")
                });
                Ok(Self::entries_from_files(files, flat_index))
            }
            Err(CachedClientError::Client(err)) if err.is_offline() => {
                Ok(FlatIndexEntries::offline())
            }
            Err(err) => Err(err.into()),
        }
    }

    /// Read a flat index from a local `--find-links` HTML file.
    async fn read_from_file(
        &self,
        path: &Path,
        flat_index: &IndexUrl,
    ) -> Result<FlatIndexEntries, Error> {
        let text = fs_err::tokio::read_to_string(path)
            .await
            .map_err(ErrorKind::Io)?;
        let files = Self::parse_html(&text, flat_index.url())
            .map_err(|err| Error::from_html_err(err, flat_index.url().clone()))?;
        Ok(Self::entries_from_files(files, flat_index))
    }

    /// Parse distributions from a flat HTML index.
    fn parse_html(text: &str, url: &DisplaySafeUrl) -> Result<Vec<File>, crate::html::Error> {
        let SimpleDetailHTML {
            project_status: _,
            base,
            files,
        } = SimpleDetailHTML::parse(text, url)?;

        let base = SmallString::from(base.as_str());
        Ok(files
            .into_iter()
            .filter_map(|file| match File::try_from_pypi(file, &base) {
                Ok(file) => Some(file),
                Err(err) => {
                    // Ignore files with unparsable version specifiers.
                    debug!("Skipping file in `{}`: {err}", url);
                    None
                }
            })
            .collect())
    }

    /// Convert distribution files into entries for a flat index.
    fn entries_from_files(
        files: impl IntoIterator<Item = File>,
        flat_index: &IndexUrl,
    ) -> FlatIndexEntries {
        let entries = files
            .into_iter()
            .filter_map(|file| {
                Some(FlatIndexEntry {
                    filename: DistFilename::try_from_normalized_filename(&file.filename)?,
                    file,
                    index: flat_index.clone(),
                })
            })
            .collect();
        FlatIndexEntries::from_entries(entries)
    }

    /// Classify a local index and read directory entries on a bounded blocking worker.
    ///
    /// Files are returned to the caller for asynchronous HTML reads.
    async fn read_local_index(
        &self,
        path: PathBuf,
        index: IndexUrl,
    ) -> Result<Option<FlatIndexEntries>, FindLinksDirectoryError> {
        let permit = self.directory_concurrency.clone().acquire_owned().await?;
        let span = Span::current();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let _entered = span.enter();
            if path.is_file() {
                Ok(None)
            } else {
                Self::read_from_directory(&path, &index).map(Some)
            }
        })
        .await?
    }

    /// Read a flat remote index from a `--find-links` directory.
    fn read_from_directory(
        path: &Path,
        flat_index: &IndexUrl,
    ) -> Result<FlatIndexEntries, FindLinksDirectoryError> {
        // The path context is provided by the caller.
        #[expect(clippy::disallowed_methods)]
        let entries = std::fs::read_dir(path)?;

        let mut dists = Vec::new();
        for entry in entries {
            let entry = entry?;
            let metadata = entry.metadata()?;

            if metadata.is_dir() {
                continue;
            }

            if metadata.is_symlink() {
                let Ok(target) = entry.path().read_link() else {
                    warn!(
                        "Skipping unreadable symlink in `--find-links` directory: {}",
                        entry.path().display()
                    );
                    continue;
                };
                if target.is_dir() {
                    continue;
                }
            }

            let filename = entry.file_name();
            let Some(filename) = filename.to_str() else {
                warn!(
                    "Skipping non-UTF-8 filename in `--find-links` directory: {}",
                    filename.to_string_lossy()
                );
                continue;
            };

            // SAFETY: The index path is itself constructed from a URL.
            let url = DisplaySafeUrl::from_file_path(entry.path()).unwrap();

            let file = File {
                dist_info_metadata: None,
                filename: filename.into(),
                hashes: HashDigests::empty(),
                requires_python: None,
                size: None,
                upload_time_utc_ms: None,
                url: FileLocation::AbsoluteUrl(UrlString::from(url)),
                yanked: None,
            };

            let Some(filename) = DistFilename::try_from_normalized_filename(filename) else {
                debug!(
                    "Ignoring `--find-links` entry (expected a wheel or source distribution filename): {}",
                    entry.path().display()
                );
                continue;
            };
            dists.push(FlatIndexEntry {
                filename,
                file,
                index: flat_index.clone(),
            });
        }

        dists.sort_by(|a, b| {
            a.filename
                .cmp(&b.filename)
                .then_with(|| a.index.cmp(&b.index))
        });

        Ok(FlatIndexEntries::from_entries(dists))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BaseClientBuilder;
    use fs_err::File;
    use std::io::Write;
    use std::sync::mpsc;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::runtime::Builder;
    use tokio::sync::oneshot;

    #[test]
    fn cancelled_directory_scan_retains_admission() -> Result<(), Box<dyn std::error::Error>> {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()?;
        let cache = Cache::temp()?;
        let http = CachedClient::new(BaseClientBuilder::default().build()?);
        let slots = Arc::new(Semaphore::new(1));
        let client = FlatIndexClient::new_with_directory_concurrency(
            &http,
            Connectivity::Online,
            &cache,
            slots.clone(),
        );
        let directory = tempdir()?;
        let index = IndexUrl::parse(&directory.path().to_string_lossy(), None)?;

        runtime.block_on(async {
            let (started, start) = oneshot::channel();
            let (release, finish) = mpsc::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                let _ = started.send(());
                finish.recv()
            });
            start.await?;

            // Poll the actual client once to queue its scan, then drop the waiting future.
            assert!(client.fetch_index(&index).now_or_never().is_none());
            assert_eq!(slots.available_permits(), 0);
            assert!(client.clone().fetch_index(&index).now_or_never().is_none());
            tokio::task::yield_now().await;
            assert_eq!(slots.available_permits(), 0);

            release.send(())?;
            blocker.await??;
            let permit = tokio::time::timeout(Duration::from_secs(10), slots.acquire()).await??;
            drop(permit);
            assert!(client.fetch_index(&index).await?.is_empty());
            Ok(())
        })
    }

    #[tokio::test]
    async fn local_index_file_and_missing_directory() -> Result<(), Box<dyn std::error::Error>> {
        let cache = Cache::temp()?;
        let http = CachedClient::new(BaseClientBuilder::default().build()?);
        let client = FlatIndexClient::new(&http, Connectivity::Online, &cache);
        let directory = tempdir()?;
        let html = directory.path().join("index.html");
        fs_err::write(&html, r#"<a href="demo-1.0.tar.gz">demo</a>"#)?;
        let index = IndexUrl::parse(&html.to_string_lossy(), None)?;
        let entries = client.fetch_index(&index).await?;
        assert_eq!(entries.entries.len(), 1);
        assert_eq!(entries.entries[0].filename.to_string(), "demo-1.0.tar.gz");

        let missing = directory.path().join("missing");
        let index = IndexUrl::parse(&missing.to_string_lossy(), None)?;
        let error = client
            .fetch_index(&index)
            .await
            .expect_err("missing directory");
        let FlatIndexError::FindLinksDirectory(path, FindLinksDirectoryError::Io(error)) = error
        else {
            return Err("expected a directory I/O error".into());
        };
        assert_eq!(path, missing);
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        Ok(())
    }

    /// Round-trip a synthetic flat-index cache entry and preserve sidecar hashes.
    #[test]
    fn cached_files_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let url = DisplaySafeUrl::parse("https://example.com/flat/")?;
        let files = FlatIndexClient::parse_html(
            r#"<a href="example-1.0.0-py3-none-any.whl" data-core-metadata="sha256=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef">example-1.0.0-py3-none-any.whl</a>"#,
            &url,
        )?;
        assert_eq!(files.len(), 1);
        let metadata_hashes = files[0].dist_info_metadata.clone();
        assert!(
            metadata_hashes
                .as_ref()
                .is_some_and(|hashes| !hashes.is_empty())
        );
        let archived = OwnedArchive::from_unarchived(&files)?;
        let files = OwnedArchive::deserialize(&archived);
        let entries =
            FlatIndexClient::entries_from_files(files, &IndexUrl::parse(url.as_str(), None)?);
        assert_eq!(entries.entries.len(), 1);
        assert_eq!(entries.entries[0].file.dist_info_metadata, metadata_hashes);
        Ok(())
    }

    #[test]
    fn read_from_directory_sorts_distributions() {
        let dir = tempdir().unwrap();

        let filenames = [
            "beta-2.0.0-py3-none-any.whl",
            "alpha-1.0.0.tar.gz",
            "alpha-1.0.0-py3-none-any.whl",
        ];

        for name in &filenames {
            let mut file = File::create(dir.path().join(name)).unwrap();
            file.write_all(b"").unwrap();
        }

        let entries = FlatIndexClient::read_from_directory(
            dir.path(),
            &IndexUrl::parse(&dir.path().to_string_lossy(), None).unwrap(),
        )
        .unwrap();

        let actual = entries
            .entries
            .iter()
            .map(|entry| entry.filename.to_string())
            .collect::<Vec<_>>();

        let mut expected = filenames
            .iter()
            .map(|name| DistFilename::try_from_normalized_filename(name).unwrap())
            .collect::<Vec<_>>();

        expected.sort();

        let expected = expected
            .into_iter()
            .map(|filename| filename.to_string())
            .collect::<Vec<_>>();

        assert_eq!(actual, expected);
    }
}
