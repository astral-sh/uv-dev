//! Discover real managed and PATH interpreters with warm and cold interpreter-query caches.

mod common;

use std::ffi::OsStr;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{is_codspeed_simulation, run_command, uv_command, uv_command_with_cache};

const VERSIONS: &[&str] = &["3.10.18", "3.11.13", "3.12.11", "3.13.4"];

fn isolate(command: &mut Command) {
    for name in [
        "RUST_LOG",
        "PYTHONEXECUTABLE",
        "__PYVENV_LAUNCHER__",
        "_PYTHON_HOST_PLATFORM",
    ] {
        command.env_remove(name);
    }
}

fn configure(mut command: Command) -> Command {
    isolate(&mut command);
    command
        .env(
            "UV_PYTHON_INSTALL_DIR",
            std::path::absolute("../../.cache/bench-python")
                .expect("Failed to locate benchmark Python directory"),
        )
        .args(["--offline", "--managed-python"]);
    command
}

fn run_output(command: &mut Command) -> Output {
    let output = command
        .output()
        .expect("Failed to execute discovery command");
    assert!(
        output.status.success(),
        "Discovery command {command:?} failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    output
}

fn interpreter_path(version: &str) -> PathBuf {
    let output = run_output(
        configure(uv_command())
            .args(["python", "find", "--system", version])
            .stdout(Stdio::piped()),
    );
    PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("Interpreter path is UTF-8")
            .trim(),
    )
}

fn path_command(cache: &Path, search_path: &OsStr) -> Command {
    let mut command = uv_command_with_cache(cache);
    isolate(&mut command);
    command
        .env("UV_PYTHON_SEARCH_PATH", search_path)
        .env("UV_PYTHON_NO_REGISTRY", "1")
        .args(["--offline", "--no-managed-python"]);
    command
}

fn verify_venv(directory: &Path) {
    let python = directory.join(".venv").join(if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python"
    });
    let mut command = Command::new(python);
    isolate(&mut command);
    let output = run_output(command.args([
        "-I",
        "-c",
        "import sys; print('.'.join(map(str, sys.version_info[:3])))",
    ]));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "3.13.4");
}

struct QueryCounts {
    queries: usize,
    cache_hits: usize,
    metadata_skips: usize,
}

/// Count interpreter probes outside the timed command, including cold-versus-warm cache behavior.
fn query_counts(
    label: &str,
    mut command: Command,
    probes: &mut Vec<serde_json::Value>,
) -> QueryCounts {
    let output = run_output(command.env(
        "RUST_LOG",
        "uv_python::discovery=trace,uv_python::interpreter=trace",
    ));
    let stderr = String::from_utf8_lossy(&output.stderr);
    let queries = stderr
        .lines()
        .filter(|line| line.contains("Querying interpreter executable at "))
        .count();
    let cache_hits = stderr
        .lines()
        .filter(|line| line.contains("Found cached interpreter info for Python "))
        .count();
    let metadata_skips = stderr
        .lines()
        .filter(|line| line.contains("version resource reports Python "))
        .count();
    let counts = QueryCounts {
        queries,
        cache_hits,
        metadata_skips,
    };
    writeln!(
        io::stderr().lock(),
        "{label}: interpreter_queries={queries}, cache_hits={cache_hits}, metadata_skips={metadata_skips}",
    )
    .expect("Failed to write discovery diagnostics");
    probes.push(serde_json::json!({
        "name": label,
        "interpreter_queries": counts.queries,
        "cache_hits": counts.cache_hits,
        "metadata_skips": counts.metadata_skips,
    }));
    counts
}

fn write_probes(binary_version: &str, probes: &[serde_json::Value]) {
    if let Some(directory) = std::env::var_os("CRITERION_HOME") {
        let directory = PathBuf::from(directory);
        fs_err::create_dir_all(&directory).expect("Failed to create Criterion output directory");
        let report = serde_json::json!({
            "binary_version": binary_version,
            "probes": probes,
        });
        fs_err::write(
            directory.join("python-discovery-probes.json"),
            serde_json::to_vec_pretty(&report).expect("Failed to serialize discovery probes"),
        )
        .expect("Failed to write discovery probes");
    }
}

