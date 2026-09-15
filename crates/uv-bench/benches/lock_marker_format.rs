use std::hint::black_box;
use std::path::Path;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_configuration::DependencyGroupsWithDefaults;
use uv_resolver::{Lock, Metadata, PackageMap, TreeDisplay, TreeJsonTarget};

fn lock_with_markers(edges: usize, alternatives: usize, reuse: Option<usize>) -> Lock {
    let marker = (0..alternatives)
        .map(|index| format!("platform_machine == 'machine-{index:04}'"))
        .collect::<Vec<_>>()
        .join(" or ");
    let dependencies = (0..edges)
        .map(|index| {
            let marker = if let Some(reuse) = reuse {
                format!(
                    "{marker} or platform_machine == 'unique-{marker_index:05}'",
                    marker_index = index / reuse,
                )
            } else {
                marker.clone()
            };
            format!("    {{ name = \"leaf-{index:05}\", marker = \"{marker}\" }},")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let leaves = (0..edges)
        .map(|index| {
            format!(
                r#"[[package]]
name = "leaf-{index:05}"
version = "1.0.0"
source = {{ registry = "https://example.com/simple" }}
"#
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let lock = format!(
        r#"version = 1
revision = 3
requires-python = ">=3.12"

[manifest]
requirements = [{{ name = "root" }}]

[[package]]
name = "root"
version = "1.0.0"
source = {{ registry = "https://example.com/simple" }}
dependencies = [
{dependencies}
]

{leaves}"#
    );

    toml::from_str(&lock).expect("benchmark lock should be valid")
}

fn format_lock_markers(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("lock_marker_format");
    let latest = PackageMap::default();
    let groups = DependencyGroupsWithDefaults::none();
    let script = Path::new("script.py");

    for (edges, alternatives, reuse) in [
        (64, 1, None),
        (256, 32, None),
        (1_024, 128, None),
        (128, 8, Some(1)),
        (128, 8, Some(2)),
    ] {
        let lock = lock_with_markers(edges, alternatives, reuse);
        let tree = TreeDisplay::new(
            &lock,
            None,
            &latest,
            usize::MAX,
            &[],
            &[],
            &groups,
            false,
            false,
            false,
        );
        let shape = match reuse {
            None => "repeated",
            Some(1) => "unique",
            Some(_) => "paired",
        };
        let parameters = format!("{edges}-edges-{alternatives}-alternatives-{shape}");

        group.bench_function(BenchmarkId::new("workspace_metadata", &parameters), |b| {
            b.iter(|| {
                black_box(
                    Metadata::from_script(script, black_box(&lock))
                        .expect("benchmark metadata should be valid"),
                );
            });
        });
        group.bench_function(BenchmarkId::new("tree_json", &parameters), |b| {
            b.iter(|| {
                black_box(
                    tree.to_json(TreeJsonTarget::Script(script))
                        .expect("benchmark tree should be valid"),
                );
            });
        });
    }

    group.finish();
}

criterion_group!(lock_marker_format, format_lock_markers);
criterion_main!(lock_marker_format);
