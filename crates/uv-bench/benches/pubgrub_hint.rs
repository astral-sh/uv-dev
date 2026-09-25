use std::collections::{BTreeSet, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::hint::black_box;
use std::str::FromStr;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_configuration::NoBinary;
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_platform_tags::PlatformTag;
use uv_resolver::PubGrubHint;

fn pubgrub_hint_identity(c: &mut Criterion<WallTime>) {
    let mut group = c.benchmark_group("pubgrub_hint_identity");
    let package = PackageName::from_str("example").expect("valid package name");

    for count in [64, 256, 1_024, 4_096] {
        let packages = (0..count)
            .map(|index| {
                PackageName::from_str(&format!("package-{index:04}")).expect("valid package name")
            })
            .collect::<Vec<_>>();
        let no_binary = PubGrubHint::NoBinary {
            package: package.clone(),
            option: NoBinary::Packages(packages.clone()),
        };
        let no_binary_other = PubGrubHint::NoBinary {
            package: package.clone(),
            option: NoBinary::Packages(packages.into_iter().rev().collect()),
        };

        group.bench_function(BenchmarkId::new("hash_no_binary", count), |benchmark| {
            benchmark.iter(|| {
                let mut hasher = DefaultHasher::new();
                black_box(&no_binary).hash(&mut hasher);
                black_box(hasher.finish())
            });
        });
        group.bench_function(BenchmarkId::new("eq_no_binary", count), |benchmark| {
            benchmark.iter(|| black_box(&no_binary) == black_box(&no_binary_other));
        });

        let tags = (0..count)
            .map(|index| {
                PlatformTag::from_str(&format!("manylinux_2_{index}_x86_64"))
                    .expect("valid platform tag")
            })
            .collect::<BTreeSet<_>>();
        let platform_tags = PubGrubHint::PlatformTags {
            package: package.clone(),
            version: Version::new([1]),
            tags: tags.clone(),
        };
        let platform_tags_other = PubGrubHint::PlatformTags {
            package: package.clone(),
            version: Version::new([2]),
            tags,
        };

        group.bench_function(BenchmarkId::new("hash_platform_tags", count), |benchmark| {
            benchmark.iter(|| {
                let mut hasher = DefaultHasher::new();
                black_box(&platform_tags).hash(&mut hasher);
                black_box(hasher.finish())
            });
        });
        group.bench_function(BenchmarkId::new("eq_platform_tags", count), |benchmark| {
            benchmark.iter(|| black_box(&platform_tags) == black_box(&platform_tags_other));
        });
    }
    group.finish();
}

criterion_group!(pubgrub_hint, pubgrub_hint_identity);
criterion_main!(pubgrub_hint);
