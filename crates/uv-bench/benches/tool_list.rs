//! Inspect realistic user-level installations of Python CLI tools.

mod common;

use std::process::Stdio;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{PreparedTools, is_codspeed_simulation, run_command, tool_fixtures};

fn tool_list(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let mut group = c.benchmark_group("tool_list");
    for count in [1, 5, 15] {
        let tools = PreparedTools::new(count);
        let output = tools
            .command()
            .args(["tool", "list"])
            .stdout(Stdio::piped())
            .output()
            .expect("Failed to inspect installed tools");
        assert!(output.status.success(), "Failed to inspect installed tools");
        let listing = String::from_utf8(output.stdout).expect("Tool listing is not UTF-8");
        for tool in tool_fixtures().iter().take(count) {
            assert!(
                listing
                    .lines()
                    .any(|line| line.starts_with(&format!("{} v{}", tool.name, tool.version))),
                "Missing tool {}",
                tool.name
            );
        }
        for (name, arguments) in [
            ("default", &[][..]),
            (
                "details",
                &[
                    "--show-paths",
                    "--show-version-specifiers",
                    "--show-with",
                    "--show-extras",
                    "--show-python",
                ][..],
            ),
        ] {
            let command = || {
                let mut command = tools.command();
                command.args(["tool", "list"]).args(arguments);
                command
            };
            run_command(&mut command());
            group.bench_function(BenchmarkId::new(name, count), |b| {
                b.iter_batched(
                    command,
                    |mut command| run_command(&mut command),
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
    targets = tool_list
}
criterion_main!(tools);
