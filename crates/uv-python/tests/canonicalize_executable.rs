use anyhow::Result;

use uv_python::canonicalize_executable;

#[test]
#[cfg(unix)]
fn canonicalize_executable_resolves_unix_symlinks() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let executable = temp_dir.path().join("python");
    fs_err::write(&executable, b"inert executable")?;
    let link = temp_dir.path().join("python-link");
    fs_err::os::unix::fs::symlink(&executable, &link)?;

    assert!(link.is_absolute());
    assert_eq!(
        canonicalize_executable(&link)?,
        fs_err::canonicalize(&executable)?
    );
    Ok(())
}

#[test]
#[cfg(windows)]
fn canonicalize_executable_preserves_windows_paths() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let root = dunce::canonicalize(temp_dir.path())?;
    let executable = root.join("python.exe");
    fs_err::write(&executable, b"inert executable")?;
    fs_err::create_dir(root.join("nested"))?;
    let path = root.join("nested").join("..").join("python.exe");

    assert!(path.is_absolute());
    assert_ne!(path, executable);
    assert_eq!(canonicalize_executable(&path)?, path);
    Ok(())
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "path must be absolute")]
fn canonicalize_executable_requires_absolute_path() {
    let _ = canonicalize_executable("relative-python");
}
