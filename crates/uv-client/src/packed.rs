use std::io::SeekFrom;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail};
use futures::TryStreamExt;
use reqwest::{Body, Method, Request, Response, ResponseBuilderExt};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tokio_util::io::ReaderStream;
use tracing::debug;

use uv_cache::{Cache, CacheBucket, CacheEntry, Freshness, WheelCache};
use uv_cache_info::Timestamp;
use uv_distribution_filename::{SourceDistExtension, WheelFilename};
use uv_distribution_types::IndexUrl;
use uv_extract::hash::{HashReader, Hasher};
use uv_fs::write_atomic;
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_pypi_types::{HashAlgorithm, HashDigest};
use uv_redacted::DisplaySafeUrl;

use crate::httpcache::{BeforeRequest, CachePolicy, CachePolicyBuilder};
use crate::{
    CacheControl, CachedClientError, Connectivity, DataWithCachePolicy, OwnedArchive,
    RegistryClient,
};

/// An original distribution archive, retained without extracting or building it.
#[derive(Debug)]
pub(crate) struct PackedArchive {
    file: fs_err::tokio::File,
    size: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct Metadata {
    hash: HashDigest,
    size: u64,
}

#[derive(Serialize, Deserialize)]
struct LocalPointer {
    timestamp: Timestamp,
    archive: Metadata,
}

/// A packed distribution's source-aware cache entry.
///
/// The HTTP pointer is separate from the original bytes, just as prepared distributions keep
/// their HTTP policy separate from the extracted archive. Only distribution consumers construct
/// these entries; unrelated HTTP requests never consult the packed cache.
#[derive(Debug, Clone)]
pub struct PackedArchiveEntry {
    cache: Cache,
    entry: CacheEntry,
    name: PackageName,
    url: DisplaySafeUrl,
    index: Option<IndexUrl>,
}

impl PackedArchiveEntry {
    /// Locate an artifact under the same source and package shards used for cached wheels.
    pub fn new(
        cache: &Cache,
        index: Option<&IndexUrl>,
        name: &PackageName,
        url: &DisplaySafeUrl,
        key: &str,
    ) -> Self {
        let source = if let Some(index) = index {
            WheelCache::Index(index)
        } else if url.scheme() == "file" {
            WheelCache::Path(url)
        } else {
            WheelCache::Url(url)
        };
        let extension = if url.scheme() == "file" {
            "rev"
        } else {
            "http"
        };
        Self {
            cache: cache.clone(),
            entry: cache.entry(
                CacheBucket::Packed,
                source.wheel_dir(name.as_ref()),
                format!("{key}.{extension}"),
            ),
            name: name.clone(),
            url: url.clone(),
            index: index.cloned(),
        }
    }

    /// Preserve the wheel cache key, with a suffix identifying the packed representation.
    pub fn wheel_key(filename: &WheelFilename) -> String {
        format!("{}.whl", filename.cache_key())
    }

    /// Registry source archives are identified by version; direct sources by their URL shard.
    pub fn source_key(version: Option<&Version>, extension: SourceDistExtension) -> String {
        if let Some(version) = version {
            format!("{version}.{extension}")
        } else {
            format!("archive.{extension}")
        }
    }

    pub(crate) fn cache_control(&self, client: &RegistryClient) -> Result<CacheControl> {
        if client.connectivity() == Connectivity::Offline {
            return Ok(CacheControl::AllowStale);
        }
        let freshness = self.cache.freshness(&self.entry, Some(&self.name), None)?;
        if freshness == Freshness::Stale {
            return Ok(CacheControl::MustRevalidate);
        }
        Ok(self
            .index
            .as_ref()
            .and_then(|index| client.artifact_cache_control(index))
            .map_or(CacheControl::from(freshness), CacheControl::Override))
    }

