use std::thread;

use anyhow::Result;
use uv_cache_info::CacheInfo;

#[test]
fn deep_cache_key_globs_do_not_overflow() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let pattern = format!("{}*", "a/".repeat(2_048));
    fs_err::write(
        directory.path().join("pyproject.toml"),
        format!("[tool.uv]\ncache-keys = [{{ file = \"*\" }}, {{ file = {pattern:?} }}]\n"),
    )?;
    let path = directory.path().to_path_buf();

    // A small, explicit stack catches recursion in insertion, collection, or node destruction.
    thread::Builder::new()
        .name("cache-key-globs".to_string())
        .stack_size(256 * 1024)
        .spawn(move || CacheInfo::from_directory(&path))?
        .join()
        .expect("cache-key thread failed")?;
    Ok(())
}
