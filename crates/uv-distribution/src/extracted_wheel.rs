use std::cmp::Reverse;
use std::fmt::Display;
use std::io;
use std::path::Path;

use either::Either;
use rayon::in_place_scope;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use tempfile::TempDir;
use tokio::io::AsyncRead;
use tokio::task::JoinHandle;

use uv_cache::{ArchiveFileId, ArchiveId, Cache};
use uv_extract::dirhash::{DirectoryDigest, DirhashTree, HashedFile, UnhashedFile, dirhash_path};
use uv_fs::PortablePath;
use uv_install_wheel::validate_and_heal_record;
use uv_threads::initialize_rayon_once;

use crate::Error;

/// Per-file digests and the hash tree of an extracted wheel.
struct HashedWheel {
    files: Vec<HashedFile>,
    tree: DirhashTree,
}

/// A temporary directory and configuration for extracting a wheel.
pub(crate) struct WheelExtractor {
    temp_dir: TempDir,
    content_addressed: bool,
}

/// An extracted wheel that owns its temporary directory until persistence.
pub(crate) struct ExtractedWheel {
    temp_dir: TempDir,
    files: ExtractedFiles,
}

/// Files extracted from a wheel, with or without content-addressing metadata.
enum ExtractedFiles {
    Unhashed(Vec<UnhashedFile>),
    Hashed(HashedWheel),
}

impl WheelExtractor {
    /// Create a temporary directory under the cache root for extracting a wheel.
    pub(crate) fn new(root: &Path, content_addressed: bool) -> io::Result<Self> {
        Ok(Self {
            temp_dir: tempfile::tempdir_in(root)?,
            content_addressed,
        })
    }

    /// Extract a wheel from a streaming reader, optionally retaining its per-file digests.
    ///
    /// See [`uv_extract::stream::unzip`] for buffering, cleanup, and download hash requirements.
    pub(crate) async fn extract_streaming<R>(
        self,
        reader: R,
    ) -> Result<ExtractedWheel, uv_extract::Error>
    where
        R: AsyncRead + Unpin,
    {
        if self.content_addressed {
            let (temp_dir, files, tree) =
                uv_extract::stream::unzip_and_hash(reader, self.temp_dir).await?;
            Ok(ExtractedWheel {
                temp_dir,
                files: ExtractedFiles::Hashed(HashedWheel { files, tree }),
            })
        } else {
            let (temp_dir, files) = uv_extract::stream::unzip(reader, self.temp_dir).await?;
            Ok(ExtractedWheel {
                temp_dir,
                files: ExtractedFiles::Unhashed(files),
            })
        }
    }

    /// Extract a wheel from a seekable file, optionally retaining its per-file digests.
    pub(crate) fn extract_seekable(
        self,
        reader: fs_err::File,
    ) -> Result<ExtractedWheel, uv_extract::Error> {
        let files = if self.content_addressed {
            let (files, tree) = uv_extract::unzip_and_hash(reader, self.temp_dir.path())?;
            ExtractedFiles::Hashed(HashedWheel { files, tree })
        } else {
            let files = uv_extract::unzip(reader, self.temp_dir.path())?;
            ExtractedFiles::Unhashed(files)
        };
        Ok(ExtractedWheel {
            temp_dir: self.temp_dir,
            files,
        })
    }
}

impl ExtractedWheel {
    /// Heal `RECORD` and prepare shared file objects while owning the temporary directory.
    pub(crate) fn spawn_finalize(
        mut self,
        cache: Cache,
        dist: String,
    ) -> JoinHandle<Result<(TempDir, ArchiveId), Error>> {
        tokio::task::spawn_blocking(move || {
            self.validate_and_heal_record(dist)?;
            let (temp_dir, hashed_wheel) = self.into_parts();
            let id = if let Some(HashedWheel { files, tree }) = hashed_wheel {
                let digest = DirectoryDigest::from(tree.hash());
                persist_archive_files(&cache, temp_dir.path(), &files)
                    .map_err(Error::CacheWrite)?;
                ArchiveId::from_digest(digest.into())
            } else {
                ArchiveId::default()
            };
            Ok((temp_dir, id))
        })
    }

    /// Return the temporary directory and optional content hashes for persistence.
    fn into_parts(self) -> (TempDir, Option<HashedWheel>) {
        let hashed_wheel = match self.files {
            ExtractedFiles::Unhashed(_) => None,
            ExtractedFiles::Hashed(wheel) => Some(wheel),
        };
        (self.temp_dir, hashed_wheel)
    }

