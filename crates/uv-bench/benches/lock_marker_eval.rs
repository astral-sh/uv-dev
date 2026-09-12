use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main, measurement::WallTime};
use rustc_hash::FxHashSet;
use uv_normalize::ExtraName;
use uv_pep508::{MarkerEnvironment, MarkerEnvironmentBuilder, MarkerTree};

fn evaluate_lock_markers(c: &mut Criterion<WallTime>) {
    let env = MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
        implementation_name: "cpython",
        implementation_version: "3.12.0",
        os_name: "posix",
        platform_machine: "x86_64",
        platform_python_implementation: "CPython",
        platform_release: "",
        platform_system: "Linux",
        platform_version: "",
        python_full_version: "3.12.0",
        python_version: "3.12",
        sys_platform: "linux",
    })
    .expect("benchmark environment should be valid");

    for width in [32, 128, 512, 2_048] {
        let source = (0..width)
            .map(|index| (format!("package-{index:05}"), format!("feature-{index:05}")))
            .collect::<Vec<_>>();
        let encoded = source
            .iter()
            .map(|(package, extra)| {
                format!("extra-{}-{package}-{extra}", package.len())
                    .parse::<ExtraName>()
                    .expect("benchmark extra should be valid")
            })
            .collect::<Vec<_>>();
        let indexed = encoded.iter().collect::<FxHashSet<_>>();
        let markers = encoded
            .iter()
            .rev()
            .map(|extra| {
                format!("sys_platform == 'linux' and extra == '{extra}'")
                    .parse::<MarkerTree>()
                    .expect("benchmark marker should be valid")
            })
            .collect::<Vec<_>>();

        if width <= 128 {
            c.bench_function(
                &format!("evaluate_lock_markers rebuild+linear {width}"),
                |benchmark| {
                    benchmark.iter(|| {
                        for marker in &markers {
                            let encoded = source
                                .iter()
                                .map(|(package, extra)| {
                                    format!("extra-{}-{package}-{extra}", package.len())
                                        .parse::<ExtraName>()
                                        .expect("benchmark extra should be valid")
                                })
                                .collect::<Vec<_>>();
                            black_box(marker.evaluate(&env, &encoded));
                        }
                    });
                },
            );
        }
        if width <= 512 {
            c.bench_function(
                &format!("evaluate_lock_markers cached+linear {width}"),
                |benchmark| {
                    benchmark.iter(|| {
                        for marker in &markers {
                            black_box(marker.evaluate(&env, &encoded));
                        }
                    });
                },
            );
        }
        c.bench_function(
            &format!("evaluate_lock_markers cached+indexed {width}"),
            |benchmark| {
                benchmark.iter(|| {
                    for marker in &markers {
                        black_box(
                            marker.evaluate_with_extra(&env, |extra| indexed.contains(extra)),
                        );
                    }
                });
            },
        );
    }
}

criterion_group!(lock_marker_eval, evaluate_lock_markers);
criterion_main!(lock_marker_eval);
