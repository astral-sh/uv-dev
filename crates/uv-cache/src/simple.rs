use std::io::{self, Read};
use std::path::{Path, PathBuf};

use uv_fastid::Id;
use uv_fs::directories;

use crate::{Cache, CacheBucket, Removal, WheelCacheKind};

/// The immutable release details referenced by a package's Simple API manifest.
///
/// The HTTP payload starts with the 16 ASCII bytes of the generation ID, so cache pruning does
/// not need to deserialize the manifest or its HTTP cache policy.
pub fn simple_detail_path(manifest: &Path, generation: &Id) -> PathBuf {
    manifest
        .with_extension("")
        .join(&**generation)
        .with_extension("blocks")
}

impl Cache {
    /// Enumerate only real index directories. Cleanup must not traverse links outside the cache.
    pub(crate) fn simple_indexes(&self) -> io::Result<Vec<PathBuf>> {
        let bucket = self.bucket(CacheBucket::Simple);
        let mut indexes = Vec::new();
        if !bucket.is_symlink() {
            for directory in directories(&bucket)? {
                if directory.ends_with(WheelCacheKind::Pypi) {
                    indexes.push(directory);
                } else if directory.ends_with(WheelCacheKind::Index) {
                    indexes.extend(directories(directory)?);
                }
            }
        }
        Ok(indexes)
    }

    /// Prune after readers release their shared cache lock. A reader that opened a manifest before
    /// an index refresh can still request blocks from the generation it originally saw.
    pub(crate) fn prune_simple_details(&self) -> io::Result<Removal> {
        let mut summary = self.removal();
        for index in self.simple_indexes()? {
            for directory in directories(index)? {
                let manifest = directory.with_extension("rkyv");
                let current = read_generation(&manifest)?
                    .map(|generation| simple_detail_path(&manifest, &generation));
                let mut retained = false;
                for entry in fs_err::read_dir(&directory)? {
                    let file = entry?.path();
                    if Some(file.as_path()) == current.as_deref() {
                        retained = true;
                    } else {
                        summary += self.remove_path(file)?;
                    }
                }
                if !retained {
                    summary += self.remove_path(directory)?;
                }
            }
        }
        Ok(summary)
    }
}

/// An abandoned write can leave a pack without a manifest. An incomplete manifest cannot
/// reference a usable pack either.
fn read_generation(manifest: &Path) -> io::Result<Option<Id>> {
    match fs_err::symlink_metadata(manifest) {
        Ok(metadata) if metadata.is_file() => (),
        Ok(_) => return Ok(None),
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    }
    let mut generation = [0; 16];
    let mut file = fs_err::File::open(manifest)?;
    match file.read_exact(&mut generation) {
        Ok(()) => Ok(str::from_utf8(&generation)
            .ok()
            .and_then(|id| id.parse().ok())),
        Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(err) => Err(err),
    }
}
