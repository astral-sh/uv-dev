//! Compare source cache-key leaf metadata collectors.
//!
//! `source_cache_key_metadata` excludes glob traversal and reuses initialized readers.
//! `source_cache_key_info` includes the complete production cache-info calculation, with readers
//! initialized outside timing except for explicitly named `fresh` cases. A fresh submitting thread
//! owns each sample batch; thread startup is excluded. These are not CLI or fresh-process timings.

// Keep the same allocator as uv, even though no symbols are referenced directly.
extern crate uv_performance_memory_allocator;

use std::env;
use std::hint::black_box;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use globwalk::GlobWalker;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use rayon::{ThreadPool, ThreadPoolBuilder};
use tempfile::TempDir;
use uv_cache_info::{
    CacheInfo, CacheInfoError, GlobEntryMetadata, GlobMetadataCollector, Timestamp,
};
use walkdir::DirEntry;

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
#[path = "cache_key_backends/uring.rs"]
mod uring;

type Timestamps = Vec<(PathBuf, Timestamp)>;

fn is_codspeed_simulation() -> bool {
    matches!(
        env::var("CODSPEED_RUNNER_MODE").as_deref(),
        Ok("instrumentation" | "simulation")
    )
}

fn on_isolated_thread<T: Send>(operation: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        scope
            .spawn(operation)
            .join()
            .expect("Cache-key benchmark thread panicked")
    })
}

/// Linux 5.15 shares io-wq across rings on one submitting task. Keep each sample's configured
/// ring on a fresh task, and use the same boundary for ordinary-I/O controls. Setup, thread
/// startup, and the final state destructor are excluded; operation results are dropped in time.
fn isolated_time<State, Output>(
    iterations: u64,
    setup: impl FnOnce() -> State + Send,
    mut operation: impl FnMut(&mut State) -> Output + Send,
) -> Duration {
    on_isolated_thread(move || {
        let mut state = setup();
        let start = Instant::now();
        for _ in 0..iterations {
            drop(black_box(operation(&mut state)));
        }
        start.elapsed()
    })
}

#[derive(Default)]
struct CapturingCollector {
    entries: Vec<DirEntry>,
}

impl GlobMetadataCollector for CapturingCollector {
    fn collect(
        &mut self,
        walker: GlobWalker,
    ) -> io::Result<impl Iterator<Item = GlobEntryMetadata>> {
        let entries = walker.collect::<Vec<_>>();
        self.entries.extend(
            entries
                .iter()
                .filter_map(|entry| entry.as_ref().ok().cloned()),
        );
        Ok(entries.into_iter().map(GlobEntryMetadata::read))
    }
}

struct CollectedOrdinary;

impl GlobMetadataCollector for CollectedOrdinary {
    fn collect(
        &mut self,
        walker: GlobWalker,
    ) -> io::Result<impl Iterator<Item = GlobEntryMetadata>> {
        Ok(walker
            .collect::<Vec<_>>()
            .into_iter()
            .map(GlobEntryMetadata::read))
    }
}

struct Workers<'pool>(&'pool ThreadPool);

impl GlobMetadataCollector for Workers<'_> {
    fn collect(
        &mut self,
        walker: GlobWalker,
    ) -> io::Result<impl Iterator<Item = GlobEntryMetadata>> {
        let entries = walker.collect::<Vec<_>>();
        let entries = self.0.install(|| {
            entries
                .into_par_iter()
                .map(GlobEntryMetadata::read)
                .collect::<Vec<_>>()
        });
        Ok(entries.into_iter())
    }
}

fn ordinary_metadata(entries: &[DirEntry]) -> Timestamps {
    entries
        .iter()
        .cloned()
        .map(|entry| GlobEntryMetadata::read(Ok(entry)))
        .filter_map(GlobEntryMetadata::into_timestamp)
        .collect()
}

fn worker_metadata(entries: &[DirEntry], pool: &ThreadPool) -> Timestamps {
    let entries = pool.install(|| {
        entries
            .par_iter()
            .cloned()
            .map(|entry| GlobEntryMetadata::read(Ok(entry)))
            .collect::<Vec<_>>()
    });
    entries
        .into_iter()
        .filter_map(GlobEntryMetadata::into_timestamp)
        .collect()
}

fn worker_pool(threads: usize) -> ThreadPool {
    ThreadPoolBuilder::new()
        .num_threads(threads)
        .stack_size(uv_configuration::min_stack_size())
        .build()
        .expect("Failed to create cache-key metadata worker pool")
}

fn cache_result(result: Result<CacheInfo, CacheInfoError>) -> Result<CacheInfo, String> {
    result.map_err(|error| error.to_string())
}

