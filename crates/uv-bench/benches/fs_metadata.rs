//! Wall-time benchmarks for metadata-heavy filesystem operations.
//!
//! Fixture construction, interpreter discovery, and validation are outside the timed regions.
//! These controlled file-count sweeps complement the whole-command workspace and resolver benches.

// Keep the same allocator as uv, even though no symbols are referenced directly.
extern crate uv_performance_memory_allocator;

use std::env;
use std::hint::black_box;
use std::process::Command;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use sha2::{Digest, Sha256};

use uv_cache::{ArchiveFileId, ArchiveId, Cache};
use uv_cache_info::CacheInfo;
use uv_distribution_types::Name;
use uv_installer::SitePackages;
use uv_python::{Interpreter, PythonEnvironment, Target};

#[path = "fs_metadata/hardlinks.rs"]
mod hardlinks;
#[path = "fixtures/installed_packages.rs"]
mod installed_packages;

fn is_codspeed_simulation() -> bool {
    matches!(
        env::var("CODSPEED_RUNNER_MODE").as_deref(),
        Ok("instrumentation" | "simulation")
    )
}

fn python_environment() -> PythonEnvironment {
    let executable = env::var_os("UV_BENCH_PYTHON").unwrap_or_else(|| "python3".into());
    let output = Command::new(executable)
        .args(["-I", "-c", "import sys; print(sys.executable)"])
        .output()
        .expect("Failed to run Python; set UV_BENCH_PYTHON to a working interpreter");
    assert!(output.status.success(), "Failed to query Python executable");
    let executable = String::from_utf8(output.stdout).expect("Python path is not valid UTF-8");
    let cache = Cache::temp()
        .expect("Failed to create interpreter cache")
        .init_no_wait()
        .expect("Failed to initialize interpreter cache")
        .expect("A fresh temporary cache should not be locked");
    let interpreter = Interpreter::query(executable.trim(), &cache)
        .expect("Failed to inspect Python interpreter");
    PythonEnvironment::from_interpreter(interpreter)
}

fn installed_package_sidecars(criterion: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }

    let base_environment = python_environment();
    let mut group = criterion.benchmark_group("installed_package_sidecars");
    for sidecars in [
        installed_packages::Sidecars::Missing,
        installed_packages::Sidecars::Present,
        installed_packages::Sidecars::Mixed,
    ] {
        for package_count in [50, 500, 1_024, 2_000] {
            let root = tempfile::tempdir().expect("Failed to create site-packages fixture");
            installed_packages::create(root.path(), package_count, sidecars);
            let environment = base_environment
                .clone()
                .with_target(Target::from(root.path().to_path_buf()))
                .expect("Failed to configure target environment");

            let packages = SitePackages::from_environment(&environment)
                .expect("Failed to index site-packages fixture");
            let actual: Vec<_> = packages
                .iter()
                .map(|distribution| distribution.name().to_string())
                .collect();
            let expected: Vec<_> = (0..package_count)
                .map(|index| format!("metadata-bench-{index:04}"))
                .collect();
            assert_eq!(actual, expected);
            drop(packages);

            group.bench_with_input(
                BenchmarkId::new(sidecars.name(), package_count),
                &environment,
                |bencher, environment| {
                    bencher.iter(|| {
                        let packages = SitePackages::from_environment(black_box(environment))
                            .expect("Failed to index site-packages fixture");
                        drop(black_box(packages));
                    });
                },
            );
        }
    }
    group.finish();
}

fn retained_file_cache(file_count: usize) -> Cache {
    let cache = Cache::temp().expect("Failed to create file cache fixture");
    let retained = cache.archive(&ArchiveId::from_digest("retained".to_owned()));
    fs_err::create_dir_all(&retained).expect("Failed to create retained archive");

    for index in 0..file_count {
        let contents = u64::try_from(index)
            .expect("File-cache fixture index should fit in u64")
            .to_le_bytes();
        let digest = hex::encode(Sha256::digest(contents));
        let path = cache.archive_file(&ArchiveFileId::from_digest(&digest));
        fs_err::create_dir_all(path.parent().expect("File-cache objects have a shard"))
            .expect("Failed to create file-cache shard");
        fs_err::write(&path, contents).expect("Failed to write file-cache object");
        fs_err::hard_link(&path, retained.join(digest))
            .expect("Failed to retain file-cache object");
    }
    cache
}

fn assert_retained_cache(cache: &Cache) {
    let removal = cache
        .prune_archive_files()
        .expect("Failed to scan retained file-cache objects");
    assert_eq!(removal.num_files, 0);
    assert_eq!(removal.num_dirs, 0);
}

fn prune_retained_archive_files(criterion: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }

    let mut group = criterion.benchmark_group("prune_retained_archive_files");
    for file_count in [10_000, 100_000] {
        let cache = retained_file_cache(file_count);
        assert_retained_cache(&cache);

        group.bench_with_input(
            BenchmarkId::from_parameter(file_count),
            &cache,
            |bencher, cache| bencher.iter(|| assert_retained_cache(black_box(cache))),
        );
    }
    group.finish();
}

fn source_cache_key_globs(criterion: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }

    let mut group = criterion.benchmark_group("source_cache_key_globs");
    for file_count in [100, 10_000] {
        let root = tempfile::tempdir().expect("Failed to create source-cache fixture");
        fs_err::write(
            root.path().join("pyproject.toml"),
            "[tool.uv]\ncache-keys = [{ file = \"src/**/*.py\" }]\n",
        )
        .expect("Failed to write source-cache keys");
        for index in 0..file_count {
            let directory = root.path().join(format!("src/package_{:04}", index / 100));
            fs_err::create_dir_all(&directory).expect("Failed to create source directory");
            fs_err::write(
                directory.join(format!("module_{index:04}.py")),
                b"VALUE = 1\n",
            )
            .expect("Failed to write source file");
        }

        let cache_info =
            CacheInfo::from_directory(root.path()).expect("Failed to compute source cache key");
        assert!(!cache_info.is_empty());
        assert_eq!(
            cache_info,
            CacheInfo::from_directory(root.path()).expect("Failed to repeat source cache key")
        );

        group.bench_with_input(
            BenchmarkId::from_parameter(file_count),
            root.path(),
            |bencher, root| {
                bencher.iter(|| {
                    black_box(
                        CacheInfo::from_directory(black_box(root))
                            .expect("Failed to compute source cache key"),
                    )
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    fs_metadata,
    installed_package_sidecars,
    prune_retained_archive_files,
    source_cache_key_globs,
    hardlinks::metadata_backends
);
criterion_main!(fs_metadata);
