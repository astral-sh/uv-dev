use std::fmt::Write;
use std::time::Instant;

use anyhow::Result;
use owo_colors::OwoColorize;

use uv_cache::Cache;
use uv_cli::PipCheckFormat;
use uv_configuration::TargetTriple;
use uv_distribution_types::DependencyMetadata;
use uv_installer::SitePackages;
use uv_preview::{Preview, PreviewFeature};
use uv_python::{
    EnvironmentPreference, PythonEnvironment, PythonPreference, PythonRequest, PythonVersion,
};
use uv_warnings::warn_user;

use crate::commands::pip::check_report::Report;
use crate::commands::pip::operations::report_target_environment;
use crate::commands::pip::{resolution_markers, resolution_tags};
use crate::commands::{ExitStatus, elapsed};
use crate::printer::Printer;

/// Check for incompatibilities in installed packages.
pub(crate) fn pip_check(
    output_format: PipCheckFormat,
    python: Option<&str>,
    system: bool,
    python_version: Option<&PythonVersion>,
    python_platform: Option<&TargetTriple>,
    dependency_metadata: &DependencyMetadata,
    cache: &Cache,
    printer: Printer,
    preview: Preview,
) -> Result<ExitStatus> {
    let start = Instant::now();

    if output_format == PipCheckFormat::Json && !preview.is_enabled(PreviewFeature::JsonOutput) {
        warn_user!(
            "The `--output-format json` option is experimental and the schema may change without warning. Pass `--preview-features {}` to disable this warning.",
            PreviewFeature::JsonOutput
        );
    }

    // Detect the current Python interpreter.
    let environment = PythonEnvironment::find(
        &python.map(PythonRequest::parse).unwrap_or_default(),
        EnvironmentPreference::from_system_flag(system, false),
        PythonPreference::default().with_system_flag(system),
        cache,
    )?;

    report_target_environment(&environment, cache, printer)?;

    // Build the installed index.
    let site_packages = SitePackages::from_environment(&environment)?;
    let packages_checked = site_packages.iter().count();

    if output_format == PipCheckFormat::Text {
        let s = if packages_checked == 1 { "" } else { "s" };
        writeln!(
            printer.stderr(),
            "{}",
            format!(
                "Checked {} {}",
                format!("{packages_checked} package{s}").bold(),
                format!("in {}", elapsed(start.elapsed())).dimmed()
            )
            .dimmed()
        )?;
    }

    // Determine the markers and tags to use for resolution.
    let markers = resolution_markers(python_version, python_platform, environment.interpreter());
    let tags = resolution_tags(python_version, python_platform, environment.interpreter())?;

    // Run the diagnostics.
    let diagnostics = site_packages.diagnostics(&markers, &tags, dependency_metadata)?;
    let report = Report::new(
        &environment,
        &markers,
        python_platform,
        packages_checked,
        diagnostics,
    );
    report.render(output_format, printer)?;
    Ok(report.exit_status())
}