struct Fixture {
    _temporary: Option<TempDir>,
    name: String,
    root: PathBuf,
    entries: Vec<DirEntry>,
    timestamps: Timestamps,
    cache_info: CacheInfo,
}

impl Fixture {
    fn capture(name: String, root: PathBuf, temporary: Option<TempDir>) -> Self {
        let cache_info =
            CacheInfo::from_directory(&root).expect("Failed to compute source cache key");
        let mut collector = CapturingCollector::default();
        assert_eq!(
            cache_info,
            CacheInfo::from_directory_with_glob_collector(&root, &mut collector)
                .expect("Failed to capture source cache-key entries")
        );
        let timestamps = ordinary_metadata(&collector.entries);
        assert!(!collector.entries.is_empty());
        assert!(!timestamps.is_empty());
        eprintln!(
            "cache_key_backends fixture {name}: {} glob leaves",
            collector.entries.len()
        );
        Self {
            _temporary: temporary,
            name,
            root,
            entries: collector.entries,
            timestamps,
            cache_info,
        }
    }

    fn synthetic(file_count: usize) -> Self {
        let root = tempfile::tempdir().expect("Failed to create source-cache fixture");
        fs_err::write(
            root.path().join("pyproject.toml"),
            "[tool.uv]\ncache-keys = [{ file = \"src/**/*.py\" }]\n",
        )
        .expect("Failed to write source-cache keys");
        // This is the same shape as the existing `source_cache_key_globs` fixture.
        for index in 0..file_count {
            let directory = root.path().join(format!("src/package_{:04}", index / 100));
            fs_err::create_dir_all(&directory).expect("Failed to create source directory");
            fs_err::write(
                directory.join(format!("module_{index:04}.py")),
                b"VALUE = 1\n",
            )
            .expect("Failed to write source file");
        }
        let path = root.path().to_path_buf();
        let fixture = Self::capture(file_count.to_string(), path, Some(root));
        assert_eq!(fixture.entries.len(), file_count);
        fixture
    }

    fn uv_source() -> Self {
        let root = if let Some(root) = env::var_os("UV_BENCH_SOURCE_ROOT") {
            PathBuf::from(root)
        } else {
            env::current_dir()
                .expect("Failed to determine working directory")
                .ancestors()
                .find(|path| {
                    path.join("crates/uv-cache-info/src/cache_info.rs")
                        .is_file()
                })
                .map_or_else(
                    || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
                    Path::to_path_buf,
                )
        };
        let root = fs_err::canonicalize(root).expect("Failed to find uv source tree");
        let manifest: toml::Value = toml::from_str(
            &fs_err::read_to_string(root.join("pyproject.toml"))
                .expect("Failed to read uv's pyproject.toml"),
        )
        .expect("Failed to parse uv's pyproject.toml");
        let keys = manifest
            .get("tool")
            .and_then(|tool| tool.get("uv"))
            .and_then(|uv| uv.get("cache-keys"))
            .and_then(toml::Value::as_array)
            .expect("uv source tree must have explicit cache keys");
        for expected in ["crates/**/*.rs", "crates/**/Cargo.toml"] {
            assert!(
                keys.iter()
                    .any(|key| { key.get("file").and_then(toml::Value::as_str) == Some(expected) })
            );
        }
        Self::capture("uv-source".to_owned(), root, None)
    }

    fn throughput(&self) -> Throughput {
        Throughput::Elements(
            u64::try_from(self.entries.len()).expect("Glob entry count should fit in u64"),
        )
    }

    fn assert_cache_info(&self, collector: &mut impl GlobMetadataCollector) {
        assert_eq!(
            self.cache_info,
            CacheInfo::from_directory_with_glob_collector(&self.root, collector)
                .expect("Failed to compute source cache key with collector"),
            "{}",
            self.name
        );
    }
}

/// Small correctness cases are checked before any timed source-tree work.
struct BehaviorFixture {
    _temporary: TempDir,
    cases: Vec<(PathBuf, Result<CacheInfo, String>)>,
    entries: Vec<DirEntry>,
    timestamps: Timestamps,
}

