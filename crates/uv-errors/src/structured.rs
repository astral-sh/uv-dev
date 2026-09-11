use std::error::Error;
use std::fmt::{self, Write};

use annotate_snippets::AnnotationKind;
use serde::Serialize;

use crate::diagnostic::is_layout_control;
use crate::report::{ResolvedError, resolve_error_chain};
use crate::source::{SourcePosition, SourceView, SourceViewKind, SourceWindowView, source_view};
use crate::{
    Diagnostic, DiagnosticFn, HintOrdering, Hints, Info, SourceSnippet, SourceSuggestion,
    SuggestionApplicability,
};

/// An experimental, serializable presentation of an error and its actual source chain.
///
/// The error array is ordered from the outer error to its innermost cause. Each entry owns its
/// information, hints, and source locations. Source text is limited to the same explicitly
/// selected windows as the terminal renderer; a retained file is never serialized wholesale.
/// Recognized URL components are masked without changing the original UTF-8 byte coordinates.
/// This uv-dev representation is not a stable machine-readable output contract.
#[derive(Serialize)]
pub(crate) struct ErrorReport {
    schema_version: u8,
    coordinates: Coordinates,
    level: String,
    errors: Vec<ReportError>,
}

impl ErrorReport {
    /// Resolve the presentation of each actual error in the chain.
    ///
    /// Explicit report hints belong to the outer error. Merge them before serialization so their
    /// priority and exact source-suggestion identity use the same rules as the terminal renderer.
    pub(crate) fn new(
        error: &(dyn Error + 'static),
        resolver: Option<DiagnosticFn>,
        hints: &Hints<'_>,
    ) -> Self {
        let (mut root, sources) = resolve_error_chain(error, resolver);
        root.diagnostic.hints.extend(hints.clone());
        Self {
            schema_version: 1,
            coordinates: Coordinates {
                line_base: 1,
                column_base: 0,
                column_encoding: "utf-8",
            },
            level: "error".to_string(),
            errors: std::iter::once(root)
                .chain(sources)
                .map(ReportError::from)
                .collect(),
        }
    }

    /// Use a custom severity, such as `warning`.
    #[must_use]
    pub(crate) fn with_level(mut self, level: &str) -> Self {
        self.level = plain_text(level);
        self
    }
}

#[derive(Serialize)]
struct Coordinates {
    line_base: u8,
    column_base: u8,
    column_encoding: &'static str,
}

#[derive(Serialize)]
struct ReportError {
    message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    sources: Vec<ReportSource>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    info: Vec<ReportInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    hints: Vec<ReportHint>,
}

impl From<ResolvedError<'_>> for ReportError {
    fn from(error: ResolvedError<'_>) -> Self {
        let message = plain_text(&error.message());
        let Diagnostic {
            snippets,
            info,
            hints,
            ..
        } = error.diagnostic;
        Self {
            message,
            sources: report_sources(&snippets),
            info: info.into_iter().map(ReportInfo::from).collect(),
            hints: report_hints(&hints),
        }
    }
}

#[derive(Serialize)]
struct ReportInfo {
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    sources: Vec<ReportSource>,
}

impl From<Info<'_>> for ReportInfo {
    fn from(info: Info<'_>) -> Self {
        Self {
            message: plain_text(&info.message),
            details: info
                .details
                .as_deref()
                .filter(|details| !details.is_empty())
                .map(plain_text),
            sources: report_sources(&info.snippets),
        }
    }
}

