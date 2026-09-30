//! Create complete frozen environments concurrently from a shared warm wheel cache.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use criterion::{
    BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main,
    measurement::WallTime,
};
use uv_bench::{
    EnvironmentFixture, environment_fixtures, fixture_path, is_codspeed_simulation, run_command,
    uv_command,
};

struct FrozenProject {
    fixture: EnvironmentFixture,
    pyproject: Vec<u8>,
    lock: Vec<u8>,
    python_directory: PathBuf,
}

impl FrozenProject {
    fn new(fixture: EnvironmentFixture) -> Self {
        let pyproject = fs_err::read(fixture_path(&format!("{}.pyproject.toml", fixture.project)))
            .expect("Failed to read project metadata");
        let lock = fs_err::read(fixture_path(&format!("{}.lock", fixture.project)))
            .expect("Failed to read project lockfile");
        let python_directory = std::path::absolute("../../.cache/bench-python")
            .expect("Failed to locate benchmark Python directory");
        Self {
            fixture,
            pyproject,
            lock,
            python_directory,
        }
    }

    fn command(&self, project: &Path, link_mode: &str) -> Command {
        fs_err::create_dir(project).expect("Failed to create project directory");
        fs_err::write(project.join("pyproject.toml"), &self.pyproject)
            .expect("Failed to write project metadata");
        fs_err::write(project.join("uv.lock"), &self.lock)
            .expect("Failed to write project lockfile");
        let mut command = uv_command();
        command
            .env("UV_PYTHON_INSTALL_DIR", &self.python_directory)
            .args(["--offline", "--no-progress", "--project"])
            .arg(project)
            .args([
                "sync",
                "--frozen",
                "--no-default-groups",
                "--no-install-project",
                "--no-build",
                "--managed-python",
                "--python",
            ])
            .arg(&self.fixture.python)
            .args(["--link-mode", link_mode])
            .args(&self.fixture.sync_args)
            .stderr(Stdio::piped());
        command
    }
}

fn concurrent_environments(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let fixture = environment_fixtures()
        .into_iter()
        .find(|fixture| fixture.name == "prefect")
        .expect("Missing Prefect environment fixture");
    let project = FrozenProject::new(fixture);
    let mut group = c.benchmark_group("concurrent_environments");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);

    for link_mode in ["symlink", "hardlink"] {
        let preflight = tempfile::tempdir().expect("Failed to create preflight directory");
        run_command(&mut project.command(&preflight.path().join("project"), link_mode));

        for processes in [1, 8, 32] {
            group.bench_function(
                BenchmarkId::new(format!("prefect_{link_mode}"), processes),
                |b| {
                    b.iter_batched(
                        || {
                            let directory = tempfile::tempdir()
                                .expect("Failed to create environment directory");
                            let commands = (0..processes)
                                .map(|index| {
                                    project.command(
                                        &directory.path().join(format!("project-{index}")),
                                        link_mode,
                                    )
                                })
                                .collect::<Vec<_>>();
                            (directory, commands)
                        },
                        |(directory, commands)| {
                            let children = commands
                                .into_iter()
                                .map(|mut command| {
                                    command.spawn().expect("Failed to start environment sync")
                                })
                                .collect::<Vec<_>>();
                            let outputs = children
                                .into_iter()
                                .map(|child| {
                                    child
                                        .wait_with_output()
                                        .expect("Failed to wait for environment sync")
                                })
                                .collect::<Vec<_>>();
                            for output in outputs {
                                assert!(
                                    output.status.success(),
                                    "Concurrent environment sync failed: {}",
                                    String::from_utf8_lossy(&output.stderr)
                                );
                            }
                            directory
                        },
                        BatchSize::PerIteration,
                    );
                },
            );
        }
    }
    group.finish();
}

criterion_group! {
    name = environments;
    config = common::walltime_criterion();
    targets = concurrent_environments
}
criterion_main!(environments);
