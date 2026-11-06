//! A Simple cache entry contains a fixed-size header, independently
//! archived releases (newest first), and the version table inside the HTTP cache envelope.
//! HTTP 200 replaces the whole file. HTTP 304 can update only the policy after the table.
use std::borrow::Borrow;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use rkyv::rancor;
use rkyv::util::AlignedVec;
use tokio::sync::{Semaphore, SemaphorePermit};
use tokio::task::JoinHandle;
use zerocopy::byteorder::little_endian::{U32, U64};
use zerocopy::{FromBytes, Immutable, IntoBytes};

use uv_pep440::Version;
use uv_pypi_types::{ProjectStatus, ResolutionMetadata};

use super::{RegistryClient, SimpleDetailMetadata, SimpleDetailMetadatum, VersionFiles};
use crate::cached_client::{Cacheable, DataWithCachePolicy};
use crate::httpcache::CachePolicy;
use crate::{CacheRejection, Connectivity, Error, ErrorKind, OwnedArchive};

#[derive(Default, FromBytes, Immutable, IntoBytes)]
#[repr(C)]
struct PageHeader {
    data_len: U64,
    manifest_checksum: U32,
}
const INLINE_THRESHOLD: usize = 32 * 1024;

/// Keep descriptors available for network, builds and other cache operations. Once the
/// budget is used, new pages own their release bytes until they drop.
static FILE_BUDGET: LazyLock<Semaphore> = LazyLock::new(|| {
    #[cfg(unix)]
    let limit = rustix::process::getrlimit(rustix::process::Resource::Nofile)
        .current
        .unwrap_or(u64::MAX);
    #[cfg(not(unix))]
    let limit = u64::MAX;
    Semaphore::new((limit / 4).min(512) as usize)
});

/// A newly parsed response owns the bytes that will be published together.
pub(super) struct ParsedPage {
    manifest: OwnedArchive<Manifest>,
    details: Vec<u8>,
}

/// The version table is resident. Large pages pin the open file that supplied the table,
/// so later reads see the same response across atomic replacement or cache cleanup.
#[derive(Debug)]
pub struct SimplePage {
    manifest: OwnedArchive<Manifest>,
    files: ReleaseFiles,
    source: Option<Box<([u8; 16], PathBuf)>>,
}

#[derive(Debug)]
enum ReleaseFiles {
    Memory(Vec<u8>),
    Disk {
        file: Mutex<fs_err::File>,
        _permit: SemaphorePermit<'static>,
    },
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct Manifest {
    project_status: ProjectStatus,
    /// Ascending PEP 440 order for binary search.
    releases: Vec<Release>,
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct Release {
    version: Version,
    #[rkyv(with = rkyv::with::Niche)]
    metadata: Option<Box<ResolutionMetadata>>,
    earliest_upload: Option<i64>,
    missing_upload: bool,
    multiple_sdists: bool,
    /// Exclusive end in the release-data region, measured in physical (newest-first) order.
    end: u64,
    checksum: u32,
}

#[derive(Debug)]
pub(crate) struct CachedPage {
    envelope: DataWithCachePolicy,
    files: ReleaseFiles,
    len: usize,
    path: PathBuf,
    manifest_checksum: u32,
}

impl Borrow<DataWithCachePolicy> for CachedPage {
    fn borrow(&self) -> &DataWithCachePolicy {
        &self.envelope
    }
}

impl ReleaseFiles {
    /// Position a reader in the immutable release region. A seek and its reads share the
    /// same lock because cloned file handles can also share a cursor.
    fn with_reader<T>(
        &self,
        offset: usize,
        read: impl FnOnce(&mut dyn Read) -> Result<T, Error>,
    ) -> Result<T, Error> {
        match self {
            Self::Memory(bytes) => read(&mut &bytes[offset..]),
            Self::Disk { file, .. } => {
                let mut file = file.lock().expect("release file mutex poisoned");
                file.seek(SeekFrom::Start((size_of::<PageHeader>() + offset) as u64))
                    .map_err(ErrorKind::Io)?;
                read(&mut *file)
            }
        }
    }
}

impl RegistryClient {
    /// Keep the damaged response out of online cache hits while the operation retries.
    pub fn reject_simple_read(&self, error: &Error) -> Option<CacheRejection> {
        if self.connectivity() == Connectivity::Online
            && let ErrorKind::InvalidCachedRelease { entry_id, .. } = error.kind()
        {
            return Some(self.cached_client().reject(*entry_id));
        }
        None
    }

    /// Retry a read-only operation once if a deferred release read needs a fresh response.
    pub async fn retry_simple_read<T>(
        &self,
        read: impl AsyncFn() -> Result<T, Error>,
    ) -> Result<T, Error> {
        let result = read().await;
        if let Err(error) = &result
            && let Some(_rejection) = self.reject_simple_read(error)
        {
            return read().await;
        }
        result
    }

    /// Run a blocking Simple cache read on the client's bounded cache-read pool.
    pub fn spawn_cache_read<T: Send + 'static>(
        &self,
        read: impl FnOnce() -> T + Send + 'static,
    ) -> JoinHandle<T> {
        self.client
            .uncached()
            .cache_read_runtime()
            .spawn_blocking(read)
    }

    /// Visit releases newest first with one sequential reader and one reusable buffer.
    pub async fn fold_version_files<T: Send + 'static>(
        &self,
        page: SimplePage,
        mut state: T,
        mut visit: impl FnMut(&mut T, VersionFiles) + Send + 'static,
    ) -> Result<T, Error> {
        self.spawn_cache_read(move || {
            page.files
                .with_reader(0, |reader| {
                    let mut reader = BufReader::new(reader);
                    let mut bytes = AlignedVec::<16>::new();
                    let mut start = 0;
                    for release in page.manifest.releases.iter().rev() {
                        let end = release.end.to_native() as usize;
                        bytes.resize(end - start, 0);
                        reader.read_exact(&mut bytes).map_err(ErrorKind::Io)?;
                        visit(&mut state, decode_release(release, &bytes)?);
                        start = end;
                    }
                    Ok(state)
                })
                .map_err(|err| page.read_error(err))
        })
        .await
        .expect("release file scan panicked")
    }
}

