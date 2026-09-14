//! Opt-in comparisons of independent installed-wheel `RECORD` leaf deletions.
//!
//! Set `UV_BENCH_UNLINK_SCRATCH`, `UV_BENCH_WHEEL_PATH`, and `UV_BENCH_WHEEL_SHA256` together to
//! enable this target. The scratch directory must already exist and must not be owned by a
//! temporary-directory guard that could remove quarantined trials. The wheel is read-only input.
//! See `wheel_record_unlinks/README.md` for the exact operation and timing boundaries.
//!
//! These leaf backends attempt every independent path and return ordered outcomes. They do not
//! implement production's serial error precedence, recursive-directory fallback, or cleanup.
//! The separate `wheel_uninstall_wheel` group calls the complete production implementation.

// Keep the same allocator as uv, even though no symbols are referenced directly.
extern crate uv_performance_memory_allocator;

use std::collections::BTreeSet;
use std::env;
use std::fmt;
use std::io::{self, Write};

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
use std::{
    num::NonZeroU32,
    path::{Path, PathBuf},
};

use criterion::{
    BenchmarkId, Criterion, Throughput, criterion_group, criterion_main, measurement::WallTime,
};

#[path = "wheel_record_unlinks/fixture.rs"]
mod fixture;

#[path = "wheel_record_unlinks/leaf.rs"]
mod leaf;

#[path = "wheel_record_unlinks/settings.rs"]
mod settings;

#[path = "wheel_record_unlinks/timing.rs"]
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
#[path = "wheel_record_unlinks/uring.rs"]
mod uring;

use fixture::{Trial, WheelFixture};
use leaf::{Outcomes, RawOutcome};

fn benchmark_note(message: fmt::Arguments<'_>) {
    writeln!(io::stderr().lock(), "{message}")
        .expect("Failed to report wheel-unlink benchmark metadata");
}

struct Fixture {
    name: String,
    wheel: WheelFixture,
    expected: Vec<RawOutcome>,
}

impl Fixture {
    fn capture(name: String, wheel: WheelFixture) -> Self {
        let trial = wheel
            .trial()
            .expect("Failed to create raw wheel-unlink oracle");
        let parent_count = trial
            .leaves()
            .relative_paths()
            .iter()
            .map(|path| path.parent().expect("Validated leaf must have a parent"))
            .collect::<BTreeSet<_>>()
            .len();
        let results = leaf::raw(trial.leaves());
        trial
            .assert_leaf_results(&results)
            .expect("Raw wheel-unlink oracle changed an unexpected path");
        assert_eq!(leaf::successful(&results), wheel.leaf_count());
        let expected = results.iter().map(leaf::comparable_raw).collect();
        benchmark_note(format_args!(
            "wheel_record_unlinks fixture {name}: leaves={}, direct-parents={parent_count}, wheel-filename={}, wheel-sha256={}, installed-manifest-sha256={}",
            wheel.leaf_count(),
            wheel.wheel_filename(),
            wheel.sha256(),
            wheel.installed_manifest_sha256(),
        ));
        Self {
            name,
            wheel,
            expected,
        }
    }

    fn trial(&self) -> Trial {
        self.wheel
            .trial()
            .expect("Failed to recreate a byte-faithful wheel-unlink trial")
    }

