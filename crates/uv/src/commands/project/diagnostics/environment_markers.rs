use std::ops::Range;
use std::str::FromStr;

use uv_errors::{
    Diagnostic, Info, SourceAnnotation, SourceEdit, SourceFile, SourceSnippet, SourceSuggestion,
    SuggestionApplicability,
};
use uv_fs::Simplified;
use uv_pep508::MarkerTree;
use uv_pypi_types::SupportedEnvironments;
use uv_toml::SourceMap;
use uv_toml::SourcePathSegment::{Index, Key};
use uv_workspace::Workspace;

/// The workspace-owned environment list that was validated.
#[derive(Debug, Clone, Copy)]
pub(crate) enum EnvironmentMarkersKind {
    Supported,
    Required,
}

impl EnvironmentMarkersKind {
    fn key(self) -> &'static str {
        match self {
            Self::Supported => "environments",
            Self::Required => "required-environments",
        }
    }

    fn declarations(self, workspace: &Workspace) -> Option<&SupportedEnvironments> {
        match self {
            Self::Supported => workspace.environments(),
            Self::Required => workspace.required_environments(),
        }
    }
}

/// Source locations and an optional exact edit for overlapping environment declarations.
#[derive(Debug)]
pub(crate) struct EnvironmentMarkersDiagnostic {
    source: SourceFile,
    primary: Range<usize>,
    related: Range<usize>,
    replacement: MarkerTree,
    suggestion: Option<SourceSuggestion>,
}

impl EnvironmentMarkersDiagnostic {
    pub(crate) fn new(
        workspace: &Workspace,
        kind: EnvironmentMarkersKind,
        environments: &SupportedEnvironments,
        left_index: usize,
        right_index: usize,
    ) -> Option<Self> {
        // Equal marker values do not establish that command-line or merged configuration came
        // from this document. Only locate the actual retained workspace-owned list.
        if !std::ptr::eq(kind.declarations(workspace)?, environments) {
            return None;
        }
        let source = SourceFile::new(
            workspace
                .install_path()
                .join("pyproject.toml")
                .portable_display()
                .to_string(),
            workspace.pyproject_toml().raw.as_str(),
        );
        Self::from_source(
            source,
            kind,
            environments.as_markers(),
            left_index,
            right_index,
        )
    }

    fn from_source(
        source: SourceFile,
        kind: EnvironmentMarkersKind,
        markers: &[MarkerTree],
        left_index: usize,
        right_index: usize,
    ) -> Option<Self> {
        if left_index >= right_index {
            return None;
        }
        let left = *markers.get(left_index)?;
        let right = *markers.get(right_index)?;
        if left.is_disjoint(right) {
            return None;
        }

        let map = SourceMap::parse(source.text()).ok()?;
        let parent = [Key("tool"), Key("uv"), Key(kind.key())];
        if map.array_len(&parent)? != markers.len() {
            return None;
        }
        for (index, expected) in markers.iter().enumerate() {
            let path = [Key("tool"), Key("uv"), Key(kind.key()), Index(index)];
            if MarkerTree::from_str(map.string(&path)?).ok()? != *expected {
                return None;
            }
        }

        let primary = map.span(&[Key("tool"), Key("uv"), Key(kind.key()), Index(right_index)])?;
        let related = map.span(&[Key("tool"), Key("uv"), Key(kind.key()), Index(left_index)])?;
        let replacement = left.negate().and(right);
        let suggestion = replacement
            .contents()
            .filter(|_| !replacement.is_false())
            .and_then(|contents| {
                // Replacing the complete TOML string value handles every original quoting and
                // escaping style. The replacement comes from the validated marker pair.
                let replacement = toml_edit::Value::from(contents.to_string()).to_string();
                SourceSuggestion::new(
                    source.clone(),
                    [SourceEdit::new(primary.clone(), replacement)],
                    SuggestionApplicability::DisplayOnly,
                )
            });

        Some(Self {
            source,
            primary,
            related,
            replacement,
            suggestion,
        })
    }

    /// Share the retained edit only with the matching semantic advice.
    pub(crate) fn suggestion(&self, replacement: MarkerTree) -> Option<SourceSuggestion> {
        if self.replacement != replacement {
            return None;
        }
        self.suggestion.clone()
    }

    pub(super) fn diagnostic(&self) -> Diagnostic<'_> {
        Diagnostic::default()
            .with_snippet(source_location(
                &self.source,
                SourceAnnotation::primary(self.primary.clone()),
            ))
            .with_info(
                Info::new("The other environment is declared here").with_snippet(source_location(
                    &self.source,
                    SourceAnnotation::secondary(self.related.clone()),
                )),
            )
    }
}

