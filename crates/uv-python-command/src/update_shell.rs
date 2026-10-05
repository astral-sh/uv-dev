use anyhow::Result;

use uv_python_managed::python_executable_dir;

use uv_cli_output::printer::Printer;
use uv_cli_types::exit::ExitStatus;

/// Ensure that the Python executable directory is in PATH.
pub async fn update_shell(printer: Printer) -> Result<ExitStatus> {
    uv_cli_output::shell::update_shell(&python_executable_dir()?, printer).await
}
