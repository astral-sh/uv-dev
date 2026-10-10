use std::io::SeekFrom;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail};
use futures::TryStreamExt;
use reqwest::{Body, Method, Request, Response, ResponseBuilderExt};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tokio_util::io::ReaderStream;
use tracing::{debug, warn};

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
    CacheControl, Connectivity, DataWithCachePolicy, ErrorKind, RegistryClient, RetryState,
};

/// An original distribution archive, retained without extracting or building it.
#[derive(Debug)]
pub(crate) struct PackedArchive {
    file: fs_err::tokio::File,
    size: u64,
}

/// A packed pointer can identify an older archive even when its HTTP policy requires refresh.
pub(crate) enum PackedArchiveRead {
    Missing,
    Stale(Vec<u8>),
    Fresh(PackedArchive, Box<CachePolicy>),
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
    fn new(
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

    /// Locate a wheel using its filename and source index.
    pub fn wheel(
        cache: &Cache,
        index: Option<&IndexUrl>,
        url: &DisplaySafeUrl,
        filename: &WheelFilename,
    ) -> Self {
        Self::new(
            cache,
            index,
            &filename.name,
            url,
            &format!("{}.whl", filename.cache_key()),
        )
    }

    /// Locate a source archive; registry identity always includes its package version.
    pub fn source(
        cache: &Cache,
        registry: Option<(&IndexUrl, &Version)>,
        name: &PackageName,
        url: &DisplaySafeUrl,
        extension: SourceDistExtension,
    ) -> Self {
        let (index, key) = if let Some((index, version)) = registry {
            (Some(index), format!("{version}.{extension}"))
        } else {
            (None, format!("archive.{extension}"))
        };
        Self::new(cache, index, name, url, &key)
    }

    /// Return whether a local pointer is present; readers still verify its retained bytes.
    pub fn has_local_pointer(&self) -> bool {
        self.url.scheme() == "file" && self.entry.path().is_file()
    }

    /// Read the retained revision for installed-package freshness checks.
    pub fn local_timestamp(&self) -> Result<Option<Timestamp>> {
        if self.url.scheme() != "file" {
            return Ok(None);
        }
        let bytes = match fs_err::read(self.entry.path()) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        let pointer: LocalPointer = rmp_serde::from_slice(&bytes)?;
        Ok(Some(pointer.timestamp))
    }

    /// Open a verified local archive, retaining the original file's revision timestamp.
    ///
    /// A changed source invalidates the packed copy. A missing source can still be served from
    /// the retained bytes, unless the caller explicitly requested cache refresh.
    pub async fn read_local(
        &self,
    ) -> Result<Option<(fs_err::tokio::File, Timestamp)>, crate::Error> {
        self.read_local_inner().await.map_err(packed_error)
    }

    async fn read_local_inner(&self) -> Result<Option<(fs_err::tokio::File, Timestamp)>> {
        if self.url.scheme() != "file" {
            return Ok(None);
        }
        let path = self
            .url
            .to_file_path()
            .map_err(|()| anyhow::anyhow!("Invalid file URL: {}", self.url))?;
        if self
            .cache
            .freshness(&self.entry, Some(&self.name), Some(&path))?
            == Freshness::Stale
        {
            return Ok(None);
        }
        let bytes = match fs_err::tokio::read(self.entry.path()).await {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        let pointer: LocalPointer = rmp_serde::from_slice(&bytes)?;
        match Timestamp::from_path(&path) {
            Ok(timestamp) if timestamp != pointer.timestamp => return Ok(None),
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
        Ok(self
            .read(&pointer.archive, None, None)
            .await?
            .map(|archive| (archive.into_file(), pointer.timestamp)))
    }

    pub(crate) fn cache_control(&self, client: &RegistryClient) -> Result<CacheControl> {
        if client.connectivity() == Connectivity::Offline {
            return Ok(CacheControl::AllowStale);
        }
        let freshness = self.cache.freshness(&self.entry, Some(&self.name), None)?;
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

        let mut request = client
            .uncached_client(&self.url)
            .get(self.url.as_str())
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .build()?;
        let cache_control = self.cache_control(client)?;
        let force_revalidation = client.connectivity() == Connectivity::Online
            && self.cache.freshness(&self.entry, Some(&self.name), None)? == Freshness::Stale;
        if force_revalidation {
            // Request revalidation independently of any configured response-policy override.
            request.headers_mut().insert(
                reqwest::header::CACHE_CONTROL,
                reqwest::header::HeaderValue::from_static("no-cache"),
            );
        }
        let downloaded = AtomicBool::new(false);
        let download = async |response: Response, _: &mut RetryState| {
            // A prefetch must not report success if the response cannot be retained for reuse.
            if !CachePolicyBuilder::new(&request)
                .build(&response)
                .to_archived()
                .is_storable()
            {
                return Err(crate::Error::from(ErrorKind::Io(std::io::Error::other(
                    format!("Response for {} does not permit caching", self.url),
                ))));
            }
            let input = response
                .bytes_stream()
                .map_err(std::io::Error::other)
                .into_async_read();
            let metadata = self
                .persist(input.compat(), expected_hash, expected_size)
                .await
                .map_err(packed_error)?;
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
            .map_err(crate::Error::from)?;
        if downloaded.load(Ordering::Relaxed) {
            return Ok(true);
        }
        // A 304 updates the retained policy without invoking the download callback.
        let bytes = fs_err::tokio::read(self.entry.path()).await?;
        let cached = DataWithCachePolicy::from_reader(bytes.as_slice())?;
        if !cached.cache_policy().is_storable() {
            bail!("Response for {} does not permit caching", self.url);
        }
        let missing = match self.read(&metadata, expected_hash, expected_size).await {
            Ok(archive) => archive.is_none(),
            Err(_) if force_revalidation => true,
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
                .map_err(crate::Error::from)?;
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
        let timestamp = match Timestamp::from_path(&path) {
            Ok(timestamp) => Some(timestamp),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => return Err(err.into()),
        };
        if self
            .cache
            .freshness(&self.entry, Some(&self.name), Some(&path))?
            != Freshness::Stale
        {
            match fs_err::tokio::read(self.entry.path()).await {
                Ok(bytes) => {
                    let pointer: LocalPointer = rmp_serde::from_slice(&bytes)?;
                    if timestamp.is_none_or(|timestamp| pointer.timestamp == timestamp)
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
        let timestamp = timestamp.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Local archive not found: {}", path.display()),
            )
        })?;
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
        let temporary = uv_fs::tempfile_in(self.entry.dir())?;
        let mut output = fs_err::tokio::File::from_std(fs_err::File::from_parts(
            temporary.as_file().try_clone()?,
            temporary.as_ref(),
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
            let actual = hashes
                .iter()
                .find(|hash| hash.algorithm() == expected.algorithm())
                .expect("the requested hash algorithm was computed")
                .clone();
            return Err(crate::Error::from(ErrorKind::PackedArchiveHashMismatch {
                url: url.clone(),
                expected: expected.clone(),
                actual,
            })
            .into());
        }
        if let Some(expected) = expected_size
            && size != expected
        {
            bail!("Size mismatch for {url}: expected {expected}, got {size}");
        }
        let hash = hashes
            .into_iter()
            .find(|hash| hash.algorithm() == HashAlgorithm::Sha256)
            .context("Missing SHA-256 digest")?;
        let destination = self.entry.with_file(hash.digest());
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
        if metadata.hash.algorithm() != HashAlgorithm::Sha256
            || metadata.hash.digest().len() != 64
            || !metadata
                .hash
                .digest()
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("Invalid packed archive digest for {url}");
        }
        let path = self.entry.with_file(metadata.hash.digest()).into_path_buf();
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
        for expected in std::iter::once(&metadata.hash).chain(expected_hash) {
            if !hashes.contains(expected) {
                let actual = hashes
                    .iter()
                    .find(|hash| hash.algorithm() == expected.algorithm())
                    .expect("the requested hash algorithm was computed")
                    .clone();
                return Err(crate::Error::from(ErrorKind::PackedArchiveHashMismatch {
                    url: url.clone(),
                    expected: expected.clone(),
                    actual,
                })
                .into());
            }
        }
        if let Some(expected) = std::iter::once(metadata.size)
            .chain(expected_size)
            .find(|expected| *expected != size)
        {
            bail!("Size mismatch for packed archive {url}: expected {expected}, got {size}");
        }
        file.seek(SeekFrom::Start(0)).await?;
        debug!("Using packed distribution: {url}");
        Ok(Some(PackedArchive { file, size }))
    }

    pub(crate) async fn read_http(
        &self,
        request: &Request,
        cache_control: &CacheControl,
    ) -> Result<PackedArchiveRead, crate::Error> {
        self.read_http_inner(request, cache_control)
            .await
            .map_err(packed_error)
    }

    async fn read_http_inner(
        &self,
        request: &Request,
        cache_control: &CacheControl,
    ) -> Result<PackedArchiveRead> {
        if request.method() != Method::GET {
            return Ok(PackedArchiveRead::Missing);
        }
        let allow_stale = matches!(cache_control, CacheControl::AllowStale);
        let must_revalidate = !allow_stale
            && (matches!(cache_control, CacheControl::MustRevalidate)
                || self.cache.freshness(&self.entry, Some(&self.name), None)? == Freshness::Stale);
        let bytes = match fs_err::tokio::read(self.entry.path()).await {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(PackedArchiveRead::Missing);
            }
            Err(err) => return Err(err.into()),
        };
        let cached = match DataWithCachePolicy::from_reader(bytes.as_slice()) {
            Ok(cached) => cached,
            Err(err) => {
                warn!(
                    "Ignoring broken packed archive metadata for {}: {err}",
                    self.url
                );
                return Ok(PackedArchiveRead::Missing);
            }
        };
        let metadata: Metadata = match rmp_serde::from_slice(cached.data()) {
            Ok(metadata) => metadata,
            Err(err) => {
                warn!(
                    "Ignoring broken packed archive metadata for {}: {err}",
                    self.url
                );
                return Ok(PackedArchiveRead::Missing);
            }
        };
        if must_revalidate {
            return Ok(PackedArchiveRead::Stale(bytes));
        }
        if allow_stale {
            if !cached.cache_policy().matches_stale_request(request) {
                return Ok(PackedArchiveRead::Stale(bytes));
            }
        } else {
            let mut request = request
                .try_clone()
                .context("Could not clone packed archive request")?;
            if !matches!(
                cached.cache_policy().before_request(&mut request),
                BeforeRequest::Fresh
            ) {
                return Ok(PackedArchiveRead::Stale(bytes));
            }
        }
        let Some(archive) = self.read(&metadata, None, None).await? else {
            return Ok(PackedArchiveRead::Stale(bytes));
        };
        let policy = rkyv::deserialize::<CachePolicy, rkyv::rancor::Error>(cached.cache_policy())
            .context("Could not deserialize packed archive cache policy")?;
        Ok(PackedArchiveRead::Fresh(archive, Box::new(policy)))
    }

    /// Drop an older archive pointer after its metadata is refreshed elsewhere.
    /// A concurrently refreshed policy or archive retains its pointer.
    pub(crate) async fn invalidate(&self, revision: &[u8]) -> Result<(), crate::Error> {
        let lock_entry = CacheEntry::from_path(self.entry.path().with_extension("lock"));
        let _lock = lock_entry.lock().await.map_err(ErrorKind::CacheLock)?;
        let bytes = match fs_err::tokio::read(self.entry.path()).await {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(ErrorKind::Io(err).into()),
        };
        if bytes == revision {
            match fs_err::tokio::remove_file(self.entry.path()).await {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(ErrorKind::Io(err).into()),
            }
        }
        Ok(())
    }

    pub(crate) async fn response(
        &self,
        request: &Request,
        cache_control: &CacheControl,
    ) -> Result<Option<(Response, Box<CachePolicy>)>> {
        let PackedArchiveRead::Fresh(archive, policy) =
            self.read_http(request, cache_control).await?
        else {
            return Ok(None);
        };
        let response = http::Response::builder()
            .url(request.url().clone())
            .header(http::header::CONTENT_LENGTH, archive.size)
            .body(Body::wrap_stream(ReaderStream::new(archive.file)))?;
        Ok(Some((Response::from(response), policy)))
    }
}

/// Remove packed payloads no longer referenced by any pointer in their source shard.
/// The caller must hold the cache's exclusive lock.
pub fn prune_packed_archives(cache: &Cache) -> Result<uv_cache::Removal> {
    let mut summary = cache.removal();
    let root = cache.bucket(CacheBucket::Packed);
    if !root.try_exists()? {
        return Ok(summary);
    }
    let mut directories = vec![root];
    while let Some(directory) = directories.pop() {
        let mut references = std::collections::HashSet::new();
        let mut payloads = Vec::new();
        let mut unreadable_pointer = false;
        for entry in fs_err::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                directories.push(path);
                continue;
            }
            let extension = path.extension().and_then(std::ffi::OsStr::to_str);
            let metadata = match extension {
                Some("http") => fs_err::read(&path)
                    .map_err(anyhow::Error::from)
                    .and_then(|bytes| {
                        DataWithCachePolicy::from_reader(std::io::Cursor::new(bytes))
                            .map_err(Into::into)
                    })
                    .and_then(|cached| {
                        rmp_serde::from_slice::<Metadata>(cached.data()).map_err(Into::into)
                    }),
                Some("rev") => fs_err::read(&path)
                    .map_err(anyhow::Error::from)
                    .and_then(|bytes| {
                        rmp_serde::from_slice::<LocalPointer>(&bytes)
                            .map(|pointer| pointer.archive)
                            .map_err(Into::into)
                    }),
                _ => {
                    if let Some(name) = path.file_name().and_then(std::ffi::OsStr::to_str)
                        && name.len() == 64
                        && name.bytes().all(|byte| byte.is_ascii_hexdigit())
                    {
                        payloads.push(path);
                    }
                    continue;
                }
            };
            match metadata {
                Ok(metadata) => {
                    references.insert(metadata.hash.digest().to_owned());
                }
                Err(err) => {
                    // An unreadable pointer may still reference any payload in this shard.
                    warn!(
                        "Could not read packed archive pointer {}: {err}",
                        path.display()
                    );
                    unreadable_pointer = true;
                }
            }
        }
        if !unreadable_pointer {
            for path in payloads {
                if !path
                    .file_name()
                    .and_then(std::ffi::OsStr::to_str)
                    .is_some_and(|name| references.contains(name))
                {
                    summary += cache.remove_path(path)?;
                }
            }
        }
    }
    Ok(summary)
}

impl PackedArchive {
    pub(crate) fn into_file(self) -> fs_err::tokio::File {
        self.file
    }
}

/// Preserve typed integrity errors while adapting other packed-cache failures to client I/O errors.
pub(crate) fn packed_error(error: anyhow::Error) -> crate::Error {
    match error.downcast::<crate::Error>() {
        Ok(error) => error,
        Err(error) => ErrorKind::Io(std::io::Error::other(error)).into(),
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Result, bail};
    use tokio::io::AsyncReadExt;
    use wiremock::matchers::{header, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use uv_cache::{Cache, Refresh};
    use uv_cache_info::Timestamp;
    use uv_distribution_filename::WheelFilename;
    use uv_redacted::DisplaySafeUrl;

    use crate::{BaseClientBuilder, CacheControl, RegistryClientBuilder};

    use super::{PackedArchiveEntry, PackedArchiveRead};

    /// A 304 can republish the same payload with a new policy while metadata refresh is in flight.
    #[tokio::test]
    async fn invalidation_retains_revalidated_pointer_with_same_payload() -> Result<()> {
        let server = MockServer::start().await;
        let cache = Cache::temp()?;
        let client =
            RegistryClientBuilder::new(BaseClientBuilder::default(), cache.clone()).build()?;
        let filename: WheelFilename = "example-1.0.0-py3-none-any.whl".parse()?;
        let url = DisplaySafeUrl::parse(&format!(
            "{server_url}/{filename}",
            server_url = server.uri()
        ))?;
        let entry = PackedArchiveEntry::wheel(&cache, None, &url, &filename);
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("etag", "\"original\"")
                    .insert_header("cache-control", "public, max-age=3600")
                    .set_body_bytes(b"original archive"),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert!(entry.download(&client, None, None).await?);
        server.verify().await;
        server.reset().await;

        let request = client
            .uncached_client(&url)
            .get(url.as_str())
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .build()?;
        let revision = match entry
            .read_http(&request, &CacheControl::MustRevalidate)
            .await?
        {
            PackedArchiveRead::Stale(revision) => revision,
            PackedArchiveRead::Missing | PackedArchiveRead::Fresh(..) => {
                bail!("expected a retained pointer requiring revalidation")
            }
        };
        let refreshed = PackedArchiveEntry::wheel(
            &cache.with_refresh(Refresh::All(Timestamp::now())),
            None,
            &url,
            &filename,
        );
        Mock::given(method("GET"))
            .and(header("if-none-match", "\"original\""))
            .respond_with(
                ResponseTemplate::new(304)
                    .insert_header("etag", "\"original\"")
                    .insert_header("cache-control", "public, max-age=7200"),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert!(!refreshed.download(&client, None, None).await?);
        assert_ne!(revision, fs_err::tokio::read(entry.entry.path()).await?);

        entry.invalidate(&revision).await?;
        let archive = match entry.read_http(&request, &CacheControl::AllowStale).await? {
            PackedArchiveRead::Fresh(archive, _) => archive,
            PackedArchiveRead::Missing | PackedArchiveRead::Stale(_) => {
                bail!("revalidated archive must remain available offline")
            }
        };
        let mut bytes = Vec::new();
        archive.into_file().read_to_end(&mut bytes).await?;
        assert_eq!(bytes, b"original archive");
        server.verify().await;
        Ok(())
    }
}
