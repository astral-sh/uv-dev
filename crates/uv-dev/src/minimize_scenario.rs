//! Reduce a real resolver counterexample to a replayable Packse fixture.

use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anstream::println;
use anyhow::{Context, Result, ensure};

use uv_python::PythonVersion;
use uv_test::TestContext;
use uv_test::packse::check::{
    LockCheckOptions, LockEvidenceMode, LockfileMode, ScenarioPlatform, ScenarioTarget,
    check_lock_scenario, check_project_lock_scenario, check_scenario,
    check_witnessed_project_lock_scenario,
};
use uv_test::packse::generate::WitnessedProjectGraph;
use uv_test::packse::minimize::{
    InterruptedReduction, MinimizedScenario, MinimizedWitnessedLockScenario,
    minimize_lock_scenario, minimize_project_lock_scenario, minimize_scenario,
    minimize_witnessed_project_lock_scenario,
};
use uv_test::packse::project::ScenarioProject;
use uv_test::packse::scenario::ScenarioDocument;
use uv_test::packse::witness::MarkerWitnessCertificate;

use crate::check_scenarios::{DEFAULT_MAX_WITNESS_WORK, read_witness, save_scenario_input};

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

    /// Reproduce the mismatch using metadata-free preview lockfiles.
    #[arg(long, requires = "lock")]
    lock_without_metadata: bool,

    /// Include explicit project-extra and dependency-group exports.
    #[arg(long, requires = "lock")]
    project_selections: bool,

    /// Re-certify this saved fixed assignment before checking each project deletion.
    #[arg(long, requires_all = ["lock", "project_selections"], value_name = "PATH")]
    witness: Option<PathBuf>,

    /// Maximum requirement evaluations in each whole-domain witness proof (defaults to 100,000).
    #[arg(long, requires = "witness", value_name = "COUNT")]
    max_witness_work: Option<usize>,

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

impl Args {
    fn lock_options(&self) -> LockCheckOptions {
        LockCheckOptions {
            max_states: self.max_states,
            lockfile: if self.lock_without_metadata {
                LockfileMode::WithoutMetadata
            } else {
                LockfileMode::Standard
            },
            evidence: LockEvidenceMode::PrintedV1,
        }
    }
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
    ensure!(
        args.witness.is_none() || (args.lock && args.project_selections),
        "--witness requires --lock and --project-selections"
    );
    ensure!(
        args.max_witness_work.is_none() || args.witness.is_some(),
        "--max-witness-work requires --witness"
    );
    let max_witness_work = args.max_witness_work.unwrap_or(DEFAULT_MAX_WITNESS_WORK);
    ensure!(
        max_witness_work > 0,
        "--max-witness-work must be greater than zero"
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
        let options = args.lock_options();
        if let Some(path) = args.witness.as_deref() {
            let graph = WitnessedProjectGraph {
                document,
                assignment: read_witness(path)?,
            };
            let result = minimize_witnessed_project_lock_scenario(
                &graph,
                &targets,
                args.max_states,
                args.max_attempts,
                max_witness_work,
                |candidate| {
                    let scenario = candidate.document.scenario()?;
                    let selections = ScenarioProject::new(&scenario)?.selection_matrix();
                    let context =
                        TestContext::new_with_versions_and_bin(&[&interpreter], uv.clone());
                    check_witnessed_project_lock_scenario(
                        &context,
                        candidate,
                        &targets,
                        &selections,
                        options,
                        max_witness_work,
                    )
                },
            )?;
            return write_witnessed_result(args, &result);
        }
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
                    check_project_lock_scenario(&context, scenario, &targets, &selections, options)
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
                    check_lock_scenario(&context, scenario, &targets, options)
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
    report_result(args, result);
    Ok(())
}

fn report_result<Failure: fmt::Display>(args: &Args, result: &MinimizedScenario<Failure>) {
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
}

fn write_witnessed_result(args: &Args, result: &MinimizedWitnessedLockScenario) -> Result<()> {
    let witness_path = args.output.with_extension("witness.json");
    ensure!(
        args.output != witness_path,
        "the scenario and witness output paths must be distinct"
    );
    let document = result.reduction.document.to_toml()?;
    let witness = witness_contents(args, &result.reduction.document, &result.witness)?;
    // Check both existing files before creating either member of the pair. The create-new writes
    // still reject a conflicting file that appears after this preflight.
    ensure_output_available(&args.output, &document)?;
    ensure_output_available(&witness_path, &witness)?;
    if let Some(parent) = args
        .output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs_err::create_dir_all(parent)?;
    }
    save_scenario_input(&witness_path, &witness)?;
    save_scenario_input(&args.output, &document)?;
    report_result(args, &result.reduction);
    println!("{}: certified fixed assignment", witness_path.display());
    Ok(())
}

