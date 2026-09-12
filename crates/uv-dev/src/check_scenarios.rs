//! Compare the real uv resolver with the bounded Packse scenario oracle.

use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use anstream::println;
use anyhow::{Context, Result, ensure};

use uv_python::PythonVersion;
use uv_test::TestContext;
use uv_test::packse::check::{
    LockCheckResult, ScenarioPlatform, ScenarioTarget, check_lock_scenario, check_scenario,
};
use uv_test::packse::generate::{SmallGraphOptions, generate_small_graph};
use uv_test::packse::scenario::Scenario;

#[derive(clap::Args)]
pub(crate) struct Args {
    /// The uv executable to check. Build it with `cargo build --package uv` first.
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

    /// Check a universal project lock and its frozen export instead of `pip compile`.
    #[arg(long)]
    lock: bool,

    /// Generate small graphs beginning at this seed instead of reading scenario files.
    #[arg(long, conflicts_with = "scenarios", requires = "output_dir")]
    seed: Option<u64>,

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
    let uv = fs_err::canonicalize(&args.uv)
        .with_context(|| format!("failed to find uv executable `{}`", args.uv.display()))?;
    let target = ScenarioTarget {
        python: args.python_version.clone(),
        platform: args.python_platform,
    };
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
        fs_err::create_dir_all(output_dir)?;
        let mut satisfiable = 0;
        let mut unsatisfiable = 0;
        for offset in 0..cases {
            let seed = first_seed
                .checked_add(u64::try_from(offset)?)
                .context("the requested seed range overflows u64")?;
            let document = generate_small_graph(seed, options, &target, args.max_states)?;
            let scenario = document.scenario()?;
            let path = output_dir.join(format!("{}.toml", scenario.name));
            save_generated_input(&path, &document.to_toml()?)?;
            let context = TestContext::new_with_versions_and_bin(&[&interpreter], uv.clone());
            let result = check_case(&context, &scenario, &target, args)
                .with_context(|| format!("generated scenario `{}` failed", path.display()))?;
            if result.satisfiable {
                satisfiable += 1;
            } else {
                unsatisfiable += 1;
            }
        }
        println!(
            "Checked {cases} generated graphs: {satisfiable} satisfiable, {unsatisfiable} unsatisfiable"
        );
        return Ok(());
    }

    for path in &args.scenarios {
        let scenario = Scenario::from_path(path)?;
        let context = TestContext::new_with_versions_and_bin(&[&interpreter], uv.clone());
        let result = check_case(&context, &scenario, &target, args)
            .with_context(|| format!("scenario `{}` failed", path.display()))?;
        println!("{}: {}", scenario.name, result.description);
    }
    Ok(())
}

struct CaseResult {
    satisfiable: bool,
    description: String,
}

fn check_case(
    context: &TestContext,
    scenario: &Scenario,
    target: &ScenarioTarget,
    args: &Args,
) -> Result<CaseResult> {
    if args.lock {
        let result = check_lock_scenario(
            context,
            scenario,
            std::slice::from_ref(target),
            args.max_states,
        )?;
        Ok(match result {
            LockCheckResult::Satisfiable {
                projections,
                checked,
            } => CaseResult {
                satisfiable: true,
                description: format!(
                    "valid lock (projections: {projections}; oracle selections: {checked})"
                ),
            },
            LockCheckResult::Unsatisfiable { witness, checked } => CaseResult {
                satisfiable: false,
                description: format!(
                    "unsatisfiable lock ({witness}; oracle selections: {checked})"
                ),
            },
        })
    } else {
        let result = check_scenario(context, scenario, target, args.max_states)?;
        if let Some(selection) = result.selection {
            Ok(CaseResult {
                satisfiable: true,
                description: format!(
                    "satisfiable (selected packages: {}; oracle selections: {})",
                    selection.len(),
                    result.checked
                ),
            })
        } else {
            Ok(CaseResult {
                satisfiable: false,
                description: format!("unsatisfiable (oracle selections: {})", result.checked),
            })
        }
    }
}

fn save_generated_input(path: &Path, contents: &str) -> Result<()> {
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
