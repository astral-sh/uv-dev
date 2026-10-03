use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_normalize::ExtraName;
use uv_pep508::{
    ExtraOperator, MarkerExpression, MarkerOperator, MarkerTree, MarkerValueExtra,
    MarkerValueString,
};

fn evaluate_only_extras(c: &mut Criterion<WallTime>) {
    for width in [1, 8, 32, 128] {
        let suffix_extras = (0..width)
            .map(|index| {
                format!("suffix-{index:08}")
                    .parse::<ExtraName>()
                    .expect("benchmark extra should be valid")
            })
            .collect::<Vec<_>>();
        let mut suffix = MarkerTree::TRUE;
        for extra in &suffix_extras {
            suffix = suffix.and(MarkerTree::expression(MarkerExpression::Extra {
                operator: ExtraOperator::Equal,
                name: MarkerValueExtra::Extra(extra.clone()),
            }));
        }

        let branch_extras = (0..width)
            .map(|index| {
                format!("branch-{index:08}")
                    .parse::<ExtraName>()
                    .expect("benchmark extra should be valid")
            })
            .collect::<Vec<_>>();
        let mut marker = MarkerTree::expression(MarkerExpression::Extra {
            operator: ExtraOperator::Equal,
            name: MarkerValueExtra::Extra(branch_extras[0].clone()),
        });
        for (index, extra) in branch_extras.iter().enumerate() {
            let branch = MarkerTree::expression(MarkerExpression::String {
                key: MarkerValueString::PlatformMachine,
                operator: MarkerOperator::Equal,
                value: format!("machine-{index:08}").into(),
            })
            .and(MarkerTree::expression(MarkerExpression::Extra {
                operator: ExtraOperator::Equal,
                name: MarkerValueExtra::Extra(extra.clone()),
            }));
            marker = marker.or(branch);
        }
        marker = marker.and(suffix);

        let extras = branch_extras
            .into_iter()
            .chain(suffix_extras)
            .collect::<Vec<_>>();
        c.bench_function(&format!("evaluate_only_extras {width}"), |benchmark| {
            benchmark.iter(|| black_box(marker).evaluate_only_extras(black_box(&extras)));
        });
    }
}

criterion_group!(uv_pep508, evaluate_only_extras);
criterion_main!(uv_pep508);
