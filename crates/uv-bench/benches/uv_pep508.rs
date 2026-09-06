use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_pep508::MarkerTree;

fn top_level_extra(c: &mut Criterion<WallTime>) {
    for alternatives in [1, 8, 32, 128] {
        let platforms = (0..alternatives)
            .map(|index| format!("platform_machine == 'machine-{index:08}'"))
            .collect::<Vec<_>>()
            .join(" or ");
        let marker = format!("extra == 'target' and ({platforms})")
            .parse::<MarkerTree>()
            .expect("benchmark input should be valid");

        c.bench_function(
            &format!("top_level_extra platform alternatives {alternatives}"),
            |benchmark| benchmark.iter(|| black_box(marker).top_level_extra()),
        );
        c.bench_function(
            &format!("top_level_extra_name platform alternatives {alternatives}"),
            |benchmark| benchmark.iter(|| black_box(marker).top_level_extra_name()),
        );
    }

    let marker = "(extra == 'a' and extra == 'b' and sys_platform == 'linux') \
        or (extra == 'b' and sys_platform != 'linux')"
        .parse::<MarkerTree>()
        .expect("benchmark input should be valid");
    c.bench_function("top_level_extra competing extras", |benchmark| {
        benchmark.iter(|| black_box(marker).top_level_extra());
    });
    c.bench_function("top_level_extra_name competing extras", |benchmark| {
        benchmark.iter(|| black_box(marker).top_level_extra_name());
    });
}

criterion_group!(uv_pep508, top_level_extra);
criterion_main!(uv_pep508);
