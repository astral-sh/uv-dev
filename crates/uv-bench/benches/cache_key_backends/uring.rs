use std::hint::black_box;
use std::io;

use criterion::{BenchmarkGroup, BenchmarkId, measurement::WallTime};
use rustix::io::Errno;
use uv_cache_info::{CacheInfo, CacheInfoError, Timestamp};

use super::{BehaviorFixture, Fixture, cache_result, isolated_time, on_isolated_thread};

#[path = "statx.rs"]
mod statx;

use statx::{Configuration, Scanner, unavailable_operation};

const CONFIGURATIONS: [(&str, Configuration); 6] = [
    (
        "io-uring-1",
        Configuration {
            queue_depth: 1,
            force_async: false,
            max_workers: None,
        },
    ),
    (
        "io-uring-8",
        Configuration {
            queue_depth: 8,
            force_async: false,
            max_workers: None,
        },
    ),
    (
        "io-uring-64",
        Configuration {
            queue_depth: 64,
            force_async: false,
            max_workers: None,
        },
    ),
    (
        "io-uring-256",
        Configuration {
            queue_depth: 256,
            force_async: false,
            max_workers: None,
        },
    ),
    (
        "io-uring-async-64",
        Configuration {
            queue_depth: 64,
            force_async: true,
            max_workers: None,
        },
    ),
    (
        "io-uring-async-64-workers-16",
        Configuration {
            queue_depth: 64,
            force_async: true,
            max_workers: Some(16),
        },
    ),
];

fn unavailable_configuration(error: &io::Error) -> bool {
    unavailable_operation(error)
        || error.kind() == io::ErrorKind::PermissionDenied
        || error.kind() == io::ErrorKind::WouldBlock
        || matches!(
            error.raw_os_error(),
            Some(code)
                if code == Errno::MFILE.raw_os_error()
                    || code == Errno::NFILE.raw_os_error()
                    || code == Errno::NOMEM.raw_os_error()
                    || code == Errno::BUSY.raw_os_error()
        )
}

fn available<T>(result: io::Result<T>, context: &str) -> Option<T> {
    if let Err(error) = &result
        && unavailable_configuration(error)
    {
        eprintln!("{context}: {error}");
        return None;
    }
    Some(result.expect(context))
}

#[derive(Clone, Copy)]
struct PreparedConfiguration {
    worker_limits: Option<[u32; 2]>,
}

fn validate_configuration(
    name: &str,
    configuration: Configuration,
    fixture: &Fixture,
    behavior: &BehaviorFixture,
) -> Option<PreparedConfiguration> {
    // On Linux 5.15 the first ring used by a task determines its io-wq defaults. Keep probes and
    // each timed sample on independent submitting tasks so configurations cannot inherit limits.
    let prepared = on_isolated_thread(|| {
        let scanner = available_scanner(name, configuration, fixture, behavior)?;
        let worker_limits = available(scanner.worker_limits(), "Failed to read io-wq limits")?;
        if let Some(workers) = configuration.max_workers {
            assert_eq!(worker_limits, Some([workers, workers]), "{name}");
        }
        Some(PreparedConfiguration { worker_limits })
    });
    if let Some(prepared) = &prepared {
        eprintln!(
            "cache_key_backends/{name}/{}: active STATX, io-wq limits {:?}, {} glob leaves",
            fixture.name,
            prepared.worker_limits,
            fixture.entries.len()
        );
    } else {
        eprintln!("cache_key_backends/{name}/{}: unavailable", fixture.name);
    }
    prepared
}

fn prepared_scanner(
    configuration: Configuration,
    fixture: &Fixture,
    prepared: PreparedConfiguration,
) -> Scanner {
    let mut scanner =
        Scanner::new(configuration).expect("Active statx configuration became unavailable");
    let probe = fixture
        .entries
        .iter()
        .find(|entry| !entry.path_is_symlink() && entry.file_type().is_file())
        .expect("Source-cache fixture must contain an ordinary file");
    assert_eq!(
        scanner.probe(probe).expect("Active statx probe failed"),
        Timestamp::from_path(probe.path()).expect("Failed to inspect statx probe input")
    );
    assert_eq!(
        scanner
            .worker_limits()
            .expect("Failed to read io-wq limits"),
        prepared.worker_limits,
        "io-wq limits changed between isolated samples"
    );
    scanner.require_ring_only();

    // Warm and validate the same-configuration task's io-wq before starting the sample timer.
    let before = scanner.activity();
    assert_eq!(
        scanner
            .metadata(&fixture.entries)
            .expect("Active statx source fixture became unavailable"),
        fixture.timestamps
    );
    scanner.assert_activity(before, fixture.entries.len());
    scanner
}

