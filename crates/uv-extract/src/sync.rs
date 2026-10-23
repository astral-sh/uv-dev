use std::path::{Path, PathBuf};

use crate::Error;
use crate::dirhash::{DirhashTree, HashedFile, UnhashedFile};

/// Extract a `.zip` archive into the target directory.
///
/// Return the extracted files and their sizes.
pub fn unzip(reader: fs_err::File, target: &Path) -> Result<Vec<UnhashedFile>, Error> {
    crate::dirhash::unzip(reader, target)
}

/// Extract a `.zip` archive into the target directory while computing a hash tree of the extracted
/// files.
///
/// The tree includes canonical relative paths, contents, and explicit empty directories. ZIP
/// entries are never followed as symlinks; non-directory entries are materialized and hashed as
/// regular files.
///
/// Return the extracted files and their sizes, along with the hash tree.
pub fn unzip_and_hash(
    reader: fs_err::File,
    target: &Path,
) -> Result<(Vec<HashedFile>, DirhashTree), Error> {
    crate::dirhash::unzip_and_hash(reader, target)
}

/// Extract the top-level directory from an unpacked archive.
///
/// The specification states:
/// > A .tar.gz source distribution (sdist) contains a single top-level directory called
/// > `{name}-{version}` (e.g. foo-1.0), containing the source files of the package.
///
/// Return the path to the top-level directory.
pub fn strip_component(source: impl AsRef<Path>) -> Result<PathBuf, Error> {
    // TODO(konstin): Verify the name of the directory.
    let top_level = fs_err::read_dir(source.as_ref())
        .map_err(Error::Io)?
        .collect::<std::io::Result<Vec<fs_err::DirEntry>>>()
        .map_err(Error::Io)?;
    match top_level.as_slice() {
        [root] => Ok(root.path()),
        [] => Err(Error::EmptyArchive),
        _ => Err(Error::NonSingularArchive(
            top_level
                .into_iter()
                .map(|entry| entry.file_name())
                .collect(),
        )),
    }
}
