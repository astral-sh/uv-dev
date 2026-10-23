use std::fmt::Write;

use anyhow::Result;
use owo_colors::OwoColorize;

use uv_cli::version::uv_self_version;
use uv_command_support::{ExitStatus, Printer, jsonl_result};
use uv_configuration::VersionFormat;

/// Display version information for uv itself (`uv self version`)
pub(crate) fn self_version(
    short: bool,
    output_format: VersionFormat,
    printer: Printer,
) -> Result<ExitStatus> {
    let version_info = uv_self_version();
    match output_format {
        VersionFormat::Text => {
            if short {
                writeln!(printer.stdout(), "{}", version_info.version().cyan())?;
            } else {
                writeln!(printer.stdout(), "uv {}", version_info.cyan())?;
            }
        }
        VersionFormat::Json | VersionFormat::Jsonl => {
            let string = if matches!(output_format, VersionFormat::Jsonl) {
                jsonl_result(&version_info)?
            } else {
                serde_json::to_string_pretty(&version_info)?
            };
            writeln!(printer.stdout_important(), "{string}")?;
        }
    }

    Ok(ExitStatus::Success)
}
