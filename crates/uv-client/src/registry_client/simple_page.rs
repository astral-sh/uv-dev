use std::borrow::Cow;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use itertools::Itertools;
use rkyv::rancor;
use rkyv::util::AlignedVec;
use zerocopy::byteorder::little_endian::U64;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};

use uv_cache::{Cache, simple_detail_path};
use uv_fastid::Id;
use uv_pep440::Version;
use uv_pypi_types::{ProjectStatus, ResolutionMetadata};

use super::{RegistryClient, SimpleDetailMetadata, SimpleDetailMetadatum, VersionFiles};
use crate::cached_client::Cacheable;
use crate::{Error, ErrorKind, OwnedArchive};

/// Group adjacent candidates so that backtracking can reuse their file details.
const RELEASES_PER_BLOCK: u32 = 8;
/// Keep small pages in a single cache entry.
const INLINE_THRESHOLD: usize = 32 * 1024;

/// A Simple API page. The version list remains resident while file details for large pages
/// are read from independently archived blocks as candidates are considered.
#[derive(Debug)]
pub struct SimplePage {
    data: SimplePageData,
    cache: Arc<BlockCache>,
}

#[derive(Debug)]
pub(super) struct SimplePageData {
    generation: Id,
    manifest: OwnedArchive<Manifest>,
    details: Details,
}

#[derive(Debug)]
enum Details {
    /// Embedded in the HTTP cache entry, including empty pages.
    Inline,
    /// Details owned by this page before publication, or when disk reads cannot be deferred.
    Memory(Vec<u8>),
    Disk(PathBuf),
}