impl BehaviorFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("Failed to create cache-key behavior fixture");
        let ordinary = root.path().join("ordinary");
        fs_err::create_dir_all(ordinary.join("src")).expect("Failed to create source directory");
        fs_err::write(
            ordinary.join("pyproject.toml"),
            "[tool.uv]\ncache-keys = [{ file = \"src/**/*.py\" }, { file = \"missing.py\" }, { dir = \"src\" }, { env = \"UV_BENCH_CACHE_KEY_VALUE\" }]\n",
        )
        .expect("Failed to write source-cache keys");
        fs_err::write(ordinary.join("src/regular.py"), b"regular\n")
            .expect("Failed to write source file");
        fs_err::write(ordinary.join("outside.py"), b"target\n")
            .expect("Failed to write symlink target");
        fs_err::create_dir(ordinary.join("outside-directory"))
            .expect("Failed to create symlink target directory");
        fs_err::write(
            ordinary.join("outside-directory/not-traversed.py"),
            b"outside\n",
        )
        .expect("Failed to write external source file");
        #[cfg(unix)]
        {
            fs_err::os::unix::fs::symlink("../outside.py", ordinary.join("src/link.py"))
                .expect("Failed to create file symlink");
            fs_err::os::unix::fs::symlink(
                "../outside-directory",
                ordinary.join("src/directory.py"),
            )
            .expect("Failed to create directory symlink");
            fs_err::os::unix::fs::symlink("../absent.py", ordinary.join("src/dangling.py"))
                .expect("Failed to create dangling symlink");
        }

        let malformed = root.path().join("malformed-manifest");
        fs_err::create_dir_all(malformed.join("src"))
            .expect("Failed to create malformed-manifest fixture");
        fs_err::write(malformed.join("pyproject.toml"), "not valid toml >.<")
            .expect("Failed to write malformed manifest");
        fs_err::write(malformed.join("setup.py"), b"# setup\n")
            .expect("Failed to write default cache-key input");

        let missing = root.path().join("missing-glob-base");
        fs_err::create_dir(&missing).expect("Failed to create missing-glob fixture");
        fs_err::write(
            missing.join("pyproject.toml"),
            "[tool.uv]\ncache-keys = [{ file = \"missing/**/*.py\" }]\n",
        )
        .expect("Failed to write missing-glob keys");

        let bad_glob = root.path().join("malformed-glob");
        fs_err::create_dir_all(bad_glob.join("src"))
            .expect("Failed to create malformed-glob fixture");
        fs_err::write(
            bad_glob.join("pyproject.toml"),
            "[tool.uv]\ncache-keys = [{ file = \"src/[\" }]\n",
        )
        .expect("Failed to write malformed-glob keys");
        assert!(CacheInfo::from_directory(&bad_glob).is_err());

        let mut collector = CapturingCollector::default();
        CacheInfo::from_directory_with_glob_collector(&ordinary, &mut collector)
            .expect("Failed to capture behavior fixture entries");
        #[cfg(unix)]
        {
            let timestamps = ordinary_metadata(&collector.entries);
            assert!(
                timestamps.contains(&(
                    ordinary.join("src/link.py"),
                    Timestamp::from_path(ordinary.join("outside.py"))
                        .expect("Failed to inspect symlink target"),
                ))
            );
            assert!(!timestamps.iter().any(|(path, _)| {
                path.ends_with("directory.py") || path.ends_with("dangling.py")
            }));
        }

        // A path removed after enumeration must retain the ordinary metadata-error behavior.
        let removed = ordinary.join("src/removed.py");
        fs_err::write(&removed, b"removed\n").expect("Failed to write removable source file");
        let entry = walkdir::WalkDir::new(&removed)
            .max_depth(0)
            .into_iter()
            .next()
            .expect("Removed-file fixture must have an entry")
            .expect("Failed to enumerate removable file");
        fs_err::remove_file(&removed).expect("Failed to remove source fixture file");
        collector.entries.push(entry);

        let timestamps = ordinary_metadata(&collector.entries);
        let cases = [ordinary, malformed, missing, bad_glob]
            .into_iter()
            .map(|path| {
                let expected = cache_result(CacheInfo::from_directory(&path));
                (path, expected)
            })
            .collect();
        Self {
            _temporary: root,
            cases,
            entries: collector.entries,
            timestamps,
        }
    }

    fn assert_collector(&self, collector: &mut impl GlobMetadataCollector) {
        for (root, expected) in &self.cases {
            assert_eq!(
                *expected,
                cache_result(CacheInfo::from_directory_with_glob_collector(
                    root, collector
                )),
                "{}",
                root.display()
            );
        }
    }
}

