use std::cell::Cell;
use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[cfg(unix)]
use fs_err::os::unix::fs::symlink;
use uv_fs::{directories, entries, files};

#[test]
fn missing_directory_is_opened_before_iteration() -> io::Result<()> {
    let temporary = tempfile::tempdir()?;
    let path = temporary.path().join("missing");

    let mut found_directories = directories(path.clone())?;
    let mut found_entries = entries(path.clone())?;
    let mut found_files = files(path.clone())?;

    fs_err::create_dir(&path)?;
    fs_err::write(path.join("created-later"), b"contents")?;
    fs_err::create_dir(path.join("created-later-directory"))?;

    assert_eq!(found_directories.next(), None);
    assert_eq!(found_entries.next(), None);
    assert_eq!(found_files.next(), None);
    Ok(())
}

#[test]
fn non_directory_open_result_is_preserved() -> io::Result<()> {
    let temporary = tempfile::tempdir()?;
    let path = temporary.path().join("file");
    fs_err::write(&path, b"contents")?;
    let invalid_path = temporary.path().join("invalid\0path");

    for (path, error_required) in [(path, false), (invalid_path, true)] {
        let expected = match path.read_dir() {
            Ok(read_dir) => {
                assert_eq!(read_dir.count(), 0);
                None
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => Some(error),
        };
        if error_required {
            assert!(
                expected.is_some(),
                "a NUL-containing path must have a non-NotFound error"
            );
        }
        if let Some(expected) = expected {
            for actual in [
                directories(&path).err().expect("directories must fail"),
                entries(&path).err().expect("entries must fail"),
                files(&path).err().expect("files must fail"),
            ] {
                assert_eq!(actual.kind(), expected.kind());
                assert_eq!(actual.raw_os_error(), expected.raw_os_error());
                assert_eq!(actual.to_string(), expected.to_string());
            }
        } else {
            assert_eq!(
                [
                    directories(&path)?.count(),
                    entries(&path)?.count(),
                    files(&path)?.count(),
                ],
                [0, 0, 0]
            );
        }
    }
    Ok(())
}

#[test]
fn entries_preserve_file_type_filters() -> io::Result<()> {
    let temporary = tempfile::tempdir()?;
    let path = temporary.path();
    let directory = path.join("directory");
    let other_directory = path.join("other-directory");
    let file = path.join("file");
    let other_file = path.join("other-file");
    fs_err::create_dir(&directory)?;
    fs_err::create_dir(&other_directory)?;
    fs_err::write(&file, b"contents")?;
    fs_err::write(&other_file, b"other contents")?;

    let expected_entries = BTreeSet::from([
        directory.clone(),
        other_directory.clone(),
        file.clone(),
        other_file.clone(),
    ]);
    #[cfg(unix)]
    let expected_entries = {
        let file_link = path.join("file-link");
        let directory_link = path.join("directory-link");
        let dangling_link = path.join("dangling-link");
        symlink(&file, &file_link)?;
        symlink(&directory, &directory_link)?;
        symlink(path.join("absent-target"), &dangling_link)?;
        expected_entries
            .into_iter()
            .chain([file_link, directory_link, dangling_link])
            .collect()
    };

    assert_eq!(entries(path)?.collect::<BTreeSet<_>>(), expected_entries);
    assert_eq!(
        directories(path)?.collect::<BTreeSet<_>>(),
        BTreeSet::from([directory, other_directory])
    );
    assert_eq!(
        files(path)?.collect::<BTreeSet<_>>(),
        BTreeSet::from([file, other_file])
    );
    Ok(())
}

struct CountedPath {
    path: PathBuf,
    conversions: Rc<Cell<usize>>,
    drops: Rc<Cell<usize>>,
}

impl AsRef<Path> for CountedPath {
    fn as_ref(&self) -> &Path {
        self.conversions.set(self.conversions.get() + 1);
        &self.path
    }
}

impl Drop for CountedPath {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

fn assert_owned_path_is_dropped<I: Iterator<Item = PathBuf>>(
    path: &Path,
    open: impl FnOnce(CountedPath) -> io::Result<I>,
) -> io::Result<()> {
    let conversions = Rc::new(Cell::new(0));
    let drops = Rc::new(Cell::new(0));
    let iterator = open(CountedPath {
        path: path.to_path_buf(),
        conversions: Rc::clone(&conversions),
        drops: Rc::clone(&drops),
    })?;

    assert_eq!(conversions.get(), 1);
    assert_eq!(drops.get(), 1);
    assert_eq!(iterator.count(), 0);
    assert_eq!(conversions.get(), 1);
    assert_eq!(drops.get(), 1);
    Ok(())
}

#[test]
fn owned_path_is_dropped_before_iteration() -> io::Result<()> {
    let temporary = tempfile::tempdir()?;
    assert_owned_path_is_dropped(temporary.path(), directories)?;
    assert_owned_path_is_dropped(temporary.path(), entries)?;
    assert_owned_path_is_dropped(temporary.path(), files)?;
    Ok(())
}