/// A fixed-size header lets cache pruning identify the live generation without decoding rkyv.
/// The modification time is recorded after the immutable detail file is published.
#[derive(FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned)]
#[repr(C)]
struct PageHeader {
    generation: [u8; 16],
    modified_nanos: U64,
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct Manifest {
    project_status: ProjectStatus,
    releases_per_block: u32,
    blocks: Vec<Block>,
    releases: Vec<Release>,
    inline: Vec<u8>,
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct Block {
    /// Exclusive end offset in the concatenated block archives.
    end: u64,
    checksum: u32,
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct Release {
    version: Version,
    #[rkyv(with = rkyv::with::Niche)]
    metadata: Option<Box<ResolutionMetadata>>,
    earliest_upload: Option<i64>,
    missing_upload: bool,
}

impl Release {
    /// These summaries answer version availability and upload cutoff queries without loading
    /// file details. Variant JSONs cannot be installed on their own.
    fn from_datum(datum: SimpleDetailMetadatum) -> (Self, VersionFiles) {
        let files = datum.files;
        let mut earliest_upload: Option<i64> = None;
        let mut missing_upload = false;
        for file in files.wheels.iter().chain(&files.source_dists) {
            if file.has_upload_time {
                let time = file.upload_time_utc_ms;
                earliest_upload = Some(earliest_upload.map_or(time, |earliest| earliest.min(time)));
            } else {
                missing_upload = true;
            }
        }
        let release = Self {
            version: datum.version,
            metadata: datum.metadata,
            earliest_upload,
            missing_upload,
        };
        (release, files)
    }
}

/// Cache state shared by the registry client and the pages it returns.
#[derive(Debug)]
pub(super) struct BlockCache {
    /// Keep the temporary directory and shared cache lock alive while pages can read from disk.
    _disk: Cache,
    /// Keep at most one block across all packages, for reuse by adjacent candidate lookups.
    block: Mutex<Option<CachedBlock>>,
}

impl BlockCache {
    pub(super) fn new(disk: Cache) -> Self {
        Self {
            _disk: disk,
            block: Mutex::default(),
        }
    }
}

#[derive(Debug)]
struct CachedBlock {
    generation: Id,
    index: usize,
    files: Arc<OwnedArchive<Vec<VersionFiles>>>,
}

impl RegistryClient {
    /// Start reading recent release details while the resolver considers other packages.
    #[cfg(target_os = "linux")]
    pub fn prefetch_recent_files(&self, page: &SimplePage) {
        const READ_AHEAD_BYTES: u64 = 128 * 1024;

        if let Details::Disk(path) = &page.data.details
            && let Some(last) = page.data.manifest.blocks.last()
        {
            let path = path.clone();
            let end = last.end.to_native();
            self.client
                .uncached()
                .cache_read_runtime()
                .spawn_blocking(move || {
                    uv_fs::prefetch_file_range(&path, end.saturating_sub(READ_AHEAD_BYTES)..end);
                });
        }
    }

    /// Fold the releases on a Simple API page newest first on the cache-read pool.
    pub async fn fold_version_files<T: Send + 'static>(
        &self,
        page: SimplePage,
        mut state: T,
        mut visit: impl FnMut(&mut T, VersionFiles) + Send + 'static,
    ) -> Result<T, Error> {
        self.client
            .uncached()
            .cache_read_runtime()
            .spawn_blocking(move || {
                let mut reader = page.data.reader()?;
                for index in (0..page.data.manifest.blocks.len()).rev() {
                    let block = page.data.read_block(index, &mut reader)?;
                    for files in block.iter().rev() {
                        let files = rkyv::deserialize(files).map_err(|err: rancor::Error| {
                            ErrorKind::ArchiveRead(err.to_string())
                        })?;
                        visit(&mut state, files);
                    }
                }
                Ok(state)
            })
            .await
            .expect("release file scan panicked")
    }
}

impl SimplePage {
    pub(super) fn new(data: SimplePageData, cache: Arc<BlockCache>) -> Self {
        Self { data, cache }
    }

    /// The number of versions on this page.
    pub fn num_versions(&self) -> usize {
        self.data.manifest.releases.len()
    }

    /// The version at the given index, in ascending PEP 440 order.
    pub fn version(&self, index: usize) -> Version {
        rkyv::deserialize::<Version, rancor::Error>(&self.data.manifest.releases[index].version)
            .expect("validated version must deserialize")
    }

    /// Find the release index for an exact version.
    pub fn find_version(&self, version: &Version) -> Option<usize> {
        self.data
            .manifest
            .releases
            .binary_search_by(|release| {
                rkyv::deserialize::<Version, rancor::Error>(&release.version)
                    .expect("validated version must deserialize")
                    .cmp(version)
            })
            .ok()
    }

    /// Whether a version contains any wheels or source distributions.
    pub fn has_installable(&self, index: usize) -> bool {
        let release = &self.data.manifest.releases[index];
        release.missing_upload || release.earliest_upload.is_some()
    }

    /// Whether a wheel or sdist qualifies under the given exclusive upload cutoff.
    pub fn any_file_before(&self, index: usize, cutoff_ms: i64, allow_missing: bool) -> bool {
        let release = &self.data.manifest.releases[index];
        (allow_missing && release.missing_upload)
            || release
                .earliest_upload
                .as_ref()
                .is_some_and(|time| time.to_native() < cutoff_ms)
    }

    /// Optional index-provided resolution metadata for a version.
    pub fn metadata(&self, index: usize) -> Option<ResolutionMetadata> {
        self.data.manifest.releases[index]
            .metadata
            .as_deref()
            .map(|value| {
                rkyv::deserialize::<ResolutionMetadata, rancor::Error>(value)
                    .expect("validated resolution metadata must deserialize")
            })
    }

    /// The project-level PEP 792 status.
    pub fn project_status(&self) -> &rkyv::Archived<ProjectStatus> {
        &self.data.manifest.project_status
    }

    /// Load the files for a single release.
    pub fn files(&self, index: usize) -> Result<VersionFiles, Error> {
        if index >= self.num_versions() {
            return Err(ErrorKind::ArchiveRead("invalid release index".into()).into());
        }
        let size = self.data.manifest.releases_per_block.to_native() as usize;
        let block_index = index / size;
        let generation = self.data.generation;
        let cached = self
            .cache
            .block
            .lock()
            .expect("block cache lock poisoned")
            .as_ref()
            .filter(|entry| entry.generation == generation && entry.index == block_index)
            .map(|entry| entry.files.clone());
        let block = if let Some(cached) = cached {
            cached
        } else {
            let mut reader = self.data.reader()?;
            let block = Arc::new(self.data.read_block(block_index, &mut reader)?);
            *self.cache.block.lock().expect("block cache lock poisoned") = Some(CachedBlock {
                generation,
                index: block_index,
                files: block.clone(),
            });
            block
        };
        rkyv::deserialize(&block[index % size])
            .map_err(|err: rancor::Error| ErrorKind::ArchiveRead(err.to_string()).into())
    }
}

enum DetailReader<'a> {
    Memory(&'a [u8]),
    Disk(fs_err::File),
}

impl SimplePageData {
    pub(super) fn from_metadata<const CACHE_LOCKED: bool>(
        page: SimpleDetailMetadata,
    ) -> Result<Self, Error> {
        let mut releases = Vec::with_capacity(page.versions.len());
        let mut blocks = Vec::new();
        let mut bytes = Vec::new();
        let mut files = Vec::with_capacity(RELEASES_PER_BLOCK as usize);
        for chunk in &page
            .versions
            .into_iter()
            .chunks(RELEASES_PER_BLOCK as usize)
        {
            for datum in chunk {
                let (release, version_files) = Release::from_datum(datum);
                releases.push(release);
                files.push(version_files);
            }
            let block = rkyv::to_bytes::<rancor::Error>(&files)
                .map_err(|err| ErrorKind::ArchiveWrite(err.to_string()))?;
            bytes.extend_from_slice(&block);
            blocks.push(Block {
                end: bytes.len() as u64,
                checksum: crc32fast::hash(&block),
            });
            files.clear();
        }

        let (inline, details) = if !CACHE_LOCKED || bytes.len() <= INLINE_THRESHOLD {
            (bytes, Details::Inline)
        } else {
            (Vec::new(), Details::Memory(bytes))
        };
        Ok(Self {
            generation: Id::secure(),
            manifest: OwnedArchive::from_unarchived(&Manifest {
                project_status: page.project_status,
                releases_per_block: RELEASES_PER_BLOCK,
                blocks,
                releases,
                inline,
            })?,
            details,
        })
    }

    fn reader(&self) -> Result<DetailReader<'_>, Error> {
        match &self.details {
            Details::Inline => Ok(DetailReader::Memory(&self.manifest.inline)),
            Details::Memory(bytes) => Ok(DetailReader::Memory(bytes)),
            Details::Disk(path) => Ok(DetailReader::Disk(
                uv_fs::open_with_write_attributes(path).map_err(ErrorKind::Io)?,
            )),
        }
    }

    fn read_block(
        &self,
        index: usize,
        reader: &mut DetailReader<'_>,
    ) -> Result<OwnedArchive<Vec<VersionFiles>>, Error> {
        self.read_block_inner(index, reader).map_err(|err| {
            let Details::Disk(path) = &self.details else {
                return err;
            };
            // A speculative read can encounter corruption in a block the solver never needs.
            // Keep the other blocks readable, and verify the whole pack on the next cache hit.
            if let DetailReader::Disk(file) = reader
                && (matches!(err.kind(), ErrorKind::ArchiveRead(_))
                    || matches!(err.kind(), ErrorKind::Io(err) if err.kind() == std::io::ErrorKind::UnexpectedEof))
                && let Err(err) = file.set_modified(UNIX_EPOCH)
            {
                tracing::warn!("Failed to mark corrupt release files for revalidation: {err}");
            }
            ErrorKind::InvalidReleaseBlock(path.clone(), index, Box::new(err)).into()
        })
    }

    fn read_block_inner(
        &self,
        index: usize,
        reader: &mut DetailReader<'_>,
    ) -> Result<OwnedArchive<Vec<VersionFiles>>, Error> {
        let block = &self.manifest.blocks[index];
        let start = index
            .checked_sub(1)
            .map_or(0, |previous| self.manifest.blocks[previous].end.to_native());
        let len = usize::try_from(block.end.to_native() - start)
            .ok()
            .filter(|len| *len <= AlignedVec::<16>::MAX_CAPACITY)
            .ok_or_else(|| ErrorKind::ArchiveRead("invalid release block size".into()))?;
        let mut bytes = AlignedVec::with_capacity(len);
        match reader {
            DetailReader::Disk(file) => {
                file.seek(SeekFrom::Start(start)).map_err(ErrorKind::Io)?;
                bytes.resize(len, 0);
                file.read_exact(&mut bytes).map_err(ErrorKind::Io)?;
            }
            DetailReader::Memory(data) => {
                let start = usize::try_from(start)
                    .map_err(|_| ErrorKind::ArchiveRead("invalid release block offset".into()))?;
                bytes.extend_from_slice(&data[start..start + len]);
            }
        }
        if crc32fast::hash(&bytes) != block.checksum.to_native() {
            return Err(ErrorKind::ArchiveRead("invalid release block checksum".into()).into());
        }
        let files = OwnedArchive::<Vec<VersionFiles>>::new(bytes)?;
        let width = self.manifest.releases_per_block.to_native() as usize;
        if files.len() != width.min(self.manifest.releases.len() - index * width) {
            return Err(ErrorKind::ArchiveRead("invalid release block count".into()).into());
        }
        Ok(files)
    }

    /// Zero means the filesystem did not supply a usable timestamp. In that case the reader
    /// verifies the checksums before accepting the cache entry.
    fn modified_nanos(metadata: &std::fs::Metadata) -> u64 {
        metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .and_then(|duration| u64::try_from(duration.as_nanos()).ok())
            .unwrap_or(0)
    }
}

/// Without a cache lock, cleanup may remove a generation at any time. Publish new responses
/// as a single manifest and snapshot existing external details before returning a page.
pub(super) struct CachedSimplePage<const CACHE_LOCKED: bool>(SimplePageData);

impl<const CACHE_LOCKED: bool> CachedSimplePage<CACHE_LOCKED> {
    pub(super) fn from_metadata(page: SimpleDetailMetadata) -> Result<Self, Error> {
        SimplePageData::from_metadata::<CACHE_LOCKED>(page).map(Self)
    }
}

impl<const CACHE_LOCKED: bool> Cacheable for CachedSimplePage<CACHE_LOCKED> {
    type Target = SimplePageData;