fn python_discovery(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let binary = run_output(uv_command().arg("--version").stdout(Stdio::piped()));
    let binary_version = String::from_utf8_lossy(&binary.stdout).trim().to_owned();
    writeln!(io::stderr().lock(), "Discovery binary: {binary_version}")
        .expect("Failed to write discovery diagnostics");
    let mut probes = Vec::new();
    let interpreters = VERSIONS
        .iter()
        .map(|version| interpreter_path(version))
        .collect::<Vec<_>>();
    let mut group = c.benchmark_group("python_discovery");
    for (name, arguments) in [
        ("find", &["python", "find", "--system", "3.11.13"][..]),
        (
            "list",
            &[
                "python",
                "list",
                "--only-installed",
                "--output-format",
                "json",
            ][..],
        ),
    ] {
        let command = || {
            let mut command = configure(uv_command());
            command.args(arguments);
            command
        };
        run_command(&mut command());
        group.bench_function(BenchmarkId::new(name, "warm"), |b| {
            b.iter_batched(
                command,
                |mut command| run_command(&mut command),
                BatchSize::PerIteration,
            );
        });
        group.bench_function(BenchmarkId::new(name, "cold"), |b| {
            b.iter_batched(
                || {
                    let cache = tempfile::tempdir().expect("Failed to create interpreter cache");
                    let mut command = configure(uv_command_with_cache(cache.path()));
                    command.args(arguments);
                    (cache, command)
                },
                |(cache, mut command)| {
                    run_command(&mut command);
                    cache
                },
                BatchSize::PerIteration,
            );
        });
    }

    // Real installations appear oldest-first, with the requested interpreter last. This covers
    // Windows `python.exe` metadata filtering without inventing executables or counting setup.
    for count in [1, 2, 4] {
        let search_path = std::env::join_paths(
            interpreters[interpreters.len() - count..]
                .iter()
                .map(|path| path.parent().expect("Interpreter has a parent directory")),
        )
        .expect("Failed to construct Python search path");
        let command = |cache: &Path| {
            let mut command = path_command(cache, &search_path);
            command.args(["python", "find", "--system", "3.13.4"]);
            command
        };
        let warm_cache = tempfile::tempdir().expect("Failed to create interpreter cache");
        let cold = query_counts(
            &format!("python_discovery/path_find/cold/{count}"),
            command(warm_cache.path()),
            &mut probes,
        );
        assert!(
            cold.queries > 0,
            "Cold discovery must query a real interpreter"
        );
        let warm = query_counts(
            &format!("python_discovery/path_find/warm/{count}"),
            command(warm_cache.path()),
            &mut probes,
        );
        assert_eq!(
            warm.queries, 0,
            "Warm discovery must not start an interpreter"
        );
        assert!(
            warm.cache_hits > 0,
            "Warm discovery must use the interpreter cache"
        );

        group.bench_function(BenchmarkId::new("path_find/warm", count), |b| {
            b.iter_batched(
                || command(warm_cache.path()),
                |mut command| run_command(&mut command),
                BatchSize::PerIteration,
            );
        });
        group.bench_function(BenchmarkId::new("path_find/cold", count), |b| {
            b.iter_batched(
                || {
                    let cache = tempfile::tempdir().expect("Failed to create interpreter cache");
                    let command = command(cache.path());
                    (cache, command)
                },
                |(cache, mut command)| {
                    run_command(&mut command);
                    cache
                },
                BatchSize::PerIteration,
            );
        });

        let venv = |cache: &Path, directory: &Path| {
            let mut command = path_command(cache, &search_path);
            command
                .args(["venv", "--no-project", "--python", "3.13.4"])
                .arg(directory.join(".venv"));
            command
        };
        let warm_cache = tempfile::tempdir().expect("Failed to create interpreter cache");
        let cold_venv = tempfile::tempdir().expect("Failed to create project directory");
        let cold = query_counts(
            &format!("python_discovery/path_venv/cold/{count}"),
            venv(warm_cache.path(), cold_venv.path()),
            &mut probes,
        );
        assert!(
            cold.queries > 0,
            "Cold virtual-environment discovery must query a real interpreter"
        );
        verify_venv(cold_venv.path());
        let warm_venv = tempfile::tempdir().expect("Failed to create project directory");
        let warm = query_counts(
            &format!("python_discovery/path_venv/warm/{count}"),
            venv(warm_cache.path(), warm_venv.path()),
            &mut probes,
        );
        assert_eq!(
            warm.queries, 0,
            "Warm virtual-environment discovery must not start an interpreter"
        );
        assert!(
            warm.cache_hits > 0,
            "Warm virtual-environment discovery must use the interpreter cache"
        );
        verify_venv(warm_venv.path());

        group.bench_function(BenchmarkId::new("path_venv/warm", count), |b| {
            b.iter_batched(
                || {
                    let directory =
                        tempfile::tempdir().expect("Failed to create project directory");
                    let command = venv(warm_cache.path(), directory.path());
                    (directory, command)
                },
                |(directory, mut command)| {
                    run_command(&mut command);
                    directory
                },
                BatchSize::PerIteration,
            );
        });
        group.bench_function(BenchmarkId::new("path_venv/cold", count), |b| {
            b.iter_batched(
                || {
                    let cache = tempfile::tempdir().expect("Failed to create interpreter cache");
                    let directory =
                        tempfile::tempdir().expect("Failed to create project directory");
                    let command = venv(cache.path(), directory.path());
                    (cache, directory, command)
                },
                |(cache, directory, mut command)| {
                    run_command(&mut command);
                    (cache, directory)
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
    write_probes(&binary_version, &probes);
}

criterion_group! {
    name = interpreters;
    config = common::walltime_criterion();
    targets = python_discovery
}
criterion_main!(interpreters);
