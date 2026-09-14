//! Compare metadata-reading strategies on the same flat file-cache shards.
//!
//! Pool and bulk-scanner initialization is outside timing. The whole-cache pruning benchmark
//! includes production setup overhead; these cases isolate directory enumeration and link counts.

use std::hint::black_box;
use std::io;
use std::path::{Path, PathBuf};

use criterion::{BenchmarkId, Criterion, Throughput, measurement::WallTime};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use rayon::{ThreadPool, ThreadPoolBuilder};
use sha2::{Digest, Sha256};

use uv_cache::{ArchiveFileId, ArchiveId, Cache, CacheBucket};
use uv_fs::HardlinkScanner;

struct Fixture {
    _cache: Cache,
    shards: Vec<PathBuf>,
    unreferenced: Vec<PathBuf>,
}

impl Fixture {
    fn new(file_count: usize) -> Self {
        let cache = super::retained_file_cache(file_count);
        let retained = cache.archive(&ArchiveId::from_digest("retained".to_owned()));
        let mut unreferenced = Vec::new();
        for index in (0..file_count).step_by(8) {
            let contents = u64::try_from(index)
                .expect("File-cache fixture index should fit in u64")
                .to_le_bytes();
            let digest = hex::encode(Sha256::digest(contents));
            fs_err::remove_file(retained.join(&digest))
                .expect("Failed to remove fixture's external hardlink");
            unreferenced.push(cache.archive_file(&ArchiveFileId::from_digest(&digest)));
        }
        unreferenced.sort_unstable();

        let mut shards = fs_err::read_dir(cache.bucket(CacheBucket::Files))
            .expect("Failed to enumerate file-cache shards")
            .map(|entry| {
                let entry = entry.expect("Failed to read file-cache shard");
                assert!(entry.file_type().expect("Failed to inspect shard").is_dir());
                entry.path()
            })
            .collect::<Vec<_>>();
        shards.sort_unstable();
        assert!(!shards.is_empty());

        Self {
            _cache: cache,
            shards,
            unreferenced,
        }
    }

    fn assert_candidates(&self, mut candidates: Vec<PathBuf>) {
        candidates.sort_unstable();
        assert_eq!(candidates, self.unreferenced);
    }
}

fn ordinary_directory(path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs_err::read_dir(path)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        match uv_fs::hardlink_count(&path) {
            Ok(1) => files.push(path),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(files)
}

fn scan_with_fallback(
    shards: &[PathBuf],
    scanner: &mut HardlinkScanner,
) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for shard in shards {
        files.extend(match scanner.files_with_one_hardlink(shard)? {
            Some(files) => files,
            None => ordinary_directory(shard)?,
        });
    }
    Ok(files)
}

fn scan_with_workers(shards: &[PathBuf], pool: &ThreadPool) -> io::Result<Vec<PathBuf>> {
    let files = pool.install(|| {
        shards
            .par_iter()
            .map(|shard| ordinary_directory(shard))
            .collect::<io::Result<Vec<_>>>()
    })?;
    Ok(files.into_iter().flatten().collect())
}

fn scan_bulk(
    shards: &[PathBuf],
    scanner: &mut HardlinkScanner,
) -> io::Result<Option<Vec<PathBuf>>> {
    let mut files = Vec::new();
    for shard in shards {
        let Some(candidates) = scanner.files_with_one_hardlink(shard)? else {
            return Ok(None);
        };
        files.extend(candidates);
    }
    Ok(Some(files))
}

pub(super) fn metadata_backends(criterion: &mut Criterion<WallTime>) {
    if super::is_codspeed_simulation() {
        return;
    }

    let mut group = criterion.benchmark_group("file_cache_metadata_backends");
    for file_count in [10_000, 100_000] {
        let fixture = Fixture::new(file_count);
        group.throughput(Throughput::Elements(
            u64::try_from(file_count).expect("File count should fit in u64"),
        ));

        let mut ordinary = HardlinkScanner::disabled();
        fixture.assert_candidates(
            scan_with_fallback(&fixture.shards, &mut ordinary)
                .expect("Failed to read ordinary hardlink counts"),
        );
        group.bench_function(BenchmarkId::new("ordinary", file_count), |bencher| {
            bencher.iter(|| {
                black_box(
                    scan_with_fallback(black_box(&fixture.shards), &mut ordinary)
                        .expect("Failed to read ordinary hardlink counts"),
                )
            });
        });

        for threads in [1, 4, 16] {
            let pool = ThreadPoolBuilder::new()
                .num_threads(threads)
                .stack_size(uv_configuration::min_stack_size())
                .build()
                .expect("Failed to create metadata-reader thread pool");
            fixture.assert_candidates(
                scan_with_workers(&fixture.shards, &pool)
                    .expect("Failed to read parallel hardlink counts"),
            );
            group.bench_function(
                BenchmarkId::new(format!("workers-{threads}"), file_count),
                |bencher| {
                    bencher.iter(|| {
                        black_box(
                            scan_with_workers(black_box(&fixture.shards), &pool)
                                .expect("Failed to read parallel hardlink counts"),
                        )
                    });
                },
            );
        }

        #[cfg(target_os = "linux")]
        let scanners = [1, 8, 64, 256].map(|queue_depth| {
            (
                format!("io-uring-{queue_depth}"),
                HardlinkScanner::new().with_queue_depth(
                    std::num::NonZeroU32::new(queue_depth).expect("non-zero queue depth"),
                ),
            )
        });
        #[cfg(not(target_os = "linux"))]
        let scanners = [("bulk".to_owned(), HardlinkScanner::new())];

        // Do not report ordinary-I/O timings under a bulk-backend label on unsupported hosts.
        for (name, mut bulk) in scanners {
            if let Some(candidates) =
                scan_bulk(&fixture.shards, &mut bulk).expect("Failed to probe bulk metadata reads")
            {
                fixture.assert_candidates(candidates);
                group.bench_function(BenchmarkId::new(name, file_count), |bencher| {
                    bencher.iter(|| {
                        black_box(
                            scan_bulk(black_box(&fixture.shards), &mut bulk)
                                .expect("Failed to read bulk hardlink counts")
                                .expect("Bulk metadata reads became unavailable"),
                        )
                    });
                });
            }
        }
    }
    group.finish();
}
