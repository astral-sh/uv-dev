//! A Simple cache file stores release archives newest first, followed by a version table
//! and its HTTP cache policy. The release fingerprint lets a reader reopen a file safely
//! after another process has refreshed the same package.
use std::borrow::Borrow;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use rkyv::rancor;
use rkyv::util::AlignedVec;
use tokio::task::JoinHandle;
use zerocopy::byteorder::little_endian::U64;
use zerocopy::{FromBytes, Immutable, IntoBytes};

use uv_cache::Cache;
use uv_pep440::Version;
use uv_pypi_types::{ProjectStatus, ResolutionMetadata};

use super::{RegistryClient, SimpleDetailMetadata, SimpleDetailMetadatum, VersionFiles};
use crate::cached_client::{Cacheable, DataWithCachePolicy};
use crate::httpcache::CachePolicy;
use crate::{Error, ErrorKind, OwnedArchive};

/// Small pages fit in memory and do not need to reopen the cache.
const INLINE_THRESHOLD: usize = 32 * 1024;

#[derive(Default, FromBytes, Immutable, IntoBytes)]
#[repr(C)]
struct PageHeader {
    fingerprint: [u8; blake3::OUT_LEN],
    details_len: U64,
}

pub(super) struct ParsedPage {
    manifest: OwnedArchive<Manifest>,
    details: Vec<u8>,
}

/// The version list stays in memory. A lookup reads only the selected release's files.
#[derive(Debug)]
pub struct SimplePage {
    manifest: OwnedArchive<Manifest>,
    files: ReleaseFiles,
}

#[derive(Debug)]
enum ReleaseFiles {
    Memory(Vec<u8>),
    Cached {
        path: PathBuf,
        fingerprint: [u8; blake3::OUT_LEN],
        /// A page may outlive the client that owns its temporary cache directory.
        temporary_cache: Option<Box<Cache>>,
    },
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct Manifest {
    project_status: ProjectStatus,
    /// Ascending PEP 440 order, for binary search.
    releases: Vec<Release>,
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct Release {
    version: Version,
    #[rkyv(with = rkyv::with::Niche)]
    metadata: Option<Box<ResolutionMetadata>>,
    earliest_upload: Option<i64>,
    missing_upload: bool,
    /// Exclusive end in the release region, which is stored newest first.
    end: u64,
}

impl ArchivedRelease {
    fn end_offset(&self) -> usize {
        usize::try_from(self.end.to_native()).expect("validated release offsets fit in usize")
    }
}

#[derive(Debug)]
pub(crate) struct CachedPage {
    envelope: DataWithCachePolicy,
    files: ReleaseFiles,
    len: usize,
}

impl Borrow<DataWithCachePolicy> for CachedPage {
    fn borrow(&self) -> &DataWithCachePolicy {
        &self.envelope
    }
}

impl ReleaseFiles {
    fn cached(path: PathBuf, fingerprint: [u8; blake3::OUT_LEN]) -> Self {
        Self::Cached {
            path,
            fingerprint,
            temporary_cache: None,
        }
    }

    fn with_reader<T>(
        &self,
        offset: usize,
        read: impl FnOnce(&mut dyn Read) -> Result<T, Error>,
    ) -> Result<T, Error> {
        match self {
            Self::Memory(bytes) => read(&mut &bytes[offset..]),
            Self::Cached {
                path, fingerprint, ..
            } => {
                let mut file = match fs_err::File::open(path) {
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                        return Err(ErrorKind::SimplePageChanged(path.clone()).into());
                    }
                    result => result.map_err(ErrorKind::Io)?,
                };
                // A refresh can replace the pathname at any time. Check and read the same handle.
                let mut current = [0; blake3::OUT_LEN];
                file.read_exact(&mut current).map_err(ErrorKind::Io)?;
                if &current != fingerprint {
                    return Err(ErrorKind::SimplePageChanged(path.clone()).into());
                }
                file.seek(SeekFrom::Start((size_of::<PageHeader>() + offset) as u64))
                    .map_err(ErrorKind::Io)?;
                read(&mut file)
            }
        }
    }
}

impl RegistryClient {
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
            page.files.with_reader(0, |reader| {
                let mut reader = BufReader::new(reader);
                let mut bytes = AlignedVec::<16>::new();
                let mut start = 0;
                for release in page.manifest.releases.iter().rev() {
                    let end = release.end_offset();
                    bytes.resize(end - start, 0);
                    reader.read_exact(&mut bytes).map_err(ErrorKind::Io)?;
                    visit(&mut state, decode_release(&bytes)?);
                    start = end;
                }
                Ok(state)
            })
        })
        .await
        .expect("release file scan panicked")
    }
}

fn decode_release(bytes: &[u8]) -> Result<VersionFiles, Error> {
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
                end: details.len() as u64,
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
        }
    }
}

impl SimplePage {
    pub(super) fn keep_temporary_cache(&mut self, cache: &Cache) {
        if cache.is_temporary()
            && let ReleaseFiles::Cached {
                temporary_cache, ..
            } = &mut self.files
        {
            *temporary_cache = Some(Box::new(cache.clone()));
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
            .map_or(0, ArchivedRelease::end_offset);
        let len = release.end_offset() - start;
        let mut bytes = AlignedVec::<16>::with_capacity(len);
        bytes.resize(len, 0);
        self.files
            .with_reader(start, |reader| {
                reader
                    .read_exact(&mut bytes)
                    .map_err(|err| ErrorKind::Io(err).into())
            })
            .and_then(|()| decode_release(&bytes))
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
        let len = usize::try_from(header.details_len.get())
            .ok()
            .filter(|&len| len <= AlignedVec::<16>::MAX_CAPACITY)
            .ok_or_else(|| ErrorKind::ArchiveRead("invalid Simple release region length".into()))?;
        let offset = (size_of::<PageHeader>() + len) as u64;
        let files = if len > INLINE_THRESHOLD {
            file.seek(SeekFrom::Start(offset)).map_err(ErrorKind::Io)?;
            ReleaseFiles::cached(path.to_owned(), header.fingerprint)
        } else {
            let mut bytes = vec![0; len];
            file.read_exact(&mut bytes).map_err(ErrorKind::Io)?;
            ReleaseFiles::Memory(bytes)
        };
        let envelope = DataWithCachePolicy::from_reader_at(file, offset)?;
        Ok(CachedPage {
            envelope,
            files,
            len,
        })
    }

    fn decode(cached: CachedPage) -> Result<SimplePage, Error> {
        let manifest = OwnedArchive::<Manifest>::new(cached.envelope.into_data())?;
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
        })
    }

    async fn write_cache(self, path: &Path, policy: &CachePolicy) -> Result<SimplePage, Error> {
        let envelope =
            DataWithCachePolicy::serialize(policy, OwnedArchive::as_bytes(&self.manifest))?;
        let path = path.to_owned();
        tokio::task::spawn_blocking(move || {
            let header = PageHeader {
                fingerprint: *blake3::hash(&self.details).as_bytes(),
                details_len: U64::new(self.details.len() as u64),
            };
            let mut temp = uv_fs::tempfile_in(path.parent().expect("a cache entry has a parent"))?;
            temp.write_all(header.as_bytes())?;
            temp.write_all(&self.details)?;
            temp.write_all(&envelope)?;
            uv_fs::persist_with_retry_sync(temp, &path)?;

            let files = if self.details.len() > INLINE_THRESHOLD {
                ReleaseFiles::cached(path, header.fingerprint)
            } else {
                ReleaseFiles::Memory(self.details)
            };
            Ok(SimplePage {
                manifest: self.manifest,
                files,
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
