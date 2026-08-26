use std::io::SeekFrom;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use futures::TryStreamExt;
use reqwest::{Body, Method, Request, Response, ResponseBuilderExt};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tokio_util::io::ReaderStream;
use tracing::debug;

use uv_cache::{Cache, CacheBucket, CacheEntry, Freshness};
use uv_cache_key::cache_digest;
use uv_extract::hash::{HashReader, Hasher};
use uv_fs::write_atomic;
use uv_normalize::PackageName;
use uv_pypi_types::{HashAlgorithm, HashDigest};
use uv_redacted::DisplaySafeUrl;

use crate::RegistryClient;

/// An original distribution archive, retained without extracting or building it.
#[derive(Debug)]
pub struct PackedArchive {
    file: fs_err::tokio::File,
    size: u64,
}

#[derive(Serialize, Deserialize)]
struct Metadata {
    hash: HashDigest,
    size: u64,
}

fn entry(cache: &Cache, url: &DisplaySafeUrl) -> CacheEntry {
    let mut url = url.clone();
    url.remove_credentials();
    url.set_fragment(None);
    cache.entry(
        CacheBucket::Packed,
        cache_digest(&url.as_str()),
        "metadata.msgpack",
    )
}

impl PackedArchive {
    /// Fetch an archive, checking the lockfile digest before publishing it to the cache.
    /// Returns whether a new archive was downloaded.
    pub async fn download(
        cache: &Cache,
        client: &RegistryClient,
        name: &PackageName,
        url: &DisplaySafeUrl,
        expected_hash: Option<&HashDigest>,
        expected_size: Option<u64>,
    ) -> Result<bool> {
        let entry = entry(cache, url);
        let _lock = entry.with_file(".lock").lock().await?;
        if cache.freshness(&entry, Some(name), None)? != Freshness::Stale
            && Self::read(cache, url, expected_hash, expected_size)
                .await?
                .is_some()
        {
            return Ok(false);
        }

        let mut algorithms = vec![HashAlgorithm::Sha256];
        algorithms.extend(expected_hash.map(HashDigest::algorithm));
        algorithms.sort();
        algorithms.dedup();
        let mut hashers = algorithms.into_iter().map(Hasher::from).collect::<Vec<_>>();
        let temporary = tempfile::NamedTempFile::new_in(entry.dir())?;
        let mut output = fs_err::tokio::File::from_std(fs_err::File::from_parts(
            temporary.reopen()?,
            temporary.path(),
        ));
        let size = if url.scheme() == "file" {
            let path = url
                .to_file_path()
                .map_err(|()| anyhow::anyhow!("Invalid file URL: {url}"))?;
            let input = fs_err::tokio::File::open(path).await?;
            let mut reader = HashReader::new(input, &mut hashers);
            tokio::io::copy(&mut reader, &mut output).await?
        } else {
            let response = client
                .uncached_client(url)
                .get(url.as_str())
                .header(reqwest::header::ACCEPT_ENCODING, "identity")
                .send()
                .await?
                .error_for_status()?;
            let input = response
                .bytes_stream()
                .map_err(std::io::Error::other)
                .into_async_read();
            let mut reader = HashReader::new(input.compat(), &mut hashers);
            tokio::io::copy(&mut reader, &mut output).await?
        };
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
        let destination = entry.with_file(&*hash.digest);
        temporary.persist(destination.path())?;
        write_atomic(entry.with_file("package").path(), name.as_ref()).await?;
        write_atomic(entry.path(), rmp_serde::to_vec(&Metadata { hash, size })?).await?;
        Ok(true)
    }

    /// Open and verify a packed archive before handing its bytes to a consumer.
    pub(crate) async fn read(
        cache: &Cache,
        url: &DisplaySafeUrl,
        expected_hash: Option<&HashDigest>,
        expected_size: Option<u64>,
    ) -> Result<Option<Self>> {
        let entry = entry(cache, url);
        let bytes = match fs_err::tokio::read(entry.path()).await {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        let metadata: Metadata = rmp_serde::from_slice(&bytes)?;
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
        let path: PathBuf = entry.with_file(&*metadata.hash.digest).into_path_buf();
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
        Ok(Some(Self { file, size }))
    }

    pub(crate) async fn response(
        cache: &Cache,
        request: &Request,
        allow_stale: bool,
    ) -> Result<Option<Response>> {
        if request.method() != Method::GET {
            return Ok(None);
        }
        let url = DisplaySafeUrl::from_url(request.url().clone());
        if !allow_stale {
            // A missing derived HTTP entry does not imply that this packed archive is fresh.
            // Check its own timestamp and package name to honor both refresh policies.
            let entry = entry(cache, &url);
            let package =
                match fs_err::tokio::read_to_string(entry.with_file("package").path()).await {
                    Ok(package) => Some(package.parse::<PackageName>()?),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
                    Err(err) => return Err(err.into()),
                };
            if cache.freshness(&entry, package.as_ref(), None)? == Freshness::Stale {
                return Ok(None);
            }
        }
        let Some(archive) = Self::read(cache, &url, None, None).await? else {
            return Ok(None);
        };
        let response = http::Response::builder()
            .url(request.url().clone())
            .header(http::header::CONTENT_LENGTH, archive.size)
            .header(
                http::header::CACHE_CONTROL,
                "public, max-age=31536000, immutable",
            )
            .body(Body::wrap_stream(ReaderStream::new(archive.file)))?;
        Ok(Some(Response::from(response)))
    }

    pub(crate) fn into_file(self) -> fs_err::tokio::File {
        self.file
    }
}
