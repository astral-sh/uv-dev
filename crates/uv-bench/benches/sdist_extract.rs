//! Source-distribution extraction over published Python and native-library source trees.

mod common;

extern crate uv_performance_memory_allocator;

use std::hint::black_box;

use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main,
    measurement::WallTime,
};
use uv_bench::{fixture_path, is_codspeed_simulation};
use uv_distribution_filename::SourceDistExtension;
use uv_preview::{MaybePreviewFeature, Preview, PreviewFeature};

fn sdist_extract(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime");
    let mut group = c.benchmark_group("sdist_extract");
    for (backend, preview) in [
        ("default", Preview::default()),
        (
            "tar_codec",
            Preview::from_feature_names(&[MaybePreviewFeature::Known(PreviewFeature::TarCodec)]),
        ),
    ] {
        uv_preview::set(preview).expect("Failed to configure tar backend");
        for (name, filename) in [
            ("flask", "flask-3.1.1.tar.gz"),
            ("django", "django-5.2.6.tar.gz"),
            ("numpy", "numpy-2.2.6.tar.gz"),
        ] {
            let bytes =
                fs_err::read(fixture_path(filename)).expect("Failed to read source distribution");
            group.throughput(Throughput::Bytes(bytes.len() as u64));
            group.bench_function(BenchmarkId::new(backend, name), |b| {
                b.iter_batched(
                    || tempfile::tempdir().expect("Failed to create extraction directory"),
                    |target| {
                        let (target, files) = runtime
                            .block_on(uv_extract::stream::archive(
                                black_box(bytes.as_slice()),
                                SourceDistExtension::TarGz,
                                target,
                            ))
                            .expect("Failed to unpack source distribution");
                        let source_tree = uv_extract::strip_component(target.path())
                            .expect("Invalid source distribution layout");
                        black_box((target, files, source_tree))
                    },
                    BatchSize::PerIteration,
                );
            });
        }
    }
    group.finish();
    uv_preview::set(Preview::default()).expect("Failed to restore preview configuration");
    uv_preview::finalize().expect("Failed to finalize preview configuration");
}

criterion_group! {
    name = source_distributions;
    config = common::walltime_criterion();
    targets = sdist_extract
}
criterion_main!(source_distributions);
