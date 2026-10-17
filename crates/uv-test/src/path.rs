use std::borrow::Cow;
use std::io::ErrorKind;
use std::path::Path;

use uv_fs::{Simplified, verbatim_path};

/// Assert that no filesystem entry exists at `path`.
///
/// The final path component is not followed, so dangling symlinks and junctions count as present.
/// Only [`ErrorKind::NotFound`] is accepted; other metadata errors fail the assertion.
#[track_caller]
pub fn assert_path_missing(path: impl AsRef<Path>) {
    let path = path.as_ref();
    match fs_err::symlink_metadata(path) {
        Ok(metadata) => panic!(
            "Expected `{}` to be missing, found {:?}",
            path.user_display(),
            metadata.file_type()
        ),
        Err(error) => assert_eq!(
            error.kind(),
            ErrorKind::NotFound,
            "Could not read metadata for `{}`: {error}",
            path.user_display()
        ),
    }
}

/// Assert that a symlink or junction stores the expected target path.
///
/// Absolute Windows targets are compared using the same extended-length path representation.
/// Relative targets remain relative, and neither target is canonicalized or checked for existence.
#[track_caller]
pub fn assert_link_target(path: impl AsRef<Path>, expected: impl AsRef<Path>) {
    let path = path.as_ref();
    let target = match fs_err::read_link(path) {
        Ok(target) => target,
        Err(error) => panic!("Could not read link `{}`: {error}", path.user_display()),
    };
    assert_eq!(
        comparable_link_target(&target),
        comparable_link_target(expected.as_ref()),
        "Unexpected target for link `{}`",
        path.user_display()
    );
}

fn comparable_link_target(target: &Path) -> Cow<'_, Path> {
    if target.is_absolute() {
        verbatim_path(target)
    } else {
        Cow::Borrowed(target)
    }
}

#[cfg(test)]
mod tests {
    use assert_fs::prelude::*;
    #[cfg(windows)]
    use uv_fs::{Simplified, verbatim_path};

    use super::{assert_link_target, assert_path_missing};

    #[test]
    fn accepts_missing_path() -> anyhow::Result<()> {
        let directory = assert_fs::TempDir::new()?;

        assert_path_missing(directory.child("missing"));

        Ok(())
    }

    #[test]
    #[should_panic(expected = "to be missing, found")]
    fn rejects_file() {
        let directory = assert_fs::TempDir::new().expect("create temporary directory");
        let file = directory.child("file");
        file.write_str("contents").expect("create file");

        assert_path_missing(file);
    }

    #[test]
    #[should_panic(expected = "to be missing, found")]
    fn rejects_directory() {
        let directory = assert_fs::TempDir::new().expect("create temporary directory");

        assert_path_missing(directory.path());
    }

    #[test]
    #[should_panic(expected = "to be missing, found")]
    fn rejects_directory_link() {
        let directory = assert_fs::TempDir::new().expect("create temporary directory");
        let target = directory.child("target");
        target.create_dir_all().expect("create target directory");
        let link = directory.child("link");
        uv_fs::create_symlink(&target, &link).expect("create directory link");

        assert_path_missing(link);
    }

    #[test]
    #[should_panic(expected = "to be missing, found")]
    fn rejects_dangling_directory_link() {
        let directory = assert_fs::TempDir::new().expect("create temporary directory");
        let target = directory.child("target");
        target.create_dir_all().expect("create target directory");
        let link = directory.child("link");
        uv_fs::create_symlink(&target, &link).expect("create directory link");
        fs_err::remove_dir(&target).expect("remove link target");

        assert_path_missing(link);
    }

    #[test]
    #[should_panic(expected = "Could not read metadata")]
    fn rejects_other_metadata_errors() {
        assert_path_missing("invalid\0path");
    }

    #[test]
    fn accepts_link_target() -> anyhow::Result<()> {
        let directory = assert_fs::TempDir::new()?;
        let target = directory.child("target");
        target.create_dir_all()?;
        let link = directory.child("link");
        uv_fs::create_symlink(&target, &link)?;

        assert_link_target(link, target);

        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn accepts_equivalent_windows_target_representations() -> anyhow::Result<()> {
        let directory = assert_fs::TempDir::new()?;
        let target = directory.child("target");
        target.create_dir_all()?;
        let link = directory.child("link");
        uv_fs::create_symlink(&target, &link)?;

        assert_link_target(&link, target.path().simplified());
        assert_link_target(&link, verbatim_path(target.path()));
        Ok(())
    }

    #[test]
    fn accepts_dangling_link_target() -> anyhow::Result<()> {
        let directory = assert_fs::TempDir::new()?;
        let target = directory.child("target");
        target.create_dir_all()?;
        let link = directory.child("link");
        uv_fs::create_symlink(&target, &link)?;
        fs_err::remove_dir(&target)?;

        assert_link_target(link, target);

        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn compares_relative_link_target() -> anyhow::Result<()> {
        let directory = assert_fs::TempDir::new()?;
        directory.child("target").create_dir_all()?;
        let link = directory.child("link");
        uv_fs::create_symlink("target", &link)?;

        assert_link_target(link, "target");

        Ok(())
    }

    #[test]
    #[should_panic(expected = "Unexpected target for link")]
    fn rejects_incorrect_link_target() {
        let directory = assert_fs::TempDir::new().expect("create temporary directory");
        let target = directory.child("target");
        target.create_dir_all().expect("create target directory");
        let link = directory.child("link");
        uv_fs::create_symlink(&target, &link).expect("create directory link");

        assert_link_target(link, directory.child("other"));
    }

    #[test]
    #[should_panic(expected = "Could not read link")]
    fn rejects_missing_link() {
        let directory = assert_fs::TempDir::new().expect("create temporary directory");

        assert_link_target(directory.child("missing"), directory.child("target"));
    }

    #[test]
    #[should_panic(expected = "Could not read link")]
    fn rejects_non_link() {
        let directory = assert_fs::TempDir::new().expect("create temporary directory");

        assert_link_target(directory.path(), directory.child("target"));
    }
}
