mod environment_markers;

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::str::FromStr;

use uv_errors::{Diagnostic, Info, SourceAnnotation, SourceFile, SourceSnippet};
use uv_normalize::PackageName;
use uv_pep440::VersionSpecifiers;
use uv_python::PythonVersionFile;
use uv_toml::SourceMap;
use uv_toml::SourcePathSegment::Key;
use uv_workspace::dependency_groups::{GroupInclude, PythonRequirementsSource};
use uv_workspace::{RequiresPythonDeclarations, Workspace};

use super::ProjectError;

pub(crate) use environment_markers::{EnvironmentMarkersDiagnostic, EnvironmentMarkersKind};

/// Resolve presentation data retained by project-level semantic errors.
pub(crate) fn diagnostic_for_error<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
    let diagnostic = match error.downcast_ref::<ProjectError>()? {
        ProjectError::RequestedPythonProjectIncompatibility(.., diagnostic)
        | ProjectError::DotPythonVersionProjectIncompatibility { diagnostic, .. }
        | ProjectError::RequiresPythonProjectIncompatibility(.., diagnostic)
        | ProjectError::DisjointRequiresPython(_, diagnostic) => diagnostic.as_deref()?,
        ProjectError::OverlappingMarkers { diagnostic, .. } => {
            return diagnostic
                .as_deref()
                .map(EnvironmentMarkersDiagnostic::diagnostic);
        }
        ProjectError::LockMismatch(..)
        | ProjectError::LockFormat(..)
        | ProjectError::MissingLockfile(..)
        | ProjectError::LockWorkspaceMismatch(..)
        | ProjectError::UnsupportedLockVersion(..)
        | ProjectError::UnparsableLockVersion(..)
        | ProjectError::LockSerialization(_)
        | ProjectError::LockedPythonIncompatibility(..)
        | ProjectError::LockedPlatformIncompatibility(_)
        | ProjectError::Conflict(_)
        | ProjectError::RequestedPythonScriptIncompatibility(..)
        | ProjectError::DotPythonVersionScriptIncompatibility(..)
        | ProjectError::RequiresPythonScriptIncompatibility(..)
        | ProjectError::MissingGroupProject(_)
        | ProjectError::MissingGroupProjects(_)
        | ProjectError::MissingGroupScript(_)
        | ProjectError::MissingDefaultGroup(_)
        | ProjectError::MissingExtraProject(..)
        | ProjectError::MissingExtraProjects(_)
        | ProjectError::MissingExtraScript(_)
        | ProjectError::DisjointEnvironment(..)
        | ProjectError::EmptyEnvironment
        | ProjectError::InvalidProjectEnvironmentDir(..)
        | ProjectError::UvLockParse(_)
        | ProjectError::PyprojectTomlParse(_)
        | ProjectError::PyprojectTomlUpdate
        | ProjectError::Pep723ScriptTomlParse(_)
        | ProjectError::MalwareFound
        | ProjectError::Osv(_)
        | ProjectError::NoSitePackages
        | ProjectError::InvalidParentEnvironmentPath
        | ProjectError::DroppedEnvironment
        | ProjectError::DependencyGroup(_)
        | ProjectError::Client(_)
        | ProjectError::ClientBuild(_)
        | ProjectError::Credentials(_)
        | ProjectError::IndexCredentials(_)
        | ProjectError::IndexUrl(_)
        | ProjectError::Python(_)
        | ProjectError::Virtualenv(_)
        | ProjectError::HashStrategy(_)
        | ProjectError::Tags(_)
        | ProjectError::FlatIndex(_)
        | ProjectError::Lock(_)
        | ProjectError::Operation(_)
        | ProjectError::Interpreter(_)
        | ProjectError::Tool(_)
        | ProjectError::Name(_)
        | ProjectError::Requirements(_)
        | ProjectError::Metadata(_)
        | ProjectError::Lowering(_)
        | ProjectError::Workspace(_)
        | ProjectError::PyprojectMut(_)
        | ProjectError::ExtraBuildRequires(_)
        | ProjectError::Fmt(_)
        | ProjectError::CacheInfo(_)
        | ProjectError::Io(_)
        | ProjectError::RetryParsing(_)
        | ProjectError::Accelerator(_)
        | ProjectError::Anyhow(_) => return None,
    };
    Some(diagnostic.diagnostic())
}

