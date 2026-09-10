use std::io;
use std::sync::Barrier;
use std::thread;

use uv_fs::cachedir::ensure_tag;

const HEADER: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

#[test]
fn cachedir_tag_creation_is_idempotent() -> io::Result<()> {
    let directory = tempfile::tempdir()?;
    let tag = directory.path().join("CACHEDIR.TAG");

    ensure_tag(directory.path())?;
    assert_eq!(fs_err::read(&tag)?, HEADER);

    ensure_tag(directory.path())?;
    assert_eq!(fs_err::read(&tag)?, HEADER);

    Ok(())
}

#[test]
fn cachedir_tag_preserves_existing_contents() -> io::Result<()> {
    let directory = tempfile::tempdir()?;
    let tag = directory.path().join("CACHEDIR.TAG");
    let contents = b"An existing tag does not need a valid header.\n";
    fs_err::write(&tag, contents)?;

    ensure_tag(directory.path())?;
    assert_eq!(fs_err::read(&tag)?, contents);

    let permissions = fs_err::metadata(&tag)?.permissions();
    let mut readonly = permissions.clone();
    readonly.set_readonly(true);
    fs_err::set_permissions(&tag, readonly)?;
    let result = ensure_tag(directory.path());
    // Restore permissions before checking the result so temporary cleanup works on Windows.
    fs_err::set_permissions(&tag, permissions)?;
    result?;
    assert_eq!(fs_err::read(&tag)?, contents);

    Ok(())
}

#[test]
fn cachedir_tag_requires_an_existing_directory() -> io::Result<()> {
    let directory = tempfile::tempdir()?;
    let missing = directory.path().join("missing");

    let error = ensure_tag(&missing).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(!missing.exists());

    Ok(())
}

#[test]
fn cachedir_tag_can_be_created_concurrently() -> io::Result<()> {
    let directory = tempfile::tempdir()?;
    let barrier = Barrier::new(4);

    thread::scope(|scope| -> io::Result<()> {
        let handles = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    ensure_tag(directory.path())
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle
                .join()
                .map_err(|_| io::Error::other("cache tag creator panicked"))??;
        }
        Ok(())
    })?;

    assert_eq!(fs_err::read(directory.path().join("CACHEDIR.TAG"))?, HEADER);
    Ok(())
}
