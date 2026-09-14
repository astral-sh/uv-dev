//! Resolve real package releases through two independent package-index mirrors.

mod common;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::{FixtureServer, is_codspeed_simulation, run_command};

fn multi_index(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let server = FixtureServer::start(&[]);
    let mut group = c.benchmark_group("multi_index");
    for (name, first, second, format) in [
        ("simple", "/simple-a/", "/simple-b/", "simple"),
        ("flat", "/flat/a", "/flat/b", "flat"),
    ] {
        group.bench_function(name, |b| {
            b.iter_batched(
                || {
                    let directory = tempfile::tempdir().expect("Failed to create project directory");
                    let requirements = directory.path().join("requirements.in");
                    fs_err::write(&requirements, "flask==3.1.2\njupyterlab==4.4.7\nnumpy==2.2.6\nsympy==1.14.0\n")
                        .expect("Failed to write requirements");
                    let config = directory.path().join("uv.toml");
                    fs_err::write(&config, format!(
                        "index-strategy = \"unsafe-best-match\"\n\n[[index]]\nname = \"first\"\nurl = \"{}\"\nformat = \"{format}\"\n\n[[index]]\nname = \"second\"\nurl = \"{}\"\nformat = \"{format}\"\ndefault = true\n",
                        server.url(first), server.url(second),
                    )).expect("Failed to write index configuration");
                    let mut command = server.command(&directory.path().join("cache"));
                    command.arg("--config-file").arg(config)
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
    group.finish();
}

criterion_group! {
    name = indexes;
    config = common::walltime_criterion();
    targets = multi_index
}
criterion_main!(indexes);
