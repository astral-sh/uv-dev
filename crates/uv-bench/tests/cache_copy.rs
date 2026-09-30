use std::io;
use std::path::PathBuf;

use uv_bench::copy_cache_with_hardlinks;

#[test]
fn cache_copy_retains_independent_hardlinks() -> io::Result<()> {
    let source = tempfile::tempdir()?;
    let destination = tempfile::tempdir()?;
    let canonical = PathBuf::from("archive-v0/wheel/package.py");
    let alias = PathBuf::from("files-v0/ab/object");
    fs_err::create_dir_all(source.path().join("archive-v0/wheel"))?;
    fs_err::create_dir_all(source.path().join("files-v0/ab"))?;
    fs_err::write(source.path().join(&canonical), "original")?;
    fs_err::hard_link(source.path().join(&canonical), source.path().join(&alias))?;

    copy_cache_with_hardlinks(
        source.path(),
        destination.path(),
        &[vec![canonical.clone(), alias.clone()]],
    )?;
    fs_err::write(destination.path().join(&canonical), "changed")?;
    assert_eq!(
        fs_err::read_to_string(destination.path().join(&alias))?,
        "changed"
    );
    assert_eq!(
        fs_err::read_to_string(source.path().join(&canonical))?,
        "original"
    );
    assert_eq!(
        fs_err::read_to_string(source.path().join(&alias))?,
        "original"
    );
    Ok(())
}

#[test]
fn cache_copy_rejects_paths_outside_the_cache() -> io::Result<()> {
    let source = tempfile::tempdir()?;
    let destination = tempfile::tempdir()?;
    let error = copy_cache_with_hardlinks(
        source.path(),
        destination.path(),
        &[vec![PathBuf::from("object"), PathBuf::from("../outside")]],
    )
    .expect_err("The manifest must not escape its cache");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    Ok(())
}