#[derive(Serialize)]
struct ReportHint {
    message: String,
    ordering: ReportHintOrdering,
    #[serde(skip_serializing_if = "Option::is_none")]
    suggestion: Option<ReportSuggestion>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum ReportHintOrdering {
    First,
    Any,
    Last,
}

fn report_hints(hints: &Hints<'_>) -> Vec<ReportHint> {
    [HintOrdering::First, HintOrdering::Any, HintOrdering::Last]
        .into_iter()
        .flat_map(|ordering| {
            hints
                .iter_for_ordering(ordering)
                .map(move |hint| ReportHint {
                    message: plain_text(&hint.message),
                    ordering: match ordering {
                        HintOrdering::First => ReportHintOrdering::First,
                        HintOrdering::Any => ReportHintOrdering::Any,
                        HintOrdering::Last => ReportHintOrdering::Last,
                    },
                    suggestion: hint.suggestion.as_ref().and_then(report_suggestion),
                })
        })
        .collect()
}

#[derive(Serialize)]
struct ReportSuggestion {
    applicability: ReportSuggestionApplicability,
    source: ReportSource,
    edits: Vec<ReportEdit>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum ReportSuggestionApplicability {
    DisplayOnly,
    Unsafe,
    Safe,
}

#[derive(Serialize)]
struct ReportEdit {
    range: ReportRange,
    replacement: String,
}

fn report_suggestion(suggestion: &SourceSuggestion) -> Option<ReportSuggestion> {
    let snippet = suggestion.snippet();
    let source = source_view(&snippet)?.into();
    let positions = suggestion.source().position_index();
    let edits = suggestion
        .edits()
        .iter()
        .map(|edit| {
            Some(ReportEdit {
                range: ReportRange {
                    start: positions.position(edit.range.start)?.into(),
                    end: positions.position(edit.range.end)?.into(),
                },
                // Replacement contents are exact decoded text, just like retained source. The
                // JSON transport escapes terminal controls without changing the edit itself.
                replacement: edit.replacement.clone(),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(ReportSuggestion {
        applicability: match suggestion.applicability() {
            SuggestionApplicability::DisplayOnly => ReportSuggestionApplicability::DisplayOnly,
            SuggestionApplicability::Unsafe => ReportSuggestionApplicability::Unsafe,
            SuggestionApplicability::Safe => ReportSuggestionApplicability::Safe,
        },
        source,
        edits,
    })
}

#[derive(Serialize)]
struct ReportSource {
    /// A display name, not a promise that the source has a reopenable filesystem path.
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(flatten)]
    content: ReportSourceContent,
}

impl From<SourceView<'_>> for ReportSource {
    fn from(view: SourceView<'_>) -> Self {
        Self {
            name: view.name,
            content: match view.kind {
                SourceViewKind::Origin => ReportSourceContent::Origin,
                SourceViewKind::Location(position) => ReportSourceContent::Location {
                    position: position.into(),
                },
                SourceViewKind::Windows(windows) => ReportSourceContent::Snippet {
                    windows: windows.into_iter().map(ReportWindow::from).collect(),
                },
            },
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ReportSourceContent {
    Origin,
    Location { position: ReportPosition },
    Snippet { windows: Vec<ReportWindow> },
}

#[derive(Serialize)]
struct ReportWindow {
    line_start: usize,
    /// Display text whose URL masks retain the original byte length and line boundaries.
    text: String,
    annotations: Vec<ReportAnnotation>,
}

impl From<SourceWindowView<'_>> for ReportWindow {
    fn from(window: SourceWindowView<'_>) -> Self {
        let positions = window.position_index();
        let annotations = window
            .annotations
            .iter()
            .filter_map(|annotation| {
                let kind = match annotation.kind {
                    AnnotationKind::Primary => ReportAnnotationKind::Primary,
                    AnnotationKind::Context => ReportAnnotationKind::Secondary,
                    _ => return None,
                };
                Some(ReportAnnotation {
                    kind,
                    range: ReportRange {
                        start: positions.position(annotation.range.start)?.into(),
                        end: positions.position(annotation.range.end)?.into(),
                    },
                    label: annotation.label.map(plain_text),
                })
            })
            .collect();
        Self {
            line_start: window.line_start,
            text: window.redacted_text().into_owned(),
            annotations,
        }
    }
}

#[derive(Serialize)]
struct ReportAnnotation {
    kind: ReportAnnotationKind,
    range: ReportRange,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum ReportAnnotationKind {
    Primary,
    Secondary,
}

#[derive(Serialize)]
struct ReportRange {
    start: ReportPosition,
    end: ReportPosition,
}

#[derive(Serialize)]
struct ReportPosition {
    line: usize,
    byte_column: usize,
}

impl From<SourcePosition> for ReportPosition {
    fn from(position: SourcePosition) -> Self {
        Self {
            line: position.line,
            byte_column: position.byte_column,
        }
    }
}

fn report_sources(snippets: &[SourceSnippet<'_>]) -> Vec<ReportSource> {
    snippets
        .iter()
        .filter_map(source_view)
        .map(ReportSource::from)
        .collect()
}

fn plain_text(text: &str) -> String {
    anstream::adapter::strip_str(text).to_string()
}

/// Write one complete JSON line. Escaping layout controls changes only the JSON transport, not the
/// decoded display text or the original coordinates that refer to it.
pub(crate) fn write_report(stream: &mut impl Write, report: &ErrorReport) -> fmt::Result {
    let mut serialized = serde_json::to_string(report).map_err(|_| fmt::Error)?;
    if serialized
        .chars()
        .any(|character| character.is_control() || is_layout_control(character))
    {
        let mut escaped = String::with_capacity(serialized.len());
        for character in serialized.chars() {
            if character.is_control() || is_layout_control(character) {
                let mut encoded = [0; 2];
                for &unit in character.encode_utf16(&mut encoded).iter() {
                    write!(escaped, "\\u{unit:04x}")?;
                }
            } else {
                escaped.push(character);
            }
        }
        serialized = escaped;
    }
    serialized.push('\n');
    stream.write_str(&serialized)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::ops::Range;

    use insta::assert_json_snapshot;

    use crate::{
        Diagnostic, Hint, HintOrdering, Hints, Info, SourceAnnotation, SourceEdit, SourceFile,
        SourceSnippet, SourceSuggestion, SuggestionApplicability,
    };

    use super::{ErrorReport, write_report};

    #[derive(Debug, thiserror::Error)]
    #[error("outer")]
    struct Outer(#[source] Middle);

    #[derive(Debug, thiserror::Error)]
    #[error("middle")]
    struct Middle(#[source] Inner);

    #[derive(Debug, thiserror::Error)]
    #[error("inner")]
    struct Inner;

    fn chain_diagnostic<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
        if error.is::<Outer>() {
            Some(
                Diagnostic::new("\u{1b}[31mouter presentation\u{1b}[0m")
                    .with_info(Info::new("outer context").with_details("first line\n  second line"))
                    .with_hints(
                        [
                            Hint::new("outer trailing").with_ordering(HintOrdering::Last),
                            Hint::new("outer immediate").with_ordering(HintOrdering::First),
                        ]
                        .into_iter()
                        .collect(),
                    )
                    .with_source(
                        Diagnostic::new("middle presentation")
                            .with_info(Info::new("middle context"))
                            .with_hints(Hint::new("middle override").into()),
                    ),
            )
        } else if error.is::<Middle>() {
            Some(
                Diagnostic::new("native middle")
                    .with_info(Info::new("native middle context"))
                    .with_hints(
                        Hint::new("middle native")
                            .with_ordering(HintOrdering::First)
                            .into(),
                    ),
            )
        } else if error.is::<Inner>() {
            Some(
                Diagnostic::default().with_hints(
                    Hint::new("inner trailing")
                        .with_ordering(HintOrdering::Last)
                        .into(),
                ),
            )
        } else {
            None
        }
    }

    #[test]
    fn retains_actual_causes_and_metadata_owners() {
        let error = Outer(Middle(Inner));
        assert_json_snapshot!(ErrorReport::new(&error, Some(chain_diagnostic), &Hints::none()).with_level("warning"), @r#"
        {
          "schema_version": 1,
          "coordinates": {
            "line_base": 1,
            "column_base": 0,
            "column_encoding": "utf-8"
          },
          "level": "warning",
          "errors": [
            {
              "message": "outer presentation",
              "info": [
                {
                  "message": "outer context",
                  "details": "first line\n  second line"
                }
              ],
              "hints": [
                {
                  "message": "outer immediate",
                  "ordering": "first"
                },
                {
                  "message": "outer trailing",
                  "ordering": "last"
                }
              ]
            },
            {
              "message": "middle presentation",
              "info": [
                {
                  "message": "native middle context"
                },
                {
                  "message": "middle context"
                }
              ],
              "hints": [
                {
                  "message": "middle native",
                  "ordering": "first"
                },
                {
                  "message": "middle override",
                  "ordering": "any"
                }
              ]
            },
            {
              "message": "inner",
              "hints": [
                {
                  "message": "inner trailing",
                  "ordering": "last"
                }
              ]
            }
          ]
        }
        "#);
    }

    #[derive(Debug, thiserror::Error)]
    #[error("invalid input")]
    struct InputError(Vec<SourceSnippet<'static>>);

    fn input_diagnostic<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
        Some(
            error
                .downcast_ref::<InputError>()?
                .0
                .iter()
                .cloned()
                .fold(Diagnostic::default(), Diagnostic::with_snippet),
        )
    }

    fn range_of(text: &str, value: &str) -> Range<usize> {
        let start = text
            .find(value)
            .expect("value exists in the source fixture");
        start..start + value.len()
    }

    #[test]
    fn source_windows_keep_exact_text_and_utf8_coordinates() -> Result<(), Box<dyn Error>> {
        let text = "name = 'café'\r\nsecret = 'do-not-show'\r\nvalue = 42\r\n";
        let error = InputError(vec![
            SourceSnippet::new(SourceFile::new("config.toml", text).with_line_start(40))
                .with_annotation(
                    SourceAnnotation::primary(range_of(text, "café")).with_label("name"),
                )
                .with_annotation(
                    SourceAnnotation::secondary(range_of(text, "42")).with_label("related"),
                ),
        ]);
        let report = ErrorReport::new(&error, Some(input_diagnostic), &Hints::none());
        assert_json_snapshot!(&report.errors[0].sources, @r#"
        [
          {
            "name": "config.toml",
            "kind": "snippet",
            "windows": [
              {
                "line_start": 40,
                "text": "name = 'café'\r\n",
                "annotations": [
                  {
                    "kind": "primary",
                    "range": {
                      "start": {
                        "line": 40,
                        "byte_column": 8
                      },
                      "end": {
                        "line": 40,
                        "byte_column": 13
                      }
                    },
                    "label": "name"
                  }
                ]
              },
              {
                "line_start": 42,
                "text": "value = 42\r\n",
                "annotations": [
                  {
                    "kind": "secondary",
                    "range": {
                      "start": {
                        "line": 42,
                        "byte_column": 8
                      },
                      "end": {
                        "line": 42,
                        "byte_column": 10
                      }
                    },
                    "label": "related"
                  }
                ]
              }
            ]
          }
        ]
        "#);
        assert!(!serde_json::to_string(&report)?.contains("do-not-show"));
        Ok(())
    }

    #[test]
    fn merged_annotations_keep_exact_utf8_coordinates() -> Result<(), Box<dyn Error>> {
        let line = "name = 'café🦀'\r\n";
        let text = line.repeat(64);
        let local_range = range_of(line, "café🦀");
        let snippet = (0..64).fold(
            SourceSnippet::new(SourceFile::new("many.toml", text.as_str()).with_line_start(40)),
            |snippet, index| {
                let offset = index * line.len();
                let range = offset + local_range.start..offset + local_range.end;
                snippet.with_annotation(if index % 2 == 0 {
                    SourceAnnotation::primary(range)
                } else {
                    SourceAnnotation::secondary(range)
                })
            },
        );
        let report = ErrorReport::new(
            &InputError(vec![snippet]),
            Some(input_diagnostic),
            &Hints::none(),
        );
        let annotations = (0..64)
            .map(|index| {
                serde_json::json!({
                    "kind": if index % 2 == 0 { "primary" } else { "secondary" },
                    "range": {
                        "start": { "line": 40 + index, "byte_column": local_range.start },
                        "end": { "line": 40 + index, "byte_column": local_range.end }
                    }
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            serde_json::to_value(&report.errors[0].sources)?,
            serde_json::json!([{
                "name": "many.toml",
                "kind": "snippet",
                "windows": [{
                    "line_start": 40,
                    "text": text,
                    "annotations": annotations
                }]
            }])
        );
        Ok(())
    }

    #[test]
    fn source_urls_keep_original_utf8_byte_coordinates() -> Result<(), Box<dyn Error>> {
        let text = "invalid = !\r--index-url https://user:sëcret🦀@example.invalid/simple?sig=秘密&safe=yes\r\n";
        let source = SourceFile::new("requirements.txt", text).with_line_start(7);
        let value = range_of(text, "safe=yes");
        let error = InputError(vec![
            SourceSnippet::new(source.clone())
                .with_annotation(SourceAnnotation::primary(range_of(text, "!")))
                .with_annotation(SourceAnnotation::secondary(value.clone())),
        ]);
        let report = ErrorReport::new(&error, Some(input_diagnostic), &Hints::none());
        let sources = serde_json::to_value(&report.errors[0].sources)?;
        let displayed = sources[0]["windows"][0]["text"]
            .as_str()
            .ok_or("the source window has text")?;
        assert_eq!(displayed.len(), text.len());
        assert_eq!(displayed.get(value.clone()), text.get(value));
        assert_eq!(source.text(), text);
        assert_json_snapshot!(sources, @r#"
        [
          {
            "kind": "snippet",
            "name": "requirements.txt",
            "windows": [
              {
                "annotations": [
                  {
                    "kind": "primary",
                    "range": {
                      "end": {
                        "byte_column": 11,
                        "line": 7
                      },
                      "start": {
                        "byte_column": 10,
                        "line": 7
                      }
                    }
                  },
                  {
                    "kind": "secondary",
                    "range": {
                      "end": {
                        "byte_column": 91,
                        "line": 7
                      },
                      "start": {
                        "byte_column": 83,
                        "line": 7
                      }
                    }
                  }
                ],
                "line_start": 7,
                "text": "invalid = !\r--index-url https://user:***********@example.invalid/simple?sig=******&safe=yes\r\n"
              }
            ]
          }
        ]
        "#);
        Ok(())
    }

    #[test]
    fn source_url_masks_do_not_change_exact_edits() -> Result<(), Box<dyn Error>> {
        let text = "url = 'https://user:original-secret@example.invalid'; marker = 'old'\n";
        let source = SourceFile::new("pyproject.toml", text);
        let range = range_of(text, "'old'");
        let replacement = "'https://user:replacement-secret@example.invalid'";
        let suggestion = SourceSuggestion::new(
            source.clone(),
            [SourceEdit::new(range.clone(), replacement)],
            SuggestionApplicability::DisplayOnly,
        )
        .ok_or("the edit has a valid source range")?;
        let hints = Hint::new("Replace the marker")
            .with_suggestion(suggestion)
            .into();
        let report = serde_json::to_value(ErrorReport::new(&Inner, None, &hints))?;
        let edit = &report["errors"][0]["hints"][0]["suggestion"]["edits"][0];
        assert_eq!(edit["replacement"], replacement);
        assert_eq!(
            edit["range"],
            serde_json::json!({
                "start": {"line": 1, "byte_column": range.start},
                "end": {"line": 1, "byte_column": range.end},
            })
        );
        assert_eq!(source.text(), text);
        Ok(())
    }

    #[test]
    fn location_only_sources_do_not_expose_annotations() {
        let source = SourceFile::new("private.toml", "prefix = 'é'\npassword = 'do-not-show'\n")
            .with_line_start(7);
        let error = InputError(vec![
            SourceSnippet::new(source)
                .with_annotation(SourceAnnotation::secondary(0..6))
                .with_annotation(SourceAnnotation::primary(11..12))
                .with_annotation(
                    SourceAnnotation::primary(12..13).with_label("hidden annotation label"),
                )
                .without_source_text(),
        ]);
        let report = ErrorReport::new(&error, Some(input_diagnostic), &Hints::none());
        assert_json_snapshot!(&report.errors[0].sources, @r#"
        [
          {
            "name": "private.toml",
            "kind": "location",
            "position": {
              "line": 7,
              "byte_column": 12
            }
          }
        ]
        "#);
    }

    #[test]
    fn invalid_sources_fall_back_to_their_names() {
        let error = InputError(vec![
            SourceSnippet::new(SourceFile::new("zero.toml", "x").with_line_start(0))
                .with_annotation(SourceAnnotation::primary(0..1)),
            SourceSnippet::new(
                SourceFile::new("overflow.toml", "x\ny").with_line_start(usize::MAX),
            )
            .with_annotation(SourceAnnotation::primary(0..1)),
            SourceSnippet::new(SourceFile::new("range.toml", "x"))
                .with_annotation(SourceAnnotation::primary(0..100)),
            SourceSnippet::new(SourceFile::new("", "hidden")).without_source_text(),
        ]);
        let report = ErrorReport::new(&error, Some(input_diagnostic), &Hints::none());
        assert_json_snapshot!(&report.errors[0].sources, @r#"
        [
          {
            "name": "zero.toml",
            "kind": "origin"
          },
          {
            "name": "overflow.toml",
            "kind": "origin"
          },
          {
            "name": "range.toml",
            "kind": "origin"
          }
        ]
        "#);
    }

    #[test]
    fn json_transport_escapes_layout_controls_without_changing_source() -> Result<(), Box<dyn Error>>
    {
        let text = "value = '\u{202e}\u{85}\u{7f}'\n";
        let error = InputError(vec![
            SourceSnippet::new(SourceFile::new("input.toml", text))
                .with_annotation(SourceAnnotation::primary(0..text.len())),
        ]);
        let report = ErrorReport::new(&error, Some(input_diagnostic), &Hints::none());
        let mut output = String::new();
        write_report(&mut output, &report)?;
        assert!(!output.trim_end_matches('\n').chars().any(|character| {
            character.is_control() || crate::diagnostic::is_layout_control(character)
        }));
        let decoded: serde_json::Value = serde_json::from_str(&output)?;
        assert_eq!(
            decoded["errors"][0]["sources"][0]["windows"][0]["text"].as_str(),
            Some(text)
        );
        Ok(())
    }
}
