use std::fmt::Write;

use anyhow::Result;
use owo_colors::OwoColorize;
use serde::{Serialize, Serializer};

use uv_cli::PipCheckFormat;
use uv_configuration::TargetTriple;
use uv_distribution_types::Diagnostic;
use uv_fs::PortablePathBuf;
use uv_installer::SitePackagesDiagnostic;
use uv_normalize::PackageName;
use uv_pypi_types::ResolverMarkerEnvironment;
use uv_python::PythonEnvironment;

use crate::commands::ExitStatus;
use crate::commands::report::{EnvironmentReport, SchemaReport};
use crate::printer::{Printer, jsonl_result};

/// The target used to check the installed distributions.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
struct CheckTargetReport {
    /// Normalized, release-only full Python version used for compatibility and marker checks.
    python_version: String,
    /// Explicit platform override, or null when using the installed interpreter's platform.
    #[cfg_attr(feature = "schemars", schemars(with = "Option<String>"))]
    python_platform: Option<TargetTriple>,
}

/// The preview `uv pip check` JSON report.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schemars", schemars(title = "uv pip check (preview)"))]
pub(super) struct Report {
    /// Format information.
    schema: SchemaReport,
    /// The installed environment being inspected.
    environment: EnvironmentReport,
    /// The target against which the environment was checked.
    target: CheckTargetReport,
    /// Number of installed distributions examined, including duplicate distributions.
    packages_checked: usize,
    /// Compatibility diagnostics, sorted by package, kind, and normalized details.
    #[serde(serialize_with = "serialize_diagnostics")]
    #[cfg_attr(feature = "schemars", schemars(with = "Vec<DiagnosticReport>"))]
    diagnostics: Vec<SitePackagesDiagnostic>,
}

/// Generate the preview `uv pip check` output schema for repository development tools.
#[cfg(feature = "schemars")]
pub fn json_schema() -> schemars::Schema {
    schemars::generate::SchemaSettings::draft07()
        .for_serialize()
        .into_generator()
        .into_root_schema_for::<Report>()
}

/// Generate the preview `uv pip check` JSONL record schema for repository development tools.
#[cfg(feature = "schemars")]
pub fn jsonl_schema() -> schemars::Schema {
    crate::commands::report::jsonl_object_schema::<Report>("uv pip check JSONL (preview)")
}

impl Report {
    pub(super) fn new(
        environment: &PythonEnvironment,
        markers: &ResolverMarkerEnvironment,
        python_platform: Option<&TargetTriple>,
        packages_checked: usize,
        diagnostics: Vec<SitePackagesDiagnostic>,
    ) -> Self {
        Self {
            schema: SchemaReport::default(),
            environment: EnvironmentReport::from(environment),
            target: CheckTargetReport {
                python_version: markers.python_full_version().version.to_string(),
                python_platform: python_platform.copied(),
            },
            packages_checked,
            diagnostics,
        }
    }

    pub(super) fn exit_status(&self) -> ExitStatus {
        if self.diagnostics.is_empty() {
            ExitStatus::Success
        } else {
            ExitStatus::Failure
        }
    }

    pub(super) fn render(&self, format: PipCheckFormat, printer: Printer) -> Result<()> {
        match format {
            PipCheckFormat::Text => self.render_text(printer),
            PipCheckFormat::Json => {
                writeln!(
                    printer.stdout_important_raw(),
                    "{}",
                    serde_json::to_string_pretty(self)?
                )?;
                Ok(())
            }
            PipCheckFormat::Jsonl => {
                writeln!(printer.stdout_important_raw(), "{}", jsonl_result(self)?)?;
                Ok(())
            }
        }
    }

