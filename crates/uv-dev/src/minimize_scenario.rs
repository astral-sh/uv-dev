//! Reduce a real fixed-environment resolver counterexample to a replayable Packse fixture.

use std::path::PathBuf;

use anstream::println;
use anyhow::{Context, Result, ensure};

use uv_python::PythonVersion;
use uv_test::TestContext;
use uv_test::packse::check::{ScenarioPlatform, ScenarioTarget, check_scenario};
use uv_test::packse::minimize::minimize_scenario;
use uv_test::packse::scenario::ScenarioDocument;

use crate::check_scenarios::save_scenario_input;

#[derive(clap::Args)]
pub(crate) struct Args {
    /// The uv executable that reproduces the mismatch.
    #[arg(long, value_name = "PATH")]
    uv: PathBuf,

    /// The exact Python marker version to resolve for.
    #[arg(long, default_value = "3.12")]
    python_version: PythonVersion,

    /// A representative target platform: linux, macos, or windows.
    #[arg(long, default_value = "linux")]
    python_platform: ScenarioPlatform,

    /// Reject graphs whose exhaustive search space exceeds this many selections.
    #[arg(long, default_value_t = 100_000)]
    max_states: usize,

    /// Maximum number of candidate deletions to check.
    #[arg(long, default_value_t = 1_000)]
    max_attempts: usize,

    /// Write the reduced replay to this file without replacing a different existing input.
    #[arg(long, value_name = "PATH")]
    output: PathBuf,

    /// The Packse fixture that reproduces a fixed-environment semantic mismatch.
    #[arg(value_name = "SCENARIO")]
    scenario: PathBuf,
}

pub(crate) fn main(args: &Args) -> Result<()> {
    ensure!(
        args.max_states > 0,
        "--max-states must be greater than zero"
    );
    ensure!(
        args.max_attempts > 0,
        "--max-attempts must be greater than zero"
    );
    let uv = fs_err::canonicalize(&args.uv)
        .with_context(|| format!("failed to find uv executable `{}`", args.uv.display()))?;
    let document = ScenarioDocument::from_path(&args.scenario)?;
    let target = ScenarioTarget {
        python: args.python_version.clone(),
        platform: args.python_platform,
    };
    let interpreter = format!("{}.{}", target.python.major(), target.python.minor());
    let result = minimize_scenario(
        &document,
        &target,
        args.max_states,
        args.max_attempts,
        |scenario| {
            let context = TestContext::new_with_versions_and_bin(&[&interpreter], uv.clone());
            check_scenario(&context, scenario, &target, args.max_states)
        },
    )?;
    if let Some(parent) = args
        .output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs_err::create_dir_all(parent)?;
    }
    save_scenario_input(&args.output, &result.document.to_toml()?)?;
    let completeness = if result.deletion_minimal {
        "deletion-minimal"
    } else {
        "candidate budget exhausted"
    };
    println!(
        "{} -> {} ({completeness}; {} accepted deletions in {} candidate checks)",
        result.original_size, result.reduced_size, result.accepted, result.attempts,
    );
    println!("{}: {}", args.output.display(), result.failure);
    Ok(())
}
