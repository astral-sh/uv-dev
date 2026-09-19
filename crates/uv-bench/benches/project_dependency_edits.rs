//! Benchmarks for applying large batches of direct project-dependency edits.

use std::fmt::Write;
use std::hint::black_box;
use std::str::FromStr;

use criterion::{Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_distribution_types::IndexName;
use uv_pep508::{MarkerTree, Requirement};
use uv_workspace::pyproject::{DependencyType, Source};
use uv_workspace::pyproject_mut::{DependencyTarget, PyProjectTomlMut};

fn project_dependency_edits(criterion: &mut Criterion<WallTime>) {
    for count in [128, 512, 2048] {
        let mut dependencies = String::new();
        for index in 0..count {
            writeln!(&mut dependencies, "    \"existing-{index:05}>=1\",")
                .expect("writing to a string is infallible");
        }
        let pyproject = format!("[project]\ndependencies = [\n{dependencies}]\n");

        let additions = (0..count)
            .map(|index| Requirement::from_str(&format!("new-{index:05}")))
            .collect::<Result<Vec<_>, _>>()
            .expect("benchmark requirements should be valid");
        let additions = additions
            .iter()
            .map(|requirement| (requirement, None))
            .collect::<Vec<_>>();
        criterion.bench_function(
            &format!("project dependency additions/{count}"),
            |benchmark| {
                benchmark.iter(|| {
                    let mut pyproject = PyProjectTomlMut::from_toml(
                        black_box(&pyproject),
                        DependencyTarget::PyProjectToml,
                    )
                    .expect("benchmark pyproject should be valid");
                    pyproject
                        .add_dependencies(&DependencyType::Production, black_box(&additions), false)
                        .expect("benchmark additions should be valid")
                });
            },
        );

        let mut sources = String::new();
        for index in 0..count {
            writeln!(&mut sources, "existing-{index:05} = {{ index = \"old\" }}")
                .expect("writing to a string is infallible");
        }
        let source_pyproject = format!("{pyproject}\n[tool.uv.sources]\n{sources}");
        let source = Source::Registry {
            index: IndexName::from_str("internal").expect("benchmark index should be valid"),
            marker: MarkerTree::TRUE,
            extra: None,
            group: None,
        };
        let sourced_additions = additions
            .iter()
            .map(|(requirement, _)| (*requirement, Some(&source)))
            .collect::<Vec<_>>();
        criterion.bench_function(
            &format!("project dependency additions with sources/{count}"),
            |benchmark| {
                benchmark.iter(|| {
                    let mut pyproject = PyProjectTomlMut::from_toml(
                        black_box(&source_pyproject),
                        DependencyTarget::PyProjectToml,
                    )
                    .expect("benchmark pyproject should be valid");
                    pyproject
                        .add_dependencies(
                            &DependencyType::Production,
                            black_box(&sourced_additions),
                            false,
                        )
                        .expect("benchmark additions should be valid")
                });
            },
        );

        let mut marked_dependencies = String::new();
        for index in 0..count {
            writeln!(
                &mut marked_dependencies,
                "    \"shared>=1; python_full_version == '3.12.{index}'\","
            )
            .expect("writing to a string is infallible");
        }
        let marked_pyproject = format!("[project]\ndependencies = [\n{marked_dependencies}]\n");
        let marked_updates = (0..count)
            .map(|index| {
                Requirement::from_str(&format!("shared>=2; python_full_version == '3.12.{index}'"))
            })
            .collect::<Result<Vec<_>, _>>()
            .expect("benchmark requirements should be valid");
        let marked_updates = marked_updates
            .iter()
            .map(|requirement| (requirement, None))
            .collect::<Vec<_>>();
        criterion.bench_function(
            &format!("project dependency marker updates/{count}"),
            |benchmark| {
                benchmark.iter(|| {
                    let mut pyproject = PyProjectTomlMut::from_toml(
                        black_box(&marked_pyproject),
                        DependencyTarget::PyProjectToml,
                    )
                    .expect("benchmark pyproject should be valid");
                    pyproject
                        .add_dependencies(
                            &DependencyType::Production,
                            black_box(&marked_updates),
                            false,
                        )
                        .expect("benchmark updates should be valid")
                });
            },
        );

        let updates = (0..count)
            .map(|index| Requirement::from_str(&format!("existing-{index:05}>=2")))
            .collect::<Result<Vec<_>, _>>()
            .expect("benchmark requirements should be valid");
        let updates = updates
            .iter()
            .map(|requirement| (requirement, None))
            .collect::<Vec<_>>();
        criterion.bench_function(
            &format!("project dependency updates/{count}"),
            |benchmark| {
                benchmark.iter(|| {
                    let mut pyproject = PyProjectTomlMut::from_toml(
                        black_box(&pyproject),
                        DependencyTarget::PyProjectToml,
                    )
                    .expect("benchmark pyproject should be valid");
                    pyproject
                        .add_dependencies(&DependencyType::Production, black_box(&updates), false)
                        .expect("benchmark updates should be valid")
                });
            },
        );

        let unrelated_dependencies = (0..count)
            .map(|index| format!("    \"production-{index:05}>=1\","))
            .collect::<Vec<_>>()
            .join("\n");
        let unrelated_extras = (0..count)
            .map(|index| format!("extra-{index:05} = [\"optional-{index:05}>=1\"]"))
            .collect::<Vec<_>>()
            .join("\n");
        let unrelated_groups = (0..count)
            .map(|index| format!("group-{index:05} = [\"group-dependency-{index:05}>=1\"]"))
            .collect::<Vec<_>>()
            .join("\n");
        let dev_dependencies = (0..count)
            .map(|index| format!("    \"standardized-{index:05}>=1\","))
            .collect::<Vec<_>>()
            .join("\n");
        let legacy_dependencies = (0..count)
            .map(|index| format!("    \"legacy-{index:05}>=1\","))
            .collect::<Vec<_>>()
            .join("\n");
        let dev_pyproject = format!(
            "[project]\ndependencies = [\n{unrelated_dependencies}\n]\n\
             [project.optional-dependencies]\n{unrelated_extras}\n\
             [dependency-groups]\nDeV = [\n{dev_dependencies}\n]\n{unrelated_groups}\n\
             [tool.uv]\ndev-dependencies = [\n{legacy_dependencies}\n]\n"
        );
        let pyproject =
            PyProjectTomlMut::from_toml(black_box(&dev_pyproject), DependencyTarget::PyProjectToml)
                .expect("benchmark pyproject should be valid");
        let names = (0..count)
            .map(|index| format!("new-{index:05}").parse())
            .collect::<Result<Vec<_>, _>>()
            .expect("benchmark names should be valid");

        if count <= 512 {
            criterion.bench_function(
                &format!("project dev dependency lookup linear/{count}"),
                |benchmark| {
                    benchmark.iter(|| {
                        for name in &names {
                            black_box(pyproject.find_dependency(black_box(name), None));
                        }
                    });
                },
            );
        }
        criterion.bench_function(
            &format!("project dev dependency lookup indexed/{count}"),
            |benchmark| {
                benchmark.iter(|| {
                    let (standardized, legacy) = pyproject.find_dev_dependency_names();
                    for name in &names {
                        black_box(standardized.contains(black_box(name)) || legacy.contains(name));
                    }
                });
            },
        );
    }
}

criterion_group!(project_dependency_edits_group, project_dependency_edits);
criterion_main!(project_dependency_edits_group);
