use std::path::{Path, PathBuf};

/// Return an immutable input prepared before running the benchmark suite.
pub fn fixture_path(filename: &str) -> PathBuf {
    let path = Path::new("../../.cache/bench-fixtures").join(filename);
    assert!(
        path.is_file(),
        "Missing benchmark fixture {}. Run `python3 scripts/benchmark/prepare-fixtures.py` from the repository root.",
        path.display()
    );
    path
}
