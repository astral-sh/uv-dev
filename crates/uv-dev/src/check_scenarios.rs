//! Compare the real uv resolver with the bounded Packse scenario oracle.

use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use anstream::println;
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::json;

use uv_python::PythonVersion;
use uv_test::TestContext;
use uv_test::packse::check::{
    LockCheckOptions, LockCheckResult, LockfileMode, ScenarioPlatform, ScenarioTarget,
    check_lock_scenario, check_lock_scenario_with_artifacts, check_project_lock_scenario,
    check_project_lock_scenario_with_artifacts, check_scenario, check_scenario_with_artifacts,
    check_witnessed_project_lock_scenario, check_witnessed_project_lock_scenario_with_artifacts,
};
use uv_test::packse::generate::{
    SmallGraphOptions, WitnessedProjectGraph, generate_marker_graph, generate_project_graph,
    generate_satisfiable_project_graph, generate_small_graph,
};
use uv_test::packse::oracle::Selection;
use uv_test::packse::project::ScenarioProject;
use uv_test::packse::scenario::ScenarioDocument;

const DEFAULT_MAX_WITNESS_WORK: usize = 100_000;

#[derive(clap::Args)]
pub(crate) struct Args {
    /// The uv executable to check. Build it with `cargo build --package uv` first.
    #[arg(long, value_name = "PATH")]
    uv: PathBuf,

    /// Exact Python marker versions to resolve for, separated by commas.
    #[arg(long, value_delimiter = ',', default_value = "3.12")]
    python_version: Vec<PythonVersion>,

    /// Representative target platforms: linux, macos, or windows, separated by commas.
    #[arg(long, value_delimiter = ',', default_value = "linux")]
    python_platform: Vec<ScenarioPlatform>,

    /// Reject graphs whose exhaustive search space exceeds this many selections.
    #[arg(long, default_value_t = 100_000)]
    max_states: usize,

    /// Check a universal project lock and its frozen export instead of `pip compile`.
    #[arg(long)]
    lock: bool,

    /// Write and consume metadata-free preview lockfiles throughout the check.
    #[arg(long, requires = "lock")]
    lock_without_metadata: bool,

    /// Check explicit project-extra and dependency-group exports from the universal lock.
    ///
    /// Generated graphs include project roots and marker projections over three Python minor lines.
    #[arg(long, requires = "lock")]
    project_selections: bool,

    /// Re-certify the fixed assignment in a saved `.witness.json` when replaying one project.
    #[arg(
        long,
        requires_all = ["lock", "project_selections"],
        conflicts_with = "seed",
        value_name = "PATH"
    )]
    witness: Option<PathBuf>,

    /// Maximum requirement evaluations in the whole-domain witness proof (defaults to 100,000).
    #[arg(long, requires_all = ["lock", "project_selections"], value_name = "COUNT")]
    max_witness_work: Option<usize>,

    /// Save failed resolver commands and their served wheels in a new directory.
    #[arg(long, value_name = "DIR")]
    failure_dir: Option<PathBuf>,

    /// Generate small graphs beginning at this seed instead of reading scenario files.
    #[arg(long, conflicts_with = "scenarios", requires = "output_dir")]
    seed: Option<u64>,

    /// Add Python and platform markers over three Python minor lines to generated graphs.
    #[arg(long, requires = "seed")]
    markers: bool,

    /// Construct generated projects around an independently checked satisfying assignment.
    #[arg(long, requires_all = ["seed", "project_selections"])]
    satisfiable: bool,

    /// Number of consecutive seeds to check (defaults to 100).
    #[arg(long, requires = "seed")]
    cases: Option<usize>,

    /// Packages per generated graph (defaults to 3, maximum 8).
    #[arg(long, requires = "seed")]
    packages: Option<usize>,

    /// Versions per generated package (defaults to 2, maximum 4).
    #[arg(long, requires = "seed")]
    versions: Option<usize>,

    /// Save complete generated inputs in this directory before checking them.
    #[arg(long, requires = "seed", value_name = "DIR")]
    output_dir: Option<PathBuf>,

    /// Packse scenario TOML files to check.
    #[arg(required_unless_present = "seed", value_name = "SCENARIO")]
    scenarios: Vec<PathBuf>,
}

