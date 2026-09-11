//! Wall-time comparisons of the eager installed-wheel sidecar read layer.
//!
//! These cases read `uv_cache.json`, `uv_build.json`, and `direct_url.json` to EOF, but exclude
//! directory enumeration, name parsing, JSON decoding, and index insertion. The `fs_metadata`
//! benchmark measures the complete production index. Set `UV_BENCH_SITE_PACKAGES` to include an
//! existing, read-only site-packages directory alongside the generated fixtures.
//!
//! The steady-state group reuses its reader or pool within each timed batch. The `_setup` group
//! includes a fresh pool or ring for every operation. Each batch runs on a fresh submitting thread
//! with a same-configuration warmup before the timer, so fresh-ring cases do not measure a fresh
//! process or the submitting task's initial io-wq setup.

// Keep the same allocator as uv, even though no symbols are referenced directly.
extern crate uv_performance_memory_allocator;

use std::env;
use std::hint::black_box;
use std::io;
use std::io::Read;
use std::path::{Path, PathBuf};

#[cfg(all(
    target_os = "linux",
    any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64",
        target_arch = "loongarch64",
        target_arch = "powerpc64"
    )
))]
use std::{fmt, io::Write, num::NonZeroU32};

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

#[path = "installed_sidecar_reads/timing.rs"]
mod timing;

#[cfg(all(
    target_os = "linux",
    any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64",
        target_arch = "loongarch64",
        target_arch = "powerpc64"
    )
))]
#[path = "installed_sidecar_reads/uring.rs"]
mod uring;

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

#[cfg(all(
    target_os = "linux",
    any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64",
        target_arch = "loongarch64",
        target_arch = "powerpc64"
    )
))]
fn checked_reader(fixture: &Fixture, queue_depth: NonZeroU32) -> io::Result<uring::Reader> {
    let mut reader = uring::Reader::new(queue_depth)?;
    fixture.assert_reads(
        &reader
            .read(&fixture.paths)
            .expect("Failed to probe io_uring sidecar reads"),
    );
    Ok(reader)
}

#[cfg(all(
    target_os = "linux",
    any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64",
        target_arch = "loongarch64",
        target_arch = "powerpc64"
    )
))]
#[derive(Clone, Copy)]
struct PreparedConfiguration {
    worker_limits: Option<[u32; 2]>,
}

#[cfg(all(
    target_os = "linux",
    any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64",
        target_arch = "loongarch64",
        target_arch = "powerpc64"
    )
))]
fn benchmark_note(message: fmt::Arguments<'_>) {
    writeln!(io::stderr().lock(), "{message}")
        .expect("Failed to report sidecar benchmark metadata");
}

#[cfg(all(
    target_os = "linux",
    any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64",
        target_arch = "loongarch64",
        target_arch = "powerpc64"
    )
))]
fn validate_reader(
    backend: &str,
    fixture: &Fixture,
    queue_depth: NonZeroU32,
) -> Option<PreparedConfiguration> {
    // Linux 5.15 can retain io-wq worker limits from the first ring used by a task. Preflights
    // need independent submitting tasks just like the timed sample batches.
    let prepared = timing::isolated(|| {
        let reader = checked_reader(fixture, queue_depth)?;
        let worker_limits = reader
            .worker_limits()
            .expect("Failed to read sidecar io-wq limits");
        Ok::<_, io::Error>(PreparedConfiguration { worker_limits })
    });
    match prepared {
        Ok(prepared) => {
            let limits = if let Some([bounded, unbounded]) = prepared.worker_limits {
                format!("bounded={bounded}, unbounded={unbounded}")
            } else {
                "unavailable".to_owned()
            };
            benchmark_note(format_args!(
                "installed_sidecar_reads/{backend}/{}/{}: active OPENAT/READ, io-wq limits {limits}",
                fixture.name,
                fixture.directories.len()
            ));
            Some(prepared)
        }
        Err(error) => {
            benchmark_note(format_args!(
                "installed_sidecar_reads/{backend}/{}/{}: unavailable ({error})",
                fixture.name,
                fixture.directories.len()
            ));
            None
        }
    }
}

