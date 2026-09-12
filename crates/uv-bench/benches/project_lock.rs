//! Validate existing real project locks instead of bypassing freshness checks with `--frozen`.

mod common;

use std::path::Path;
use std::process::Command;

use criterion::{
    BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main,
    measurement::WallTime,
};
use uv_bench::{
    copy_cache, fixture_path, is_codspeed_simulation, run_command, source_fixture,
    uv_command_with_cache,
};

fn lock_command(cache: &Path, project: &Path, lock: &toml::Table) -> Command {
    let mut command = uv_command_with_cache(cache);
    let build_constraints =
        fs_err::read_to_string("../../scripts/benchmark/source-build-constraints.txt")
            .expect("Missing build constraints")
            .lines()
            .map(|line| format!("{line:?}"))
            .collect::<Vec<_>>()
            .join(",");
    let config = project.with_file_name("build-constraints.toml");
    fs_err::write(
        &config,
        format!("build-constraint-dependencies=[{build_constraints}]\n"),
    )
    .expect("Failed to write build constraints");
    command
        .arg("--config-file")
        .arg(config)
        .env(
            "UV_PYTHON_INSTALL_DIR",
            std::path::absolute("../../.cache/bench-python")
                .expect("Failed to locate benchmark Python"),
        )
        .args(["--offline", "--no-progress", "--project"])
        .arg(project)
        .args([
            "lock",
            "--locked",
            "--managed-python",
            "--python",
            "3.12.11",
            "--find-links",
        ])
        .arg(std::path::absolute("../../.cache/bench-fixtures").expect("Missing fixtures"));
    // `--no-config` isolates the command from the host. Restore only the resolution policy
    // recorded in the frozen lock, including relative cutoffs and package-specific exceptions.
    if let Some(options) = lock.get("options").and_then(toml::Value::as_table) {
        if let Some(cutoff) = options
            .get("exclude-newer-span")
            .or_else(|| options.get("exclude-newer"))
            .and_then(toml::Value::as_str)
        {
            command.args(["--exclude-newer", cutoff]);
        }
        if let Some(packages) = options
            .get("exclude-newer-package")
            .and_then(toml::Value::as_table)
        {
            for (name, cutoff) in packages {
                command.arg("--exclude-newer-package").arg(format!(
                    "{name}={}",
                    cutoff.as_str().expect("Invalid package cutoff")
                ));
            }
        }
    }
    command
}

fn project_lock(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let mut group = c.benchmark_group("project_lock");
    group.sampling_mode(SamplingMode::Flat);
    for name in ["packse", "uv", "prefect"] {
        let contents = fs_err::read_to_string(fixture_path(&format!("{name}.lock")))
            .expect("Failed to read frozen lock");
        let lock: toml::Table = toml::from_str(&contents).expect("Invalid frozen lock");
        let metadata = fixture_path(&format!("{name}.pyproject.toml"));
        group.bench_function(BenchmarkId::new("unchanged", name), |b| {
            b.iter_batched(
                || {
                    let directory = tempfile::tempdir().expect("Failed to create project");
                    let project = directory.path().join("project");
                    if name == "prefect" {
                        // Prefect has a dynamic version. Retain its actual backend and source
                        // files so a metadata-build regression remains a valid operation.
                        copy_cache(&source_fixture(name), &project)
                            .expect("Failed to copy project source");
                    } else {
                        fs_err::create_dir_all(&project).expect("Failed to create project");
                    }
                    fs_err::copy(&metadata, project.join("pyproject.toml"))
                        .expect("Failed to copy project metadata");
                    fs_err::write(project.join("uv.lock"), &contents)
                        .expect("Failed to copy lockfile");
                    let command = lock_command(&directory.path().join("cache"), &project, &lock);
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
    name = locks;
    config = common::walltime_criterion();
    targets = project_lock
}
criterion_main!(locks);
