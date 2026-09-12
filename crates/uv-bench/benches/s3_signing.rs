//! Resolve packages through several authenticated S3 index prefixes.

mod common;

use std::fmt::Write;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::{FixtureServer, is_codspeed_simulation, run_command};

fn s3_signing(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let server = FixtureServer::start_s3(&[]);
    c.bench_function("s3_signing/four_indexes", |b| {
        b.iter_batched(
            || {
                let directory = tempfile::tempdir().expect("Failed to create project directory");
                let requirements = directory.path().join("requirements.in");
                fs_err::write(&requirements, "flask==3.1.2\njupyterlab==4.4.7\nnumpy==2.2.6\nsympy==1.14.0\n")
                    .expect("Failed to write requirements");
                let mut config = "index-strategy = \"unsafe-best-match\"\n".to_owned();
                for name in ["a", "b", "c", "d"] {
                    writeln!(config, "\n[[index]]\nname = \"{name}\"\nurl = \"{}\"\nauthenticate = \"always\"\ndefault = {}", server.url(&format!("/simple-{name}/")), name == "d")
                        .expect("Failed to format index configuration");
                }
                let config_file = directory.path().join("uv.toml");
                fs_err::write(&config_file, config).expect("Failed to write index configuration");
                let mut command = server.command(&directory.path().join("cache"));
                for (name, _) in std::env::vars_os() {
                    if name.to_string_lossy().starts_with("AWS_") {
                        command.env_remove(name);
                    }
                }
                command
                    .env("UV_S3_ENDPOINT_URL", server.url("/"))
                    .env("AWS_ACCESS_KEY_ID", "benchmark-access-key")
                    .env("AWS_SECRET_ACCESS_KEY", "benchmark-secret-key")
                    .env("AWS_REGION", "us-east-1")
                    .env("AWS_EC2_METADATA_DISABLED", "true")
                    .env("AWS_CONFIG_FILE", directory.path().join("missing-config"))
                    .env("AWS_SHARED_CREDENTIALS_FILE", directory.path().join("missing-credentials"))
                    .args(["--preview-features", "s3-endpoint", "--config-file"])
                    .arg(config_file)
                    .args(["pip", "compile", "--no-deps", "--no-build", "--no-header", "--no-annotate", "--python-platform", "aarch64-manylinux2014"])
                    .arg(requirements);
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

criterion_group! {
    name = signing;
    config = common::walltime_criterion();
    targets = s3_signing
}
criterion_main!(signing);