    /// Fetch an archive, checking the lockfile digest before publishing it to the cache.
    /// Returns whether a new archive was downloaded.
    pub async fn download(
        &self,
        client: &RegistryClient,
        expected_hash: Option<&HashDigest>,
        expected_size: Option<u64>,
    ) -> Result<bool> {
        let lock_entry = CacheEntry::from_path(self.entry.path().with_extension("lock"));
        let _lock = lock_entry.lock().await?;
        if self.url.scheme() == "file" {
            return self.download_local(expected_hash, expected_size).await;
        }

        let request = client
            .uncached_client(&self.url)
            .get(self.url.as_str())
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .build()?;
        let cache_control = self.cache_control(client)?;
        let downloaded = AtomicBool::new(false);
        let download = async |response: Response| {
            // A prefetch must not report success if the response cannot be retained for reuse.
            if !CachePolicyBuilder::new(&request)
                .build(&response)
                .to_archived()
                .is_storable()
            {
                return Err(std::io::Error::other(format!(
                    "Response for {} does not permit caching",
                    self.url
                )));
            }
            let input = response
                .bytes_stream()
                .map_err(std::io::Error::other)
                .into_async_read();
            let metadata = self
                .persist(input.compat(), expected_hash, expected_size)
                .await
                .map_err(std::io::Error::other)?;
            downloaded.store(true, Ordering::Relaxed);
            Ok(metadata)
        };
        let metadata = client
            .cached_client()
            .get_serde_with_retry(
                request
                    .try_clone()
                    .context("Could not clone packed archive request")?,
                &self.entry,
                cache_control.clone(),
                &download,
            )
            .await
            .map_err(packed_client_error)?;
        if downloaded.load(Ordering::Relaxed) {
            return Ok(true);
        }
        let missing = match self.read(&metadata, expected_hash, expected_size).await {
            Ok(archive) => archive.is_none(),
            Err(_) if matches!(cache_control, CacheControl::MustRevalidate) => true,
            Err(err) => return Err(err),
        };
        if missing {
            // A valid HTTP pointer can outlive its payload, e.g., after manual cache cleanup.
            client
                .cached_client()
                .skip_cache_with_retry(
                    request
                        .try_clone()
                        .context("Could not clone packed archive request")?,
                    &self.entry,
                    cache_control,
                    &download,
                )
                .await
                .map_err(packed_client_error)?;
        }
        Ok(downloaded.load(Ordering::Relaxed))
    }

