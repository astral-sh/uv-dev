use std::path::Path;

use anyhow::{Result, bail};
use owo_colors::OwoColorize;

use uv_fs::Simplified;
use uv_python_interpreter::PythonEnvironment;
use uv_python_types::{Prefix, Target};

/// Apply the `--target` or `--prefix` directory to the environment.
/// Call `on_directory` before applying it so diagnostics retain the calling command's context.
pub(crate) fn apply_target_or_prefix(
    environment: PythonEnvironment,
    target: Option<Target>,
    prefix: Option<Prefix>,
    on_directory: impl FnOnce(&'static str, &Path),
) -> std::io::Result<PythonEnvironment> {
    if let Some(target) = target {
        on_directory("--target", target.root());
        environment.with_target(target)
    } else if let Some(prefix) = prefix {
        on_directory("--prefix", prefix.root());
        environment.with_prefix(prefix)
    } else {
        Ok(environment)
    }
}

/// Reject externally managed environments unless modifying them is explicitly allowed.
/// Returns `true` when the externally managed environment check was bypassed.
pub(crate) fn check_externally_managed(
    environment: &PythonEnvironment,
    break_system_packages: bool,
) -> Result<bool> {
    let Some(externally_managed) = environment.interpreter().is_externally_managed() else {
        return Ok(false);
    };
    if break_system_packages {
        return Ok(true);
    }

    if let Some(error) = externally_managed.into_error() {
        bail!(
            "The interpreter at `{}` is externally managed, and indicates the following:\n\n{}\n\nConsider creating a virtual environment with `uv venv`.",
            environment.root().user_display().cyan(),
            textwrap::indent(&error, "  ").green(),
        );
    }
    bail!(
        "The interpreter at `{}` is externally managed. Instead, create a virtual environment with `uv venv`.",
        environment.root().user_display().cyan()
    )
}
