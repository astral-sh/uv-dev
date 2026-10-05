use owo_colors::OwoColorize;
use std::fmt::Write;

use uv_cache::Cache;
use uv_fs::Simplified;

use uv_cli_output::printer::Printer;
use uv_cli_types::exit::ExitStatus;

/// Show the cache directory.
pub fn cache_dir(cache: &Cache, printer: Printer) -> anyhow::Result<ExitStatus> {
    writeln!(
        printer.stdout(),
        "{}",
        cache.root().simplified_display().cyan()
    )?;
    Ok(ExitStatus::Success)
}
