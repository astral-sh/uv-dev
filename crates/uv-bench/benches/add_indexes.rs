//! Benchmarks updating a `pyproject.toml` with many configured indexes.

extern crate uv_performance_memory_allocator;

use std::hint::black_box;
use std::path::Path;
use std::str::FromStr;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};

use uv_distribution_types::Index;
use uv_workspace::pyproject_mut::{DependencyTarget, PyProjectTomlMut};

fn add_indexes(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("add_indexes");

    for size in [32, 128, 512, 2048] {
        let indexes = (0..size)
            .map(|index| {
                Index::from_str(&format!("idx-{index}=https://index-{index}.example/simple"))
                    .expect("valid index")
            })
            .collect::<Vec<_>>();
        let references = indexes.iter().collect::<Vec<_>>();

        group.bench_function(format!("individual/{size}"), |bench| {
            bench.iter_batched(
                || {
                    PyProjectTomlMut::from_toml(
                        "[project]\nname = \"project\"\n",
                        DependencyTarget::PyProjectToml,
                    )
                    .expect("valid pyproject.toml")
                },
                |mut pyproject| {
                    for index in references.iter().rev() {
                        pyproject
                            .add_index(black_box(index), Path::new("."))
                            .expect("valid index table");
                    }
                    black_box(pyproject);
                },
                BatchSize::SmallInput,
            );
        });

        group.bench_function(format!("batched/{size}"), |bench| {
            bench.iter_batched(
                || {
                    PyProjectTomlMut::from_toml(
                        "[project]\nname = \"project\"\n",
                        DependencyTarget::PyProjectToml,
                    )
                    .expect("valid pyproject.toml")
                },
                |mut pyproject| {
                    pyproject
                        .add_indexes(black_box(&references), Path::new("."))
                        .expect("valid index tables");
                    black_box(pyproject);
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

criterion_group!(add_indexes_benches, add_indexes);
criterion_main!(add_indexes_benches);