    fn from_aligned_bytes(mut data: AlignedVec, path: &Path) -> Result<SimplePageData, Error> {
        let (header, manifest) = PageHeader::ref_from_prefix(&data)
            .map_err(|_| ErrorKind::ArchiveRead("missing Simple API page header".into()))?;
        let generation = std::str::from_utf8(&header.generation)
            .map_err(|err| ErrorKind::ArchiveRead(err.to_string()))?
            .parse()
            .map_err(ErrorKind::ArchiveRead)?;
        let timestamp = header.modified_nanos.get();
        let len = manifest.len();
        data.copy_within(size_of::<PageHeader>().., 0);
        data.resize(len, 0);
        let manifest = OwnedArchive::<Manifest>::new(data)?;
        let width = manifest.releases_per_block.to_native() as usize;
        if width == 0 || manifest.blocks.len() != manifest.releases.len().div_ceil(width) {
            return Err(ErrorKind::ArchiveRead("invalid release block count".into()).into());
        }

        let mut expected_len = 0;
        for block in manifest.blocks.iter() {
            let end = block.end.to_native();
            if end <= expected_len {
                return Err(ErrorKind::ArchiveRead("invalid release block bounds".into()).into());
            }
            expected_len = end;
        }
        let mut page = SimplePageData {
            generation,
            manifest,
            details: Details::Inline,
        };
        let invalid_len = || ErrorKind::ArchiveRead("invalid release data length".into());
        let mut reader = if !page.manifest.inline.is_empty() || expected_len == 0 {
            if page.manifest.inline.len() as u64 != expected_len {
                return Err(invalid_len().into());
            }
            page.reader()?
        } else {
            let path = simple_detail_path(path, &generation);
            let metadata = fs_err::metadata(&path).map_err(ErrorKind::Io)?;
            if metadata.len() != expected_len {
                return Err(invalid_len().into());
            }
            if CACHE_LOCKED {
                if timestamp != 0 && SimplePageData::modified_nanos(&metadata) == timestamp {
                    page.details = Details::Disk(path);
                    return Ok(page);
                }
                let file = uv_fs::open_with_write_attributes(&path).map_err(ErrorKind::Io)?;
                page.details = Details::Disk(path);
                DetailReader::Disk(file)
            } else {
                let mut bytes = Vec::new();
                fs_err::File::open(&path)
                    .map_err(ErrorKind::Io)?
                    .take(expected_len.saturating_add(1))
                    .read_to_end(&mut bytes)
                    .map_err(ErrorKind::Io)?;
                if bytes.len() as u64 != expected_len {
                    return Err(invalid_len().into());
                }
                page.details = Details::Memory(bytes);
                page.reader()?
            }
        };
        for index in 0..page.manifest.blocks.len() {
            page.read_block(index, &mut reader)?;
        }
        // Restoring a cache may change an immutable file's timestamp. Record verification on the
        // open file so subsequent cache hits can skip reading its contents again.
        if let DetailReader::Disk(file) = reader
            && timestamp != 0
            && let Some(modified) = UNIX_EPOCH.checked_add(Duration::from_nanos(timestamp))
        {
            let _ = file.set_modified(modified);
        }
        Ok(page)
    }