/// The Python request and declarations that participated in a project compatibility error.
///
/// The semantic requirements and their presentation are kept separate: retaining these sources
/// does not alter workspace identity, requirement intersections, or resolver cache keys.
#[derive(Debug, Default)]
pub(crate) struct PythonRequirementsDiagnostic {
    sources: Vec<SourceSnippet<'static>>,
    includes: Vec<PythonRequirementInclude>,
    workspace_non_trivial: bool,
}

#[derive(Debug)]
struct PythonRequirementInclude {
    package: PackageName,
    include: GroupInclude,
    source: SourceSnippet<'static>,
}

impl PythonRequirementsDiagnostic {
    pub(crate) fn new(
        workspace: &Workspace,
        requires_python: &RequiresPythonDeclarations,
    ) -> Option<Self> {
        let mut sources = Vec::new();
        let mut source_files = BTreeMap::new();
        let mut includes = Vec::new();
        let mut seen_includes = BTreeSet::new();
        for ((package, group), declaration) in requires_python {
            let Some(member) = workspace.packages().get(package) else {
                continue;
            };
            let source = source_files.entry(package.clone()).or_insert_with(|| {
                PythonRequirementsSource::new(member.root(), member.pyproject_toml())
            });
            sources.push(if let Some(group) = group {
                source.requires_python_source(group, &declaration.specifiers)
            } else {
                project_requires_python_source(
                    source.source_file(),
                    source.source_map(),
                    &declaration.specifiers,
                )
            });
            for include in &declaration.includes {
                if !seen_includes.insert((package.clone(), include.clone())) {
                    continue;
                }
                if let Some(source) = source.include_source(include) {
                    includes.push(PythonRequirementInclude {
                        package: package.clone(),
                        include: include.clone(),
                        source,
                    });
                }
            }
        }
        (!sources.is_empty()).then_some(Self {
            sources,
            includes,
            workspace_non_trivial: workspace.packages().len() > 1,
        })
    }

    pub(crate) fn with_python_request(
        file: &PythonVersionFile,
        diagnostic: Option<Self>,
    ) -> Option<Self> {
        let Some(source) = file.version_source() else {
            return diagnostic;
        };
        let mut diagnostic = diagnostic.unwrap_or_default();
        diagnostic.sources.insert(0, source);
        Some(diagnostic)
    }

    fn diagnostic(&self) -> Diagnostic<'_> {
        let mut diagnostic = self
            .sources
            .iter()
            .cloned()
            .fold(Diagnostic::default(), Diagnostic::with_snippet);
        for related in &self.includes {
            let message = if self.workspace_non_trivial {
                format!(
                    "Group `{}:{}` is included by `{}:{}` here",
                    related.package,
                    related.include.included,
                    related.package,
                    related.include.group,
                )
            } else {
                format!(
                    "Group `{}` is included by `{}` here",
                    related.include.included, related.include.group,
                )
            };
            diagnostic =
                diagnostic.with_info(Info::new(message).with_snippet(related.source.clone()));
        }
        diagnostic
    }
}

