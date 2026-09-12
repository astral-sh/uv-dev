use std::fmt::Write;
use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_configuration::{ExcludeDependency, Excludes};
use uv_pep440::Version;

fn scoped_entries(count: usize, versionless: bool) -> Vec<ExcludeDependency> {
    let mut source = String::from("entries = [\n");
    if versionless {
        source.push_str("{ package = { name = 'parent' }, dependencies = ['fallback'] },\n");
    }
    for version in 0..count {
        writeln!(
            source,
            "{{ package = {{ name = 'parent', version = '1.{version}' }}, dependencies = ['child'] }},"
        )
        .expect("writing to a string cannot fail");
    }
    source.push(']');

    toml::from_str::<toml::Table>(&source)
        .expect("benchmark configuration should be valid")
        .remove("entries")
        .expect("benchmark configuration should contain entries")
        .try_into()
        .expect("benchmark exclusions should be valid")
}

fn scoped_exclusions(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("scoped_exclusions");
    let parent = "parent".parse().expect("valid package name");
    let child = "child".parse().expect("valid package name");
    let fallback = "fallback".parse().expect("valid package name");

    for count in [1, 4, 16, 64, 256, 1_024, 4_096] {
        let entries = scoped_entries(count, false);
        let with_fallback = scoped_entries(count, true);
        let excludes = Excludes::from_entries(entries.iter().cloned());
        let excludes_with_fallback = Excludes::from_entries(with_fallback.iter().cloned());
        let versions = (0..count)
            .map(|version| Version::new([1, version as u64]))
            .collect::<Vec<_>>();
        let late = Version::new([1, (count - 1) as u64]);
        let missing = Version::new([2]);

        group.bench_function(BenchmarkId::new("build", count), |benchmark| {
            benchmark.iter(|| Excludes::from_entries(black_box(entries.iter().cloned())));
        });
        group.bench_function(BenchmarkId::new("hit", count), |benchmark| {
            benchmark.iter(|| {
                black_box(
                    versions
                        .iter()
                        .filter(|version| excludes.contains_for(&parent, version, &child))
                        .count(),
                )
            });
        });
        group.bench_function(BenchmarkId::new("late", count), |benchmark| {
            benchmark.iter(|| {
                black_box(
                    (0..count)
                        .filter(|_| excludes.contains_for(&parent, &late, &child))
                        .count(),
                )
            });
        });
        group.bench_function(BenchmarkId::new("miss", count), |benchmark| {
            benchmark.iter(|| {
                black_box(
                    (0..count)
                        .filter(|_| excludes.contains_for(&parent, &missing, &child))
                        .count(),
                )
            });
        });
        group.bench_function(BenchmarkId::new("fallback", count), |benchmark| {
            benchmark.iter(|| {
                black_box(
                    (0..count)
                        .filter(|_| {
                            excludes_with_fallback.contains_for(&parent, &missing, &fallback)
                        })
                        .count(),
                )
            });
        });
    }
    group.finish();
}

criterion_group!(scoped_exclusion_benches, scoped_exclusions);
criterion_main!(scoped_exclusion_benches);
