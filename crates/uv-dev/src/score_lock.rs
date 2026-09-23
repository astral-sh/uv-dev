//! Report source-qualified version counts for an existing universal lockfile.

use std::path::PathBuf;

use anstream::println;
use anyhow::{Context, Result};

use uv_test::packse::lock_score::score_lock_versions;

#[derive(clap::Args)]
pub(crate) struct Args {
    /// The existing `uv.lock` to inspect. No resolution or lockfile update is performed.
    #[arg(value_name = "LOCKFILE")]
    lockfile: PathBuf,
}

pub(crate) fn main(args: &Args) -> Result<()> {
    let contents = fs_err::read_to_string(&args.lockfile)
        .with_context(|| format!("failed to read lockfile `{}`", args.lockfile.display()))?;
    let score = score_lock_versions(&contents)?;
    println!("{}", serde_json::to_string_pretty(&score)?);
    Ok(())
}
