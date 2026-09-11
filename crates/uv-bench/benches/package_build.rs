//! Native package builds over a real Python source tree.

mod common;

extern crate uv_performance_memory_allocator;

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::{fixture_path, is_codspeed_simulation};
use uv_distribution_filename::SourceDistExtension;
use uv_preview::Preview;

fn package_build(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    uv_preview::set(Preview::default()).expect("Failed to initialize preview configuration");
    uv_preview::finalize().expect("Failed to finalize preview configuration");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime");
    let bytes = fs_err::read(fixture_path("django-5.2.6.tar.gz"))
        .expect("Failed to read source distribution");
    let (source, _) = runtime
        .block_on(uv_extract::stream::archive(
            bytes.as_slice(),
            SourceDistExtension::TarGz,
            tempfile::tempdir().expect("Failed to create source directory"),
        ))
        .expect("Failed to unpack source distribution");
    let source_tree =
        uv_extract::strip_component(source.path()).expect("Invalid source distribution layout");
    // Adapt only the packaging configuration; the Python, data, docs, and test files are Django's.
    fs_err::write(
        source_tree.join("pyproject.toml"),
        format!(
            r#"[project]
name = "django"
version = "5.2.6"
requires-python = ">=3.10"

[build-system]
requires = ["uv_build=={}"]
build-backend = "uv_build"

[tool.uv.build-backend]
module-root = ""
source-include = ["docs/**", "tests/**"]
"#,
            uv_version::version(),
        ),
    )
    .expect("Failed to configure native build backend");

    c.bench_function("package_build/wheel/django", |b| {
        b.iter_batched(
            || tempfile::tempdir().expect("Failed to create wheel output directory"),
            |output| {
                let wheel = uv_build_backend::build_wheel(
                    black_box(&source_tree),
                    output.path(),
                    None,
                    uv_version::version(),
                    false,
                )
                .expect("Failed to build wheel");
                black_box((output, wheel))
            },
            BatchSize::PerIteration,
        );
    });
    c.bench_function("package_build/sdist/django", |b| {
        b.iter_batched(
            || tempfile::tempdir().expect("Failed to create sdist output directory"),
            |output| {
                let sdist = uv_build_backend::build_source_dist(
                    black_box(&source_tree),
                    output.path(),
                    uv_version::version(),
                    false,
                )
                .expect("Failed to build source distribution");
                black_box((output, sdist))
            },
            BatchSize::PerIteration,
        );
    });
}

criterion_group! {
    name = packages;
    config = common::walltime_criterion();
    targets = package_build
}
criterion_main!(packages);
