use std::collections::HashSet;
use std::fmt::Write;

use owo_colors::OwoColorize;

use uv_cli_output::printer::Printer;
use uv_distribution_types::{DependencyMetadata, Diagnostic, ResolutionDiagnostic};
use uv_installer::SitePackages;
use uv_normalize::PackageName;
use uv_platform_tags::Tags;
use uv_pypi_types::ResolverMarkerEnvironment;
use uv_python_interpreter::PythonEnvironment;

use crate::error::Error;

/// Report any diagnostics on resolved distributions.
pub fn diagnose_resolution(
    diagnostics: &[ResolutionDiagnostic],
    printer: Printer,
) -> Result<(), Error> {
    for diagnostic in diagnostics {
        writeln!(
            printer.stderr(),
            "{}{} {}",
            "warning".yellow().bold(),
            ":".bold(),
            diagnostic.message().bold()
        )?;
    }
    Ok(())
}

/// Report any diagnostics on installed distributions in the Python environment.
pub fn diagnose_environment<'a>(
    relevant_packages: impl Iterator<Item = &'a PackageName>,
    venv: &PythonEnvironment,
    markers: &ResolverMarkerEnvironment,
    tags: &Tags,
    dependency_metadata: &DependencyMetadata,
    printer: Printer,
) -> Result<(), Error> {
    let site_packages = SitePackages::from_environment(venv)?;
    let relevant_packages = relevant_packages.collect::<HashSet<_>>();
    for diagnostic in site_packages.diagnostics(markers, tags, dependency_metadata)? {
        // Only surface diagnostics that are "relevant" to the current resolution.
        if relevant_packages
            .iter()
            .any(|name| diagnostic.includes(name))
        {
            writeln!(
                printer.stderr(),
                "{}{} {}",
                "warning".yellow().bold(),
                ":".bold(),
                diagnostic.message().bold()
            )?;
        }
    }
    Ok(())
}
