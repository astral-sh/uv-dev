use std::path::{Path, PathBuf};

use futures::{FutureExt, StreamExt};
use reqwest::Response;
use rustc_hash::FxHashSet;
use tokio::sync::Semaphore;
use tracing::{Instrument, debug, info_span, warn};
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
}

impl<'a> FlatIndexClient<'a> {
    /// Create a new [`FlatIndexClient`].
    pub fn new(client: &'a CachedClient, connectivity: Connectivity, cache: &'a Cache) -> Self {
        Self {
            client,
            connectivity,
            cache,
        }
    }

    /// Read the directories and flat remote indexes from `--find-links`.
    pub async fn fetch_all(
        &self,
        indexes: impl Iterator<Item = &IndexUrl>,
        download_concurrency: &Semaphore,
    ) -> Result<FlatIndexEntries, FlatIndexError> {
        let local_concurrency = Semaphore::new(16);
        // The same location can be inherited from several requirements or configuration files.
        // Match the full URL so different credentials and query parameters remain independent.
        let mut seen = FxHashSet::default();
        let mut fetches = futures::stream::iter(indexes.filter(|index| seen.insert(index.url())))
            .map(async |index| {
                let _permit = match index {
                    IndexUrl::Path(_) => local_concurrency.acquire().await,
                    IndexUrl::Pypi(_) | IndexUrl::Url(_) => download_concurrency.acquire().await,
                };
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
            // The semaphores bound active reads. Consume completed fetches immediately so
            // slow indexes cannot keep later locations from using an available permit.
            .buffer_unordered(usize::MAX);

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
                if path.is_file() {
                    self.read_from_file(&path, index)
                        .await
                        .map_err(|err| FlatIndexError::FindLinksFile(path.clone(), err))
                } else {
                    Self::read_from_directory(&path, index)
                        .map_err(|err| FlatIndexError::FindLinksDirectory(path.clone(), err))
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
                    debug!("Skipping file in {}: {err}", url);
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
    use std::convert::Infallible;
    use std::io::Write;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use fs_err::File;
    use http_body_util::Full;
    use hyper::body::Bytes;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use tempfile::tempdir;
    use tokio::net::TcpListener;
    use tokio::sync::{Barrier, Notify};

    use super::*;

    #[tokio::test]
    async fn completed_fetches_start_later_indexes() -> anyhow::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let last_started = Arc::new(Notify::new());
        let server = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let last_started = Arc::clone(&last_started);
                tokio::spawn(async move {
                    let service =
                        service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                            let last_started = Arc::clone(&last_started);
                            async move {
                                match request.uri().path() {
                                    "/0" => last_started.notified().await,
                                    "/16" => last_started.notify_one(),
                                    _ => {}
                                }
                                Ok::<_, Infallible>(hyper::Response::new(Full::new(
                                    Bytes::from_static(
                                        b"<a href='example-1.0-py3-none-any.whl'>example</a>",
                                    ),
                                )))
                            }
                        });
                    hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await
                });
            }
        });
        let indexes = (0..17)
            .map(|index| IndexUrl::parse(&format!("http://{address}/{index}"), None))
            .collect::<Result<Vec<_>, _>>()?;
        let cache = Cache::temp()?.init().await?;
        let client = CachedClient::new(crate::BaseClientBuilder::default().build()?);
        let flat = FlatIndexClient::new(&client, Connectivity::Online, &cache);
        let semaphore = Semaphore::new(16);
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            flat.fetch_all(indexes.iter(), &semaphore),
        )
        .await;
        server.abort();
        let entries = result??;
        assert_eq!(entries.entries.len(), 17);
        assert!(entries.entries.windows(2).all(|pair| {
            pair[0]
                .filename
                .cmp(&pair[1].filename)
                .then(pair[0].index.cmp(&pair[1].index))
                .is_le()
        }));
        Ok(())
    }

    #[tokio::test]
    async fn fetch_all_uses_the_shared_download_limit() -> anyhow::Result<()> {
        for limit in [1, 2, 9, 16, 17, 50] {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let address = listener.local_addr()?;
            let expected_parallelism = limit.min(34);
            let barrier = Arc::new(Barrier::new(expected_parallelism));
            let started = Arc::new(AtomicUsize::new(0));
            let active = Arc::new(AtomicUsize::new(0));
            let peak = Arc::new(AtomicUsize::new(0));
            let server_started = Arc::clone(&started);
            let server_peak = Arc::clone(&peak);
            let server = tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let barrier = Arc::clone(&barrier);
                    let started = Arc::clone(&server_started);
                    let active = Arc::clone(&active);
                    let peak = Arc::clone(&server_peak);
                    tokio::spawn(async move {
                        let service =
                            service_fn(move |_: hyper::Request<hyper::body::Incoming>| {
                                let barrier = Arc::clone(&barrier);
                                let started = Arc::clone(&started);
                                let active = Arc::clone(&active);
                                let peak = Arc::clone(&peak);
                                async move {
                                    let concurrent = active.fetch_add(1, Ordering::SeqCst) + 1;
                                    peak.fetch_max(concurrent, Ordering::SeqCst);
                                    if started.fetch_add(1, Ordering::SeqCst) < expected_parallelism
                                    {
                                        barrier.wait().await;
                                    }
                                    active.fetch_sub(1, Ordering::SeqCst);
                                    Ok::<_, Infallible>(hyper::Response::new(Full::new(
                                        Bytes::from_static(
                                            b"<a href='example-1.0-py3-none-any.whl'>example</a>",
                                        ),
                                    )))
                                }
                            });
                        hyper::server::conn::http1::Builder::new()
                            .serve_connection(TokioIo::new(stream), service)
                            .await
                    });
                }
            });
            let indexes = (0..34)
                .map(|index| IndexUrl::parse(&format!("http://{address}/{index}"), None))
                .collect::<Result<Vec<_>, _>>()?;
            let cache = Cache::temp()?.init().await?;
            let client = CachedClient::new(crate::BaseClientBuilder::default().retries(0).build()?);
            let flat = FlatIndexClient::new(&client, Connectivity::Online, &cache);
            let semaphore = Semaphore::new(limit);
            let result = tokio::time::timeout(Duration::from_secs(5), async {
                tokio::try_join!(
                    flat.fetch_all(indexes[..17].iter(), &semaphore),
                    flat.fetch_all(indexes[17..].iter(), &semaphore),
                )
            })
            .await;
            server.abort();
            let (first, second) = result??;
            for (result, expected) in [(first, &indexes[..17]), (second, &indexes[17..])] {
                let mut expected = expected.to_vec();
                expected.sort();
                assert_eq!(
                    result
                        .entries
                        .into_iter()
                        .map(|entry| entry.index)
                        .collect::<Vec<_>>(),
                    expected,
                );
            }
            assert_eq!(started.load(Ordering::SeqCst), 34);
            assert_eq!(peak.load(Ordering::SeqCst), expected_parallelism);
        }
        Ok(())
    }

    #[tokio::test]
    async fn local_indexes_do_not_need_a_download_permit() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let filename = "example-1.0-py3-none-any.whl";
        File::create(directory.path().join(filename))?;
        let indexes = [IndexUrl::parse(&directory.path().to_string_lossy(), None)?];
        let cache = Cache::temp()?.init().await?;
        let client = CachedClient::new(crate::BaseClientBuilder::default().build()?);
        let flat = FlatIndexClient::new(&client, Connectivity::Online, &cache);
        let semaphore = Semaphore::new(0);
        let entries = tokio::time::timeout(
            Duration::from_secs(2),
            flat.fetch_all(indexes.iter(), &semaphore),
        )
        .await??;
        assert_eq!(entries.entries.len(), 1);
        assert_eq!(entries.entries[0].filename.to_string(), filename);
        Ok(())
    }

    #[tokio::test]
    async fn duplicate_urls_are_fetched_once() -> Result<(), Box<dyn std::error::Error>> {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        for version in [1, 2] {
            Mock::given(method("GET"))
                .and(path("/flat"))
                .and(query_param("version", version.to_string()))
                .respond_with(ResponseTemplate::new(200).set_body_raw(
                    format!("<a href='/example-{version}.0-py3-none-any.whl'>wheel</a>"),
                    "text/html",
                ))
                .expect(1)
                .mount(&server)
                .await;
        }
        let first = IndexUrl::parse(&format!("{}/flat?version=1", server.uri()), None)?;
        let second = IndexUrl::parse(&format!("{}/flat?version=2", server.uri()), None)?;
        let indexes = [&first, &first, &second, &first, &second];
        let cache = Cache::temp()?;
        let client = CachedClient::new(crate::BaseClientBuilder::default().build()?);
        let entries = FlatIndexClient::new(&client, Connectivity::Online, &cache)
            .fetch_all(indexes.into_iter(), &Semaphore::new(50))
            .await?;
        assert_eq!(entries.entries.len(), 2);
        assert_eq!(
            entries.entries[0].filename.to_string(),
            "example-1.0-py3-none-any.whl"
        );
        assert_eq!(
            entries.entries[1].filename.to_string(),
            "example-2.0-py3-none-any.whl"
        );
        Ok(())
    }

    #[tokio::test]
    async fn duplicate_urls_keep_distinct_credentials() -> Result<(), Box<dyn std::error::Error>> {
        use wiremock::matchers::{basic_auth, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        for (username, version) in [("first", 1), ("second", 2)] {
            Mock::given(method("GET"))
                .and(path("/flat"))
                .and(basic_auth(username, "secret"))
                .respond_with(ResponseTemplate::new(200).set_body_raw(
                    format!("<a href='/example-{version}.0-py3-none-any.whl'>wheel</a>"),
                    "text/html",
                ))
                .expect(1)
                .mount(&server)
                .await;
        }
        let first = IndexUrl::parse(
            &format!("{}/flat", server.uri()).replacen("http://", "http://first:secret@", 1),
            None,
        )?;
        let second = IndexUrl::parse(
            &format!("{}/flat", server.uri()).replacen("http://", "http://second:secret@", 1),
            None,
        )?;
        let indexes = [&first, &first, &second, &second];
        let cache = Cache::temp()?;
        let client = CachedClient::new(crate::BaseClientBuilder::default().build()?);
        let entries = FlatIndexClient::new(&client, Connectivity::Online, &cache)
            .fetch_all(indexes.into_iter(), &Semaphore::new(50))
            .await?;
        assert_eq!(entries.entries.len(), 2);
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
