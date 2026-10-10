use owo_colors::OwoColorize;
use std::fmt::Write;

use uv_auth::TextCredentialStore;
use uv_fs::Simplified;

use uv_cli_output::printer::Printer;

/// Show the credentials directory.
pub fn dir(printer: Printer) -> anyhow::Result<()> {
    let root = TextCredentialStore::directory_path()?;
    writeln!(printer.stdout(), "{}", root.simplified_display().cyan())?;
    Ok(())
}
