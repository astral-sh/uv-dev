use std::error::Error;
use std::io;

use uv_fs::{ReadDirError, read_dir_sorted};

#[test]
fn sorts_all_entries() -> Result<(), Box<dyn Error>> {
    let tempdir = tempfile::tempdir()?;
    fs_err::create_dir(tempdir.path().join("z_directory"))?;
    fs_err::write(tempdir.path().join("a_file"), "")?;

    assert_eq!(
        read_dir_sorted(tempdir.path(), |_| Ok(true))?,
        [
            tempdir.path().join("a_file"),
            tempdir.path().join("z_directory"),
        ]
    );
    Ok(())
}

#[test]
fn filters_entries_with_capturing_predicate() -> Result<(), Box<dyn Error>> {
    let tempdir = tempfile::tempdir()?;
    fs_err::create_dir(tempdir.path().join("z_directory"))?;
    fs_err::create_dir(tempdir.path().join("a_directory"))?;
    fs_err::write(tempdir.path().join("m_file"), "")?;
    let mut visited = 0;

    let paths = read_dir_sorted(tempdir.path(), |entry| {
        visited += 1;
        Ok(entry.file_type()?.is_dir())
    })?;

    assert_eq!(visited, 3);
    assert_eq!(
        paths,
        [
            tempdir.path().join("a_directory"),
            tempdir.path().join("z_directory"),
        ]
    );
    Ok(())
}

#[test]
fn accepts_empty_directory_and_rejected_entries() -> Result<(), Box<dyn Error>> {
    let tempdir = tempfile::tempdir()?;
    assert!(read_dir_sorted(tempdir.path(), |_| Ok(true))?.is_empty());

    fs_err::write(tempdir.path().join("file"), "")?;
    assert!(read_dir_sorted(tempdir.path(), |_| Ok(false))?.is_empty());
    Ok(())
}

#[test]
fn reports_directory_opening_errors() -> Result<(), Box<dyn Error>> {
    let tempdir = tempfile::tempdir()?;
    let error = read_dir_sorted(tempdir.path().join("missing"), |_| Ok(true))
        .expect_err("A missing directory should be reported");
    match error {
        ReadDirError::Open(err) => assert_eq!(err.kind(), io::ErrorKind::NotFound),
        ReadDirError::Read(err) => return Err(err.into()),
    }

    let file = tempdir.path().join("file");
    fs_err::write(&file, "")?;
    match read_dir_sorted(file, |_| Ok(true)).expect_err("A file is not a directory") {
        ReadDirError::Open(_) => {}
        ReadDirError::Read(err) => return Err(err.into()),
    }
    Ok(())
}

#[test]
fn stops_on_filter_error_without_treating_directory_as_missing() -> Result<(), Box<dyn Error>> {
    let tempdir = tempfile::tempdir()?;
    fs_err::write(tempdir.path().join("a_file"), "")?;
    fs_err::write(tempdir.path().join("z_file"), "")?;
    let mut visited = 0;

    let error = read_dir_sorted(tempdir.path(), |_| {
        visited += 1;
        Err(io::Error::new(io::ErrorKind::NotFound, "entry disappeared"))
    })
    .expect_err("The filter error should be reported");

    assert_eq!(visited, 1);
    match error {
        ReadDirError::Read(err) => assert_eq!(err.kind(), io::ErrorKind::NotFound),
        ReadDirError::Open(err) => return Err(err.into()),
    }
    Ok(())
}