    async fn serialize_for_cache(&mut self, manifest: &Path) -> Result<Cow<'_, [u8]>, Error> {
        let page = &mut self.0;
        if let Details::Memory(bytes) = &page.details {
            let path = simple_detail_path(manifest, &page.generation);
            fs_err::tokio::create_dir_all(
                path.parent()
                    .expect("release files have a package directory"),
            )
            .await
            .map_err(ErrorKind::CacheWrite)?;
            uv_fs::write_atomic(&path, bytes)
                .await
                .map_err(ErrorKind::CacheWrite)?;
            page.details = Details::Disk(path);
        }
        let timestamp = if let Details::Disk(path) = &page.details {
            SimplePageData::modified_nanos(
                &fs_err::tokio::metadata(path).await.map_err(ErrorKind::Io)?,
            )
        } else {
            0
        };
        let manifest = OwnedArchive::as_bytes(&page.manifest);
        let header = PageHeader {
            generation: *page
                .generation
                .as_bytes()
                .as_array()
                .expect("IDs have 16 bytes"),
            modified_nanos: U64::new(timestamp),
        };
        let mut bytes = Vec::with_capacity(size_of::<PageHeader>() + manifest.len());
        bytes.extend_from_slice(header.as_bytes());
        bytes.extend_from_slice(manifest);
        Ok(Cow::Owned(bytes))
    }

