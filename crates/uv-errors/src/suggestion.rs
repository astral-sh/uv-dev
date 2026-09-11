use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use crate::{SourceAnnotation, SourceFile, SourceSnippet};

/// Whether a suggested edit is suitable for automatic application after its source is verified.
///
/// This does not establish that a source display name is a writable path. The diagnostics layer
/// only describes edits to a retained, decoded source snapshot; it never applies them.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionApplicability {
    /// The suggestion is explanatory and must not be applied automatically.
    #[default]
    DisplayOnly,
    /// Applying the suggestion may change behavior in a way that requires user review.
    Unsafe,
    /// The producer knows that the suggested change preserves the intended behavior.
    Safe,
}

/// One replacement in a retained source snapshot.
///
/// The range uses half-open UTF-8 byte offsets into [`SourceFile::text`]. An empty range inserts
/// text; an empty replacement deletes the selected text.
#[derive(Clone)]
pub struct SourceEdit {
    pub(crate) range: Range<usize>,
    pub(crate) replacement: String,
}

impl SourceEdit {
    /// Describe a replacement. [`SourceSuggestion::new`] validates its source range.
    pub fn new(range: Range<usize>, replacement: impl Into<String>) -> Self {
        Self {
            range,
            replacement: replacement.into(),
        }
    }

    /// The half-open UTF-8 byte range in the retained source.
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    /// The exact decoded replacement text.
    pub fn replacement(&self) -> &str {
        &self.replacement
    }
}

impl fmt::Debug for SourceEdit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Replacements can contain user-authored values. Debug logging must not expose text that
        // the producer only intended to show in a particular user-facing hint.
        formatter
            .debug_struct("SourceEdit")
            .field("range", &self.range)
            .field("replacement_len", &self.replacement.len())
            .finish()
    }
}

/// A validated, indivisible set of edits to one retained source snapshot.
///
/// Source text is hidden by default. A producer may opt in to a preview after checking every
/// complete source line that the edits would display. Replacement text is supplied explicitly by
/// the producer and must be suitable for user-facing output.
#[derive(Clone, Debug)]
pub struct SourceSuggestion {
    data: Arc<SuggestionData>,
    show_source: bool,
}

#[derive(Debug)]
struct SuggestionData {
    source: SourceFile,
    edits: Vec<SourceEdit>,
    applicability: SuggestionApplicability,
    preview_line_numbers_fit: bool,
}

impl SourceSuggestion {
    /// Validate and retain edits to a single decoded source.
    ///
    /// Edits are sorted in source order. Invalid UTF-8 ranges, an empty edit set, overlapping
    /// edits, and insertions touching another edit are rejected. Callers can retain their plain
    /// hint when an exact replacement cannot be represented.
    pub fn new(
        source: SourceFile,
        edits: impl IntoIterator<Item = SourceEdit>,
        applicability: SuggestionApplicability,
    ) -> Option<Self> {
        if source.line_start() == 0 {
            return None;
        }
        let original_newlines = source.text().bytes().filter(|byte| *byte == b'\n').count();
        let original_last_line = source.line_start().checked_add(original_newlines)?;

        let mut edits = edits.into_iter().collect::<Vec<_>>();
        if edits.is_empty()
            || edits.iter().any(|edit| {
                source.text().get(edit.range.clone()).is_none()
                    || (edit.range.is_empty() && edit.replacement.is_empty())
            })
        {
            return None;
        }
        edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
        if edits.windows(2).any(|pair| {
            pair[0].range.end > pair[1].range.start
                || (pair[0].range.end == pair[1].range.start
                    && (pair[0].range.is_empty() || pair[1].range.is_empty()))
        }) {
            return None;
        }

        // The terminal patch renderer can number a spliced block relative to a later original
        // line. Reserve room for every original and inserted line, including lines an edit may
        // remove. The original coordinates remain usable if only this preview bound overflows.
        let preview_line_numbers_fit = original_last_line
            .checked_add(original_newlines)
            .and_then(|line| {
                edits.iter().try_fold(line, |line, edit| {
                    line.checked_add(
                        edit.replacement
                            .bytes()
                            .filter(|byte| *byte == b'\n')
                            .count(),
                    )
                })
            })
            .is_some();

        Some(Self {
            data: Arc::new(SuggestionData {
                source,
                edits,
                applicability,
                preview_line_numbers_fit,
            }),
            show_source: false,
        })
    }

