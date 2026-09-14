use std::path::{Path, PathBuf};

/// Published wheels spanning Python source, application assets, and native extensions.
pub const WHEEL_FIXTURES: &[(&str, &str)] = &[
    ("flask", "flask-3.1.2-py3-none-any.whl"),
    ("jupyterlab", "jupyterlab-4.4.7-py3-none-any.whl"),
    (
        "numpy",
        "numpy-2.2.6-cp311-cp311-manylinux_2_17_aarch64.manylinux2014_aarch64.whl",
    ),
    ("sympy", "sympy-1.14.0-py3-none-any.whl"),
];

/// Filesystem-heavy workloads use walltime because simulation instruments every file operation.
pub fn is_codspeed_simulation() -> bool {
    matches!(
        std::env::var("CODSPEED_RUNNER_MODE").as_deref(),
        Ok("instrumentation" | "simulation")
    )
}

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
