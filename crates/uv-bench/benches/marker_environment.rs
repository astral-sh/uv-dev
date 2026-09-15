use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use uv_pep508::{MarkerEnvironment, MarkerEnvironmentBuilder, MarkerTree};

fn marker_environment(criterion: &mut Criterion) {
    let environment = MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
        implementation_name: "cpython",
        implementation_version: "3.11.5",
        os_name: "posix",
        platform_machine: "machine-00000000",
        platform_python_implementation: "CPython",
        platform_release: "test",
        platform_system: "Linux",
        platform_version: "test",
        python_full_version: "3.11.5",
        python_version: "3.11",
        sys_platform: "linux",
    })
    .expect("valid marker environment");

    let mut group = criterion.benchmark_group("marker_environment");
    for alternatives in [16, 64, 256] {
        let mut marker = MarkerTree::FALSE;
        for index in 0..alternatives {
            let term =
                format!("platform_machine == 'machine-{index:08}' and extra == 'extra-{index}'")
                    .parse::<MarkerTree>()
                    .expect("valid marker");
            marker = marker.or(term);
        }

        group.bench_function(BenchmarkId::new("bdd", alternatives), |benchmark| {
            benchmark
                .iter(|| black_box(marker).only_extras_for_environment(black_box(&environment)));
        });
    }
    group.finish();
}

criterion_group!(benches, marker_environment);
criterion_main!(benches);
