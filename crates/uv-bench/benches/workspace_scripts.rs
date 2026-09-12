//! Discover PEP 723 scripts in real exported project trees.

mod common;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{copy_cache, is_codspeed_simulation, run_command, source_fixture, uv_command};

#[cfg(target_os = "linux")]
fn script_candidates(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .map(|entry| entry.expect("Failed to enumerate source tree"))
        .filter(|entry| {
            entry.file_type().is_file()
                && entry.path().extension().is_none_or(|extension| {
                    extension.eq_ignore_ascii_case("py") || extension.eq_ignore_ascii_case("pyw")
                })
        })
        .map(|entry| {
            // Dirty pages from the fixture copy are ineligible for eviction.
            fs_err::File::open(entry.path())
                .expect("Failed to open script candidate")
                .sync_data()
                .expect("Failed to flush script candidate data");
            entry.into_path()
        })
        .collect()
}

fn workspace_scripts(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let mut group = c.benchmark_group("workspace_scripts");
    for name in ["sampleproject", "flask", "uv", "django"] {
        // A source inside `.cache` can inherit the checkout's ignore rule for that directory.
        let project = tempfile::tempdir().expect("Failed to create source directory");
        copy_cache(&source_fixture(name), project.path()).expect("Failed to copy source fixture");
        let command = || {
            let mut command = uv_command();
            command
                .args(["--offline", "--project"])
                .arg(project.path())
                .args([
                    "--preview-features",
                    "workspace-list-scripts",
                    "workspace",
                    "list",
                    "--scripts",
                ]);
            command
        };
        run_command(&mut command());
        group.bench_function(BenchmarkId::new("warm", name), |b| {
            b.iter_batched(
                command,
                |mut command| run_command(&mut command),
                BatchSize::PerIteration,
            );
        });

        #[cfg(target_os = "linux")]
        {
            let candidates = script_candidates(project.path());
            group.bench_function(BenchmarkId::new("advised_cold_data", name), |b| {
                b.iter_batched(
                    || {
                        for path in &candidates {
                            let file =
                                fs_err::File::open(path).expect("Failed to open script candidate");
                            rustix::fs::fadvise(&file, 0, None, rustix::fs::Advice::DontNeed)
                                .expect("Failed to advise eviction of script candidate data");
                        }
                        command()
                    },
                    |mut command| run_command(&mut command),
                    BatchSize::PerIteration,
                );
            });
        }
    }
    group.finish();
}

criterion_group! {
    name = workspaces;
    config = common::walltime_criterion();
    targets = workspace_scripts
}
criterion_main!(workspaces);
