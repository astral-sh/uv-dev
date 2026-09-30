//! Prune content-addressed caches populated by real project dependencies.

mod common;

use std::path::{Path, PathBuf, absolute};
use std::process::{Command, Stdio};

use criterion::{
    BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main,
    measurement::WallTime,
};
use serde::Deserialize;

use uv_bench::{
    copy_cache_with_hardlinks, is_codspeed_simulation, run_command, uv_command_with_cache,
};

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct InstalledPackage {
    name: String,
    version: String,
}

#[derive(Deserialize)]
struct Manifest {
    name: String,
    python: String,
    installed: Vec<InstalledPackage>,
    hardlinks: Vec<Vec<PathBuf>>,
}

struct Fixture {
    directory: PathBuf,
    manifest: Manifest,
}

impl Fixture {
    fn load(name: &str) -> Self {
        let directory = absolute(Path::new("../../.cache/bench-file-caches").join(name))
            .expect("Failed to locate file-cache fixture");
        let manifest: Manifest =
            serde_json::from_slice(&fs_err::read(directory.join("manifest.json")).expect(
                "Missing file cache. Run `python3 scripts/benchmark/prepare-file-caches.py`.",
            ))
            .expect("Invalid file-cache manifest");
        assert_eq!(manifest.name, name);
        assert!(!manifest.installed.is_empty());
        assert!(!manifest.hardlinks.is_empty());
        Self {
            directory,
            manifest,
        }
    }
}

struct PreparedCache {
    directory: tempfile::TempDir,
    cache: PathBuf,
    environment: PathBuf,
}

impl PreparedCache {
    fn new(fixture: &Fixture, keep_environment: bool) -> Self {
        let directory = tempfile::tempdir().expect("Failed to create file-cache directory");
        let cache = directory.path().join("cache");
        let environment = directory.path().join("environment");
        copy_cache_with_hardlinks(
            &fixture.directory.join("cache"),
            &cache,
            &fixture.manifest.hardlinks,
        )
        .expect("Failed to copy file-cache fixture");
        let prepared = Self {
            directory,
            cache,
            environment,
        };
        run_command(
            prepared
                .command()
                .args(["venv", "--managed-python", "--python"])
                .arg(&fixture.manifest.python)
                .arg(&prepared.environment),
        );
        run_command(
            prepared
                .command()
                .args([
                    "--preview-features",
                    "content-addressed-cache",
                    "pip",
                    "sync",
                    "--python",
                ])
                .arg(&prepared.environment)
                .args([
                    "--only-binary",
                    ":all:",
                    "--no-binary",
                    "dill",
                    "--build-constraints",
                ])
                .arg(
                    absolute("../../scripts/benchmark/file-cache-locks/build-constraints.txt")
                        .expect("Failed to locate build constraints"),
                )
                .args(["--require-hashes", "--link-mode", "hardlink"])
                .arg(fixture.directory.join("requirements.txt")),
        );
        prepared.assert_installed(fixture);
        if !keep_environment {
            fs_err::remove_dir_all(&prepared.environment)
                .expect("Failed to remove fixture environment");
        }
        prepared
    }

    fn command(&self) -> Command {
        let mut command = uv_command_with_cache(&self.cache);
        command
            .env(
                "UV_PYTHON_INSTALL_DIR",
                absolute("../../.cache/bench-python").expect("Failed to locate benchmark Python"),
            )
            .args(["--offline", "--no-progress"]);
        command
    }

    fn prune_command(&self, ci: bool) -> Command {
        let mut command = self.command();
        command.args(["cache", "prune"]);
        if ci {
            command.arg("--ci");
        }
        command
    }

    fn assert_installed(&self, fixture: &Fixture) {
        let output = self
            .command()
            .args(["pip", "list", "--python"])
            .arg(&self.environment)
            .args(["--format", "json"])
            .stdout(Stdio::piped())
            .output()
            .expect("Failed to inspect fixture environment");
        assert!(output.status.success());
        let installed: Vec<InstalledPackage> =
            serde_json::from_slice(&output.stdout).expect("Invalid installed package list");
        assert_eq!(installed, fixture.manifest.installed);
    }
}

fn file_cache_prune(criterion: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let mut group = criterion.benchmark_group("file_cache_prune");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    for name in ["packse", "airflow_multicloud"] {
        let fixture = Fixture::load(name);
        for (operation, ci, keep_environment) in [
            ("retained", false, true),
            ("ci_retained", true, true),
            ("ci_removed_environment", true, false),
        ] {
            if keep_environment {
                let prepared = PreparedCache::new(&fixture, true);
                run_command(&mut prepared.prune_command(ci));
                prepared.assert_installed(&fixture);
            }
            group.bench_function(BenchmarkId::new(operation, name), |bencher| {
                bencher.iter_batched(
                    || {
                        let prepared = PreparedCache::new(&fixture, keep_environment);
                        let command = prepared.prune_command(ci);
                        (prepared, command)
                    },
                    |(prepared, mut command)| {
                        run_command(&mut command);
                        prepared.directory
                    },
                    BatchSize::PerIteration,
                );
            });
        }
    }
    group.finish();
}

criterion_group! {
    name = pruning;
    config = common::walltime_criterion();
    targets = file_cache_prune
}
criterion_main!(pruning);
