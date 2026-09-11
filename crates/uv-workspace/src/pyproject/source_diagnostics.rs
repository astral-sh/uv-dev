use std::ops::Range;
use std::str::FromStr;

use uv_errors::{Diagnostic, Info, SourceAnnotation, SourceFile, SourceSnippet};
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
    primary: SourceSnippet<'static>,
    related: Option<SourceSnippet<'static>>,
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
                    primary: source_location(source, SourceAnnotation::primary(primary)),
                    related: Some(source_location(
                        source,
                        SourceAnnotation::secondary(related),
                    )),
                })
            }
            SourcesProvenance::Empty => {
                if map.array_len(&path)? != 0 {
                    return None;
                }
                Some(Self {
                    primary: source_location(source, SourceAnnotation::primary(map.span(&path)?)),
                    related: None,
                })
            }
        }
    }

    pub(super) fn diagnostic(&self) -> Diagnostic<'_> {
        let diagnostic = Diagnostic::default().with_snippet(self.primary.clone());
        if let Some(related) = &self.related {
            diagnostic.with_info(
                Info::new("The other source is declared here").with_snippet(related.clone()),
            )
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

fn source_location(
    source: &SourceFile,
    annotation: SourceAnnotation<'static>,
) -> SourceSnippet<'static> {
    // Source declarations can contain authenticated URLs, and marker string values are arbitrary
    // user input. A location identifies the exact occurrence without exposing either value.
    SourceSnippet::new(source.clone())
        .with_annotation(annotation)
        .without_source_text()
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::str::FromStr;

    use insta::assert_snapshot;
    use uv_errors::{ErrorOptions, Hinted, Hints, SourceFile, write_error_chain_with_options};
    use uv_normalize::PackageName;
    use uv_pep508::MarkerTree;

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
        let source = "# café\r\n[tool.uv.sources]\r\n\"My.Package\" = [\r\n  { index = 'private', marker = \"sys_platform == 'linux'\", extra = 'other' },\r\n  { url = 'https://user:secret@example.com/one.whl', marker = \"sys_platform == 'linux'\" },\r\n  { url = 'https://user:secret@example.com/two.whl', marker = \"python_version >= '3.12'\" },\r\n]\r\n";
        assert_snapshot!(format_error(source), @"
        error: Failed to parse `tool.uv.sources`
          cause: Source markers must be disjoint, but the following markers overlap: `sys_platform == 'linux'` and `python_full_version >= '3.12'`.
           --> pyproject.toml:6:63
          info: The other source is declared here
           --> pyproject.toml:5:63
          hint: replace `python_full_version >= '3.12'` with `python_full_version >= '3.12' and sys_platform != 'linux'`
        ");
    }

    #[test]
    fn missing_source_marker_in_array_tables() {
        let source = "[[tool.uv.sources.demo]]\nindex = 'first'\nmarker = \"sys_platform == 'linux'\"\n[[tool.uv.sources.demo]]\nurl = 'https://user:secret@example.com/demo.whl'\n";
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
            Some(SourceError::OverlappingMarkers(..))
        ));
        assert_snapshot!(format_error(source), @"
        error: Failed to parse `tool.uv.sources`
          cause: Source markers must be disjoint, but the following markers overlap: `sys_platform == 'linux'` and `sys_platform == 'linux'`.
           --> pyproject.toml:5:30
          info: The other source is declared here
           --> pyproject.toml:3:31
          hint: replace `sys_platform == 'linux'` with `python_version < '0'`
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
}
