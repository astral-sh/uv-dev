//! Parse and match the real wheel filenames in Prefect's universal lockfile.

use std::hint::black_box;
use std::str::FromStr;

use criterion::{
    BenchmarkId, Criterion, Throughput, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::fixture_path;
use uv_distribution_filename::WheelFilename;
use uv_platform_tags::{Arch, Os, Platform, Tags, TagsOptions};

fn wheel_tags(c: &mut Criterion<WallTime>) {
    let input = fs_err::read_to_string(fixture_path("prefect.lock"))
        .expect("Failed to read Prefect lockfile");
    let lock: toml::Value = toml::from_str(&input).expect("Invalid Prefect lockfile");
    let filenames = lock["package"]
        .as_array()
        .expect("Missing lockfile packages")
        .iter()
        .filter_map(|package| package.get("wheels").and_then(toml::Value::as_array))
        .flatten()
        .map(|wheel| {
            wheel["url"]
                .as_str()
                .expect("Missing wheel URL")
                .rsplit('/')
                .next()
                .expect("Missing wheel filename")
        })
        .collect::<Vec<_>>();
    let wheels = filenames
        .iter()
        .map(|filename| WheelFilename::from_str(filename).expect("Invalid wheel filename"))
        .collect::<Vec<_>>();
    assert!(!wheels.is_empty(), "The lockfile must contain wheels");

    let mut group = c.benchmark_group("wheel_tags");
    group.throughput(Throughput::Elements(wheels.len() as u64));
    group.bench_function("parse_prefect_filenames", |b| {
        b.iter(|| {
            for filename in &filenames {
                black_box(
                    WheelFilename::from_str(black_box(filename)).expect("Invalid wheel filename"),
                );
            }
        });
    });
    for (name, platform) in [
        (
            "linux_aarch64",
            Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 31,
                },
                Arch::Aarch64,
            ),
        ),
        (
            "linux_x86_64",
            Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 31,
                },
                Arch::X86_64,
            ),
        ),
        (
            "macos_aarch64",
            Platform::new(
                Os::Macos {
                    major: 14,
                    minor: 0,
                },
                Arch::Aarch64,
            ),
        ),
        ("windows_x86_64", Platform::new(Os::Windows, Arch::X86_64)),
    ] {
        let tags = Tags::from_env(
            platform,
            (3, 11),
            "cpython",
            (3, 11),
            TagsOptions {
                manylinux_compatible: true,
                is_cross: true,
                ..TagsOptions::default()
            },
        )
        .expect("Invalid target platform");
        group.bench_with_input(BenchmarkId::new("compatibility", name), &tags, |b, tags| {
            b.iter(|| {
                for wheel in &wheels {
                    black_box(black_box(wheel).compatibility(black_box(tags)));
                }
            });
        });
    }
    group.finish();
}

criterion_group!(wheels, wheel_tags);
criterion_main!(wheels);
