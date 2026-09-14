use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

/// Run an optimized uv binary without inheriting user-specific uv configuration.
pub fn uv_command() -> Command {
    let root = std::path::absolute("../..").expect("Failed to locate repository root");
    let binary = std::env::var_os("UV_BENCH_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            root.join("target/profiling")
                .join(format!("uv{}", std::env::consts::EXE_SUFFIX))
        });
    assert!(
        binary.is_file(),
        "Missing benchmark binary {}. Run `cargo build --locked --profile profiling --bin uv`.",
        binary.display()
    );
    let mut command = Command::new(binary);
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("UV_") {
            command.env_remove(name);
        }
    }
    command
        .env_remove("VIRTUAL_ENV")
        .env_remove("CONDA_PREFIX")
        .env("UV_PYTHON_DOWNLOADS", "never")
        .args(["--no-config", "--cache-dir"])
        .arg(root.join(".cache"))
        .stdout(Stdio::null());
    command
}

/// Execute a benchmark command, retaining errors for failed fixture setup or invocations.
pub fn run_command(command: &mut Command) {
    let output = command
        .output()
        .expect("Failed to execute benchmark command");
    assert!(
        output.status.success(),
        "Benchmark command {command:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A frozen Prefect runtime environment, installed from the prepared package cache.
pub struct PreparedEnvironment {
    directory: tempfile::TempDir,
    python: PathBuf,
}

impl PreparedEnvironment {
    /// Install the real runtime dependency graph without building the Prefect checkout.
    pub fn prefect() -> Self {
        let directory = tempfile::tempdir().expect("Failed to create project directory");
        fs_err::copy(
            fixture_path("prefect.pyproject.toml"),
            directory.path().join("pyproject.toml"),
        )
        .expect("Failed to copy project metadata");
        fs_err::copy(
            fixture_path("prefect.lock"),
            directory.path().join("uv.lock"),
        )
        .expect("Failed to copy project lockfile");
        let mut environment = Self {
            directory,
            python: PathBuf::new(),
        };
        run_command(environment.command().args([
            "sync",
            "--frozen",
            "--no-default-groups",
            "--no-install-project",
            "--managed-python",
            "--python",
            "3.11.13",
        ]));
        let output = environment
            .command()
            .args(["python", "find"])
            .arg(environment.directory.path().join(".venv"))
            .stdout(Stdio::piped())
            .output()
            .expect("Failed to locate environment Python");
        assert!(
            output.status.success(),
            "Failed to locate environment Python"
        );
        environment.python = PathBuf::from(
            String::from_utf8(output.stdout)
                .expect("Python path is not UTF-8")
                .trim(),
        );
        assert!(
            environment.python.starts_with(environment.directory.path()),
            "Python must belong to the benchmark environment"
        );
        environment
    }

    /// Return an offline command using this project and the pinned managed interpreter.
    pub fn command(&self) -> Command {
        let mut command = uv_command();
        command
            .env(
                "UV_PYTHON_INSTALL_DIR",
                std::path::absolute("../../.cache/bench-python")
                    .expect("Failed to locate benchmark Python directory"),
            )
            .args(["--offline", "--project"])
            .arg(self.directory.path());
        command
    }

    /// The virtual environment's Python executable.
    pub fn python(&self) -> &Path {
        &self.python
    }
}
