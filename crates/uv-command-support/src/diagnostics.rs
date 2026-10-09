use std::error::Error;

use uv_errors::Diagnostic;

/// Resolve TOML presentation data without changing an error or its source chain.
pub fn diagnostic_for_error<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
    uv_settings::diagnostic_for_error(error)
        .or_else(|| uv_workspace::pyproject::diagnostic_for_error(error))
        .or_else(|| uv_pypi_types::diagnostic_for_error(error))
}
