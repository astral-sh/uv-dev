use anyhow::Result;

use uv_tool::tool_executable_dir;

use uv_cli_output::printer::Printer;
use uv_cli_types::exit::ExitStatus;

/// Ensure that the tool executable directory is in PATH.
pub async fn update_shell(printer: Printer) -> Result<ExitStatus> {
    uv_cli_output::shell::update_shell(&tool_executable_dir()?, printer).await
}