    fn render_text(&self, printer: Printer) -> Result<()> {
        if self.diagnostics.is_empty() {
            writeln!(
                printer.stderr(),
                "{}",
                "All installed packages are compatible".to_string().dimmed()
            )?;
        } else {
            let incompats = if self.diagnostics.len() == 1 {
                "incompatibility"
            } else {
                "incompatibilities"
            };
            writeln!(
                printer.stderr(),
                "{}",
                format!(
                    "Found {}",
                    format!("{} {}", self.diagnostics.len(), incompats).bold()
                )
                .dimmed()
            )?;

            for diagnostic in &self.diagnostics {
                writeln!(printer.stderr(), "{}", diagnostic.message().bold())?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
struct DiagnosticReport {
    /// Normalized name of the package with the incompatibility.
    package: PackageName,
    #[serde(flatten)]
    detail: DiagnosticKindReport,
}

#[derive(Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
enum DiagnosticKindReport {
    DuplicatePackage {
        /// Portable paths to the installed metadata locations, sorted lexicographically.
        paths: Vec<String>,
    },
    IncompatibleDependency {
        /// Normalized PEP 508 requirement with credential-safe URLs.
        requirement: String,
        /// Version of the dependency installed in the inspected environment.
        installed_version: String,
    },
    IncompatiblePlatform,
    IncompatiblePythonVersion {
        /// Python versions required by the package.
        requires_python: String,
        /// Python version installed in the inspected environment.
        installed_version: String,
    },
    MetadataUnavailable {
        /// Portable path to the installed distribution's metadata location.
        path: String,
    },
    MissingDependency {
        /// Normalized PEP 508 requirement with credential-safe URLs.
        requirement: String,
    },
    TagsUnavailable {
        /// Portable path to the installed distribution's metadata location.
        path: String,
    },
}

impl From<&SitePackagesDiagnostic> for DiagnosticReport {
    fn from(diagnostic: &SitePackagesDiagnostic) -> Self {
        let (package, detail) = match diagnostic {
            SitePackagesDiagnostic::MetadataUnavailable { package, path } => (
                package,
                DiagnosticKindReport::MetadataUnavailable {
                    path: PortablePathBuf::from(path.as_path()).to_string(),
                },
            ),
            SitePackagesDiagnostic::TagsUnavailable { package, path } => (
                package,
                DiagnosticKindReport::TagsUnavailable {
                    path: PortablePathBuf::from(path.as_path()).to_string(),
                },
            ),
            SitePackagesDiagnostic::IncompatiblePythonVersion {
                package,
                version,
                requires_python,
            } => (
                package,
                DiagnosticKindReport::IncompatiblePythonVersion {
                    installed_version: version.to_string(),
                    requires_python: requires_python.to_string(),
                },
            ),
            SitePackagesDiagnostic::IncompatiblePlatform { package } => {
                (package, DiagnosticKindReport::IncompatiblePlatform)
            }
            SitePackagesDiagnostic::MissingDependency {
                package,
                requirement,
            } => (
                package,
                DiagnosticKindReport::MissingDependency {
                    requirement: requirement.to_string(),
                },
            ),
            SitePackagesDiagnostic::IncompatibleDependency {
                package,
                version,
                requirement,
            } => (
                package,
                DiagnosticKindReport::IncompatibleDependency {
                    installed_version: version.to_string(),
                    requirement: requirement.to_string(),
                },
            ),
            SitePackagesDiagnostic::DuplicatePackage { package, paths } => {
                let mut paths = paths
                    .iter()
                    .map(|path| PortablePathBuf::from(path.as_path()).to_string())
                    .collect::<Vec<_>>();
                paths.sort_unstable();
                (package, DiagnosticKindReport::DuplicatePackage { paths })
            }
        };
        Self {
            package: package.clone(),
            detail,
        }
    }
}

fn serialize_diagnostics<S>(
    diagnostics: &[SitePackagesDiagnostic],
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    // Sort the serialized values, not interpreter discovery or marker construction order.
    let mut diagnostics = diagnostics
        .iter()
        .map(DiagnosticReport::from)
        .collect::<Vec<_>>();
    diagnostics.sort_unstable();
    diagnostics.serialize(serializer)
}