fn ensure_output_available(path: &Path, contents: &str) -> Result<()> {
    match fs_err::read_to_string(path) {
        Ok(existing) => ensure!(
            existing == contents,
            "refusing to overwrite different scenario input `{}`",
            path.display()
        ),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn witness_contents(
    args: &Args,
    document: &ScenarioDocument,
    witness: &MarkerWitnessCertificate,
) -> Result<String> {
    let contents = serde_json::to_string_pretty(&serde_json::json!({
        "assignment": witness.assignment(),
        "marker_certificate": witness,
        "scenario": document.scenario()?.name,
        "source_scenario": args.scenario,
        "source_witness": args.witness,
        "lock_options": args.lock_options(),
        "max_witness_work": args.max_witness_work.unwrap_or(DEFAULT_MAX_WITNESS_WORK),
        "requested_targets": ScenarioTarget::matrix(&args.python_version, &args.python_platform)
            .iter()
            .map(|target| serde_json::json!({
                "python": target.python.to_string(),
                "platform": target.platform.as_str(),
            }))
            .collect::<Vec<_>>(),
    }))?;
    Ok(format!("{contents}\n"))
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
    if let Some(witnesses) = &interrupted.witnesses {
        save_scenario_input(
            &directory.join("candidate.witness.json"),
            &witness_contents(args, &interrupted.candidate, &witnesses.candidate)?,
        )?;
        save_scenario_input(
            &directory.join("last-reproducer.witness.json"),
            &witness_contents(
                args,
                &interrupted.last_reproducer,
                &witnesses.last_reproducer,
            )?,
        )?;
    }
    save_scenario_input(
        &directory.join("failure.json"),
        &serde_json::to_string_pretty(&serde_json::json!({
            "error": format!("{error:#}"),
            "candidate_checks": interrupted.attempts,
            "accepted_deletions": interrupted.accepted,
            "source_scenario": args.scenario,
            "uv": args.uv,
            "lock": args.lock,
            "lock_options": args.lock.then_some(args.lock_options()),
            "project_selections": args.project_selections,
            "source_witness": args.witness,
            "max_witness_work": args.witness.as_ref().map(|_| {
                args.max_witness_work.unwrap_or(DEFAULT_MAX_WITNESS_WORK)
            }),
            "witnesses_captured": interrupted.witnesses.is_some(),
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
    use anyhow::bail;
    use clap::Parser;

    use super::*;
    use uv_test::packse::check::LockScenarioFailureKind;
    use uv_test::packse::minimize::{InterruptedWitnesses, ScenarioSize};
    use uv_test::packse::witness::certify_project_marker_witness;

    fn reduction_args(directory: &Path) -> Args {
        Args {
            uv: PathBuf::from("uv"),
            python_version: vec!["3.12".parse().expect("valid Python version")],
            python_platform: vec![ScenarioPlatform::Linux],
            lock: true,
            lock_without_metadata: true,
            project_selections: true,
            witness: None,
            max_witness_work: None,
            max_states: 27,
            max_attempts: 10,
            output: directory.join("reduced.toml"),
            scenario: directory.join("original.toml"),
        }
    }

    fn witnessed_result() -> Result<MinimizedWitnessedLockScenario> {
        let document: ScenarioDocument = r#"
name = "witnessed-output"
[root]
requires = ["a; sys_platform == 'win32'"]
requires_python = ">=3.12,<3.15"
[expected]
satisfiable = true
[packages.a.versions."1.0.0"]
requires = ["missing; sys_platform != 'win32'"]
"#
        .parse()?;
        let assignment = [("a".parse()?, "1.0.0".parse()?)].into_iter().collect();
        let witness = certify_project_marker_witness(&document, &assignment, 1_000)?;
        let size = ScenarioSize {
            packages: 1,
            versions: 1,
            requirements: 2,
        };
        Ok(MinimizedWitnessedLockScenario {
            reduction: MinimizedScenario {
                document,
                failure: LockScenarioFailureKind::FalseUnsatisfiable,
                original_size: size,
                reduced_size: size,
                attempts: 3,
                accepted: 0,
                deletion_minimal: true,
            },
            witness,
        })
    }

    #[test]
    fn retains_interrupted_inputs_without_overwriting_evidence() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut args = reduction_args(directory.path());
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
            witnesses: None,
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
        assert_eq!(failure["lock_options"]["max_states"], 27);
        assert_eq!(failure["lock_options"]["lockfile"], "without-metadata");
        assert_eq!(failure["witnesses_captured"], false);
        assert!(failure["source_witness"].is_null());
        assert!(failure["max_witness_work"].is_null());
        assert!(write_interruption(&args, &evidence, interrupted, &error).is_err());
        assert_eq!(
            fs_err::read_to_string(evidence.join("candidate.toml"))?,
            candidate_toml
        );

        args.lock_without_metadata = false;
        let standard = directory.path().join("standard");
        write_interruption(&args, &standard, interrupted, &error)?;
        let failure: serde_json::Value =
            serde_json::from_slice(&fs_err::read(standard.join("failure.json"))?)?;
        assert_eq!(failure["lock_options"]["lockfile"], "standard");

        args.lock = false;
        let fixed = directory.path().join("fixed");
        write_interruption(&args, &fixed, interrupted, &error)?;
        let failure: serde_json::Value =
            serde_json::from_slice(&fs_err::read(fixed.join("failure.json"))?)?;
        assert!(failure["lock_options"].is_null());
        Ok(())
    }

    #[test]
    fn witness_flags_require_project_lock_reduction() -> Result<()> {
        let crate::Cli::MinimizeScenario(args) = crate::Cli::try_parse_from([
            "uv-dev",
            "minimize-scenario",
            "--uv",
            "uv",
            "--output",
            "reduced.toml",
            "--lock",
            "--project-selections",
            "--witness",
            "graph.witness.json",
            "--max-witness-work",
            "123",
            "graph.toml",
        ])?
        else {
            bail!("expected the scenario reducer");
        };
        assert_eq!(args.max_witness_work, Some(123));
        assert_eq!(args.witness, Some(PathBuf::from("graph.witness.json")));

        for flags in [
            vec!["--witness", "graph.witness.json"],
            vec!["--lock", "--witness", "graph.witness.json"],
            vec![
                "--lock",
                "--project-selections",
                "--max-witness-work",
                "123",
            ],
        ] {
            let mut arguments = vec![
                "uv-dev",
                "minimize-scenario",
                "--uv",
                "uv",
                "--output",
                "reduced.toml",
            ];
            arguments.extend(flags);
            arguments.push("graph.toml");
            assert!(crate::Cli::try_parse_from(arguments).is_err());
        }
        Ok(())
    }

    #[test]
    fn writes_a_replayable_witness_pair_without_overwriting_inputs() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut args = reduction_args(directory.path());
        args.witness = Some(directory.path().join("original.witness.json"));
        args.max_witness_work = Some(1_000);
        let result = witnessed_result()?;
        write_witnessed_result(&args, &result)?;
        let witness_path = args.output.with_extension("witness.json");
        assert_eq!(read_witness(&witness_path)?, *result.witness.assignment());
        let document = ScenarioDocument::from_path(&args.output)?;
        certify_project_marker_witness(&document, &read_witness(&witness_path)?, 1_000)?;
        let stored: serde_json::Value = serde_json::from_slice(&fs_err::read(&witness_path)?)?;
        assert_eq!(stored["scenario"], "witnessed-output");
        assert_eq!(stored["marker_certificate"]["checked_requirements"], 2);
        assert_eq!(stored["lock_options"]["lockfile"], "without-metadata");
        write_witnessed_result(&args, &result)?;

        args.output = directory.path().join("conflict.toml");
        let conflicting_witness = args.output.with_extension("witness.json");
        fs_err::write(&conflicting_witness, "different witness\n")?;
        assert!(write_witnessed_result(&args, &result).is_err());
        assert!(!args.output.exists());
        assert_eq!(
            fs_err::read_to_string(conflicting_witness)?,
            "different witness\n"
        );
        Ok(())
    }

    #[test]
    fn interrupted_witnesses_can_be_recertified_without_the_original_file() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut args = reduction_args(directory.path());
        args.witness = Some(directory.path().join("absent-original.witness.json"));
        args.max_witness_work = Some(1_000);
        let result = witnessed_result()?;
        let document = result.reduction.document;
        let assignment = result.witness.assignment().clone();
        let error = anyhow::anyhow!("failed to start uv").context(InterruptedReduction {
            candidate: document.clone(),
            last_reproducer: document.clone(),
            attempts: 3,
            accepted: 1,
            witnesses: Some(InterruptedWitnesses {
                candidate: certify_project_marker_witness(&document, &assignment, 1_000)?,
                last_reproducer: result.witness,
            }),
        });
        let interrupted = error
            .downcast_ref::<InterruptedReduction>()
            .expect("interruption context");
        let evidence = args.output.with_extension("interrupted");
        write_interruption(&args, &evidence, interrupted, &error)?;
        for name in ["candidate", "last-reproducer"] {
            let document = ScenarioDocument::from_path(&evidence.join(format!("{name}.toml")))?;
            let witness = read_witness(&evidence.join(format!("{name}.witness.json")))?;
            assert_eq!(witness, assignment);
            certify_project_marker_witness(&document, &witness, 1_000)?;
        }
        let failure: serde_json::Value =
            serde_json::from_slice(&fs_err::read(evidence.join("failure.json"))?)?;
        assert_eq!(failure["witnesses_captured"], true);
        assert_eq!(failure["max_witness_work"], 1_000);
        Ok(())
    }
}
