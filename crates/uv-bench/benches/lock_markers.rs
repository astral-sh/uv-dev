//! Parses locks with repeated or distinct dependency markers.

extern crate uv_performance_memory_allocator;

use std::fmt::Write as _;
use std::hint::black_box;
use std::str::FromStr;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

use uv_lock::Lock;
use uv_normalize::{ExtraName, PackageName};
use uv_pep508::MarkerTree;
use uv_pypi_types::{ConflictItem, ConflictSet, Conflicts};
use uv_resolver_types::{ConflictMarker, UniversalMarker};

fn conflict_world(count: usize) -> MarkerTree {
    let package = PackageName::from_str("repro").expect("valid benchmark package");
    let items = (0..count)
        .map(|index| {
            let extra =
                ExtraName::from_str(&format!("e{index:03}")).expect("valid benchmark extra");
            ConflictItem::from((package.clone(), extra))
        })
        .collect::<Vec<_>>();
    let mut conflicts = Conflicts::empty();
    conflicts.push(ConflictSet::try_from(items).expect("valid benchmark conflicts"));
    UniversalMarker::new(MarkerTree::TRUE, ConflictMarker::from_conflicts(&conflicts)).combined()
}

fn generated_lock(package_count: usize, extra_count: usize, distinct: bool) -> String {
    let world = conflict_world(extra_count);
    let mut output = String::new();
    output.push_str("version = 1\nrevision = 3\nrequires-python = \">=3.12\"\n");

    for index in 0..package_count {
        writeln!(
            output,
            "\n[[package]]\nname = \"package-{index:05}\"\nversion = \"1.0.0\"\nsource = {{ registry = \"https://example.com/simple\" }}"
        )
        .expect("writing to a string cannot fail");
        if index > 0 {
            let environment = MarkerTree::from_str(&format!(
                "python_full_version >= '3.13.{}'",
                if distinct { index } else { 0 }
            ))
            .expect("valid benchmark marker");
            // Older locks can repeat the implication of a large conflict world on each edge.
            let marker = world
                .implies(environment)
                .try_to_string()
                .expect("nontrivial benchmark marker");
            writeln!(
                output,
                "dependencies = [\n    {{ name = \"package-{:05}\", marker = \"{marker}\" }},\n]",
                index - 1
            )
            .expect("writing to a string cannot fail");
        }
    }

    output
}

fn lock_markers(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("lock_markers");
    for (name, input) in [
        (
            "ordinary-small",
            include_str!("../../../test/packages/built-by-uv/uv.lock"),
        ),
        (
            "ordinary-large",
            include_str!("../../../scripts/benchmark/uv.lock"),
        ),
    ] {
        let expected: Lock = toml::from_str(input).expect("valid benchmark lock");
        assert_eq!(
            Lock::from_toml(input).expect("valid benchmark lock"),
            expected
        );
        group.throughput(Throughput::Bytes(input.len() as u64));
        group.bench_with_input(BenchmarkId::new("parse", name), input, |bench, input| {
            bench.iter(|| Lock::from_toml(black_box(input)).expect("valid benchmark lock"));
        });
    }
    for (name, packages, extras, distinct) in [
        ("small-repeated", 128, 2, false),
        ("large-repeated", 128, 32, false),
        ("large-distinct", 128, 32, true),
    ] {
        let input = generated_lock(packages, extras, distinct);
        let expected: Lock = toml::from_str(&input).expect("valid benchmark lock");
        assert_eq!(
            Lock::from_canonical_toml(&input).expect("canonical benchmark lock"),
            expected,
            "{name}: lock differs from TOML"
        );
        group.throughput(Throughput::Bytes(input.len() as u64));
        group.bench_with_input(BenchmarkId::new("parse", name), &input, |bench, input| {
            bench.iter(|| Lock::from_toml(black_box(input)).expect("valid benchmark lock"));
        });
    }
    group.finish();
}

criterion_group!(benches, lock_markers);
criterion_main!(benches);
