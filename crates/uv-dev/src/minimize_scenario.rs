//! Reduce a real resolver counterexample to a replayable Packse fixture.

use std::fmt;
use std::path::{Path, PathBuf};

use anstream::println;
use anyhow::{Context, Result, ensure};

use uv_python::PythonVersion;
use uv_test::TestContext;
use uv_test::packse::check::{
    ScenarioPlatform, ScenarioTarget, check_lock_scenario, check_project_lock_scenario,
    check_scenario,
};
use uv_test::packse::minimize::{
    InterruptedReduction, MinimizedScenario, minimize_lock_scenario,
    minimize_project_lock_scenario, minimize_scenario,
};
use uv_test::packse::project::ScenarioProject;
use uv_test::packse::scenario::ScenarioDocument;

use crate::check_scenarios::save_scenario_input;

#[derive(clap::Args)]
pub(crate) struct Args {
    /// The uv executable that reproduces the mismatch.
    #[arg(long, value_name = "PATH")]
    uv: PathBuf,

    /// Exact Python marker versions to resolve for, separated by commas.
    #[arg(long, value_delimiter = ',', default_value = "3.12")]
    python_version: Vec<PythonVersion>,

    /// Representative target platforms: linux, macos, or windows, separated by commas.
    #[arg(long, value_delimiter = ',', default_value = "linux")]
    python_platform: Vec<ScenarioPlatform>,

    /// Reduce a universal project lock instead of a fixed `pip compile` result.
    #[arg(long)]
    lock: bool,

    /// Include explicit project-extra and dependency-group exports.
    #[arg(long, requires = "lock")]
    project_selections: bool,

    /// Reject graphs whose exhaustive search space exceeds this many selections.
    #[arg(long, default_value_t = 100_000)]
    max_states: usize,

    /// Maximum number of candidate deletions to check.
    #[arg(long, default_value_t = 1_000)]
    max_attempts: usize,

    /// Write the reduced replay to this file without replacing a different existing input.
    #[arg(long, value_name = "PATH")]
    output: PathBuf,

    /// The Packse fixture that reproduces a resolver or lockfile mismatch.
    #[arg(value_name = "SCENARIO")]
    scenario: PathBuf,
}

pub(crate) fn main(args: &Args) -> Result<()> {
    match minimize(args) {
        Ok(()) => Ok(()),
        Err(error) => {
            let Some(interrupted) = error.downcast_ref::<InterruptedReduction>() else {
                return Err(error);
            };
            let directory = args.output.with_extension("interrupted");
            if let Err(capture_error) = write_interruption(args, &directory, interrupted, &error) {
                return Err(error.context(format!(
                    "failed to save interrupted reduction to `{}`: {capture_error:#}",
                    directory.display()
                )));
            }
            Err(error.context(format!(
                "interrupted reduction saved to `{}`",
                directory.display()
            )))
        }
    }
}

fn minimize(args: &Args) -> Result<()> {
    ensure!(
        args.max_states > 0,
        "--max-states must be greater than zero"
    );
    ensure!(
        args.max_attempts > 0,
        "--max-attempts must be greater than zero"
    );
    let targets = ScenarioTarget::matrix(&args.python_version, &args.python_platform);
    let target = targets.first().context("at least one target is required")?;
    ensure!(
        args.lock || targets.len() == 1,
        "fixed-environment reduction requires exactly one Python version and platform"
    );
    let uv = fs_err::canonicalize(&args.uv)
        .with_context(|| format!("failed to find uv executable `{}`", args.uv.display()))?;
    let document = ScenarioDocument::from_path(&args.scenario)?;
    let interpreter = format!("{}.{}", target.python.major(), target.python.minor());
    if args.lock {
        let result = if args.project_selections {
            minimize_project_lock_scenario(
                &document,
                &targets,
                args.max_states,
                args.max_attempts,
                |scenario| {
                    let selections = ScenarioProject::new(scenario)?.selection_matrix();
                    let context =
                        TestContext::new_with_versions_and_bin(&[&interpreter], uv.clone());
                    check_project_lock_scenario(
                        &context,
                        scenario,
                        &targets,
                        &selections,
                        args.max_states,
                    )
                },
            )?
        } else {
            minimize_lock_scenario(
                &document,
                &targets,
                args.max_states,
                args.max_attempts,
                |scenario| {
                    let context =
                        TestContext::new_with_versions_and_bin(&[&interpreter], uv.clone());
                    check_lock_scenario(&context, scenario, &targets, args.max_states)
                },
            )?
        };
        return write_result(args, &result);
    }
    let result = minimize_scenario(
        &document,
        target,
        args.max_states,
        args.max_attempts,
        |scenario| {
            let context = TestContext::new_with_versions_and_bin(&[&interpreter], uv.clone());
            check_scenario(&context, scenario, target, args.max_states)
        },
    )?;
    write_result(args, &result)
}