    /// Heal the wheel's `RECORD` and keep its hash tree consistent with the repaired contents.
    fn validate_and_heal_record(&mut self, dist: impl Display) -> Result<(), Error> {
        let root = self.temp_dir.path();
        let files = match &self.files {
            ExtractedFiles::Unhashed(files) => {
                Either::Left(files.iter().map(|file| (file.path(), file.size())))
            }
            ExtractedFiles::Hashed(wheel) => {
                Either::Right(wheel.files.iter().map(|file| (file.path(), file.size())))
            }
        };
        let Some(record_path) =
            validate_and_heal_record(root, files, dist).map_err(Error::InstallWheelError)?
        else {
            return Ok(());
        };
        let ExtractedFiles::Hashed(hashed_wheel) = &mut self.files else {
            return Ok(());
        };

        let hash = dirhash_path(&root.join(&record_path)).map_err(|err| {
            Error::Extract(
                record_path.display().to_string(),
                uv_extract::Error::from(err),
            )
        })?;
        let record_path = PortablePath::from(record_path.as_path()).to_string();
        hashed_wheel
            .tree
            .update_file(&record_path, hash)
            .map_err(|err| Error::Extract(record_path, uv_extract::Error::from(err)))
    }
}

/// Share extracted files other than `RECORD` while keeping the unpublished archive complete.
fn persist_archive_files(cache: &Cache, archive: &Path, files: &[HashedFile]) -> io::Result<()> {
    initialize_rayon_once();
    let targets = files
        .par_iter()
        // Keep RECORD private, since it may have been healed after hashing.
        .filter(|file| !file.path().ends_with("RECORD"))
        .map(|file| {
            let id = ArchiveFileId::from_digest(&file.object_digest_hex());
            (archive.join(file.path()), cache.archive_file(&id))
        })
        .collect::<Vec<_>>();

    // Group files by shard so its directory is created once and its files are linked by the
    // same worker, avoiding contention between workers on each shard directory.
    let mut shards: FxHashMap<&Path, Vec<_>> = FxHashMap::default();
    for (source, target) in &targets {
        let Some(parent) = target.parent() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "archive file path must have a parent directory",
            ));
        };
        shards.entry(parent).or_default().push((source, target));
    }

    let mut shards = shards
        .into_iter()
        .map(|(parent, files)| (parent, files, Ok(())))
        .collect::<Vec<_>>();
    // Start larger shards first so their work can overlap the remaining directory creation.
    shards.sort_unstable_by_key(|(_, files, _)| Reverse(files.len()));

    // Creating shards concurrently contends on their shared parent. Keep creation on this
    // thread, while workers link files in the shards that are already available.
    in_place_scope(|scope| -> io::Result<()> {
        for (parent, files, result) in &mut shards {
            fs_err::create_dir_all(parent)?;
            scope.spawn(move |_| {
                *result = files
                    .iter()
                    .try_for_each(|(source, target)| persist_archive_file(source, target));
            });
        }
        Ok(())
    })?;

    shards.into_iter().try_for_each(|(_, _, result)| result)
}

/// Publish a shared object and retain a hardlink in the archive, with a copy fallback.
fn persist_archive_file(src: &Path, dst: &Path) -> io::Result<()> {
    // The shard already exists, and most objects are new, so try linking before checking for an
    // existing object. This avoids an extra filesystem lookup for every new object.
    match fs_err::hard_link(src, dst) {
        Ok(()) => return Ok(()),
        Err(_) if dst.try_exists()? => {}
        Err(_) => return uv_fs::copy_atomic_sync(src, dst),
    }

    // This archive is still private, so it is safe to replace its extracted copy before publication.
    if let Err(err) = fs_err::remove_file(src)
        && err.kind() != io::ErrorKind::NotFound
    {
        return Err(err);
    }

    fs_err::hard_link(dst, src).or_else(|_| uv_fs::copy_atomic_sync(dst, src))
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::io;
    use std::sync::mpsc;
    use std::time::Duration;

    use tokio::runtime::Builder;
    use tokio::sync::oneshot;
    use uv_cache::Cache;

    use super::{ExtractedFiles, ExtractedWheel};

    #[test]
    fn cancelled_finalization_owns_its_temporary_directory() -> Result<(), Box<dyn Error>> {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()?;
        let cache = Cache::temp()?;
        let temp_dir = tempfile::tempdir_in(cache.root())?;
        let path = temp_dir.path().to_path_buf();
        fs_err::create_dir(path.join("demo-1.0.dist-info"))?;
        fs_err::write(path.join("demo-1.0.dist-info/RECORD"), "")?;
        let extracted = ExtractedWheel {
            temp_dir,
            files: ExtractedFiles::Unhashed(Vec::new()),
        };

        runtime.block_on(async {
            let (started, start) = oneshot::channel();
            let (release, finish) = mpsc::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                let _ = started.send(());
                finish.recv()
            });
            start.await?;

            // Queue finalization behind another worker, then abandon its result.
            let finalization = extracted.spawn_finalize(cache.clone(), "demo".to_owned());
            drop(finalization);
            assert!(path.is_dir());

            // The async executor can keep making progress while the worker is queued.
            tokio::task::yield_now().await;
            assert!(path.is_dir());
            release.send(())?;
            blocker.await??;

            tokio::time::timeout(Duration::from_secs(10), async {
                while path.try_exists()? {
                    tokio::task::yield_now().await;
                }
                Ok::<_, io::Error>(())
            })
            .await??;
            assert!(cache.root().is_dir());
            Ok(())
        })
    }
}
