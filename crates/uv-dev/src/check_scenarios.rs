//! Compare the real uv resolver with the bounded Packse scenario oracle.

use std::path::PathBuf;

use anstream::println;
use anyhow::{Context, Result, ensure};

use uv_python::PythonVersion;
use uv_test::TestContext;
use uv_test::packse::check::{
    LockCheckResult, ScenarioPlatform, ScenarioTarget, check_lock_scenario, check_scenario,
};
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

    /// Packse scenario TOML files to check.
    #[arg(required = true, value_name = "SCENARIO")]
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

    for path in &args.scenarios {
        let scenario = Scenario::from_path(path)?;
        let context = TestContext::new_with_versions_and_bin(&[&interpreter], uv.clone());
        if args.lock {
            let result = check_lock_scenario(
                &context,
                &scenario,
                std::slice::from_ref(&target),
                args.max_states,
            )
            .with_context(|| format!("scenario `{}` failed", path.display()))?;
            match result {
                LockCheckResult::Satisfiable {
                    projections,
                    checked,
                } => println!(
                    "{}: valid lock ({projections} projections; {checked} oracle selections)",
                    scenario.name
                ),
                LockCheckResult::Unsatisfiable { witness, checked } => println!(
                    "{}: unsatisfiable lock ({witness}; {checked} oracle selections)",
                    scenario.name
                ),
            }
            continue;
        }
        let result = check_scenario(&context, &scenario, &target, args.max_states)
            .with_context(|| format!("scenario `{}` failed", path.display()))?;
        if let Some(selection) = result.selection {
            println!(
                "{}: satisfiable ({} packages; {} oracle selections)",
                scenario.name,
                selection.len(),
                result.checked
            );
        } else {
            println!(
                "{}: unsatisfiable ({} oracle selections)",
                scenario.name, result.checked
            );
        }
    }
    Ok(())
}
