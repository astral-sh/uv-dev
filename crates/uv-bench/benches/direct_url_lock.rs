//! Offline freshness checks for locks that contain actual remote wheel URLs.

mod common;

use std::process::Command;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::{FixtureServer, is_codspeed_simulation, run_command};

fn direct_url_lock(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let server = FixtureServer::start(&[]);
    let mut group = c.benchmark_group("direct_url_lock");
    for (name, filename) in [
        ("wheel", "wheel-0.45.1-py3-none-any.whl"),
        ("packaging", "packaging-26.3-py3-none-any.whl"),
        (
            "numpy",
            "numpy-2.2.6-cp311-cp311-manylinux_2_17_aarch64.manylinux2014_aarch64.whl",
        ),
    ] {
        let directory = tempfile::tempdir().expect("Failed to create wheel project");
        let project = directory.path().join("project");
        fs_err::create_dir_all(&project).expect("Failed to create project directory");
        let requirement = format!("{name} @ {}", server.url(&format!("/files/{filename}")));
        fs_err::write(
            project.join("pyproject.toml"),
            format!(
                "[project]\nname = 'wheel-consumer'\nversion = '0.1.0'\nrequires-python = '==3.11.*'\ndependencies = [{requirement:?}]\n"
            ),
        )
        .expect("Failed to write project metadata");
        let cache = directory.path().join("cache");
        let command = || -> Command {
            let mut command = server.command(&cache);
            command
                .arg("--project")
                .arg(&project)
                .args(["lock", "--index-url"])
                .arg(server.url("/simple/"));
            command
        };
        // The lock and cache are produced by the ordinary resolver. A populated metadata cache
        // also keeps the full-validation path usable when comparing implementations.
        run_command(&mut command());
        run_command(
            server
                .command(&cache)
                .args([
                    "pip",
                    "install",
                    "--no-index",
                    "--no-deps",
                    "--python-platform",
                    "aarch64-manylinux2014",
                    "--target",
                ])
                .arg(directory.path().join("environment"))
                .arg(server.url(&format!("/files/{filename}"))),
        );
        let contents = fs_err::read(project.join("uv.lock")).expect("Missing generated lock");
        group.bench_function(BenchmarkId::new("offline_cached", name), |b| {
            b.iter(|| run_command(command().args(["--offline", "--locked"])));
        });
        assert_eq!(
            fs_err::read(project.join("uv.lock")).expect("Missing lock after validation"),
            contents,
            "Freshness checks must not change the lock"
        );
    }
    group.finish();
}

criterion_group! {
    name = locks;
    config = common::walltime_criterion();
    targets = direct_url_lock
}
criterion_main!(locks);
