use std::path::{Path, PathBuf};
use std::process::Command;

use uv_bench::uv_command_with_cache;

pub(crate) const VERSIONS: &[&str] = &["3.13.4", "3.12.11", "3.11.13", "3.10.18"];

pub(crate) fn archive_directory() -> PathBuf {
    let path = std::path::absolute("../../.cache/bench-python-archives")
        .expect("Failed to locate Python archives");
    assert!(
        path.join("manifest.json").is_file(),
        "Run `python3 scripts/benchmark/prepare-python-archives.py`"
    );
    path
}

pub(crate) fn command(directory: &Path, archive_cache: &Path) -> Command {
    let mut command = uv_command_with_cache(&directory.join("cache"));
    command
        .env("UV_PYTHON_DOWNLOADS", "manual")
        .env(
            "UV_PYTHON_DOWNLOADS_JSON_URL",
            std::path::absolute("../../scripts/benchmark/python-archives.json")
                .expect("Failed to locate Python download metadata"),
        )
        .env("UV_PYTHON_INSTALL_DIR", directory.join("python"))
        .env("UV_PYTHON_BIN_DIR", directory.join("bin"))
        .env("UV_PYTHON_CACHE_DIR", archive_cache)
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env("no_proxy", "127.0.0.1,localhost")
        .arg("--no-progress");
    command
}
