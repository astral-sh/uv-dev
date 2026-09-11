#![cfg(windows)]

use std::os::windows::ffi::OsStrExt;

use uv_fs::link::{LinkError, LinkMode, LinkOptions, OnExistingDirectory, link_dir};

/// The `uv-fs` test executable does not opt into `longPathAware`, so this exercises the legacy
/// Windows path limit regardless of the machine's `LongPathsEnabled` registry setting.
#[test]
fn copy_merge_long_destination() -> Result<(), LinkError> {
    let source_dir = tempfile::tempdir()?;
    let target_dir = tempfile::tempdir()?;

    // Keep the temporary file below MAX_PATH while making the final destination exceed it.
    let mut target = dunce::simplified(target_dir.path()).to_path_buf();
    while target.as_os_str().encode_wide().count() < 100 {
        target.push("long-path-component");
    }
    fs_err::create_dir_all(&target)?;

    let filename = format!("{}.txt", "a".repeat(180));
    let destination = target.join(&filename);
    assert!(destination.as_os_str().encode_wide().count() > 260);

    fs_err::write(source_dir.path().join(&filename), "new content")?;

    // Rust's rename supports this destination even with the legacy Windows path limit.
    let original = target.join("original");
    fs_err::write(&original, "old content")?;
    fs_err::rename(&original, &destination)?;
    assert_eq!(fs_err::read_to_string(&destination)?, "old content");

    let options =
        LinkOptions::new(LinkMode::Copy).with_on_existing_directory(OnExistingDirectory::Merge);
    assert_eq!(
        link_dir(source_dir.path(), &target, &options)?,
        LinkMode::Copy
    );
    assert_eq!(fs_err::read_to_string(&destination)?, "new content");
    assert_eq!(fs_err::read_dir(&target)?.count(), 1);

    Ok(())
}
