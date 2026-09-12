use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

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
    let cache = std::path::absolute("../../.cache").expect("Failed to locate benchmark cache");
    uv_command_with_cache(&cache)
}

/// Run an optimized uv binary with an isolated cache directory.
pub fn uv_command_with_cache(cache: &Path) -> Command {
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
        .arg(cache)
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

/// A loopback server replaying the prepared package artifacts and index records.
pub struct FixtureServer {
    process: Child,
    base_url: String,
}

impl FixtureServer {
    /// Start the replay server, optionally adding package records from immutable lockfiles.
    pub fn start(lockfiles: &[&str]) -> Self {
        Self::start_inner(lockfiles, false)
    }

    /// Start a replay server that accepts only AWS Signature Version 4 requests.
    pub fn start_s3(lockfiles: &[&str]) -> Self {
        Self::start_inner(lockfiles, true)
    }

    fn start_inner(lockfiles: &[&str], require_s3: bool) -> Self {
        let python_directory = std::path::absolute("../../.cache/bench-python")
            .expect("Failed to locate benchmark Python directory");
        let output = uv_command()
            .env("UV_PYTHON_INSTALL_DIR", &python_directory)
            .args([
                "--offline",
                "python",
                "find",
                "--system",
                "--managed-python",
                "3.11.13",
            ])
            .stdout(Stdio::piped())
            .output()
            .expect("Failed to locate replay-server Python");
        assert!(output.status.success(), "Replay-server Python is missing");
        let python = String::from_utf8(output.stdout).expect("Python path is not UTF-8");
        let mut command = Command::new(python.trim());
        command.arg(
            std::path::absolute("../../scripts/benchmark/serve-fixtures.py")
                .expect("Failed to locate fixture server"),
        );
        if require_s3 {
            command.arg("--require-s3");
        }
        for lockfile in lockfiles {
            command.arg("--lockfile").arg(
                std::path::absolute(fixture_path(lockfile)).expect("Failed to locate lockfile"),
            );
        }
        let mut process = command
            .stdout(Stdio::piped())
            .spawn()
            .expect("Failed to start fixture server");
        let mut base_url = String::new();
        BufReader::new(process.stdout.take().expect("Missing server output"))
            .read_line(&mut base_url)
            .expect("Failed to read fixture server address");
        let base_url = base_url.trim().to_owned();
        assert!(
            base_url.starts_with("http://127.0.0.1:"),
            "Invalid fixture server address: {base_url:?}"
        );
        Self { process, base_url }
    }

    /// Build an absolute URL on this replay server.
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// Copy a frozen project and redirect every registry source and reference to this server.
    pub fn project(&self, name: &str) -> tempfile::TempDir {
        fn replace_registries(value: &mut toml::Value, registry: &str) {
            match value {
                toml::Value::Table(table) => {
                    for (key, value) in table {
                        if key == "registry" && value.is_str() {
                            *value = toml::Value::String(registry.to_owned());
                        } else {
                            replace_registries(value, registry);
                        }
                    }
                }
                toml::Value::Array(values) => {
                    for value in values {
                        replace_registries(value, registry);
                    }
                }
                _ => {}
            }
        }

        let project = tempfile::tempdir().expect("Failed to create project directory");
        fs_err::copy(
            fixture_path(&format!("{name}.pyproject.toml")),
            project.path().join("pyproject.toml"),
        )
        .expect("Failed to copy project metadata");
        let mut lock: toml::Value = toml::from_str(
            &fs_err::read_to_string(fixture_path(&format!("{name}.lock")))
                .expect("Failed to read project lockfile"),
        )
        .expect("Invalid project lockfile");
        replace_registries(&mut lock, &self.url("/simple/"));
        fs_err::write(
            project.path().join("uv.lock"),
            toml::to_string(&lock).expect("Failed to serialize project lockfile"),
        )
        .expect("Failed to write project lockfile");
        project
    }

    /// Run uv with an isolated cache and the pinned interpreter, bypassing loopback proxies.
    pub fn command(&self, cache: &Path) -> Command {
        let mut command = uv_command_with_cache(cache);
        command
            .env("NO_PROXY", "127.0.0.1,localhost")
            .env("no_proxy", "127.0.0.1,localhost")
            .env(
                "UV_PYTHON_INSTALL_DIR",
                std::path::absolute("../../.cache/bench-python")
                    .expect("Failed to locate benchmark Python directory"),
            )
            .env("UV_PYTHON", "3.11.13")
            .env("UV_MANAGED_PYTHON", "true")
            .arg("--no-progress");
        command
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// A real, frozen dependency graph and the arguments that select its workload.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct EnvironmentFixture {
    pub name: String,
    pub project: String,
    pub python: String,
    pub sync_args: Vec<String>,
    pub installed_requirement: String,
}

/// Small, medium, and large package graphs, measured with the same Python version.
pub fn environment_fixtures() -> Vec<EnvironmentFixture> {
    serde_json::from_str(include_str!("../../../scripts/benchmark/environments.json"))
        .expect("Invalid environment fixture manifest")
}

/// A frozen project environment, installed from the prepared package cache.
pub struct PreparedEnvironment {
    directory: tempfile::TempDir,
    python: PathBuf,
    fixture: EnvironmentFixture,
    cache: PathBuf,
}

impl PreparedEnvironment {
    /// Install the real runtime dependency graph without building the Prefect checkout.
    pub fn prefect() -> Self {
        let mut fixture = environment_fixtures()
            .into_iter()
            .find(|fixture| fixture.name == "prefect")
            .expect("Missing Prefect environment fixture");
        "3.11.13".clone_into(&mut fixture.python);
        Self::from_fixture(&fixture)
    }

    /// Install a selected frozen graph without building its project checkout.
    pub fn from_fixture(fixture: &EnvironmentFixture) -> Self {
        let cache = std::path::absolute("../../.cache").expect("Failed to locate benchmark cache");
        Self::from_fixture_with_cache(fixture, &cache)
    }

    /// Install a selected frozen graph using an isolated, already populated cache.
    pub fn from_fixture_with_cache(fixture: &EnvironmentFixture, cache: &Path) -> Self {
        let directory = tempfile::tempdir().expect("Failed to create project directory");
        fs_err::copy(
            fixture_path(&format!("{}.pyproject.toml", fixture.project)),
            directory.path().join("pyproject.toml"),
        )
        .expect("Failed to copy project metadata");
        fs_err::copy(
            fixture_path(&format!("{}.lock", fixture.project)),
            directory.path().join("uv.lock"),
        )
        .expect("Failed to copy project lockfile");
        let mut environment = Self {
            directory,
            python: PathBuf::new(),
            fixture: fixture.clone(),
            cache: cache.to_owned(),
        };
        run_command(&mut environment.sync_command());
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

    /// Reconcile exactly the dependency groups used to prepare this environment.
    pub fn sync_command(&self) -> Command {
        let mut command = self.command();
        command
            .args([
                "sync",
                "--frozen",
                "--no-default-groups",
                "--no-install-project",
                "--managed-python",
                "--python",
            ])
            .arg(&self.fixture.python)
            .args(&self.fixture.sync_args);
        command
    }

    /// Return an offline command using this project and the pinned managed interpreter.
    pub fn command(&self) -> Command {
        let mut command = uv_command_with_cache(&self.cache);
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
