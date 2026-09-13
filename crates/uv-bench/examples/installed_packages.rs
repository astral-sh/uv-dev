//! Create the full-index benchmark's wheel fixture for fresh-process CLI measurements.

use std::io::{self, Write};
use std::num::NonZeroUsize;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};

#[path = "../benches/fixtures/installed_packages.rs"]
mod installed_packages;

#[derive(Parser)]
struct Args {
    /// A new directory in which to create the installed packages.
    directory: PathBuf,

    /// Number of installed wheels to create.
    #[arg(long)]
    packages: NonZeroUsize,

    /// Which optional sidecars to include.
    #[arg(long, value_enum, default_value = "missing")]
    sidecars: SidecarLayout,
}

#[derive(Clone, Copy, ValueEnum)]
enum SidecarLayout {
    Missing,
    Present,
    Mixed,
}

impl From<SidecarLayout> for installed_packages::Sidecars {
    fn from(value: SidecarLayout) -> Self {
        match value {
            SidecarLayout::Missing => Self::Missing,
            SidecarLayout::Present => Self::Present,
            SidecarLayout::Mixed => Self::Mixed,
        }
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let sidecars = installed_packages::Sidecars::from(args.sidecars);
    fs_err::create_dir(&args.directory).with_context(|| {
        format!(
            "Failed to create a new fixture directory at {}",
            args.directory.display()
        )
    })?;
    installed_packages::create(&args.directory, args.packages.get(), sidecars);

    let packages = (0..args.packages.get())
        .map(|index| {
            serde_json::json!({
                "name": format!("metadata-bench-{index:04}"),
                "version": "1.0.0",
            })
        })
        .collect::<Vec<_>>();
    let manifest = serde_json::json!({
        "directory": args.directory,
        "package_count": args.packages.get(),
        "sidecars": sidecars.name(),
        "packages": packages,
    });
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, &manifest)?;
    writeln!(stdout)?;
    Ok(())
}
