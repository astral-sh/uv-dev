//! Benchmark installed-package discovery and indexing using the production implementation.
//!
//! Run with `cargo bench --profile profiling -p uv-bench --bench site_packages`.
//! Set `UV_BENCH_PYTHON` to select Python and `UV_BENCH_SITE_PACKAGES_ROOT` to put the
//! persistent synthetic fixtures elsewhere. Reuse the same root for before/after runs.
//! Set `UV_BENCH_DIRECTORIES_ONLY=1` with a separate root to omit excluded top-level files.

// Keep the same allocator as uv, even though no symbols are referenced directly.
extern crate uv_performance_memory_allocator;

use std::env;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::Command;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};

use uv_cache::Cache;
use uv_distribution_types::Name;
use uv_installer::SitePackages;
use uv_python::{Interpreter, PythonEnvironment, Target};

/// Use deterministic names whose creation order differs from their sorted order.
fn package_name(index: usize) -> String {
    let shuffled_index = u32::try_from(index)
        .expect("Fixture index should fit in u32")
        .wrapping_mul(2_654_435_761);
    format!("package_{shuffled_index:08x}_{index}")
}

fn create_site_packages(root: &Path, package_count: usize, directories_only: bool) -> PathBuf {
    let site_packages = root.join(format!("packages_{package_count}"));
    fs_err::create_dir_all(&site_packages).expect("Failed to create site-packages fixture");

    // Persistent paths keep directory enumeration and path lengths identical across builds.
    // Setup and metadata writes are outside the timed loop.
    for index in 0..package_count {
        let name = package_name(index);
        fs_err::create_dir_all(site_packages.join(&name))
            .expect("Failed to create package directory");
        let dist_info = site_packages.join(format!("{name}-1.0.0.dist-info"));
        fs_err::create_dir_all(&dist_info).expect("Failed to create dist-info directory");
        fs_err::write(
            dist_info.join("METADATA"),
            format!("Metadata-Version: 2.1\nName: {name}\nVersion: 1.0.0\n"),
        )
        .expect("Failed to write package metadata");
        fs_err::write(
            dist_info.join("WHEEL"),
            "Wheel-Version: 1.0\nGenerator: uv-bench\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
        )
        .expect("Failed to write wheel metadata");
        if !directories_only {
            fs_err::write(site_packages.join(format!("{name}.py")), "")
                .expect("Failed to write excluded Python module");
            fs_err::write(site_packages.join(format!("{name}.pth")), "")
                .expect("Failed to write excluded path configuration");
        }
    }

    assert_eq!(
        fs_err::read_dir(&site_packages)
            .expect("Failed to read fixture directory")
            .count(),
        package_count * if directories_only { 2 } else { 4 },
        "Fixture contains unexpected entries; use a fresh dedicated fixture root"
    );
    fs_err::canonicalize(site_packages).expect("Failed to resolve fixture path")
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

fn discover_site_packages(criterion: &mut Criterion<WallTime>) {
    let base_environment = python_environment();
    let directories_only = env::var_os("UV_BENCH_DIRECTORIES_ONLY").is_some();
    let root = env::var_os("UV_BENCH_SITE_PACKAGES_ROOT").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/site-packages-bench"),
        PathBuf::from,
    );

    let mut group = criterion.benchmark_group("site_packages_from_environment");
    for package_count in [0, 50, 500, 5_000] {
        let site_packages = create_site_packages(&root, package_count, directories_only);
        let environment = base_environment
            .clone()
            .with_target(Target::from(site_packages))
            .expect("Failed to configure target environment");

        // Validate discovery and both the names and ordering before measuring.
        let packages = SitePackages::from_environment(&environment)
            .expect("Failed to index site-packages fixture");
        let actual: Vec<_> = packages
            .iter()
            .map(|dist| dist.name().to_string())
            .collect();
        let mut expected: Vec<_> = (0..package_count)
            .map(|index| package_name(index).replace('_', "-"))
            .collect();
        expected.sort_unstable();
        assert_eq!(actual, expected);
        drop(packages);

        group.bench_with_input(
            BenchmarkId::from_parameter(package_count),
            &environment,
            |bencher, environment| {
                bencher.iter(|| {
                    let packages = SitePackages::from_environment(black_box(environment))
                        .expect("Failed to index site-packages fixture");
                    // Include construction and destruction of the actual index.
                    drop(black_box(packages));
                });
            },
        );
    }
    group.finish();
}

criterion_group!(site_packages, discover_site_packages);
criterion_main!(site_packages);
