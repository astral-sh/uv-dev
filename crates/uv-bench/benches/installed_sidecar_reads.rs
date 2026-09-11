//! Wall-time comparisons of the eager installed-wheel sidecar read layer.
//!
//! These cases read `uv_cache.json`, `uv_build.json`, and `direct_url.json` to EOF, but exclude
//! directory enumeration, name parsing, JSON decoding, and index insertion. The `fs_metadata`
//! benchmark measures the complete production index. Set `UV_BENCH_SITE_PACKAGES` to include an
//! existing, read-only site-packages directory alongside the generated fixtures.

// Keep the same allocator as uv, even though no symbols are referenced directly.
extern crate uv_performance_memory_allocator;

use std::env;
use std::hint::black_box;
use std::io;
use std::io::Read;
use std::path::{Path, PathBuf};

use criterion::{
    BenchmarkId, Criterion, Throughput, criterion_group, criterion_main, measurement::WallTime,
};
use rayon::iter::ParallelIterator;
use rayon::slice::ParallelSlice;
use rayon::{ThreadPool, ThreadPoolBuilder};

use uv_cache_info::CacheInfo;
use uv_distribution_types::{BuildInfo, InstalledDist, InstalledDistInfo, InstalledDistKind};
use uv_pypi_types::DirectUrl;

#[path = "fixtures/installed_packages.rs"]
mod installed_packages;

const SIDECAR_NAMES: [&str; 3] = ["uv_cache.json", "uv_build.json", "direct_url.json"];

type ReadResult = io::Result<Option<Vec<u8>>>;

struct Fixture {
    _root: Option<tempfile::TempDir>,
    name: &'static str,
    directories: Vec<PathBuf>,
    paths: Vec<PathBuf>,
    expected: Vec<ReadResult>,
}

impl Fixture {
    fn new(package_count: usize, sidecars: installed_packages::Sidecars) -> Self {
        let root = tempfile::tempdir().expect("Failed to create installed-sidecar fixture");
        installed_packages::create(root.path(), package_count, sidecars);
        let directories = installed_directories(root.path());
        assert_eq!(directories.len(), package_count);
        Self::from_directories(sidecars.name(), directories, Some(root))
    }

    fn external(root: &Path) -> Self {
        let directories = installed_directories(root);
        assert!(
            !directories.is_empty(),
            "UV_BENCH_SITE_PACKAGES must contain installed .dist-info distributions"
        );
        Self::from_directories("external", directories, None)
    }

    fn from_directories(
        name: &'static str,
        directories: Vec<PathBuf>,
        root: Option<tempfile::TempDir>,
    ) -> Self {
        let paths = directories
            .iter()
            .flat_map(|directory| SIDECAR_NAMES.map(|name| directory.join(name)))
            .collect::<Vec<_>>();
        let expected = read_ordinary(&paths);
        let fixture = Self {
            _root: root,
            name,
            directories,
            paths,
            expected,
        };
        fixture.assert_production_values();
        fixture
    }

    fn assert_production_values(&self) {
        let (packages, remainder) = self.expected.as_chunks::<{ SIDECAR_NAMES.len() }>();
        assert!(remainder.is_empty());
        for (directory, contents) in self.directories.iter().zip(packages) {
            let cache_info = contents[0]
                .as_ref()
                .expect("Failed to read fixture cache info")
                .as_deref()
                .map(serde_json::from_slice::<CacheInfo>)
                .transpose()
                .expect("Failed to parse fixture cache info");
            let build_info = contents[1]
                .as_ref()
                .expect("Failed to read fixture build info")
                .as_deref()
                .map(serde_json::from_slice::<BuildInfo>)
                .transpose()
                .expect("Failed to parse fixture build info");
            let direct_url = contents[2]
                .as_ref()
                .expect("Failed to read fixture direct URL")
                .as_deref()
                .map(serde_json::from_slice::<DirectUrl>)
                .transpose()
                .expect("Failed to parse fixture direct URL");

            let distribution = InstalledDist::from(
                InstalledDistInfo::try_from_path(directory)
                    .expect("Failed to read production installed-wheel metadata")
                    .expect("Fixture directory must contain an installed wheel"),
            );
            let actual = match distribution.kind {
                InstalledDistKind::Registry(distribution) => {
                    Some((distribution.cache_info, distribution.build_info, None))
                }
                InstalledDistKind::Url(distribution) => Some((
                    distribution.cache_info,
                    distribution.build_info,
                    Some(*distribution.direct_url),
                )),
                InstalledDistKind::EggInfoFile(_)
                | InstalledDistKind::EggInfoDirectory(_)
                | InstalledDistKind::LegacyEditable(_) => None,
            };
            assert_eq!(actual, Some((cache_info, build_info, direct_url)));
        }
    }