pub(crate) fn main(args: &Args) -> Result<()> {
    ensure!(
        args.max_states > 0,
        "--max-states must be greater than zero"
    );
    ensure!(
        args.max_witness_work.is_none() || args.satisfiable || args.witness.is_some(),
        "--max-witness-work requires --satisfiable or --witness"
    );
    ensure!(
        args.max_witness_work.unwrap_or(DEFAULT_MAX_WITNESS_WORK) > 0,
        "--max-witness-work must be greater than zero"
    );
    ensure!(
        args.witness.is_none() || args.scenarios.len() == 1,
        "--witness requires exactly one scenario file"
    );
    let replay_witness = args.witness.as_deref().map(read_witness).transpose()?;
    let uv = fs_err::canonicalize(&args.uv)
        .with_context(|| format!("failed to find uv executable `{}`", args.uv.display()))?;
    let targets = ScenarioTarget::matrix(&args.python_version, &args.python_platform);
    let target = targets.first().context("at least one target is required")?;
    // Use any installed patch release in this minor line. The resolver receives the exact marker
    // version separately, so the result does not depend on that interpreter's patch release.
    let interpreter = format!("{}.{}", target.python.major(), target.python.minor());

    if let Some(first_seed) = args.seed {
        let output_dir = args
            .output_dir
            .as_ref()
            .context("--seed requires --output-dir")?;
        let cases = args.cases.unwrap_or(100);
        ensure!(cases > 0, "--cases must be greater than zero");
        let options = SmallGraphOptions {
            packages: args.packages.unwrap_or(3),
            versions: args.versions.unwrap_or(2),
        };
        let minor_lines = if args.markers || args.project_selections {
            3
        } else {
            1
        };
        ensure!(
            targets.iter().all(|other| {
                other.python.major() == target.python.major()
                    && other.python.minor() >= target.python.minor()
                    && u16::from(other.python.minor())
                        < u16::from(target.python.minor()) + minor_lines
            }),
            "generated graphs cover {minor_lines} Python minor line(s) starting at {}.{}",
            target.python.major(),
            target.python.minor(),
        );
        fs_err::create_dir_all(output_dir)?;
        let mut satisfiable = 0;
        let mut unsatisfiable = 0;
        let mut export_projections = 0;
        for offset in 0..cases {
            let seed = first_seed
                .checked_add(u64::try_from(offset)?)
                .context("the requested seed range overflows u64")?;
            let (document, witness, assignment) = if args.satisfiable {
                let graph =
                    generate_satisfiable_project_graph(seed, options, &targets, args.max_states)?;
                let universal_certificate = graph.certify_universal_witness()?;
                let checked_projections = graph.check_witness(&targets)?;
                let assignment = graph.assignment.clone();
                let witness = serde_json::to_string_pretty(&json!({
                    "seed": seed,
                    "assignment": graph.assignment,
                    "universal_certificate": universal_certificate,
                    "checked_targets": targets.iter().map(|target| json!({
                        "python_version": target.python.to_string(),
                        "python_platform": target.platform.as_str(),
                    })).collect::<Vec<_>>(),
                    "checked_projections": checked_projections,
                }))?;
                (
                    graph.document,
                    Some(format!("{witness}\n")),
                    Some(assignment),
                )
            } else if args.project_selections {
                (
                    generate_project_graph(seed, options, target, args.max_states)?,
                    None,
                    None,
                )
            } else if args.markers {
                (
                    generate_marker_graph(seed, options, target, args.max_states)?,
                    None,
                    None,
                )
            } else {
                (
                    generate_small_graph(seed, options, target, args.max_states)?,
                    None,
                    None,
                )
            };
            let scenario = document.scenario()?;
            let path = output_dir.join(format!("{}.toml", scenario.name));
            save_scenario_input(&path, &document.to_toml()?)?;
            if let Some(witness) = witness {
                save_scenario_input(&path.with_extension("witness.json"), &witness)?;
            }
            let result = check_case(
                &uv,
                &interpreter,
                &document,
                &targets,
                args,
                assignment.as_ref(),
            )
            .with_context(|| format!("generated scenario `{}` failed", path.display()))?;
            satisfiable += result.satisfiable;
            unsatisfiable += result.unsatisfiable;
            export_projections += result.export_projections;
        }
        if args.project_selections {
            let kind = if args.satisfiable {
                "witness-backed project"
            } else {
                "project"
            };
            println!(
                "Checked {cases} generated {kind} graphs (locks: {satisfiable} satisfiable, {unsatisfiable} unsatisfiable; export projections: {export_projections})"
            );
            return Ok(());
        }
        let kind = if args.lock { "locks" } else { "projections" };
        println!(
            "Checked {cases} generated graphs ({kind}: {satisfiable} satisfiable, {unsatisfiable} unsatisfiable)"
        );
        return Ok(());
    }

    for path in &args.scenarios {
        let document = ScenarioDocument::from_path(path)?;
        let scenario = document.scenario()?;
        let result = check_case(
            &uv,
            &interpreter,
            &document,
            &targets,
            args,
            replay_witness.as_ref(),
        )
        .with_context(|| format!("scenario `{}` failed", path.display()))?;
        println!("{}: {}", scenario.name, result.description);
    }
    Ok(())
}

