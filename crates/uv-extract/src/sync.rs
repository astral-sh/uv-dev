use std::path::{Path, PathBuf};

use crate::Error;
use crate::dirhash::{DirhashTree, HashedFile, UnhashedFile};

/// Unzip a `.zip` archive into the target directory.
///
/// Returns the list of unpacked files and their sizes.
pub fn unzip(reader: fs_err::File, target: &Path) -> Result<Vec<UnhashedFile>, Error> {
    crate::dirhash::unzip(reader, target)
}

/// Unzip a `.zip` archive into the target directory while computing a hash tree of the extracted
/// files.
///
/// The tree includes canonical relative paths, contents, and explicit empty directories. ZIP
/// entries are never followed as symlinks; non-directory entries are materialized and hashed as
/// regular files.
///
/// Returns the list of unpacked files and their sizes, along with the hash tree.
pub fn unzip_and_hash(
    reader: fs_err::File,
    target: &Path,
) -> Result<(Vec<HashedFile>, DirhashTree), Error> {
    crate::dirhash::unzip_and_hash(reader, target)
}

/// Extract the top-level directory from an unpacked archive.
///
/// The specification says:
/// > A .tar.gz source distribution (sdist) contains a single top-level directory called
/// > `{name}-{version}` (e.g. foo-1.0), containing the source files of the package.
///
/// This function returns the path to that top-level directory.
pub fn strip_component(source: impl AsRef<Path>) -> Result<PathBuf, Error> {
    // TODO(konstin): Verify the name of the directory.
    let mut top_level = fs_err::read_dir(source.as_ref()).map_err(Error::Io)?;
    let root = top_level
        .next()
        .ok_or(Error::EmptyArchive)?
        .map_err(Error::Io)?;
    let Some(other) = top_level.next().transpose().map_err(Error::Io)? else {
        return Ok(root.path());
    };

    let mut names = vec![root.file_name(), other.file_name()];
    for entry in top_level {
        names.push(entry.map_err(Error::Io)?.file_name());
    }
    Err(Error::NonSingularArchive(names))
}
