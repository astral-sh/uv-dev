use std::fmt::Write as _;
use std::hint::black_box;
use std::path::Path;

use criterion::{Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_workspace::dependency_groups::FlatDependencyGroups;
use uv_workspace::pyproject::PyProjectToml;

fn dependency_group_chains(c: &mut Criterion<WallTime>) {
    for depth in [64, 256, 1024, 4096] {
        let mut input = String::from("[dependency-groups]\n");
        for index in 0..depth {
            writeln!(
                input,
                "g-{index:04} = [{{ include-group = 'g-{:04}' }}]",
                index + 1
            )
            .expect("writing dependency groups into a string should succeed");
        }
        writeln!(input, "g-{depth:04} = []")
            .expect("writing dependency groups into a string should succeed");
        let pyproject = PyProjectToml::from_string(input, "pyproject.toml")
            .expect("benchmark input should be valid");

        c.bench_function(&format!("dependency group chain {depth}"), |benchmark| {
            benchmark.iter(|| {
                FlatDependencyGroups::from_pyproject_toml(
                    Path::new("pyproject.toml"),
                    black_box(&pyproject),
                )
                .expect("acyclic groups should resolve")
            });
        });
    }
}

fn dependency_group_cycles(c: &mut Criterion<WallTime>) {
    for depth in [64, 256, 1024, 4096] {
        let mut input = String::from("[dependency-groups]\n");
        for index in 0..depth {
            writeln!(
                input,
                "g-{index:04} = [{{ include-group = 'g-{:04}' }}]",
                index + 1
            )
            .expect("writing dependency groups into a string should succeed");
        }
        writeln!(input, "g-{depth:04} = [{{ include-group = 'g-0000' }}]")
            .expect("writing dependency groups into a string should succeed");
        let pyproject = PyProjectToml::from_string(input, "pyproject.toml")
            .expect("benchmark input should be valid");

        c.bench_function(&format!("dependency group cycle {depth}"), |benchmark| {
            benchmark.iter(|| {
                FlatDependencyGroups::from_pyproject_toml(
                    Path::new("pyproject.toml"),
                    black_box(&pyproject),
                )
                .expect_err("cyclic groups should fail")
            });
        });
    }
}

criterion_group!(
    dependency_groups,
    dependency_group_chains,
    dependency_group_cycles
);
criterion_main!(dependency_groups);
