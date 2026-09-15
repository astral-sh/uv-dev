use std::hint::black_box;
use std::str::FromStr;

use criterion::{Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_pep508::MarkerTree;

fn visit_extras(c: &mut Criterion<WallTime>) {
    for width in [16, 64, 256] {
        let conditions = (0..width)
            .map(|index| format!("platform_machine == 'machine-{index:08}'"))
            .collect::<Vec<_>>()
            .join(" or ");
        let marker = MarkerTree::from_str(&format!("extra == 'target' and ({conditions})"))
            .expect("benchmark marker should be valid");

        c.bench_function(
            &format!("visit_extras {width} platform ranges"),
            |benchmark| {
                benchmark.iter(|| {
                    let mut visited = 0;
                    black_box(marker).visit_extras(|_, _| visited += 1);
                    black_box(visited)
                });
            },
        );
    }
}

criterion_group!(uv_pep508, visit_extras);
criterion_main!(uv_pep508);