fn project_requires_python_source(
    source: &SourceFile,
    map: Option<&SourceMap<'_>>,
    requires_python: &VersionSpecifiers,
) -> SourceSnippet<'static> {
    let span = map.and_then(|map| {
        let path = [Key("project"), Key("requires-python")];
        let declared = VersionSpecifiers::from_str(map.string(&path)?).ok()?;
        // The semantic constraint chooses the declaration, not its rendered spelling.
        if &declared != requires_python {
            return None;
        }
        map.span(&path)
    });
    let mut snippet = SourceSnippet::new(source.clone());
    if let Some(span) = span {
        snippet = snippet.with_annotation(
            SourceAnnotation::primary(span)
                .with_label(format!("requires Python `{requires_python}`")),
        );
    }
    snippet
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::str::FromStr;

    use insta::assert_snapshot;

    use uv_errors::{Diagnostic, ErrorOptions, Hints, SourceFile, write_error_chain_with_options};
    use uv_pep440::VersionSpecifiers;
    use uv_toml::SourceMap;

    use super::{PythonRequirementsDiagnostic, project_requires_python_source};

    #[derive(Debug, thiserror::Error)]
    #[error("Python requirements are incompatible")]
    struct TestError(PythonRequirementsDiagnostic);

    fn diagnostic<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
        Some(error.downcast_ref::<TestError>()?.0.diagnostic())
    }

    fn format_source(source: &str, requires_python: &str) -> anyhow::Result<String> {
        let map = SourceMap::parse(source).ok();
        let source = SourceFile::new("pyproject.toml", source);
        let error = TestError(PythonRequirementsDiagnostic {
            sources: vec![project_requires_python_source(
                &source,
                map.as_ref(),
                &VersionSpecifiers::from_str(requires_python)?,
            )],
            ..PythonRequirementsDiagnostic::default()
        });
        let mut output = String::new();
        write_error_chain_with_options(
            &error,
            &Hints::none(),
            ErrorOptions::default()
                .with_width_override(80)
                .with_diagnostic(diagnostic)
                .with_stream(&mut output),
        )?;
        Ok(anstream::adapter::strip_str(&output).to_string())
    }

    #[test]
    fn direct_requires_python_retains_decoded_toml_coordinates() -> anyhow::Result<()> {
        assert_snapshot!(
            format_source(
                "# café\r\n[project]\r\n\"requires\\u002dpython\" = \">=3.\\u0031\\u0032\"\r\n",
                ">=3.12",
            )?,
            @r#"
        error: Python requirements are incompatible
           --> pyproject.toml:3:26
            |
          3 | "requires\u002dpython" = ">=3.\u0031\u0032"
            |                          ^^^^^^^^^^^^^^^^^^ requires Python `>=3.12`
        "#
        );
        Ok(())
    }

    #[test]
    fn direct_requires_python_keeps_source_context() -> anyhow::Result<()> {
        assert_snapshot!(
            format_source(
                "project = { requires-python = '>=3.12', version = '0.1.0' }\n",
                ">=3.12",
            )?,
            @"
        error: Python requirements are incompatible
           --> pyproject.toml:1:31
            |
          1 | project = { requires-python = '>=3.12', version = '0.1.0' }
            |                               ^^^^^^^^ requires Python `>=3.12`
        "
        );
        assert_snapshot!(
            format_source(
                "[project]\nrequires-python = '>=3.12' # required by the application\n",
                ">=3.12",
            )?,
            @"
        error: Python requirements are incompatible
           --> pyproject.toml:2:19
            |
          2 | requires-python = '>=3.12' # required by the application
            |                   ^^^^^^^^ requires Python `>=3.12`
        "
        );
        assert_snapshot!(
            format_source(
                "[project]\nrequires-python = \"\"\"\n>=3.12\"\"\"\n",
                ">=3.12",
            )?,
            @r#"
        error: Python requirements are incompatible
           --> pyproject.toml:2:19
            |
          2 |   requires-python = """
            |  ___________________^
          3 | | >=3.12"""
            | |_________^ requires Python `>=3.12`
        "#
        );
        Ok(())
    }

    #[test]
    fn direct_requires_python_requires_a_matching_semantic_declaration() -> anyhow::Result<()> {
        assert_snapshot!(
            format_source("[project]\nrequires-python = '>=3.13'\n", ">=3.12")?,
            @"
        error: Python requirements are incompatible
           --> pyproject.toml
        "
        );
        Ok(())
    }
}
