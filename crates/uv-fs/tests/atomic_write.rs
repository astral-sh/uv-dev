use std::io;
use std::path::Path;

#[cfg(feature = "tokio")]
use uv_fs::write_atomic;
use uv_fs::{copy_atomic_sync, write_atomic_sync};

#[test]
fn write_atomic_sync_rejects_parentless_target() -> io::Result<()> {
    let error = write_atomic_sync(Path::new(""), "content").expect_err("target has no parent");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "Write path must have a parent");

    let tempdir = tempfile::tempdir()?;
    let target = tempdir.path().join("target");
    fs_err::write(&target, "previous")?;
    write_atomic_sync(&target, "content")?;
    assert_eq!(fs_err::read_to_string(&target)?, "content");
    Ok(())
}

#[test]
fn copy_atomic_sync_rejects_parentless_target() -> io::Result<()> {
    let tempdir = tempfile::tempdir()?;
    let source = tempdir.path().join("source");
    fs_err::write(&source, "content")?;

    let error = copy_atomic_sync(&source, Path::new("")).expect_err("target has no parent");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "Write path must have a parent");

    let target = tempdir.path().join("target");
    fs_err::write(&target, "previous")?;
    copy_atomic_sync(&source, &target)?;
    assert_eq!(fs_err::read_to_string(&source)?, "content");
    assert_eq!(fs_err::read_to_string(&target)?, "content");
    Ok(())
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn write_atomic_rejects_parentless_target() -> io::Result<()> {
    let error = write_atomic(Path::new(""), "content")
        .await
        .expect_err("target has no parent");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "Write path must have a parent");

    let tempdir = tempfile::tempdir()?;
    let target = tempdir.path().join("target");
    fs_err::write(&target, "previous")?;
    write_atomic(&target, "content").await?;
    assert_eq!(fs_err::read_to_string(&target)?, "content");
    Ok(())
}