fn decode_release(release: &ArchivedRelease, bytes: &[u8]) -> Result<VersionFiles, Error> {
    if crc32fast::hash(bytes) != release.checksum.to_native() {
        return Err(ErrorKind::ArchiveRead("Simple release checksum mismatch".into()).into());
    }
    rkyv::from_bytes::<VersionFiles, rancor::Error>(bytes)
        .map_err(|err| ErrorKind::ArchiveRead(err.to_string()).into())
}

impl ParsedPage {
    pub(super) fn from_metadata(page: SimpleDetailMetadata) -> Result<Self, Error> {
        let mut releases = Vec::with_capacity(page.versions.len());
        let mut details = Vec::new();
        for SimpleDetailMetadatum {
            version,
            files,
            metadata,
        } in page.versions.into_iter().rev()
        {
            let mut earliest_upload: Option<i64> = None;
            let mut missing_upload = false;
            for file in files.wheels.iter().chain(&files.source_dists) {
                if file.has_upload_time {
                    let time = file.upload_time_utc_ms;
                    earliest_upload =
                        Some(earliest_upload.map_or(time, |earliest| earliest.min(time)));
                } else {
                    missing_upload = true;
                }
            }
            let bytes = rkyv::to_bytes::<rancor::Error>(&files)
                .map_err(|err| ErrorKind::ArchiveWrite(err.to_string()))?;
            details.extend_from_slice(&bytes);
            releases.push(Release {
                version,
                metadata,
                earliest_upload,
                missing_upload,
                multiple_sdists: files.source_dists.len() > 1,
                end: details.len() as u64,
                checksum: crc32fast::hash(&bytes),
            });
        }
        releases.reverse();
        Ok(Self {
            manifest: OwnedArchive::from_unarchived(&Manifest {
                project_status: page.project_status,
                releases,
            })?,
            details,
        })
    }

    pub(super) fn into_page(self) -> SimplePage {
        SimplePage {
            manifest: self.manifest,
            files: ReleaseFiles::Memory(self.details),
            source: None,
        }
    }
}

impl SimplePage {
    fn read_error(&self, error: Error) -> Error {
        if let Some((entry_id, path)) = self.source.as_deref() {
            ErrorKind::InvalidCachedRelease {
                path: path.clone(),
                entry_id: *entry_id,
                source: Box::new(error),
            }
            .into()
        } else {
            error
        }
    }

    pub fn num_versions(&self) -> usize {
        self.manifest.releases.len()
    }

    /// The version at the given index, in ascending PEP 440 order.
    pub fn version(&self, index: usize) -> Version {
        rkyv::deserialize::<Version, rancor::Error>(&self.manifest.releases[index].version)
            .expect("validated version must deserialize")
    }