fn write_result<Failure: fmt::Display>(
    args: &Args,
    result: &MinimizedScenario<Failure>,
) -> Result<()> {
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

fn write_interruption(
    args: &Args,
    directory: &Path,
    interrupted: &InterruptedReduction,
    error: &anyhow::Error,
) -> Result<()> {
    if let Some(parent) = directory
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs_err::create_dir_all(parent)?;
    }
    fs_err::create_dir(directory)?;
    save_scenario_input(
        &directory.join("candidate.toml"),
        &interrupted.candidate.to_toml()?,
    )?;
    save_scenario_input(
        &directory.join("last-reproducer.toml"),
        &interrupted.last_reproducer.to_toml()?,
    )?;
    save_scenario_input(
        &directory.join("failure.json"),
        &serde_json::to_string_pretty(&serde_json::json!({
            "error": format!("{error:#}"),
            "candidate_checks": interrupted.attempts,
            "accepted_deletions": interrupted.accepted,
            "source_scenario": args.scenario,
            "uv": args.uv,
            "lock": args.lock,
            "project_selections": args.project_selections,
            "max_states": args.max_states,
            "targets": ScenarioTarget::matrix(&args.python_version, &args.python_platform)
                .iter()
                .map(|target| serde_json::json!({
                    "python": target.python.to_string(),
                    "platform": target.platform.as_str(),
                }))
                .collect::<Vec<_>>(),
        }))?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_interrupted_inputs_without_overwriting_evidence() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let args = Args {
            uv: PathBuf::from("uv"),
            python_version: vec!["3.12".parse().expect("valid Python version")],
            python_platform: vec![ScenarioPlatform::Linux],
            lock: true,
            project_selections: true,
            max_states: 27,
            max_attempts: 10,
            output: directory.path().join("reduced.toml"),
            scenario: directory.path().join("original.toml"),
        };
        let last_reproducer: ScenarioDocument = r#"
name = "interrupted"
[root]
requires = ["a"]
[expected]
satisfiable = true
[packages.a.versions."1"]
"#
        .parse()?;
        let candidate: ScenarioDocument = r#"
name = "interrupted"
[root]
requires = []
[expected]
satisfiable = true
[packages.a.versions."1"]
"#
        .parse()?;
        let candidate_toml = candidate.to_toml()?;
        let last_reproducer_toml = last_reproducer.to_toml()?;
        let error = anyhow::anyhow!("failed to start uv").context(InterruptedReduction {
            candidate,
            last_reproducer,
            attempts: 3,
            accepted: 1,
        });
        let interrupted = error
            .downcast_ref::<InterruptedReduction>()
            .expect("interruption context");
        let evidence = args.output.with_extension("interrupted");
        write_interruption(&args, &evidence, interrupted, &error)?;
        assert_eq!(
            fs_err::read_to_string(evidence.join("candidate.toml"))?,
            candidate_toml
        );
        assert_eq!(
            fs_err::read_to_string(evidence.join("last-reproducer.toml"))?,
            last_reproducer_toml
        );
        let failure: serde_json::Value =
            serde_json::from_slice(&fs_err::read(evidence.join("failure.json"))?)?;
        assert_eq!(failure["candidate_checks"], 3);
        assert_eq!(failure["accepted_deletions"], 1);
        assert_eq!(failure["project_selections"], true);
        assert!(write_interruption(&args, &evidence, interrupted, &error).is_err());
        assert_eq!(
            fs_err::read_to_string(evidence.join("candidate.toml"))?,
            candidate_toml
        );
        Ok(())
    }
}
