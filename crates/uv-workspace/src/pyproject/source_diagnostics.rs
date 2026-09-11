use std::ops::Range;
use std::str::FromStr;

use uv_errors::{
    Diagnostic, Info, SourceAnnotation, SourceEdit, SourceFile, SourceSnippet, SourceSuggestion,
    SuggestionApplicability,
};
use uv_normalize::PackageName;
use uv_pep508::MarkerTree;
use uv_toml::SourcePathSegment::{Index, Key};
use uv_toml::{SourceMap, SourcePathSegment};

/// The exact source occurrence considered by marker validation.
#[derive(Debug, Clone, Copy)]
pub(super) struct SourceMarker {
    pub(super) index: usize,
    pub(super) marker: MarkerTree,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum SourcesProvenance {
    Markers {
        count: usize,
        left: SourceMarker,
        right: SourceMarker,
        missing: Option<usize>,
    },
    Empty,
}

/// Source locations retained for a `tool.uv.sources` validation error.
#[derive(Debug)]
pub struct SourcesDiagnostic {
    source: SourceFile,
    primary: Range<usize>,
    related: Option<Range<usize>>,
    replacement: Option<MarkerTree>,
}

impl SourcesDiagnostic {
    pub(super) fn new(
        source: &SourceFile,
        package: &PackageName,
        provenance: SourcesProvenance,
    ) -> Option<Self> {
        let map = SourceMap::parse(source.text()).ok()?;
        let parent = [Key("tool"), Key("uv"), Key("sources")];
        let mut keys = map
            .keys(&parent)?
            .filter(|key| PackageName::from_str(key).is_ok_and(|candidate| &candidate == package));
        let key = keys.next()?;
        if keys.next().is_some() {
            return None;
        }
        let path = [Key("tool"), Key("uv"), Key("sources"), Key(key)];

        match provenance {
            SourcesProvenance::Markers {
                count,
                left,
                right,
                missing,
            } => {
                if map.array_len(&path)? != count || left.marker.is_disjoint(right.marker) {
                    return None;
                }
                let (primary, related) = match missing {
                    Some(index) if index == left.index && left.marker.contents().is_none() => {
                        (left, right)
                    }
                    Some(index) if index == right.index && right.marker.contents().is_none() => {
                        (right, left)
                    }
                    Some(_) => return None,
                    None => (right, left),
                };
                let primary = marker_span(&map, &path, primary)?;
                let related = marker_span(&map, &path, related)?;
                Some(Self {
                    source: source.clone(),
                    primary,
                    related: Some(related),
                    replacement: missing
                        .is_none()
                        .then(|| left.marker.negate().and(right.marker)),
                })
            }
            SourcesProvenance::Empty => {
                if map.array_len(&path)? != 0 {
                    return None;
                }
                Some(Self {
                    source: source.clone(),
                    primary: map.span(&path)?,
                    related: None,
                    replacement: None,
                })
            }
        }
    }

    /// Propose an edit only for the exact marker pair retained during validation.
    pub(super) fn suggestion(&self, replacement: MarkerTree) -> Option<SourceSuggestion> {
        if self.replacement != Some(replacement) || replacement.is_false() {
            return None;
        }
        let replacement = toml_edit::Value::from(replacement.contents()?.to_string()).to_string();
        // The exact edit is display-only. Hint previews are independent of the source snippets
        // attached to the diagnostic.
        SourceSuggestion::new(
            self.source.clone(),
            [SourceEdit::new(self.primary.clone(), replacement)],
            SuggestionApplicability::DisplayOnly,
        )
    }

    pub(super) fn diagnostic(&self) -> Diagnostic<'_> {
        let diagnostic = Diagnostic::default().with_snippet(source_snippet(
            &self.source,
            SourceAnnotation::primary(self.primary.clone()),
        ));
        if let Some(related) = &self.related {
            diagnostic.with_info(Info::new("The other source is declared here").with_snippet(
                source_snippet(&self.source, SourceAnnotation::secondary(related.clone())),
            ))
        } else {
            diagnostic
        }
    }
}