    pub fn find_version(&self, version: &Version) -> Option<usize> {
        self.manifest
            .releases
            .binary_search_by(|release| {
                rkyv::deserialize::<Version, rancor::Error>(&release.version)
                    .expect("validated version must deserialize")
                    .cmp(version)
            })
            .ok()
    }

    pub fn has_installable(&self, index: usize) -> bool {
        let release = &self.manifest.releases[index];
        release.missing_upload || release.earliest_upload.is_some()
    }

    /// Whether a wheel or sdist qualifies under the exclusive upload cutoff.
    pub fn any_file_before(&self, index: usize, cutoff_ms: i64, allow_missing: bool) -> bool {
        let release = &self.manifest.releases[index];
        (allow_missing && release.missing_upload)
            || release
                .earliest_upload
                .as_ref()
                .is_some_and(|t| t.to_native() < cutoff_ms)
    }

    /// Prioritization retains all wheels but only one source distribution.
    pub fn has_multiple_sdists(&self, index: usize) -> bool {
        self.manifest.releases[index].multiple_sdists
    }

    pub fn metadata(&self, index: usize) -> Option<ResolutionMetadata> {
        self.manifest.releases[index]
            .metadata
            .as_deref()
            .map(|value| {
                rkyv::deserialize::<ResolutionMetadata, rancor::Error>(value)
                    .expect("validated resolution metadata must deserialize")
            })
    }

    pub fn project_status(&self) -> &rkyv::Archived<ProjectStatus> {
        &self.manifest.project_status
    }

    /// Read and validate just this release, excluding all adjacent releases.
    pub fn files(&self, index: usize) -> Result<VersionFiles, Error> {
        let releases = &self.manifest.releases;
        let release = releases
            .get(index)
            .ok_or_else(|| ErrorKind::ArchiveRead("invalid release index".into()))?;
        let start = releases
            .get(index + 1)
            .map_or(0, |next| next.end.to_native() as usize);
        let len = release.end.to_native() as usize - start;
        let mut bytes = AlignedVec::<16>::with_capacity(len);
        bytes.resize(len, 0);
        self.files
            .with_reader(start, |reader| {
                reader
                    .read_exact(&mut bytes)
                    .map_err(|err| ErrorKind::Io(err).into())
            })
            .and_then(|()| decode_release(release, &bytes))
            .map_err(|err| self.read_error(err))
    }
}

impl Cacheable for ParsedPage {
    type Target = SimplePage;
    type Cached = CachedPage;

    fn read_cache(path: &Path) -> Result<CachedPage, Error> {
        let mut file = fs_err::File::open(path).map_err(ErrorKind::Io)?;
        let mut header = PageHeader::default();
        file.read_exact(header.as_mut_bytes())
            .map_err(ErrorKind::Io)?;
        let len = usize::try_from(header.data_len.get())
            .ok()
            .filter(|len| *len <= AlignedVec::<16>::MAX_CAPACITY)
            .ok_or_else(|| ErrorKind::ArchiveRead("invalid Simple release region length".into()))?;
        let offset = (size_of::<PageHeader>() + len) as u64;
        let (files, envelope) = if len > INLINE_THRESHOLD
            && let Ok(permit) = FILE_BUDGET.try_acquire()
        {
            file.seek(SeekFrom::Start(offset)).map_err(ErrorKind::Io)?;
            let envelope = DataWithCachePolicy::from_reader_at(&mut file, offset)?;
            (
                ReleaseFiles::Disk {
                    file: Mutex::new(file),
                    _permit: permit,
                },
                envelope,
            )
        } else {
            let mut reader = BufReader::new(file);
            // The length comes from disk. Grow as bytes arrive instead of reserving an
            // unverified length from a truncated cache entry.
            let mut bytes = Vec::with_capacity(len.min(INLINE_THRESHOLD));
            reader
                .by_ref()
                .take(len as u64)
                .read_to_end(&mut bytes)
                .map_err(ErrorKind::Io)?;
            let envelope = DataWithCachePolicy::from_reader_at(reader, offset)?;
            (ReleaseFiles::Memory(bytes), envelope)
        };
        Ok(CachedPage {
            envelope,
            files,
            len,
            path: path.to_path_buf(),
            manifest_checksum: header.manifest_checksum.get(),
        })
    }