struct CaseResult {
    satisfiable: usize,
    unsatisfiable: usize,
    export_projections: usize,
    description: String,
}

#[derive(Deserialize)]
struct SavedWitness {
    assignment: Selection,
}

fn read_witness(path: &Path) -> Result<Selection> {
    // Stored certificates are evidence only; the actual scenario is certified again by the checker.
    let witness: SavedWitness = serde_json::from_slice(&fs_err::read(path)?)
        .with_context(|| format!("failed to read scenario witness `{}`", path.display()))?;
    Ok(witness.assignment)
}

fn check_case(
    uv: &Path,
    interpreter: &str,
    document: &ScenarioDocument,
    targets: &[ScenarioTarget],
    args: &Args,
    witness: Option<&Selection>,
) -> Result<CaseResult> {
    let scenario = document.scenario()?;
    if args.lock {
        let options = LockCheckOptions {
            max_states: args.max_states,
            lockfile: if args.lock_without_metadata {
                LockfileMode::WithoutMetadata
            } else {
                LockfileMode::Standard
            },
        };
        let context = TestContext::new_with_versions_and_bin(&[interpreter], uv.to_path_buf());
        let failure_dir = args.failure_dir.clone().or_else(|| {
            args.output_dir
                .as_ref()
                .map(|directory| directory.join(format!("{}.lock.failure", scenario.name)))
        });
        let selections = args
            .project_selections
            .then(|| ScenarioProject::new(&scenario).map(|project| project.selection_matrix()))
            .transpose()?;
        let result = if let Some(assignment) = witness {
            let selections = selections
                .as_deref()
                .context("a whole-domain witness requires project selections")?;
            let graph = WitnessedProjectGraph {
                document: document.clone(),
                assignment: assignment.clone(),
            };
            let max_witness_work = args.max_witness_work.unwrap_or(DEFAULT_MAX_WITNESS_WORK);
            if let Some(failure_dir) = failure_dir.as_deref() {
                check_witnessed_project_lock_scenario_with_artifacts(
                    &context,
                    &graph,
                    targets,
                    selections,
                    options,
                    max_witness_work,
                    failure_dir,
                )
            } else {
                check_witnessed_project_lock_scenario(
                    &context,
                    &graph,
                    targets,
                    selections,
                    options,
                    max_witness_work,
                )
            }
        } else {
            match (selections.as_deref(), failure_dir.as_deref()) {
                (Some(selections), Some(failure_dir)) => {
                    check_project_lock_scenario_with_artifacts(
                        &context,
                        document,
                        targets,
                        selections,
                        options,
                        failure_dir,
                    )
                }
                (Some(selections), None) => {
                    check_project_lock_scenario(&context, &scenario, targets, selections, options)
                }
                (None, Some(failure_dir)) => check_lock_scenario_with_artifacts(
                    &context,
                    document,
                    targets,
                    options,
                    failure_dir,
                ),
                (None, None) => check_lock_scenario(&context, &scenario, targets, options),
            }
        }?;
        let exports = selections
            .as_ref()
            .map(|selections| format!("project exports: {}; ", selections.len()))
            .unwrap_or_default();
        Ok(match result {
            LockCheckResult::Satisfiable {
                projections,
                checked,
            } => CaseResult {
                satisfiable: 1,
                unsatisfiable: 0,
                export_projections: projections,
                description: format!(
                    "valid lock ({exports}projections: {projections}; oracle selections: {checked})"
                ),
            },
            LockCheckResult::Unsatisfiable { witness, checked } => CaseResult {
                satisfiable: 0,
                unsatisfiable: 1,
                export_projections: 0,
                description: format!(
                    "unsatisfiable lock ({witness}; oracle selections: {checked})"
                ),
            },
        })
    } else {
        let mut satisfiable = 0;
        let mut unsatisfiable = 0;
        let mut checked = 0;
        let mut description = String::new();
        for target in targets {
            let failure_dir = args.failure_dir.clone().or_else(|| {
                let output_dir = args.output_dir.as_ref()?;
                let suffix = if targets.len() == 1 {
                    String::new()
                } else {
                    format!("-py{}-{}", target.python, target.platform)
                };
                Some(output_dir.join(format!("{}{suffix}.failure", scenario.name)))
            });
            let context = TestContext::new_with_versions_and_bin(&[interpreter], uv.to_path_buf());
            let result = if let Some(failure_dir) = failure_dir {
                check_scenario_with_artifacts(
                    &context,
                    document,
                    target,
                    args.max_states,
                    &failure_dir,
                )
            } else {
                check_scenario(&context, &scenario, target, args.max_states)
            }
            .with_context(|| format!("projection for {target} failed"))?;
            checked += result.checked;
            if let Some(selection) = result.selection {
                satisfiable += 1;
                description = format!(
                    "satisfiable (selected packages: {}; oracle selections: {})",
                    selection.len(),
                    result.checked
                );
            } else {
                unsatisfiable += 1;
                description = format!("unsatisfiable (oracle selections: {})", result.checked);
            }
        }
        if targets.len() > 1 {
            description = format!(
                "{satisfiable} satisfiable, {unsatisfiable} unsatisfiable projections (oracle selections: {checked})"
            );
        }
        Ok(CaseResult {
            satisfiable,
            unsatisfiable,
            export_projections: 0,
            description,
        })
    }
}

pub(crate) fn save_scenario_input(path: &Path, contents: &str) -> Result<()> {
    match fs_err::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => file.write_all(contents.as_bytes())?,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => ensure!(
            fs_err::read_to_string(path)? == contents,
            "refusing to overwrite different scenario input `{}`",
            path.display()
        ),
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_witnesses_supply_only_the_assignment() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("scenario.witness.json");
        fs_err::write(
            &path,
            serde_json::to_vec(&json!({
                "assignment": {"a": "1.0.0"},
                "universal_certificate": {"assigned_packages": 999},
                "checked_projections": 999,
            }))?,
        )?;
        assert_eq!(
            read_witness(&path)?,
            [("a".parse()?, "1.0.0".parse()?)].into_iter().collect()
        );
        fs_err::write(
            &path,
            r#"{"universal_certificate": {"assigned_packages": 1}}"#,
        )?;
        assert!(read_witness(&path).is_err());
        Ok(())
    }
}
