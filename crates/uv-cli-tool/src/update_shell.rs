use anyhow::Result;

use uv_tool::tool_executable_dir;

use uv_cli_common::printer::Printer;
use uv_cli_common::{exit_status::ExitStatus, update_shell};

/// Ensure that the tool executable directory is in PATH.
pub async fn update_shell(printer: Printer) -> Result<ExitStatus> {
    update_shell::update_shell(&tool_executable_dir()?, printer).await
}