    fn decode(cached: CachedPage) -> Result<SimplePage, Error> {
        let source = Some(Box::new((cached.envelope.entry_id(), cached.path)));
        let table = cached.envelope.into_data();
        if crc32fast::hash(&table) != cached.manifest_checksum {
            return Err(
                ErrorKind::ArchiveRead("invalid Simple version table checksum".into()).into(),
            );
        }
        let manifest = OwnedArchive::<Manifest>::new(table)?;
        let mut start = 0;
        for release in manifest.releases.iter().rev() {
            let end = release.end.to_native();
            if end <= start || end > cached.len as u64 {
                return Err(ErrorKind::ArchiveRead("invalid Simple release bounds".into()).into());
            }
            start = end;
        }
        if start != cached.len as u64 {
            return Err(
                ErrorKind::ArchiveRead("invalid Simple release region length".into()).into(),
            );
        }
        Ok(SimplePage {
            manifest,
            files: cached.files,
            source,
        })
    }

    async fn write_cache(self, path: &Path, policy: &CachePolicy) -> Result<SimplePage, Error> {
        let entry_id = DataWithCachePolicy::new_entry_id();
        let table = OwnedArchive::as_bytes(&self.manifest);
        let header = PageHeader {
            data_len: U64::new(self.details.len() as u64),
            manifest_checksum: U32::new(crc32fast::hash(table)),
        };
        let envelope = DataWithCachePolicy::serialize_with_id(policy, table, entry_id)?;
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let mut temp = uv_fs::tempfile_in(path.parent().expect("cache entry has a parent"))?;
            temp.write_all(header.as_bytes())?;
            temp.write_all(&self.details)?;
            temp.write_all(&envelope)?;
            // Pin before publication: another writer can replace the path immediately.
            // If unrelated work has consumed the available descriptors, keep the parsed bytes.
            let files = if self.details.len() > INLINE_THRESHOLD
                && let Ok(permit) = FILE_BUDGET.try_acquire()
                && let Ok(handle) = temp.as_file().try_clone()
            {
                let file = fs_err::File::from_parts(handle, &path);
                ReleaseFiles::Disk {
                    file: Mutex::new(file),
                    _permit: permit,
                }
            } else {
                ReleaseFiles::Memory(self.details)
            };
            uv_fs::persist_with_retry_sync(temp, &path)?;
            Ok(SimplePage {
                manifest: self.manifest,
                files,
                source: Some(Box::new((entry_id, path))),
            })
        })
        .await
        .expect("Simple cache publication panicked")
        .map_err(|err| ErrorKind::CacheWrite(err).into())
    }

    fn into_target(self) -> SimplePage {
        self.into_page()
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use anyhow::Result;
    use uv_normalize::PackageName;
    use uv_redacted::SafeUrl;

    use super::{ParsedPage, ReleaseFiles, SimpleDetailMetadata};
    use crate::cached_client::Cacheable;
    use crate::httpcache::CachePolicyBuilder;

    fn large_page(version: u8) -> Result<ParsedPage> {
        let reason = "x".repeat(40 * 1024);
        let html = format!(
            r#"<a href="demo-{version}.0.tar.gz" data-yanked="{reason}">demo-{version}.0.tar.gz</a>"#
        );
        Ok(ParsedPage::from_metadata(SimpleDetailMetadata::from_html(
            &html,
            &PackageName::from_str("demo")?,
            &SafeUrl::parse("https://example.org/simple/demo/")?,
        )?)?)
    }

    /// Readers opened before replacement or removal must still get the releases in
    /// their own version tables, including a page returned directly after publication.
    #[tokio::test]
    async fn pinned_simple_readers_survive_replacement_and_removal() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("demo.rkyv");
        let request = reqwest::Request::new(
            http::Method::GET,
            "https://example.org/simple/demo/".parse()?,
        );
        let response = reqwest::Response::from(
            http::Response::builder()
                .header("cache-control", "public, max-age=3600")
                .body(Vec::new())?,
        );
        let policy = CachePolicyBuilder::new(&request).build(&response);

        let published = large_page(1)?.write_cache(&path, &policy).await?;
        let cached = ParsedPage::decode(ParsedPage::read_cache(&path)?)?;
        assert!(matches!(cached.files, ReleaseFiles::Disk { .. }));
        let replacement = large_page(2)?.write_cache(&path, &policy).await?;
        assert_eq!(fs_err::read_dir(temp.path())?.count(), 1);
        fs_err::remove_file(&path)?;

        assert_eq!(
            published.files(0)?.source_dists[0].filename(),
            "demo-1.0.tar.gz"
        );
        assert_eq!(
            cached.files(0)?.source_dists[0].filename(),
            "demo-1.0.tar.gz"
        );
        assert_eq!(
            replacement.files(0)?.source_dists[0].filename(),
            "demo-2.0.tar.gz"
        );
        Ok(())
    }
}