fn source_location(
    source: &SourceFile,
    annotation: SourceAnnotation<'static>,
) -> SourceSnippet<'static> {
    // Marker values and neighboring configuration can contain arbitrary private text. The
    // original occurrence is useful without exposing its complete physical line.
    SourceSnippet::new(source.clone())
        .with_annotation(annotation)
        .without_source_text()
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::str::FromStr;

    use anyhow::{Context, Result};
    use insta::assert_snapshot;
    use uv_errors::{
        Diagnostic, ErrorFormat, ErrorOptions, Hinted, Hints, SourceFile, SuggestionApplicability,
        write_error_chain_with_options,
    };
    use uv_pep508::MarkerTree;
    use uv_toml::SourceMap;
    use uv_toml::SourcePathSegment::{Index, Key};
    use uv_workspace::pyproject::PyProjectToml;

    use crate::commands::project::ProjectError;
    use crate::commands::project::diagnostics::diagnostic_for_error;

    use super::{EnvironmentMarkersDiagnostic, EnvironmentMarkersKind};

    fn markers(values: &[&str]) -> Result<Vec<MarkerTree>> {
        values
            .iter()
            .map(|value| MarkerTree::from_str(value).map_err(Into::into))
            .collect()
    }

    fn error(
        source: &str,
        kind: EnvironmentMarkersKind,
        markers: &[MarkerTree],
        left_index: usize,
        right_index: usize,
    ) -> Result<ProjectError> {
        let left = *markers.get(left_index).context("left marker")?;
        let right = *markers.get(right_index).context("right marker")?;
        let diagnostic = EnvironmentMarkersDiagnostic::from_source(
            SourceFile::new("pyproject.toml", source),
            kind,
            markers,
            left_index,
            right_index,
        )
        .context("the retained environment declarations match")?;
        Ok(ProjectError::OverlappingMarkers {
            left: left.try_to_string().unwrap_or_else(|| "true".to_string()),
            right: right.try_to_string().unwrap_or_else(|| "true".to_string()),
            replacement: left.negate().and(right),
            diagnostic: Some(Box::new(diagnostic)),
        })
    }

    fn presentation<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
        let project_error = error.downcast_ref::<ProjectError>()?;
        Some(
            diagnostic_for_error(error)
                .unwrap_or_default()
                .with_hints(project_error.own_hints()),
        )
    }

    fn format_error(error: &ProjectError, format: ErrorFormat) -> Result<String> {
        let mut output = String::new();
        write_error_chain_with_options(
            error,
            &Hints::none(),
            ErrorOptions::default()
                .with_format(format)
                .with_width_override(usize::MAX)
                .with_diagnostic(presentation)
                .with_stream(&mut output),
        )?;
        Ok(anstream::adapter::strip_str(&output).to_string())
    }

    #[test]
    fn environment_suggestion_targets_the_validated_occurrence() -> Result<()> {
        let source = "# café\r\n[tool.uv]\r\nenvironments = [\r\n  \"sys_platform == 'linux'\",\r\n  \"sys_platform == 'win32'\",\r\n  \"sys_platform == \\u0027linux\\u0027 or sys_platform == 'darwin'\", # sentinel-secret\r\n]\r\n[tool.private]\r\ntoken = 'sentinel-secret'\r\n";
        let markers = markers(&[
            "sys_platform == 'linux'",
            "sys_platform == 'win32'",
            "sys_platform == 'linux' or sys_platform == 'darwin'",
        ])?;
        let error = error(source, EnvironmentMarkersKind::Supported, &markers, 0, 2)?;
        assert!(error.source().is_none());
        let ProjectError::OverlappingMarkers {
            replacement,
            diagnostic,
            ..
        } = &error
        else {
            anyhow::bail!("expected an environment marker conflict");
        };
        let suggestion = diagnostic
            .as_deref()
            .and_then(|diagnostic| diagnostic.suggestion(*replacement))
            .context("the non-empty marker remainder has an exact edit")?;
        assert_eq!(
            suggestion.applicability(),
            SuggestionApplicability::DisplayOnly
        );
        let [edit] = suggestion.edits() else {
            anyhow::bail!("expected one complete marker-value edit");
        };
        let path = [Key("tool"), Key("uv"), Key("environments"), Index(2)];
        let original = SourceMap::parse(source)?;
        assert_eq!(Some(edit.range()), original.span(&path));

        let mut updated = suggestion.source().text().to_string();
        updated.replace_range(edit.range(), edit.replacement());
        let map = SourceMap::parse(&updated)?;
        let declared = MarkerTree::from_str(map.string(&path).context("updated marker")?)?;
        assert_eq!(declared, *replacement);
        assert!(markers[0].is_disjoint(declared));
        assert!(markers[1].is_disjoint(declared));
        PyProjectToml::from_string(updated, "pyproject.toml")?;

        assert_snapshot!(format_error(&error, ErrorFormat::Text)?, @"
        error: Supported environments must be disjoint, but the following markers overlap: `sys_platform == 'linux'` and `sys_platform == 'darwin' or sys_platform == 'linux'`
           --> pyproject.toml:6:3
          info: The other environment is declared here
           --> pyproject.toml:4:3

        hint: replace `sys_platform == 'darwin' or sys_platform == 'linux'` with `sys_platform == 'darwin'`
           --> pyproject.toml:6:3
        ");
        let json = format_error(&error, ErrorFormat::Json)?;
        assert!(!json.contains("sentinel-secret"));
        let report: serde_json::Value = serde_json::from_str(&json)?;
        assert_eq!(report["errors"].as_array().map(Vec::len), Some(1));
        assert_eq!(
            report["errors"][0]["hints"][0]["suggestion"]["source"]["kind"],
            "location"
        );
        assert_eq!(
            report["errors"][0]["hints"][0]["suggestion"]["edits"][0]["replacement"],
            edit.replacement()
        );
        Ok(())
    }

    #[test]
    fn required_environment_uses_its_own_field() -> Result<()> {
        let source = "[tool.uv]\nenvironments = [\"sys_platform == 'linux'\", \"sys_platform == 'linux'\"]\nrequired-environments = [\n  \"sys_platform == 'linux'\",\n  \"sys_platform == 'win32'\",\n  \"sys_platform == 'linux'\",\n]\n";
        let markers = markers(&[
            "sys_platform == 'linux'",
            "sys_platform == 'win32'",
            "sys_platform == 'linux'",
        ])?;
        let error = error(source, EnvironmentMarkersKind::Required, &markers, 0, 2)?;
        assert_snapshot!(format_error(&error, ErrorFormat::Text)?, @"
        error: Supported environments must be disjoint, but the following markers overlap: `sys_platform == 'linux'` and `sys_platform == 'linux'`
           --> pyproject.toml:6:3
          info: The other environment is declared here
           --> pyproject.toml:4:3

        hint: make the environment markers disjoint, or remove one of the overlapping environments
        ");
        Ok(())
    }

    #[test]
    fn mismatched_environment_sequence_has_no_location() -> Result<()> {
        let source = SourceFile::new(
            "pyproject.toml",
            "tool.uv.environments = [\"sys_platform == 'linux'\", \"sys_platform == 'win32'\", \"sys_platform == 'linux'\"]\n",
        );
        // The selected pair and count still match, but the retained list is a different input.
        let markers = markers(&[
            "sys_platform == 'linux'",
            "sys_platform == 'darwin'",
            "sys_platform == 'linux'",
        ])?;
        assert!(
            EnvironmentMarkersDiagnostic::from_source(
                source,
                EnvironmentMarkersKind::Supported,
                &markers,
                0,
                2,
            )
            .is_none()
        );
        Ok(())
    }

    #[test]
    fn a_fully_covered_environment_has_no_edit() -> Result<()> {
        let source =
            "tool.uv.environments = [\"sys_platform != 'win32'\", \"sys_platform == 'linux'\"]\n";
        let markers = markers(&["sys_platform != 'win32'", "sys_platform == 'linux'"])?;
        let error = error(source, EnvironmentMarkersKind::Supported, &markers, 0, 1)?;
        let ProjectError::OverlappingMarkers {
            replacement,
            diagnostic,
            ..
        } = &error
        else {
            anyhow::bail!("expected an environment marker conflict");
        };
        assert!(replacement.is_false());
        assert!(
            diagnostic
                .as_deref()
                .and_then(|diagnostic| diagnostic.suggestion(*replacement))
                .is_none()
        );
        assert_snapshot!(format_error(&error, ErrorFormat::Text)?, @"
        error: Supported environments must be disjoint, but the following markers overlap: `sys_platform != 'win32'` and `sys_platform == 'linux'`
           --> pyproject.toml:1:52
          info: The other environment is declared here
           --> pyproject.toml:1:25

        hint: make the environment markers disjoint, or remove one of the overlapping environments
        ");
        Ok(())
    }
}