fn marker_span(
    map: &SourceMap<'_>,
    parent: &[SourcePathSegment<'_>],
    occurrence: SourceMarker,
) -> Option<Range<usize>> {
    let mut path = parent.to_vec();
    path.push(Index(occurrence.index));
    let declaration = map.span(&path)?;
    path.push(Key("marker"));
    if let Some(marker) = map.string(&path) {
        if MarkerTree::from_str(marker).ok()? != occurrence.marker {
            return None;
        }
        map.span(&path)
    } else if map.span(&path).is_none() && occurrence.marker.is_true() {
        Some(declaration)
    } else {
        None
    }
}

fn source_snippet(
    source: &SourceFile,
    annotation: SourceAnnotation<'static>,
) -> SourceSnippet<'static> {
    SourceSnippet::new(source.clone()).with_annotation(annotation)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::str::FromStr;

    use insta::assert_snapshot;
    use uv_errors::{
        ErrorOptions, Hinted, Hints, SourceFile, SuggestionApplicability,
        write_error_chain_with_options,
    };
    use uv_normalize::PackageName;
    use uv_pep508::MarkerTree;
    use uv_toml::SourceMap;
    use uv_toml::SourcePathSegment::{Index, Key};

    use crate::pyproject::{PyProjectToml, SourceError, diagnostic_for_error};

    use super::{SourceMarker, SourcesDiagnostic, SourcesProvenance};

    fn format_error(source: &str) -> String {
        let error = PyProjectToml::from_string(source.to_string(), "pyproject.toml")
            .expect_err("invalid source declarations");
        assert!(
            error
                .source()
                .expect("source validation error")
                .is::<SourceError>()
        );
        let mut output = String::new();
        write_error_chain_with_options(
            &error,
            &Hints::none(),
            ErrorOptions::default()
                .with_stream(&mut output)
                .with_width_override(usize::MAX)
                .with_diagnostic(|error| {
                    diagnostic_for_error(error).or_else(|| {
                        error
                            .downcast_ref::<SourceError>()
                            .map(|error| uv_errors::Diagnostic::default().with_hints(error.hints()))
                    })
                }),
        )
        .expect("format source diagnostic");
        anstream::adapter::strip_str(&output).to_string()
    }

    #[test]
    fn overlapping_source_occurrences() {
        let source = "# café\r\n[tool.uv.sources]\r\n\"My.Package\" = [\r\n  { index = 'private', marker = \"sys_platform == 'linux'\", extra = 'other' },\r\n  { url = 'https://example.com/one.whl', marker = \"sys_platform == 'linux'\" },\r\n  { url = 'https://example.com/two.whl', marker = \"python_version >= '3.12'\" },\r\n]\r\n";
        assert_snapshot!(format_error(source), @"
        error: Failed to parse `tool.uv.sources`
          cause: Source markers must be disjoint, but the following markers overlap: `sys_platform == 'linux'` and `python_full_version >= '3.12'`.
           --> pyproject.toml:6:63
          info: The other source is declared here
           --> pyproject.toml:5:63

        hint: replace `python_full_version >= '3.12'` with `python_full_version >= '3.12' and sys_platform != 'linux'`
           --> pyproject.toml:6:63
        ");
    }

    #[test]
    fn missing_source_marker_in_array_tables() {
        let source = "[[tool.uv.sources.demo]]\nindex = 'first'\nmarker = \"sys_platform == 'linux'\"\n[[tool.uv.sources.demo]]\nurl = 'https://example.com/demo.whl'\n";
        assert_snapshot!(format_error(source), @r#"
        error: Failed to parse `tool.uv.sources`
          cause: When multiple sources are provided, each source must include a platform marker (e.g., `marker = "sys_platform == 'linux'"`)
           --> pyproject.toml:4:1
          info: The other source is declared here
           --> pyproject.toml:3:10
        "#);
    }

    #[test]
    fn empty_source_array() {
        assert_snapshot!(format_error("tool.uv.sources.demo = []\n"), @"
        error: Failed to parse `tool.uv.sources`
          cause: Must provide at least one source
           --> pyproject.toml:1:24
        ");
    }

    #[test]
    fn mismatched_source_occurrence_has_no_location() {
        let marker = MarkerTree::from_str("sys_platform == 'linux'").expect("valid marker");
        let source = "[tool.uv.sources]\ndemo = [{ index = 'first', marker = \"sys_platform == 'linux'\" }, { index = 'second', marker = \"sys_platform == 'win32'\" }]\n";
        let diagnostic = SourcesDiagnostic::new(
            &SourceFile::new("pyproject.toml", source),
            &PackageName::from_str("demo").expect("valid package name"),
            SourcesProvenance::Markers {
                count: 2,
                left: SourceMarker { index: 0, marker },
                right: SourceMarker { index: 1, marker },
                missing: None,
            },
        );
        assert!(diagnostic.is_none());
    }

    #[test]
    fn non_adjacent_source_markers_conflict() {
        let source = "[tool.uv.sources]\ndemo = [\n  { index = 'first', marker = \"sys_platform == 'linux'\" },\n  { index = 'middle', marker = \"sys_platform == 'win32'\" },\n  { index = 'last', marker = \"sys_platform == 'linux'\" },\n]\n";
        let error = PyProjectToml::from_string(source.to_string(), "pyproject.toml")
            .expect_err("non-adjacent source markers overlap");
        assert!(matches!(
            error
                .source()
                .and_then(|error| error.downcast_ref::<SourceError>()),
            Some(SourceError::OverlappingMarkers { .. })
        ));
        assert_snapshot!(format_error(source), @"
        error: Failed to parse `tool.uv.sources`
          cause: Source markers must be disjoint, but the following markers overlap: `sys_platform == 'linux'` and `sys_platform == 'linux'`.
           --> pyproject.toml:5:30
          info: The other source is declared here
           --> pyproject.toml:3:31

        hint: make the source markers disjoint, or remove one of the overlapping sources
        ");
    }

    #[test]
    fn non_adjacent_source_marker_is_missing() {
        let source = "[tool.uv.sources]\ndemo = [\n  { index = 'first' },\n  { index = 'middle', marker = \"sys_platform == 'linux'\", extra = 'other' },\n  { index = 'last', marker = \"sys_platform == 'linux'\" },\n]\n";
        let error = PyProjectToml::from_string(source.to_string(), "pyproject.toml")
            .expect_err("a non-adjacent source is missing its marker");
        assert!(matches!(
            error
                .source()
                .and_then(|error| error.downcast_ref::<SourceError>()),
            Some(SourceError::MissingMarkers)
        ));
        assert_snapshot!(format_error(source), @r#"
        error: Failed to parse `tool.uv.sources`
          cause: When multiple sources are provided, each source must include a platform marker (e.g., `marker = "sys_platform == 'linux'"`)
           --> pyproject.toml:3:3
          info: The other source is declared here
           --> pyproject.toml:5:30
        "#);
    }

    #[test]
    fn overlapping_source_markers_in_distinct_scopes() {
        let source = "[tool.uv.sources]\ndemo = [\n  { index = 'first', marker = \"sys_platform == 'linux'\", extra = 'one' },\n  { index = 'middle', marker = \"sys_platform == 'linux'\", group = 'dev' },\n  { index = 'last', marker = \"sys_platform == 'linux'\", extra = 'two' },\n]\n";
        PyProjectToml::from_string(source.to_string(), "pyproject.toml")
            .expect("different source scopes may overlap");
    }

    #[test]
    fn marker_suggestion_targets_the_validated_toml_occurrence() {
        let source = "# café\r\n[tool.uv.sources]\r\n\"My.Package\" = [\r\n  { index = 'other', marker = \"sys_platform == 'linux'\", extra = 'other' },\r\n  { url = 'https://example.com/one.whl', marker = \"sys_platform == 'linux'\" },\r\n  { url = 'https://example.com/two.whl', marker = \"python_version >= \\u00273.12\\u0027\" },\r\n]\r\n";
        let error = PyProjectToml::from_string(source.to_string(), "pyproject.toml")
            .expect_err("source markers overlap");
        let Some(SourceError::OverlappingMarkers {
            replacement,
            suggestion: Some(suggestion),
            ..
        }) = error.source().and_then(|error| error.downcast_ref())
        else {
            panic!("the actual source error should own the exact suggestion");
        };
        assert_eq!(
            suggestion.applicability(),
            SuggestionApplicability::DisplayOnly
        );
        let [edit] = suggestion.edits() else {
            panic!("one marker value should be replaced");
        };
        let path = [
            Key("tool"),
            Key("uv"),
            Key("sources"),
            Key("My.Package"),
            Index(2),
            Key("marker"),
        ];
        let original = SourceMap::parse(source).expect("valid TOML");
        assert_eq!(Some(edit.range()), original.span(&path));

        let mut updated = suggestion.source().text().to_string();
        updated.replace_range(edit.range(), edit.replacement());
        let map = SourceMap::parse(&updated).expect("the edit is valid TOML");
        assert_eq!(
            map.string(&path)
                .and_then(|marker| MarkerTree::from_str(marker).ok()),
            Some(*replacement)
        );
        PyProjectToml::from_string(updated, "pyproject.toml")
            .expect("the selected source markers are now disjoint");
    }

    #[test]
    fn a_fully_covered_marker_does_not_suggest_an_impossible_replacement() {
        let source = "[tool.uv.sources]\ndemo = [{ index = 'one', marker = \"sys_platform == 'linux'\" }, { index = 'two', marker = \"sys_platform == 'linux'\" }]\n";
        let error = PyProjectToml::from_string(source.to_string(), "pyproject.toml")
            .expect_err("the markers are identical");
        let Some(SourceError::OverlappingMarkers {
            replacement,
            suggestion,
            ..
        }) = error.source().and_then(|error| error.downcast_ref())
        else {
            panic!("expected a marker conflict");
        };
        assert!(replacement.is_false());
        assert!(suggestion.is_none());
        assert!(!format_error(source).contains("python_version < '0'"));
    }
}