    /// Show the complete source lines touched by the edits as a replacement preview.
    ///
    /// If the preview's line numbers cannot be represented, text output falls back to locations.
    #[must_use]
    #[cfg(test)]
    fn with_source_text(mut self) -> Self {
        self.show_source = true;
        self
    }

    /// The source snapshot to which every edit refers.
    pub fn source(&self) -> &SourceFile {
        &self.data.source
    }

    /// The validated edits, in source order.
    pub fn edits(&self) -> &[SourceEdit] {
        &self.data.edits
    }

    /// The producer's assessment of whether the edit is suitable for automatic application.
    pub fn applicability(&self) -> SuggestionApplicability {
        self.data.applicability
    }

    pub(crate) fn show_source(&self) -> bool {
        self.show_source
    }

    pub(crate) fn preview_line_numbers_fit(&self) -> bool {
        self.data.preview_line_numbers_fit
    }

    pub(crate) fn snippet(&self) -> SourceSnippet<'static> {
        let mut snippet = self.edits().iter().fold(
            SourceSnippet::new(self.source().clone()),
            |snippet, edit| snippet.with_annotation(SourceAnnotation::primary(edit.range.clone())),
        );
        if !self.show_source {
            snippet = snippet.without_source_text();
        }
        snippet
    }

    /// Merge only references to the exact same retained suggestion.
    ///
    /// Equal display messages or equal replacement text do not establish source identity. Every
    /// reference must also permit a preview before the merged hint can expose source text.
    pub(crate) fn unambiguous_with(&self, other: &Self) -> Option<Self> {
        Arc::ptr_eq(&self.data, &other.data).then(|| Self {
            data: Arc::clone(&self.data),
            show_source: self.show_source && other.show_source,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::ops::Range;

    use insta::{assert_json_snapshot, assert_snapshot};

    use super::{SourceEdit, SourceSuggestion, SuggestionApplicability};
    use crate::{
        Diagnostic, ErrorFormat, ErrorOptions, ErrorReport, Hint, HintOrdering, Hints, SourceFile,
        write_error_chain_with_options,
    };

    fn suggestion(edits: impl IntoIterator<Item = SourceEdit>) -> Option<SourceSuggestion> {
        SourceSuggestion::new(
            SourceFile::new("input", "aé\nz"),
            edits,
            SuggestionApplicability::DisplayOnly,
        )
    }

    #[test]
    fn validates_ranges_and_edit_boundaries() {
        assert!(suggestion([]).is_none());
        assert!(suggestion([SourceEdit::new(1..2, "x")]).is_none());
        assert!(suggestion([SourceEdit::new(2..3, "x")]).is_none());
        assert!(suggestion([SourceEdit::new(0..99, "x")]).is_none());
        assert!(suggestion([SourceEdit::new(1..1, "")]).is_none());
        assert!(suggestion([SourceEdit::new(0..3, "x"), SourceEdit::new(1..3, "y")]).is_none());
        assert!(suggestion([SourceEdit::new(0..1, "x"), SourceEdit::new(1..1, "y")]).is_none());
        assert!(suggestion([SourceEdit::new(1..1, "x"), SourceEdit::new(1..3, "y")]).is_none());
        assert!(suggestion([SourceEdit::new(1..1, "x"), SourceEdit::new(1..1, "y")]).is_none());

        let suggestion = suggestion([SourceEdit::new(1..3, ""), SourceEdit::new(0..1, "A")])
            .expect("adjacent replacements are unambiguous");
        assert_eq!(suggestion.edits()[0].range, 0..1);
        assert_eq!(suggestion.edits()[1].range, 1..3);
        assert!(
            super::SourceSuggestion::new(
                SourceFile::new("input", "text\n").with_line_start(usize::MAX),
                [SourceEdit::new(0..1, "T")],
                SuggestionApplicability::Safe,
            )
            .is_none()
        );
    }

    #[test]
    fn identity_and_preview_permission_are_explicit() {
        let original = suggestion([SourceEdit::new(1..3, "e")]).expect("valid edit");
        let shown = original.clone().with_source_text();
        assert!(
            !shown
                .unambiguous_with(&original)
                .expect("same retained suggestion")
                .show_source()
        );
        assert!(
            shown
                .unambiguous_with(&shown)
                .expect("same retained suggestion")
                .show_source()
        );
        let independently_created = suggestion([SourceEdit::new(1..3, "e")]).expect("valid edit");
        assert!(original.unambiguous_with(&independently_created).is_none());
    }

    #[test]
    fn debug_output_does_not_disclose_replacements() {
        let suggestion =
            suggestion([SourceEdit::new(0..1, "sentinel-secret")]).expect("valid edit");
        assert!(!format!("{suggestion:?}").contains("sentinel-secret"));
    }

    #[test]
    fn duplicate_hint_messages_do_not_establish_edit_identity() {
        let original = suggestion([SourceEdit::new(1..3, "e")]).expect("valid edit");
        let mut hints = Hints::from(
            Hint::new("replace the value")
                .with_ordering(HintOrdering::Last)
                .with_suggestion(original.clone().with_source_text()),
        );
        hints.extend(Hints::from(
            Hint::new("replace the value").with_suggestion(original),
        ));
        assert_eq!(hints.0.len(), 1);
        assert_eq!(hints.0[0].ordering, HintOrdering::Any);
        assert!(
            !hints.0[0]
                .suggestion
                .as_ref()
                .expect("same retained edit")
                .show_source()
        );

        hints.extend(Hints::from(Hint::new("replace the value").with_suggestion(
            suggestion([SourceEdit::new(1..3, "e")]).expect("independent valid edit"),
        )));
        assert!(hints.0[0].suggestion.is_none());

        let mut plain = Hints::from("replace the value");
        plain.extend(Hints::from(Hint::new("replace the value").with_suggestion(
            suggestion([SourceEdit::new(1..3, "e")]).expect("valid edit"),
        )));
        assert!(plain.0[0].suggestion.is_none());
    }

    #[derive(Debug, thiserror::Error)]
    #[error("failed to load input")]
    struct Outer(#[source] Suggested);

    #[derive(Debug, thiserror::Error)]
    #[error("invalid declaration")]
    struct Suggested(SourceSuggestion);

    fn diagnostic<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
        if error.is::<Outer>() {
            Some(
                Diagnostic::default().with_hints(
                    Hint::new("check the input")
                        .with_ordering(HintOrdering::Last)
                        .into(),
                ),
            )
        } else {
            let error = error.downcast_ref::<Suggested>()?;
            Some(
                Diagnostic::default().with_hints(
                    Hint::new("replace the selected values")
                        .with_suggestion(error.0.clone())
                        .into(),
                ),
            )
        }
    }

    fn render(suggestion: SourceSuggestion, format: ErrorFormat) -> String {
        let mut output = String::new();
        write_error_chain_with_options(
            &Outer(Suggested(suggestion)),
            &Hints::none(),
            ErrorOptions::default()
                .with_stream(&mut output)
                .with_width_override(usize::MAX)
                .with_format(format)
                .with_diagnostic(diagnostic),
        )
        .expect("format suggestion");
        if format == ErrorFormat::Text {
            anstream::adapter::strip_str(&output).to_string()
        } else {
            output
        }
    }

    fn range_of(text: &str, value: &str) -> Range<usize> {
        let start = text.find(value).expect("value exists in fixture");
        start..start + value.len()
    }

    fn example() -> SourceSuggestion {
        let text = "name = 'café'\r\nsecret = 'sentinel-secret'\r\ncount = 2\r\n";
        SourceSuggestion::new(
            SourceFile::new("config.toml", text).with_line_start(40),
            [
                SourceEdit::new(range_of(text, "café"), "coffee"),
                SourceEdit::new(range_of(text, "2"), "3"),
            ],
            SuggestionApplicability::Unsafe,
        )
        .expect("valid edits")
    }

    #[test]
    fn edit_previews_keep_their_owner_and_selected_windows() {
        let hidden = render(example(), ErrorFormat::Text);
        let shown = render(example().with_source_text(), ErrorFormat::Text);
        assert!(!hidden.contains("sentinel-secret"));
        assert!(!shown.contains("sentinel-secret"));
        assert_snapshot!(hidden, @"
        error: failed to load input
          cause: invalid declaration
          hint: replace the selected values
           --> config.toml:40:9
           ::: config.toml:42:9
          hint: check the input
        ");
        assert_snapshot!(shown, @"
        error: failed to load input
          cause: invalid declaration
          hint: replace the selected values
            --> config.toml:40:9
             |
          40 - name = 'café'
          40 + name = 'coffee'
             |
          42 - count = 2
          42 + count = 3
             |
          hint: check the input
        ");
    }

    #[test]
    fn structured_edits_use_original_utf8_coordinates() -> Result<(), Box<dyn Error>> {
        let error = Outer(Suggested(example()));
        let report = ErrorReport::new(&error, Some(diagnostic));
        let value = serde_json::to_value(&report)?;
        assert!(!serde_json::to_string(&report)?.contains("sentinel-secret"));
        assert_json_snapshot!(&value["errors"][1]["hints"], @r#"
        [
          {
            "message": "replace the selected values",
            "ordering": "any",
            "suggestion": {
              "applicability": "unsafe",
              "edits": [
                {
                  "range": {
                    "end": {
                      "byte_column": 13,
                      "line": 40
                    },
                    "start": {
                      "byte_column": 8,
                      "line": 40
                    }
                  },
                  "replacement": "coffee"
                },
                {
                  "range": {
                    "end": {
                      "byte_column": 9,
                      "line": 42
                    },
                    "start": {
                      "byte_column": 8,
                      "line": 42
                    }
                  },
                  "replacement": "3"
                }
              ],
              "source": {
                "kind": "location",
                "name": "config.toml",
                "position": {
                  "byte_column": 8,
                  "line": 40
                }
              }
            }
          }
        ]
        "#);

        let shown = Outer(Suggested(example().with_source_text()));
        let shown = serde_json::to_value(ErrorReport::new(&shown, Some(diagnostic)))?;
        assert_json_snapshot!(&shown["errors"][1]["hints"][0]["suggestion"]["source"], @r#"
        {
          "kind": "snippet",
          "name": "config.toml",
          "windows": [
            {
              "annotations": [
                {
                  "kind": "primary",
                  "range": {
                    "end": {
                      "byte_column": 13,
                      "line": 40
                    },
                    "start": {
                      "byte_column": 8,
                      "line": 40
                    }
                  }
                }
              ],
              "line_start": 40,
              "text": "name = 'café'\r\n"
            },
            {
              "annotations": [
                {
                  "kind": "primary",
                  "range": {
                    "end": {
                      "byte_column": 9,
                      "line": 42
                    },
                    "start": {
                      "byte_column": 8,
                      "line": 42
                    }
                  }
                }
              ],
              "line_start": 42,
              "text": "count = 2\r\n"
            }
          ]
        }
        "#);
        Ok(())
    }

    #[test]
    fn edit_previews_handle_valid_eof_and_multiline_ranges() {
        for text in ["", "é", "aé\r\nz\n", "a\u{009b}é\n"] {
            for start in 0..=text.len() {
                for end in start..=text.len() {
                    if let Some(suggestion) = SourceSuggestion::new(
                        SourceFile::new("input", text),
                        [SourceEdit::new(start..end, "next\nline")],
                        SuggestionApplicability::DisplayOnly,
                    ) {
                        let _ = render(suggestion.with_source_text(), ErrorFormat::Text);
                    }
                }
            }
        }
    }

    #[test]
    fn replacement_transport_preserves_exact_text() -> Result<(), Box<dyn Error>> {
        let replacement = "\u{1b}[2J\u{202e}café\n";
        let suggestion = SourceSuggestion::new(
            SourceFile::new("input", "old\n"),
            [SourceEdit::new(0..3, replacement)],
            SuggestionApplicability::DisplayOnly,
        )
        .expect("valid edit")
        .with_source_text();
        let text = render(suggestion.clone(), ErrorFormat::Text);
        assert!(!text.contains('\u{1b}'));
        assert!(!text.contains('\u{202e}'));
        let json = render(suggestion, ErrorFormat::Json);
        assert!(!json.contains('\u{1b}'));
        assert!(!json.contains('\u{202e}'));
        let decoded: serde_json::Value = serde_json::from_str(&json)?;
        assert_eq!(
            decoded["errors"][1]["hints"][0]["suggestion"]["edits"][0]["replacement"],
            replacement
        );
        Ok(())
    }

    #[test]
    fn legacy_explicit_hints_retain_their_edits() -> Result<(), Box<dyn Error>> {
        let error = anyhow::anyhow!("inner").context("outer");
        let hints = Hints::from(
            Hint::new("replace the selected values")
                .with_ordering(HintOrdering::First)
                .with_suggestion(example().with_source_text()),
        );
        let mut text = String::new();
        write_error_chain_with_options(
            error.as_ref(),
            &hints,
            ErrorOptions::default()
                .with_stream(&mut text)
                .with_width_override(usize::MAX),
        )?;
        assert_snapshot!(anstream::adapter::strip_str(&text), @"
        error: outer
          cause: inner

        hint: replace the selected values
            --> config.toml:40:9
             |
          40 - name = 'café'
          40 + name = 'coffee'
             |
          42 - count = 2
          42 + count = 3
             |
        ");

        let mut json = String::new();
        write_error_chain_with_options(
            error.as_ref(),
            &hints,
            ErrorOptions::default()
                .with_stream(&mut json)
                .with_format(ErrorFormat::Json),
        )?;
        let value: serde_json::Value = serde_json::from_str(&json)?;
        assert_eq!(value["errors"][0]["hints"][0]["ordering"], "last");
        assert!(value["errors"][0]["hints"][0]["suggestion"].is_object());
        assert!(value["errors"][1].get("hints").is_none());
        Ok(())
    }

    #[test]
    fn overflowing_preview_line_numbers_fall_back_to_a_location() -> Result<(), Box<dyn Error>> {
        let suggestion = SourceSuggestion::new(
            SourceFile::new("input", "x").with_line_start(usize::MAX),
            [SourceEdit::new(0..1, "a\nb")],
            SuggestionApplicability::DisplayOnly,
        )
        .expect("the original coordinates are valid")
        .with_source_text();
        let text = render(suggestion.clone(), ErrorFormat::Text);
        assert!(text.contains(&format!("input:{}:1", usize::MAX)));
        assert!(!text.contains(" - x"));
        let json = render(suggestion, ErrorFormat::Json);
        let value: serde_json::Value = serde_json::from_str(&json)?;
        let suggestion = &value["errors"][1]["hints"][0]["suggestion"];
        assert_eq!(suggestion["source"]["kind"], "snippet");
        assert_eq!(suggestion["edits"][0]["range"]["start"]["line"], usize::MAX);
        assert_eq!(suggestion["edits"][0]["replacement"], "a\nb");
        Ok(())
    }
}