    fn assert_reads(&self, actual: &[ReadResult]) {
        assert_eq!(actual.len(), self.expected.len());
        for (actual, expected) in actual.iter().zip(&self.expected) {
            assert_eq!(comparable(actual), comparable(expected));
        }
    }

    fn benchmark_id(&self, backend: &str) -> BenchmarkId {
        BenchmarkId::new(format!("{}/{backend}", self.name), self.directories.len())
    }
}

fn installed_directories(root: &Path) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    for entry in fs_err::read_dir(root).expect("Failed to enumerate site-packages") {
        let path = entry.expect("Failed to read site-packages entry").path();
        if path
            .extension()
            .is_some_and(|extension| extension == "dist-info")
        {
            directories.push(path);
        }
    }
    directories.sort_unstable();
    directories
}

fn comparable(result: &ReadResult) -> Result<Option<&[u8]>, (io::ErrorKind, Option<i32>)> {
    match result {
        Ok(contents) => Ok(contents.as_deref()),
        Err(error) => Err((error.kind(), error.raw_os_error())),
    }
}

fn read_file(path: &Path) -> ReadResult {
    let mut file = match fs_err::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)?;
    Ok(Some(contents))
}

fn read_ordinary(paths: &[PathBuf]) -> Vec<ReadResult> {
    paths.iter().map(|path| read_file(path)).collect()
}

fn read_with_workers(paths: &[PathBuf], pool: &ThreadPool) -> Vec<ReadResult> {
    let packages = pool.install(|| {
        paths
            .par_chunks(SIDECAR_NAMES.len())
            .map(read_ordinary)
            .collect::<Vec<_>>()
    });
    packages.into_iter().flatten().collect()
}

fn worker_pool(threads: usize) -> ThreadPool {
    ThreadPoolBuilder::new()
        .num_threads(threads)
        .stack_size(uv_configuration::min_stack_size())
        .build()
        .expect("Failed to create sidecar-reader thread pool")
}

fn installed_sidecar_read_backends(criterion: &mut Criterion<WallTime>) {
    if matches!(
        env::var("CODSPEED_RUNNER_MODE").as_deref(),
        Ok("instrumentation" | "simulation")
    ) {
        return;
    }

    let mut fixtures = Vec::new();
    for sidecars in [
        installed_packages::Sidecars::Missing,
        installed_packages::Sidecars::Present,
        installed_packages::Sidecars::Mixed,
    ] {
        for package_count in [50, 500, 1_024, 2_000] {
            fixtures.push(Fixture::new(package_count, sidecars));
        }
    }
    if let Some(root) = env::var_os("UV_BENCH_SITE_PACKAGES") {
        fixtures.push(Fixture::external(Path::new(&root)));
    }

    for include_setup in [false, true] {
        let mut group = criterion.benchmark_group(if include_setup {
            "installed_sidecar_read_backends_setup"
        } else {
            "installed_sidecar_read_backends"
        });
        for fixture in &fixtures {
            group.throughput(Throughput::Elements(
                u64::try_from(fixture.directories.len()).expect("Package count should fit in u64"),
            ));
            group.bench_function(fixture.benchmark_id("ordinary"), |bencher| {
                bencher.iter(|| black_box(read_ordinary(black_box(&fixture.paths))));
            });

            for threads in [1, 4, 16] {
                let pool = worker_pool(threads);
                fixture.assert_reads(&read_with_workers(&fixture.paths, &pool));
                if include_setup {
                    drop(pool);
                    group.bench_function(
                        fixture.benchmark_id(&format!("workers-{threads}")),
                        |bencher| {
                            bencher.iter(|| {
                                let pool = worker_pool(threads);
                                black_box(read_with_workers(black_box(&fixture.paths), &pool))
                            });
                        },
                    );
                } else {
                    group.bench_function(
                        fixture.benchmark_id(&format!("workers-{threads}")),
                        |bencher| {
                            bencher.iter(|| {
                                black_box(read_with_workers(black_box(&fixture.paths), &pool))
                            });
                        },
                    );
                }
            }
        }
        group.finish();
    }
}

criterion_group!(fs_reads, installed_sidecar_read_backends);
criterion_main!(fs_reads);
