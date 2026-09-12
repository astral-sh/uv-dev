//! Start installed Python entrypoints with small and larger import graphs.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use criterion::{BatchSize, Criterion, criterion_group, criterion_main, measurement::WallTime};
use uv_bench::{
    PreparedEnvironment, environment_fixtures, fixture_path, is_codspeed_simulation, run_command,
    uv_command,
};

fn sample_environment(directory: &Path) -> PathBuf {
    let environment = directory.join(".venv");
    run_command(
        uv_command()
            .env(
                "UV_PYTHON_INSTALL_DIR",
                std::path::absolute("../../.cache/bench-python").unwrap(),
            )
            .args([
                "--offline",
                "venv",
                "--no-project",
                "--managed-python",
                "--python",
                "3.12.11",
            ])
            .arg(&environment),
    );
    let output = uv_command()
        .args(["--offline", "python", "find"])
        .arg(&environment)
        .stdout(Stdio::piped())
        .output()
        .expect("Failed to locate sample Python");
    assert!(output.status.success(), "Failed to locate sample Python");
    let python = PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("Python path is not UTF-8")
            .trim(),
    );
    run_command(
        uv_command()
            .args(["--offline", "pip", "install", "--no-deps", "--python"])
            .arg(&python)
            .arg(std::path::absolute(fixture_path("sampleproject-4.0.0-py3-none-any.whl")).unwrap())
            .arg(std::path::absolute(fixture_path("peppercorn-0.6-py3-none-any.whl")).unwrap()),
    );
    python
}

fn entrypoint(python: &Path, name: &str, arguments: &[&str], directory: &Path) -> Command {
    let executable = python
        .parent()
        .expect("Python has no parent directory")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    assert!(
        executable.is_file(),
        "Missing entrypoint {}",
        executable.display()
    );
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .current_dir(directory)
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH")
        .env_remove("VIRTUAL_ENV")
        .env_remove("CONDA_PREFIX")
        .stdout(Stdio::null());
    command
}

fn entrypoint_startup(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let directory = tempfile::tempdir().expect("Failed to create entrypoint directory");
    // The published PyPA example has a genuinely minimal console script, without an argument
    // parser that imports `re` itself. Larger applications show how wrapper cost is amortized.
    let sample = sample_environment(directory.path());
    let docs = environment_fixtures()
        .into_iter()
        .find(|fixture| fixture.name == "uv_docs")
        .expect("Missing documentation environment fixture");
    let docs = PreparedEnvironment::from_fixture(&docs);
    let mut group = c.benchmark_group("entrypoint_startup");
    for (name, python, executable, arguments) in [
        ("sampleproject", sample.as_path(), "sample", &[][..]),
        ("pygments", docs.python(), "pygmentize", &["-V"][..]),
        ("mkdocs", docs.python(), "mkdocs", &["--version"][..]),
    ] {
        let command = || entrypoint(python, executable, arguments, directory.path());
        run_command(&mut command());
        group.bench_function(name, |b| {
            b.iter_batched(
                command,
                |mut command| run_command(&mut command),
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

criterion_group! {
    name = entrypoints;
    config = common::walltime_criterion();
    targets = entrypoint_startup
}
criterion_main!(entrypoints);