    async fn download_local(
        &self,
        expected_hash: Option<&HashDigest>,
        expected_size: Option<u64>,
    ) -> Result<bool> {
        let path = self
            .url
            .to_file_path()
            .map_err(|()| anyhow::anyhow!("Invalid file URL: {}", self.url))?;
        let timestamp = Timestamp::from_path(&path)?;
        if self
            .cache
            .freshness(&self.entry, Some(&self.name), Some(&path))?
            != Freshness::Stale
        {
            match fs_err::tokio::read(self.entry.path()).await {
                Ok(bytes) => {
                    let pointer: LocalPointer = rmp_serde::from_slice(&bytes)?;
                    if pointer.timestamp == timestamp
                        && self
                            .read(&pointer.archive, expected_hash, expected_size)
                            .await?
                            .is_some()
                    {
                        return Ok(false);
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
        }
        let archive = self
            .persist(
                fs_err::tokio::File::open(&path).await?,
                expected_hash,
                expected_size,
            )
            .await?;
        write_atomic(
            self.entry.path(),
            rmp_serde::to_vec_named(&LocalPointer { timestamp, archive })?,
        )
        .await?;
        Ok(true)
    }

    async fn persist(
        &self,
        input: impl tokio::io::AsyncRead + Unpin,
        expected_hash: Option<&HashDigest>,
        expected_size: Option<u64>,
    ) -> Result<Metadata> {
        let url = &self.url;
        let mut algorithms = vec![HashAlgorithm::Sha256];
        algorithms.extend(expected_hash.map(HashDigest::algorithm));
        algorithms.sort();
        algorithms.dedup();
        let mut hashers = algorithms.into_iter().map(Hasher::from).collect::<Vec<_>>();
        let temporary = tempfile::NamedTempFile::new_in(self.entry.dir())?;
        let mut output = fs_err::tokio::File::from_std(fs_err::File::from_parts(
            temporary.reopen()?,
            temporary.path(),
        ));
        let mut reader = HashReader::new(input, &mut hashers);
        let size = tokio::io::copy(&mut reader, &mut output).await?;
        output.flush().await?;
        drop(output);
        let hashes = hashers
            .into_iter()
            .map(HashDigest::from)
            .collect::<Vec<_>>();
        if let Some(expected) = expected_hash
            && !hashes.contains(expected)
        {
            bail!("Hash mismatch for {url}: expected {expected}");
        }
        if let Some(expected) = expected_size
            && size != expected
        {
            bail!("Size mismatch for {url}: expected {expected}, got {size}");
        }
        let hash = hashes
            .into_iter()
            .find(|hash| hash.algorithm == HashAlgorithm::Sha256)
            .context("Missing SHA-256 digest")?;
        let destination = self.entry.with_file(&*hash.digest);
        temporary.persist(destination.path())?;
        Ok(Metadata { hash, size })
    }

    /// Open and verify a packed archive before handing its bytes to a consumer.
    async fn read(
        &self,
        metadata: &Metadata,
        expected_hash: Option<&HashDigest>,
        expected_size: Option<u64>,
    ) -> Result<Option<PackedArchive>> {
        let url = &self.url;
        // Do not allow damaged metadata to escape the cache shard.
        if metadata.hash.algorithm != HashAlgorithm::Sha256
            || metadata.hash.digest.len() != 64
            || !metadata
                .hash
                .digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("Invalid packed archive digest for {url}");
        }
        let path = self.entry.with_file(&*metadata.hash.digest).into_path_buf();
        let mut file = match fs_err::tokio::File::open(path).await {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        let mut algorithms = vec![HashAlgorithm::Sha256];
        algorithms.extend(expected_hash.map(HashDigest::algorithm));
        algorithms.sort();
        algorithms.dedup();
        let mut hashers = algorithms.into_iter().map(Hasher::from).collect::<Vec<_>>();
        let mut reader = HashReader::new(&mut file, &mut hashers);
        let size = tokio::io::copy(&mut reader, &mut tokio::io::sink()).await?;
        let hashes = hashers
            .into_iter()
            .map(HashDigest::from)
            .collect::<Vec<_>>();
        if size != metadata.size
            || !hashes.contains(&metadata.hash)
            || expected_hash.is_some_and(|expected| !hashes.contains(expected))
            || expected_size.is_some_and(|expected| size != expected)
        {
            bail!("Hash or size mismatch for packed archive {url}");
        }
        file.seek(SeekFrom::Start(0)).await?;
        debug!("Using packed distribution: {url}");
        Ok(Some(PackedArchive { file, size }))
    }

    pub(crate) async fn read_http(
        &self,
        request: &Request,
        cache_control: &CacheControl,
    ) -> Result<Option<(PackedArchive, Box<CachePolicy>)>> {
        if request.method() != Method::GET {
            return Ok(None);
        }
        let allow_stale = matches!(cache_control, CacheControl::AllowStale);
        if !allow_stale
            && (matches!(cache_control, CacheControl::MustRevalidate)
                || self.cache.freshness(&self.entry, Some(&self.name), None)? == Freshness::Stale)
        {
            return Ok(None);
        }
        let bytes = match fs_err::tokio::read(self.entry.path()).await {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        let cached = DataWithCachePolicy::from_reader(std::io::Cursor::new(bytes))?;
        if allow_stale {
            if !cached.cache_policy.matches_stale_request(request) {
                return Ok(None);
            }
        } else {
            let mut request = request
                .try_clone()
                .context("Could not clone packed archive request")?;
            if !matches!(
                cached.cache_policy.before_request(&mut request),
                BeforeRequest::Fresh
            ) {
                return Ok(None);
            }
        }
        let metadata: Metadata = rmp_serde::from_slice(&cached.data)?;
        let Some(archive) = self.read(&metadata, None, None).await? else {
            return Ok(None);
        };
        Ok(Some((
            archive,
            Box::new(OwnedArchive::deserialize(&cached.cache_policy)),
        )))
    }

    pub(crate) async fn response(
        &self,
        request: &Request,
        cache_control: &CacheControl,
    ) -> Result<Option<(Response, Box<CachePolicy>)>> {
        let Some((archive, policy)) = self.read_http(request, cache_control).await? else {
            return Ok(None);
        };
        let response = http::Response::builder()
            .url(request.url().clone())
            .header(http::header::CONTENT_LENGTH, archive.size)
            .body(Body::wrap_stream(ReaderStream::new(archive.file)))?;
        Ok(Some((Response::from(response), policy)))
    }
}

impl PackedArchive {
    pub(crate) fn into_file(self) -> fs_err::tokio::File {
        self.file
    }
}

fn packed_client_error(error: CachedClientError<std::io::Error>) -> anyhow::Error {
    match error {
        CachedClientError::Client(error) => error.into(),
        CachedClientError::Callback { err, .. } => err.into(),
    }
}