    fn assert_ordinary(&self, trial: &Trial, results: &Outcomes) {
        assert_eq!(results.len(), self.expected.len());
        for (actual, expected) in results.iter().zip(&self.expected) {
            assert_eq!(leaf::comparable(actual), expected.map_err(|(kind, _)| kind));
        }
        trial
            .assert_leaf_results(results)
            .expect("Wheel-unlink leaf oracle failed");
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
    fn assert_ring(&self, trial: &Trial, results: &Outcomes) {
        assert_eq!(results.len(), self.expected.len());
        for (actual, expected) in results.iter().zip(&self.expected) {
            assert_eq!(leaf::comparable_raw(actual), *expected);
        }
        trial
            .assert_leaf_results(results)
            .expect("io_uring wheel-unlink leaf oracle failed");
    }

    fn benchmark_id(&self, backend: &str) -> BenchmarkId {
        BenchmarkId::new(backend, &self.name)
    }

    fn throughput(&self) -> Throughput {
        Throughput::Elements(
            u64::try_from(self.wheel.leaf_count()).expect("Leaf count should fit in u64"),
        )
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
fn checked_unlinker(
    fixture: &Fixture,
    scratch: &Path,
    queue_depth: NonZeroU32,
) -> io::Result<uring::Unlinker> {
    let mut unlinker = uring::Unlinker::new(queue_depth)?;
    let probe_root = tempfile::Builder::new()
        .prefix("uv-unlink-ring-probe-")
        .tempdir_in(scratch)
        .expect("Failed to create disposable UNLINKAT probe root");
    fs_err::write(probe_root.path().join("probe"), b"probe\n")
        .expect("Failed to create disposable UNLINKAT probe file");
    let probe = fixture::OwnedLeafTrial::new(probe_root, vec![PathBuf::from("probe")])
        .expect("Failed to validate disposable UNLINKAT probe");
    let before = unlinker.activity();
    let mut results = unlinker
        .unlink(&probe)
        .expect("Fatal io_uring control failure during UNLINKAT probe");
    probe
        .assert_leaf_results(&results)
        .expect("UNLINKAT probe changed an unexpected path");
    let result = results
        .pop()
        .expect("UNLINKAT probe must return one result");
    if let Err(error) = &result
        && uring::unavailable_operation(error)
    {
        return result.map(|()| unlinker);
    }
    result.expect("Disposable UNLINKAT probe failed");
    assert!(results.is_empty());
    unlinker.assert_activity(before, 1, 1);

    // A full same-configuration warmup uses a separate tree from every measured operation.
    let trial = fixture.trial();
    let results = unlinker
        .unlink(trial.leaves())
        .expect("Fatal io_uring control failure during wheel-unlink warmup");
    fixture.assert_ring(&trial, &results);
    Ok(unlinker)
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
fn prepare_configuration(
    backend: &str,
    fixture: &Fixture,
    scratch: &Path,
    queue_depth: NonZeroU32,
) -> Option<PreparedConfiguration> {
    // Linux 5.15 keeps io-wq on its submitting task, so each QD also gets an isolated preflight.
    let prepared = timing::isolated(|| {
        let unlinker = checked_unlinker(fixture, scratch, queue_depth)?;
        let worker_limits = unlinker
            .worker_limits()
            .expect("Failed to read wheel-unlink io-wq limits");
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
                "wheel_record_unlinks/{backend}/{}: active UNLINKAT, fallback=0, io-wq limits {limits}",
                fixture.name,
            ));
            Some(prepared)
        }
        Err(error) => {
            benchmark_note(format_args!(
                "wheel_record_unlinks/{backend}/{}: unavailable ({error})",
                fixture.name,
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
fn assert_configuration(unlinker: &uring::Unlinker, prepared: PreparedConfiguration) {
    assert_eq!(
        unlinker
            .worker_limits()
            .expect("Failed to read wheel-unlink io-wq limits"),
        prepared.worker_limits,
        "io-wq limits changed between isolated wheel-unlink samples"
    );
}

fn wheel_record_unlink_backends(criterion: &mut Criterion<WallTime>) {
    if matches!(
        env::var("CODSPEED_RUNNER_MODE").as_deref(),
        Ok("instrumentation" | "simulation")
    ) {
        return;
    }
    let Some(inputs) = settings::read(settings::INPUT_NAMES.map(env::var_os))
        .expect("Invalid wheel-unlink benchmark inputs")
    else {
        return;
    };

    let external = WheelFixture::from_wheel(&inputs.wheel, &inputs.sha256, &inputs.scratch)
        .expect("Failed to validate external wheel input");
    let external_name = format!("wheel-{}", external.wheel_filename());
    let synthetic = WheelFixture::manyfiles(&inputs.scratch)
        .expect("Failed to construct synthetic valid manyfiles wheel");
    assert_eq!(synthetic.leaf_count(), 10_007);
    let fixtures = [
        Fixture::capture("synthetic-manyfiles".to_owned(), synthetic),
        Fixture::capture(external_name, external),
    ];

    let mut group = criterion.benchmark_group("wheel_record_unlinks");
    for fixture in &fixtures {
        group.throughput(fixture.throughput());
        group.bench_function(fixture.benchmark_id("ordinary"), |bencher| {
            bencher.iter_custom(|iterations| {
                timing::isolated_trials(
                    iterations,
                    || {
                        let trial = fixture.trial();
                        fixture.assert_ordinary(&trial, &leaf::ordinary(trial.leaves()));
                    },
                    || fixture.trial(),
                    |(), trial| leaf::ordinary(trial.leaves()),
                    |trial, results| fixture.assert_ordinary(trial, results),
                    |()| {},
                )
            });
        });
        for threads in [1, 4, 16] {
            group.bench_function(
                fixture.benchmark_id(&format!("workers-{threads}")),
                |bencher| {
                    bencher.iter_custom(|iterations| {
                        timing::isolated_trials(
                            iterations,
                            || {
                                let pool = leaf::worker_pool(threads);
                                let trial = fixture.trial();
                                fixture.assert_ordinary(
                                    &trial,
                                    &leaf::with_workers(trial.leaves(), &pool),
                                );
                                pool
                            },
                            || fixture.trial(),
                            |pool, trial| leaf::with_workers(trial.leaves(), pool),
                            |trial, results| fixture.assert_ordinary(trial, results),
                            |_| {},
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
        for queue_depth in [1, 16, 64, 256] {
            let queue_depth = NonZeroU32::new(queue_depth).expect("non-zero queue depth");
            let backend = format!("io-uring-{queue_depth}");
            let Some(prepared) =
                prepare_configuration(&backend, fixture, &inputs.scratch, queue_depth)
            else {
                continue;
            };
            group.bench_function(fixture.benchmark_id(&backend), |bencher| {
                bencher.iter_custom(|iterations| {
                    timing::isolated_trials(
                        iterations,
                        || {
                            let unlinker = checked_unlinker(fixture, &inputs.scratch, queue_depth)
                                .expect("io_uring UNLINKAT became unavailable");
                            assert_configuration(&unlinker, prepared);
                            unlinker
                        },
                        || fixture.trial(),
                        |unlinker, trial| {
                            unlinker
                                .unlink(trial.leaves())
                                .expect("Fatal io_uring control failure during wheel unlink")
                        },
                        |trial, results| fixture.assert_ring(trial, results),
                        |unlinker| assert_configuration(unlinker, prepared),
                    )
                });
            });
        }
    }
    group.finish();

    let mut group = criterion.benchmark_group("wheel_uninstall_wheel");
    for fixture in &fixtures {
        group.throughput(fixture.throughput());
        group.bench_function(fixture.benchmark_id("production"), |bencher| {
            bencher.iter_custom(|iterations| {
                timing::isolated_trials(
                    iterations,
                    || {
                        let trial = fixture.trial();
                        let result = uv_install_wheel::uninstall_wheel(
                            trial.dist_info(),
                            fixture.wheel.name(),
                            trial.layout(),
                        )
                        .expect("Production wheel-uninstall warmup failed");
                        trial
                            .assert_production_success(&result)
                            .expect("Production wheel-uninstall warmup oracle failed");
                    },
                    || fixture.trial(),
                    |(), trial| {
                        uv_install_wheel::uninstall_wheel(
                            trial.dist_info(),
                            fixture.wheel.name(),
                            trial.layout(),
                        )
                        .expect("Production wheel uninstall failed")
                    },
                    |trial, result| {
                        trial
                            .assert_production_success(result)
                            .expect("Production wheel-uninstall oracle failed");
                    },
                    |()| {},
                )
            });
        });
    }
    group.finish();
}

criterion_group!(wheel_unlinks, wheel_record_unlink_backends);
criterion_main!(wheel_unlinks);
