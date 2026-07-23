//! Benchmarks for parsing and checking the format of `uv.lock` files.

// Don't optimize the alloc crate away due to it being otherwise unused.
// https://github.com/rust-lang/rust/issues/64402
extern crate uv_performance_memory_allocator;

use std::fmt::Write;
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv::commands::find_lock_format_error;
use uv_resolver::Lock;

const NORMAL_LOCKFILE: &str = include_str!("../../../uv.lock");
const LARGE_PACKAGE_COUNT: usize = 10_000;

fn large_lockfile() -> String {
    let mut lockfile = String::new();
    lockfile.push_str("version = 1\nrevision = 3\nrequires-python = \">=3.12\"\n");

    for package_index in 0..LARGE_PACKAGE_COUNT {
        writeln!(
            lockfile,
            r#"
[[package]]
name = "benchmark-package-{package_index:05}"
version = "1.0.0"
source = {{ registry = "https://pypi.org/simple" }}
sdist = {{ url = "https://example.com/benchmark_package_{package_index:05}-1.0.0.tar.gz", hash = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", size = 1024 }}
wheels = [
    {{ url = "https://example.com/benchmark_package_{package_index:05}-1.0.0-py3-none-any.whl", hash = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", size = 1024 }},
    {{ url = "https://example.com/benchmark_package_{package_index:05}-1.0.0-cp312-cp312-manylinux_2_17_x86_64.whl", hash = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", size = 2048 }},
    {{ url = "https://example.com/benchmark_package_{package_index:05}-1.0.0-cp312-cp312-manylinux_2_17_aarch64.whl", hash = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", size = 2048 }},
    {{ url = "https://example.com/benchmark_package_{package_index:05}-1.0.0-cp312-cp312-macosx_11_0_arm64.whl", hash = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", size = 2048 }},
    {{ url = "https://example.com/benchmark_package_{package_index:05}-1.0.0-cp312-cp312-win_amd64.whl", hash = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", size = 2048 }},
]"#
        )
        .expect("Writing a synthetic lockfile should not fail");
    }

    lockfile
}

fn lockfile_check(c: &mut Criterion<WallTime>) {
    let large_lockfile = large_lockfile();
    let normal_unformatted = unformatted_at_end(NORMAL_LOCKFILE);
    let large_unformatted = unformatted_at_end(&large_lockfile);

    assert_eq!(find_lock_format_error(NORMAL_LOCKFILE), None);
    assert_eq!(find_lock_format_error(&large_lockfile), None);
    assert_eq!(
        find_lock_format_error(&normal_unformatted),
        Some(NORMAL_LOCKFILE.lines().count())
    );
    assert_eq!(
        find_lock_format_error(&large_unformatted),
        Some(large_lockfile.lines().count())
    );

    let mut group = c.benchmark_group("lockfile_check");

    group.bench_function("parse/normal", |benchmark| {
        benchmark.iter(|| {
            toml::from_str::<Lock>(black_box(NORMAL_LOCKFILE))
                .expect("Repository lockfile should be valid")
        });
    });
    group.bench_function("parse/large", |benchmark| {
        benchmark.iter(|| {
            toml::from_str::<Lock>(black_box(&large_lockfile))
                .expect("Synthetic lockfile should be valid")
        });
    });

    for (name, lockfile) in [
        ("normal/formatted", NORMAL_LOCKFILE),
        ("normal/unformatted-end", normal_unformatted.as_str()),
        ("large/formatted", large_lockfile.as_str()),
        ("large/unformatted-end", large_unformatted.as_str()),
    ] {
        group.bench_function(format!("parse-and-format/{name}"), |benchmark| {
            benchmark.iter(|| {
                let lock = toml::from_str::<Lock>(black_box(lockfile))
                    .expect("Benchmark lockfile should be valid");
                black_box((lock, find_lock_format_error(lockfile)))
            });
        });
    }

    group.finish();
}

fn unformatted_at_end(lockfile: &str) -> String {
    let contents = lockfile
        .strip_suffix('\n')
        .expect("Benchmark lockfile should end in a newline");
    format!("{contents} \n")
}

criterion_group!(lockfile, lockfile_check);
criterion_main!(lockfile);
