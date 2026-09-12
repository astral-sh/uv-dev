//! Reuse content-addressed tool environments for pinned and `@latest` invocations.

mod common;

use std::path::Path;
use std::process::Command;

use criterion::{
    BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main,
    measurement::WallTime,
};
use uv_bench::{
    ToolFixture, copy_cache, is_codspeed_simulation, run_command, tool_fixtures,
    uv_command_with_cache,
};

struct CachedTool {
    directory: tempfile::TempDir,
    tool: ToolFixture,
}

impl CachedTool {
    fn prepare(tool: &ToolFixture) -> Self {
        let directory = tempfile::tempdir().expect("Failed to create tool cache");
        let source = Path::new("../../.cache/bench-tool-caches").join(&tool.name);
        assert!(
            source.is_dir(),
            "Missing tool cache. Run `python3 scripts/benchmark/prepare-tools.py --individual-caches`."
        );
        copy_cache(&source, &directory.path().join("cache")).expect("Failed to copy tool cache");
        let cached = Self {
            directory,
            tool: tool.clone(),
        };
        run_command(&mut cached.command(false));
        cached
    }

    fn command(&self, latest: bool) -> Command {
        let mut command = uv_command_with_cache(&self.directory.path().join("cache"));
        command
            .env("UV_TOOL_DIR", self.directory.path().join("tools"))
            .env("UV_TOOL_BIN_DIR", self.directory.path().join("bin"))
            .env(
                "UV_PYTHON_INSTALL_DIR",
                std::path::absolute("../../.cache/bench-python")
                    .expect("Failed to locate benchmark Python"),
            )
            .args([
                "--offline",
                "tool",
                "run",
                "--isolated",
                "--no-build",
                "--managed-python",
                "--python",
                "3.12.11",
                "--constraints",
            ])
            .arg(self.tool.constraints());
        if latest {
            // Keep the latest root release fixed while allowing the already-pinned transitive
            // dependencies to retain their own release dates.
            command
                .arg("--exclude-newer-package")
                .arg(format!("{}=2025-06-12T00:00:00Z", self.tool.name))
                .arg("--from")
                .arg(format!("{}@latest", self.tool.name));
        } else {
            command.arg("--from").arg(self.tool.requirement());
        }
        command.arg(&self.tool.executable).arg("--version");
        command
    }
}

fn cached_tool_run(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let mut group = c.benchmark_group("cached_tool_run");
    group.sampling_mode(SamplingMode::Flat);
    for tool in tool_fixtures().into_iter().filter(|tool| tool.cached_run) {
        for (name, latest) in [("warm", false), ("latest", true)] {
            group.bench_function(BenchmarkId::new(name, &tool.name), |b| {
                b.iter_batched(
                    || {
                        let cached = CachedTool::prepare(&tool);
                        let command = cached.command(latest);
                        (cached, command)
                    },
                    |(cached, mut command)| {
                        run_command(&mut command);
                        cached
                    },
                    BatchSize::PerIteration,
                );
            });
        }
    }
    group.finish();
}

criterion_group! {
    name = tools;
    config = common::walltime_criterion();
    targets = cached_tool_run
}
criterion_main!(tools);
