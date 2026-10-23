use std::io;
use std::path::Path;

use uv_fs::is_same_file_allow_missing;

fn assert_identity(left: &Path, right: &Path, expected: Option<bool>) {
    assert_eq!(is_same_file_allow_missing(left, right), expected);
    assert_eq!(is_same_file_allow_missing(right, left), expected);
}

#[test]
fn existing_file_identity() -> io::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let original = temp_dir.path().join("original");
    let hardlink = temp_dir.path().join("hardlink");
    let separate = temp_dir.path().join("separate");
    fs_err::write(&original, b"same contents")?;
    fs_err::hard_link(&original, &hardlink)?;
    fs_err::write(&separate, b"same contents")?;

    assert_identity(&original, &original, Some(true));
    assert_identity(&original, &hardlink, Some(true));
    assert_identity(&original, &separate, Some(false));
    Ok(())
}

#[test]
fn missing_file_identity() -> io::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let parent = temp_dir.path().join("parent");
    let other_parent = temp_dir.path().join("other-parent");
    let parent_alias = parent.join("child").join("..");
    fs_err::create_dir_all(parent.join("child"))?;
    fs_err::create_dir(&other_parent)?;
    assert_ne!(parent, parent_alias);
    assert_eq!(
        fs_err::canonicalize(&parent)?,
        fs_err::canonicalize(&parent_alias)?
    );

    let missing = parent.join("missing");
    let same_missing = parent_alias.join("missing");
    let other_name = parent_alias.join("other-name");
    let other_location = other_parent.join("missing");
    let unknown_left = temp_dir.path().join("unknown-left").join("missing");
    let unknown_right = temp_dir.path().join("unknown-right").join("missing");
    let present = parent.join("present");
    fs_err::write(&present, b"present")?;

    for path in [
        &missing,
        &same_missing,
        &other_name,
        &other_location,
        &unknown_left,
        &unknown_right,
    ] {
        assert!(!path.try_exists()?);
    }

    // Exact paths are equal even if their parent directories are also missing.
    assert_identity(&unknown_left, &unknown_left, Some(true));

    // If the files are missing, compare the existing parents and final names.
    assert_identity(&missing, &same_missing, Some(true));
    assert_identity(&missing, &other_name, Some(false));
    assert_identity(&missing, &other_location, Some(false));
    assert_identity(&present, &same_missing, Some(false));

    // Different paths with missing parents cannot be classified.
    assert_identity(&missing, &unknown_left, None);
    assert_identity(&unknown_left, &unknown_right, None);
    Ok(())
}
