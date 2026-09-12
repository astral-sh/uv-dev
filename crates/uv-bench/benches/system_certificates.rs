//! Native certificate-store initialization during ordinary metadata requests on macOS.

mod common;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{FixtureServer, is_codspeed_simulation, run_command};

fn system_certificates(c: &mut Criterion<WallTime>) {
    if !cfg!(target_os = "macos") || is_codspeed_simulation() {
        return;
    }
    let server = FixtureServer::start(&[]);
    let requirements = [
        "sampleproject==4.0.0",
        "flask==3.1.2",
        "jupyterlab==4.4.7",
        "numpy==2.2.6",
        "sympy==1.14.0",
        "packaging==26.3",
        "wheel==0.45.1",
        "setuptools==80.9.0",
        "flit-core==3.12.0",
        "hatchling==1.27.0",
        "versioningit==3.3.0",
        "pathspec==0.12.1",
        "pluggy==1.6.0",
        "editables==0.5",
        "click==8.4.2",
    ];
    let mut group = c.benchmark_group("system_certificates/macos");
    for count in [1, 5, 15] {
        for (mode, system_certs) in [("bundled", false), ("system", true)] {
            group.bench_function(BenchmarkId::new(mode, count), |b| {
                b.iter_batched(
                    || {
                        let directory = tempfile::tempdir().expect("Failed to create cache");
                        let path = directory.path().join("requirements.in");
                        fs_err::write(&path, requirements[..count].join("\n"))
                            .expect("Failed to write requirements");
                        let mut command = server.command(&directory.path().join("cache"));
                        command
                            .env_remove("SSL_CERT_FILE")
                            .env_remove("SSL_CERT_DIR")
                            .env_remove("SSL_CLIENT_CERT");
                        if system_certs {
                            command.arg("--system-certs");
                        }
                        command
                            .args([
                                "pip",
                                "compile",
                                "--no-deps",
                                "--no-build",
                                "--no-header",
                                "--no-annotate",
                                "--python-platform",
                                "aarch64-manylinux2014",
                                "--index-url",
                            ])
                            .arg(server.url("/simple/"))
                            .arg(path);
                        (directory, command)
                    },
                    |(directory, mut command)| {
                        run_command(&mut command);
                        directory
                    },
                    BatchSize::PerIteration,
                );
            });
        }
    }
    group.finish();
}

criterion_group! {
    name = certificates;
    config = common::walltime_criterion();
    targets = system_certificates
}
criterion_main!(certificates);
