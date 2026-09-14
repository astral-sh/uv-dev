//! Audit a historical Prefect lockfile against a real advisory affecting its Requests version.

mod common;

use std::process::Stdio;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::{FixtureServer, is_codspeed_simulation};

fn osv_audit(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let server = FixtureServer::start(&["prefect-audit.lock"]);
    let project = server.project("prefect-audit");
    c.bench_function("osv_audit/prefect", |b| {
        b.iter_batched(
            || {
                let cache = tempfile::tempdir().expect("Failed to create audit cache");
                let mut command = server.command(cache.path());
                command
                    .arg("--project")
                    .arg(project.path())
                    .args([
                        "audit",
                        "--frozen",
                        "--no-default-groups",
                        "--output-format",
                        "json",
                        "--service-url",
                    ])
                    .arg(server.url("/osv/"))
                    .stdout(Stdio::piped());
                (cache, command)
            },
            |(cache, mut command)| {
                let output = command.output().expect("Failed to execute audit");
                assert_eq!(
                    output.status.code(),
                    Some(1),
                    "Audit did not report the vulnerability: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(
                    String::from_utf8_lossy(&output.stdout).contains("GHSA-9hjg-9r4m-mvj7"),
                    "Expected Requests advisory is missing"
                );
                cache
            },
            BatchSize::PerIteration,
        );
    });
}

criterion_group! {
    name = audits;
    config = common::walltime_criterion();
    targets = osv_audit
}
criterion_main!(audits);