fn source_cache_keys(criterion: &mut Criterion) {
    if is_codspeed_simulation() {
        return;
    }

    let behavior = BehaviorFixture::new();
    behavior.assert_collector(&mut CollectedOrdinary);
    let pools = [1, 4, 16].map(|threads| {
        let pool = worker_pool(threads);
        behavior.assert_collector(&mut Workers(&pool));
        assert_eq!(
            behavior.timestamps,
            worker_metadata(&behavior.entries, &pool)
        );
        (threads, pool)
    });

    let fixtures = [
        Fixture::synthetic(100),
        Fixture::synthetic(10_000),
        Fixture::uv_source(),
    ];

    let mut group = criterion.benchmark_group("source_cache_key_metadata");
    for fixture in &fixtures {
        group.throughput(fixture.throughput());
        group.bench_function(BenchmarkId::new("ordinary", &fixture.name), |bencher| {
            bencher.iter_custom(|iterations| {
                isolated_time(
                    iterations,
                    || (),
                    |()| ordinary_metadata(black_box(&fixture.entries)),
                )
            });
        });
        for (threads, pool) in &pools {
            assert_eq!(fixture.timestamps, worker_metadata(&fixture.entries, pool));
            group.bench_function(
                BenchmarkId::new(format!("workers-{threads}"), &fixture.name),
                |bencher| {
                    bencher.iter_custom(|iterations| {
                        isolated_time(
                            iterations,
                            || (),
                            |()| worker_metadata(black_box(&fixture.entries), pool),
                        )
                    });
                },
            );
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
        uring::benchmark_metadata(&mut group, fixture, &behavior);
    }
    group.finish();

    let mut group = criterion.benchmark_group("source_cache_key_info");
    let default = tempfile::tempdir().expect("Failed to create default cache-key fixture");
    fs_err::create_dir(default.path().join("src")).expect("Failed to create default source root");
    fs_err::write(
        default.path().join("pyproject.toml"),
        "[project]\nname = \"default\"\n",
    )
    .expect("Failed to write default source manifest");
    let default_info = CacheInfo::from_directory(default.path())
        .expect("Failed to compute default source cache key");
    let mut default_collector = CapturingCollector::default();
    assert_eq!(
        default_info,
        CacheInfo::from_directory_with_glob_collector(default.path(), &mut default_collector)
            .expect("Failed to repeat default source cache key")
    );
    assert!(default_collector.entries.is_empty());
    group.bench_function(BenchmarkId::new("production", "default"), |bencher| {
        bencher.iter_custom(|iterations| {
            isolated_time(
                iterations,
                || (),
                |()| {
                    CacheInfo::from_directory(black_box(default.path()))
                        .expect("Failed to compute default source cache key")
                },
            )
        });
    });

    for fixture in &fixtures {
        group.throughput(fixture.throughput());
        group.bench_function(BenchmarkId::new("production", &fixture.name), |bencher| {
            bencher.iter_custom(|iterations| {
                isolated_time(
                    iterations,
                    || (),
                    |()| {
                        CacheInfo::from_directory(black_box(&fixture.root))
                            .expect("Failed to compute source cache key")
                    },
                )
            });
        });
        fixture.assert_cache_info(&mut CollectedOrdinary);
        group.bench_function(BenchmarkId::new("ordinary", &fixture.name), |bencher| {
            bencher.iter_custom(|iterations| {
                isolated_time(
                    iterations,
                    || (),
                    |()| {
                        CacheInfo::from_directory_with_glob_collector(
                            black_box(&fixture.root),
                            &mut CollectedOrdinary,
                        )
                        .expect("Failed to compute ordinary collected cache key")
                    },
                )
            });
        });
        for (threads, pool) in &pools {
            fixture.assert_cache_info(&mut Workers(pool));
            group.bench_function(
                BenchmarkId::new(format!("workers-{threads}"), &fixture.name),
                |bencher| {
                    bencher.iter_custom(|iterations| {
                        isolated_time(
                            iterations,
                            || (),
                            |()| {
                                CacheInfo::from_directory_with_glob_collector(
                                    black_box(&fixture.root),
                                    &mut Workers(pool),
                                )
                                .expect("Failed to compute worker-collected cache key")
                            },
                        )
                    });
                },
            );
            if *threads == 4 || *threads == 16 {
                group.bench_function(
                    BenchmarkId::new(format!("workers-{threads}-fresh"), &fixture.name),
                    |bencher| {
                        bencher.iter_custom(|iterations| {
                            isolated_time(
                                iterations,
                                || (),
                                |()| {
                                    let pool = worker_pool(*threads);
                                    CacheInfo::from_directory_with_glob_collector(
                                        black_box(&fixture.root),
                                        &mut Workers(&pool),
                                    )
                                    .expect("Failed to compute fresh worker-collected cache key")
                                },
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
        uring::benchmark_cache_info(&mut group, fixture, &behavior);
    }
    group.finish();
}

criterion_group!(cache_key_backends, source_cache_keys);
criterion_main!(cache_key_backends);