#[cfg(all(
    target_os = "linux",
    any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64",
        target_arch = "loongarch64",
        target_arch = "powerpc64"
    )
))]
fn prepared_reader(
    fixture: &Fixture,
    queue_depth: NonZeroU32,
    prepared: PreparedConfiguration,
) -> uring::Reader {
    let reader =
        checked_reader(fixture, queue_depth).expect("io_uring sidecar reads became unavailable");
    assert_eq!(
        reader
            .worker_limits()
            .expect("Failed to read sidecar io-wq limits"),
        prepared.worker_limits,
        "io-wq limits changed between isolated sidecar samples"
    );
    reader
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
                bencher.iter_custom(|iterations| {
                    timing::isolated_time(
                        iterations,
                        || fixture.assert_reads(&read_ordinary(&fixture.paths)),
                        |()| read_ordinary(black_box(&fixture.paths)),
                    )
                });
            });

            for threads in [1, 4, 16] {
                if include_setup {
                    group.bench_function(
                        fixture.benchmark_id(&format!("workers-{threads}")),
                        |bencher| {
                            bencher.iter_custom(|iterations| {
                                timing::isolated_time(
                                    iterations,
                                    || {
                                        let pool = worker_pool(threads);
                                        fixture.assert_reads(&read_with_workers(
                                            &fixture.paths,
                                            &pool,
                                        ));
                                    },
                                    |()| {
                                        let pool = worker_pool(threads);
                                        read_with_workers(black_box(&fixture.paths), &pool)
                                    },
                                )
                            });
                        },
                    );
                } else {
                    group.bench_function(
                        fixture.benchmark_id(&format!("workers-{threads}")),
                        |bencher| {
                            bencher.iter_custom(|iterations| {
                                timing::isolated_time(
                                    iterations,
                                    || {
                                        let pool = worker_pool(threads);
                                        fixture.assert_reads(&read_with_workers(
                                            &fixture.paths,
                                            &pool,
                                        ));
                                        pool
                                    },
                                    |pool| read_with_workers(black_box(&fixture.paths), pool),
                                )
                            });
                        },
                    );
                }
            }

            #[cfg(all(
                target_os = "linux",
                any(
                    target_arch = "x86_64",
                    target_arch = "aarch64",
                    target_arch = "riscv64",
                    target_arch = "loongarch64",
                    target_arch = "powerpc64"
                )
            ))]
            for queue_depth in [1, 8, 64, 256] {
                let queue_depth = NonZeroU32::new(queue_depth).expect("non-zero queue depth");
                let backend = if include_setup {
                    format!("io-uring-{queue_depth}-fresh-ring")
                } else {
                    format!("io-uring-{queue_depth}")
                };
                // Omit unavailable rings; ordinary-I/O time must not acquire an io_uring label.
                let Some(prepared) = validate_reader(&backend, fixture, queue_depth) else {
                    continue;
                };
                if include_setup {
                    group.bench_function(fixture.benchmark_id(&backend), |bencher| {
                        bencher.iter_custom(|iterations| {
                            timing::isolated_time(
                                iterations,
                                || {
                                    drop(prepared_reader(fixture, queue_depth, prepared));
                                },
                                |()| {
                                    let mut reader = uring::Reader::new(queue_depth)
                                        .expect("io_uring sidecar reads became unavailable");
                                    reader
                                        .read(black_box(&fixture.paths))
                                        .expect("Failed to read sidecars with io_uring")
                                },
                            )
                        });
                    });
                } else {
                    group.bench_function(fixture.benchmark_id(&backend), |bencher| {
                        bencher.iter_custom(|iterations| {
                            timing::isolated_time(
                                iterations,
                                || prepared_reader(fixture, queue_depth, prepared),
                                |reader| {
                                    reader
                                        .read(black_box(&fixture.paths))
                                        .expect("Failed to read sidecars with io_uring")
                                },
                            )
                        });
                    });
                }
            }
        }
        group.finish();
    }
}

criterion_group!(fs_reads, installed_sidecar_read_backends);
criterion_main!(fs_reads);