fn available_scanner(
    name: &str,
    configuration: Configuration,
    fixture: &Fixture,
    behavior: &BehaviorFixture,
) -> Option<Scanner> {
    let mut scanner = available(
        Scanner::new(configuration),
        "Failed to initialize statx reader",
    )?;
    let probe = fixture
        .entries
        .iter()
        .find(|entry| !entry.path_is_symlink() && entry.file_type().is_file())
        .expect("Source-cache fixture must contain an ordinary file");
    let timestamp = available(scanner.probe(probe), "Failed to probe active statx reader")?;
    assert_eq!(
        timestamp,
        Timestamp::from_path(probe.path()).expect("Failed to inspect statx probe input"),
        "{name}"
    );

    // Exercise the ordinary diagnostic path before switching to strict benchmark-only mode.
    let timestamps = available(
        scanner.metadata(&behavior.entries),
        "Failed to read statx behavior fixture",
    )?;
    assert_eq!(timestamps, behavior.timestamps, "{name}");
    for (root, expected) in &behavior.cases {
        let actual = CacheInfo::from_directory_with_glob_collector(root, &mut scanner);
        if let Err(CacheInfoError::Io(error)) = &actual
            && unavailable_configuration(error)
        {
            return None;
        }
        assert_eq!(*expected, cache_result(actual), "{name}");
    }

    // Timed rows must contain only real, successful ring operations: no ordinary retries, and
    // exactly one successful STATX per leaf yielded by the production clustered-glob walker.
    scanner.require_ring_only();
    let before = scanner.activity();
    let timestamps = available(
        scanner.metadata(&fixture.entries),
        "Failed to read statx source fixture",
    )?;
    scanner.assert_activity(before, fixture.entries.len());
    assert_eq!(timestamps, fixture.timestamps, "{name}");

    let before = scanner.activity();
    let actual = CacheInfo::from_directory_with_glob_collector(&fixture.root, &mut scanner);
    if let Err(CacheInfoError::Io(error)) = &actual
        && unavailable_configuration(error)
    {
        return None;
    }
    assert_eq!(
        actual.expect("Failed to validate io_uring source cache info"),
        fixture.cache_info,
        "{name}"
    );
    scanner.assert_activity(before, fixture.entries.len());
    Some(scanner)
}

pub(super) fn benchmark_metadata(
    group: &mut BenchmarkGroup<'_, WallTime>,
    fixture: &Fixture,
    behavior: &BehaviorFixture,
) {
    for (name, configuration) in CONFIGURATIONS {
        let Some(prepared) = validate_configuration(name, configuration, fixture, behavior) else {
            continue;
        };
        group.bench_function(BenchmarkId::new(name, &fixture.name), |bencher| {
            bencher.iter_custom(|iterations| {
                isolated_time(
                    iterations,
                    || prepared_scanner(configuration, fixture, prepared),
                    |scanner| {
                        let before = scanner.activity();
                        let timestamps = scanner
                            .metadata(black_box(&fixture.entries))
                            .expect("Active io_uring cache-key metadata became unavailable");
                        scanner.assert_activity(before, fixture.entries.len());
                        timestamps
                    },
                )
            });
        });
    }
}

pub(super) fn benchmark_cache_info(
    group: &mut BenchmarkGroup<'_, WallTime>,
    fixture: &Fixture,
    behavior: &BehaviorFixture,
) {
    for (name, configuration) in CONFIGURATIONS {
        let Some(prepared) = validate_configuration(name, configuration, fixture, behavior) else {
            continue;
        };
        group.bench_function(BenchmarkId::new(name, &fixture.name), |bencher| {
            bencher.iter_custom(|iterations| {
                isolated_time(
                    iterations,
                    || prepared_scanner(configuration, fixture, prepared),
                    |scanner| {
                        let before = scanner.activity();
                        let cache_info = CacheInfo::from_directory_with_glob_collector(
                            black_box(&fixture.root),
                            scanner,
                        )
                        .expect("Active io_uring cache-key collection became unavailable");
                        scanner.assert_activity(before, fixture.entries.len());
                        cache_info
                    },
                )
            });
        });

        if name == "io-uring-64" {
            group.bench_function(
                BenchmarkId::new("io-uring-64-fresh-ring", &fixture.name),
                |bencher| {
                    bencher.iter_custom(|iterations| {
                        isolated_time(
                            iterations,
                            || drop(prepared_scanner(configuration, fixture, prepared)),
                            |()| {
                                // The sample's submitting task already owns the same-QD io-wq.
                                // Each operation includes ring creation and destruction, not
                                // fresh-process or first-task io-wq initialization.
                                let mut scanner = Scanner::new(configuration).expect(
                                    "Active io_uring cache-key configuration became unavailable",
                                );
                                scanner.require_ring_only();
                                let before = scanner.activity();
                                let cache_info = CacheInfo::from_directory_with_glob_collector(
                                    black_box(&fixture.root),
                                    &mut scanner,
                                )
                                .expect("Fresh io_uring cache-key collection became unavailable");
                                scanner.assert_activity(before, fixture.entries.len());
                                cache_info
                            },
                        )
                    });
                },
            );
        }
    }
}
