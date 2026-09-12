//! Read Home Assistant's real shared requirements and constraint files.

mod common;

use std::hint::black_box;
use std::path::PathBuf;

use criterion::{
    BenchmarkGroup, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{FixtureServer, fixture_path, is_codspeed_simulation};
use uv_client::{BaseClientBuilder, Connectivity};
use uv_preview::Preview;
use uv_requirements::{RequirementsSource, RequirementsSpecification};

const FILES: &[&str] = &[
    "requirements.txt",
    "requirements_test.txt",
    "requirements_test_all.txt",
    "requirements_test_pre_commit.txt",
    "homeassistant/package_constraints.txt",
];

const WORKLOADS: &[(&str, &[&str])] = &[
    ("core", &["requirements.txt"]),
    ("core_tests", &["requirements.txt", "requirements_test.txt"]),
    (
        "full_tests",
        &["requirements.txt", "requirements_test_all.txt"],
    ),
];

fn register(
    group: &mut BenchmarkGroup<'_, WallTime>,
    runtime: &tokio::runtime::Runtime,
    client: &BaseClientBuilder<'_>,
    kind: &str,
    name: &str,
    sources: &[RequirementsSource],
) -> (usize, usize) {
    let read = || {
        runtime
            .block_on(RequirementsSpecification::from_sources(
                sources,
                &[],
                &[],
                &[],
                None,
                client,
            ))
            .expect("Failed to read requirements include graph")
    };
    let expected = read();
    let counts = (expected.requirements.len(), expected.constraints.len());
    assert!(counts.0 > 0);
    group.bench_function(BenchmarkId::new(kind, name), |b| {
        b.iter(|| black_box(read()));
    });
    counts
}

fn requirements_includes(c: &mut Criterion<WallTime>) {
    uv_preview::set(Preview::default()).expect("Failed to configure preview features");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create Tokio runtime");
    let directory = tempfile::tempdir().expect("Failed to create requirements directory");
    for path in FILES {
        let target = directory.path().join(path);
        fs_err::create_dir_all(target.parent().expect("Missing parent directory"))
            .expect("Failed to create requirements subdirectory");
        fs_err::copy(
            fixture_path(&format!("homeassistant-{}", path.replace('/', "-"))),
            target,
        )
        .expect("Failed to copy requirements fixture");
    }
    let local_client = BaseClientBuilder::default().connectivity(Connectivity::Offline);
    let remote_client = BaseClientBuilder::default();
    let server = (!is_codspeed_simulation()).then(|| FixtureServer::start(&[]));
    let mut group = c.benchmark_group("requirements_includes");
    for (name, roots) in WORKLOADS {
        let local = roots
            .iter()
            .map(|root| RequirementsSource::RequirementsTxt(directory.path().join(root)))
            .collect::<Vec<_>>();
        let counts = register(&mut group, &runtime, &local_client, "local", name, &local);
        if let Some(server) = &server {
            let remote = roots
                .iter()
                .map(|root| {
                    RequirementsSource::RequirementsTxt(PathBuf::from(
                        server.url(&format!("/requirements/{root}")),
                    ))
                })
                .collect::<Vec<_>>();
            assert_eq!(
                register(
                    &mut group,
                    &runtime,
                    &remote_client,
                    "remote",
                    name,
                    &remote
                ),
                counts
            );
        }
    }
    group.finish();
}

criterion_group! {
    name = includes;
    config = common::walltime_criterion();
    targets = requirements_includes
}
criterion_main!(includes);
