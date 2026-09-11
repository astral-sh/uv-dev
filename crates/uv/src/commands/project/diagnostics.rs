use std::error::Error;
use std::ops::Range;
use std::str::FromStr;

use uv_errors::{Diagnostic, SourceAnnotation, SourceFile, SourceSnippet};
use uv_fs::Simplified;
use uv_pep440::VersionSpecifiers;
use uv_python::PythonVersionFile;
use uv_toml::SourceMap;
use uv_toml::SourcePathSegment::Key;
use uv_workspace::{RequiresPythonSources, Workspace};

use super::ProjectError;

/// Resolve presentation data retained by project-level semantic errors.
pub(crate) fn diagnostic_for_error<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
    let diagnostic = match error.downcast_ref::<ProjectError>()? {
        ProjectError::RequestedPythonProjectIncompatibility(.., diagnostic)
        | ProjectError::DotPythonVersionProjectIncompatibility { diagnostic, .. }
        | ProjectError::RequiresPythonProjectIncompatibility(.., diagnostic)
        | ProjectError::DisjointRequiresPython(_, diagnostic) => diagnostic.as_deref()?,
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
        | ProjectError::OverlappingMarkers(..)
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

/// The Python request and direct declarations that participated in a project compatibility error.
///
/// The semantic requirements and their presentation are kept separate: retaining these sources
/// does not alter workspace identity, requirement intersections, or resolver cache keys.
#[derive(Debug)]
pub(crate) struct PythonRequirementsDiagnostic {
    sources: Vec<SourceSnippet<'static>>,
}

impl PythonRequirementsDiagnostic {
    pub(crate) fn new(
        workspace: &Workspace,
        requires_python: &RequiresPythonSources,
    ) -> Option<Self> {
        let sources = requires_python
            .iter()
            .filter_map(|((package, group), requires_python)| {
                // A flattened group bound can contain several inherited declarations. Its map
                // entry does not identify an individual source location.
                if group.is_some() {
                    return None;
                }
                let member = workspace.packages().get(package)?;
                let source = SourceFile::new(
                    member
                        .root()
                        .join("pyproject.toml")
                        .portable_display()
                        .to_string(),
                    member.pyproject_toml().raw.as_str(),
                );
                Some(project_requires_python_source(source, requires_python))
            })
            .collect::<Vec<_>>();
        (!sources.is_empty()).then_some(Self { sources })
    }

    pub(crate) fn with_python_request(
        file: &PythonVersionFile,
        diagnostic: Option<Self>,
    ) -> Option<Self> {
        let Some(source) = file.version_source() else {
            return diagnostic;
        };
        let mut diagnostic = diagnostic.unwrap_or_else(|| Self {
            sources: Vec::new(),
        });
        diagnostic.sources.insert(0, source);
        Some(diagnostic)
    }

    fn diagnostic(&self) -> Diagnostic<'_> {
        self.sources
            .iter()
            .cloned()
            .fold(Diagnostic::default(), Diagnostic::with_snippet)
    }
}

fn project_requires_python_source(
    source: SourceFile,
    requires_python: &VersionSpecifiers,
) -> SourceSnippet<'static> {
    let field = SourceMap::parse(source.text()).ok().and_then(|map| {
        let path = [Key("project"), Key("requires-python")];
        let declared = VersionSpecifiers::from_str(map.string(&path)?).ok()?;
        // The semantic constraint chooses the declaration, not its rendered spelling.
        if &declared != requires_python {
            return None;
        }
        Some(ProjectField {
            key: map.key_span(&[Key("project")], "requires-python")?,
            value: map.span(&path)?,
        })
    });
    let show_source = field
        .as_ref()
        .is_some_and(|field| field.is_standalone_assignment(&source));
    let mut snippet = SourceSnippet::new(source);
    if let Some(field) = field {
        snippet = snippet.with_annotation(
            SourceAnnotation::primary(field.value)
                .with_label(format!("requires Python `{requires_python}`")),
        );
    }
    if show_source {
        snippet
    } else {
        snippet.without_source_text()
    }
}

struct ProjectField {
    key: Range<usize>,
    value: Range<usize>,
}

impl ProjectField {
    /// Version specifiers are already part of the semantic error message. Show their retained
    /// spelling only when the physical line cannot contain unrelated configuration or comments.
    fn is_standalone_assignment(&self, source: &SourceFile) -> bool {
        let Some(window) = source.line_range_for_span(self.value.clone()) else {
            return false;
        };
        if source.line_range_for_span(self.key.clone()) != Some(window.clone()) {
            return false;
        }
        let text = source.text();
        text.get(window.start..self.key.start)
            .is_some_and(|prefix| prefix.trim().is_empty())
            && text
                .get(self.key.end..self.value.start)
                .is_some_and(|separator| separator.trim() == "=")
            && text
                .get(self.value.end..window.end)
                .is_some_and(|suffix| suffix.trim().is_empty())
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::str::FromStr;

    use insta::assert_snapshot;

    use uv_errors::{Diagnostic, ErrorOptions, Hints, SourceFile, write_error_chain_with_options};
    use uv_pep440::VersionSpecifiers;

    use super::{PythonRequirementsDiagnostic, project_requires_python_source};

    #[derive(Debug, thiserror::Error)]
    #[error("Python requirements are incompatible")]
    struct TestError(PythonRequirementsDiagnostic);

    fn diagnostic<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
        Some(error.downcast_ref::<TestError>()?.0.diagnostic())
    }

    fn format_source(source: &str, requires_python: &str) -> anyhow::Result<String> {
        let error = TestError(PythonRequirementsDiagnostic {
            sources: vec![project_requires_python_source(
                SourceFile::new("pyproject.toml", source),
                &VersionSpecifiers::from_str(requires_python)?,
            )],
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
    fn direct_requires_python_does_not_expose_adjacent_values() -> anyhow::Result<()> {
        assert_snapshot!(
            format_source(
                "project = { requires-python = '>=3.12', private = 'secret' }\n",
                ">=3.12",
            )?,
            @"
        error: Python requirements are incompatible
           --> pyproject.toml:1:31
        "
        );
        assert_snapshot!(
            format_source(
                "[project]\nrequires-python = '>=3.12' # secret\n",
                ">=3.12",
            )?,
            @"
        error: Python requirements are incompatible
           --> pyproject.toml:2:19
        "
        );
        assert_snapshot!(
            format_source(
                "[project]\nrequires-python = \"\"\"\n>=3.12\"\"\"\n",
                ">=3.12",
            )?,
            @"
        error: Python requirements are incompatible
           --> pyproject.toml:2:19
        "
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