    fn into_target(self) -> SimplePageData {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;
    use std::io::Write as _;
    use std::str::FromStr;
    use std::time::UNIX_EPOCH;

    use anyhow::Result;
    use rkyv::util::AlignedVec;
    use uv_cache::simple_detail_path;
    use uv_normalize::PackageName;
    use uv_redacted::SafeUrl;

    use super::{CachedSimplePage, RELEASES_PER_BLOCK, SimpleDetailMetadata};
    use crate::cached_client::Cacheable;

    /// Include enough file metadata to store release details in an external pack.
    fn large_package(versions: u32) -> Result<SimpleDetailMetadata> {
        let reason = "x".repeat(64 * 1024);
        let mut html = String::new();
        for version in 1..=versions {
            writeln!(
                html,
                r#"<a href="example-{version}.0.tar.gz" data-yanked="{reason}">example-{version}.0.tar.gz</a>"#
            )?;
        }
        Ok(SimpleDetailMetadata::from_html(
            &html,
            &PackageName::from_str("example")?,
            &SafeUrl::parse("https://example.com/simple/example/")?,
        )?)
    }

    /// A cache restore can change a valid pack's mtime. After verifying the contents once,
    /// subsequent cache hits should be able to load just the requested block.
    #[tokio::test]
    async fn restored_pack_recovers_recorded_modification_time() -> Result<()> {
        let cache = tempfile::tempdir()?;
        let manifest = cache.path().join("example.rkyv");
        let mut page = CachedSimplePage::<true>::from_metadata(large_package(1)?)?;
        let mut bytes = AlignedVec::new();
        bytes.extend_from_slice(&page.serialize_for_cache(&manifest).await?);

        let pack = simple_detail_path(&manifest, &page.0.generation);
        let recorded = fs_err::metadata(&pack)?.modified()?;
        fs_err::OpenOptions::new()
            .write(true)
            .open(&pack)?
            .set_modified(UNIX_EPOCH)?;

        CachedSimplePage::<true>::from_aligned_bytes(bytes, &manifest)?;
        assert_eq!(fs_err::metadata(&pack)?.modified()?, recorded);
        Ok(())
    }

    /// A speculative read can fail while another release in the same pack is still usable.
    #[tokio::test]
    async fn corrupt_block_leaves_other_releases_readable() -> Result<()> {
        let cache = tempfile::tempdir()?;
        let manifest = cache.path().join("example.rkyv");
        let mut page =
            CachedSimplePage::<true>::from_metadata(large_package(RELEASES_PER_BLOCK + 1)?)?;
        let mut bytes = AlignedVec::new();
        bytes.extend_from_slice(&page.serialize_for_cache(&manifest).await?);

        let pack = simple_detail_path(&manifest, &page.0.generation);
        let mut file = fs_err::OpenOptions::new().write(true).open(&pack)?;
        let timestamp = file.metadata()?.modified()?;
        file.write_all(b"bad")?;
        file.set_modified(timestamp)?;
        drop(file);

        let snapshot = CachedSimplePage::<true>::from_aligned_bytes(bytes.clone(), &manifest)?;
        assert!(snapshot.read_block(0, &mut snapshot.reader()?).is_err());
        let intact = snapshot.read_block(1, &mut snapshot.reader()?)?;
        assert_eq!(intact.len(), 1);
        assert!(CachedSimplePage::<true>::from_aligned_bytes(bytes, &manifest).is_err());
        Ok(())
    }

    /// Cleanup can delete the external generation immediately after a lockless cache hit.
    #[tokio::test]
    async fn lockless_page_snapshots_external_details() -> Result<()> {
        let cache = tempfile::tempdir()?;
        let manifest = cache.path().join("example.rkyv");
        let mut page = CachedSimplePage::<true>::from_metadata(large_package(1)?)?;
        let mut bytes = AlignedVec::new();
        bytes.extend_from_slice(&page.serialize_for_cache(&manifest).await?);

        let snapshot = CachedSimplePage::<false>::from_aligned_bytes(bytes, &manifest)?;
        fs_err::remove_file(simple_detail_path(&manifest, &page.0.generation))?;
        let mut reader = snapshot.reader()?;
        let files = snapshot.read_block(0, &mut reader)?;
        assert_eq!(files.len(), 1);
        Ok(())
    }

    /// A lockless writer must publish one self-contained file: a concurrent prune could
    /// otherwise remove its detail file before the writer publishes the manifest.
    #[tokio::test]
    async fn lockless_page_is_published_inline() -> Result<()> {
        let cache = tempfile::tempdir()?;
        let manifest = cache.path().join("example.rkyv");
        let mut page = CachedSimplePage::<false>::from_metadata(large_package(1)?)?;
        let mut bytes = AlignedVec::new();
        bytes.extend_from_slice(&page.serialize_for_cache(&manifest).await?);
        assert!(!simple_detail_path(&manifest, &page.0.generation).exists());
        let snapshot = CachedSimplePage::<true>::from_aligned_bytes(bytes, &manifest)?;
        let mut reader = snapshot.reader()?;
        let files = snapshot.read_block(0, &mut reader)?;
        assert_eq!(files.len(), 1);
        Ok(())
    }
}
