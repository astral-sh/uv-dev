use std::borrow::Cow;
use std::fmt::{self, Write};
use std::ops::Range;
use std::sync::Arc;

use annotate_snippets::renderer::{AnsiColor, Effects};
use annotate_snippets::{AnnotationKind, Element, Group, Level, Origin, Renderer, Snippet};

/// Immutable source text retained when an error is produced.
///
/// Ranges refer to the bytes in [`Self::text`], after any decoding or redaction performed by the
/// producer. The renderer never reopens the file. The name should be safe to display to the user.
#[derive(Clone)]
struct SourceFile {
    name: Arc<str>,
    text: Arc<str>,
    line_start: usize,
}

impl SourceFile {
    /// Retain the display name and complete decoded contents of a source file.
    #[cfg(test)]
    fn new(name: impl Into<Arc<str>>, text: impl Into<Arc<str>>) -> Self {
        Self {
            name: name.into(),
            text: text.into(),
            line_start: 1,
        }
    }

    /// Set the known, one-based starting line of an excerpt.
    ///
    /// The excerpt must start at a line boundary, and annotation ranges must be relative to the
    /// excerpt. An invalid line number causes the renderer to show only the source name.
    #[cfg(test)]
    #[must_use]
    fn with_line_start(mut self, line_start: usize) -> Self {
        self.line_start = line_start;
        self
    }

    /// The user-facing source name.
    fn name(&self) -> &str {
        &self.name
    }

    /// The exact decoded text used to calculate annotation ranges.
    fn text(&self) -> &str {
        &self.text
    }

    /// The complete source lines that a zero-context annotation would display for this byte span.
    ///
    /// This uses the renderer's LF-delimited line boundaries, including any CR bytes. Producers
    /// can inspect the same text before deciding whether a source excerpt is safe to show. An
    /// invalid UTF-8 range returns `None`.
    pub fn lines_for_span(&self, span: Range<usize>) -> Option<&str> {
        let lines = SourceLines::new(self.text());
        let (first, last) = lines.annotation_lines(&span)?;
        self.text().get(lines.range(first, last)?)
    }
}

impl fmt::Debug for SourceFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Configuration files can contain credentials, so verbose error logging must not dump
        // their complete retained contents.
        formatter
            .debug_struct("SourceFile")
            .field("name", &self.name)
            .field("len", &self.text.len())
            .field("line_start", &self.line_start)
            .finish()
    }
}

/// An annotation on a byte range in a [`SourceFile`].
#[derive(Clone, Debug)]
struct SourceAnnotation<'a> {
    range: Range<usize>,
    label: Option<Cow<'a, str>>,
    kind: AnnotationKind,
}

#[cfg(test)]
impl<'a> SourceAnnotation<'a> {
    /// Identify the source text responsible for the diagnostic.
    fn primary(range: Range<usize>) -> Self {
        Self {
            range,
            label: None,
            kind: AnnotationKind::Primary,
        }
    }

    /// Identify related source text that helps explain the diagnostic.
    fn secondary(range: Range<usize>) -> Self {
        Self {
            range,
            label: None,
            kind: AnnotationKind::Context,
        }
    }

    /// Describe why the source text is highlighted.
    #[must_use]
    fn with_label(mut self, label: impl Into<Cow<'a, str>>) -> Self {
        self.label = Some(label.into());
        self
    }
}

/// The annotations to show for one retained source.
///
/// Only annotated lines are shown by default. Producers should select or redact source excerpts
/// when even the annotated lines can contain secrets. Invalid byte ranges are ignored; if no
/// valid annotations remain, only the source name is shown.
#[derive(Clone, Debug)]
pub(crate) struct SourceSnippet<'a> {
    source: SourceFile,
    annotations: Vec<SourceAnnotation<'a>>,
    context_lines: usize,
    show_source: bool,
}

#[cfg(test)]
impl<'a> SourceSnippet<'a> {
    /// Refer to a source without inventing a location within it.
    fn new(source: SourceFile) -> Self {
        Self {
            source,
            annotations: Vec::new(),
            context_lines: 0,
            show_source: true,
        }
    }

    /// Add a primary or secondary source annotation.
    #[must_use]
    fn with_annotation(mut self, annotation: SourceAnnotation<'a>) -> Self {
        self.annotations.push(annotation);
        self
    }

    /// Opt in to showing this many surrounding lines for each annotation.
    #[must_use]
    fn with_context_lines(mut self, context_lines: usize) -> Self {
        self.context_lines = context_lines;
        self
    }

    /// Show a location without exposing the source text.
    ///
    /// The first valid primary annotation determines the location, or the first valid annotation
    /// when there is no primary annotation. Invalid ranges still fall back to the source name.
    #[must_use]
    fn without_source_text(mut self) -> Self {
        self.show_source = false;
        self
    }
}

pub(crate) enum SourceLevel {
    Error,
    Warning,
    Info,
}

impl SourceLevel {
    pub(crate) fn for_error(level: &str) -> Self {
        match level {
            "warning" => Self::Warning,
            "info" => Self::Info,
            _ => Self::Error,
        }
    }

    fn level(self) -> Level<'static> {
        match self {
            Self::Error => Level::ERROR,
            Self::Warning => Level::WARNING,
            Self::Info => Level::INFO,
        }
    }
}

pub(crate) fn write_snippets(
    stream: &mut impl Write,
    snippets: &[SourceSnippet<'_>],
    width: Option<usize>,
    level: SourceLevel,
) -> fmt::Result {
    if snippets.is_empty() {
        return Ok(());
    }

    let mut group = Group::with_level(level.level());
    for snippet in snippets {
        group = group.elements(source_elements(snippet));
    }
    if group.is_empty() {
        return Ok(());
    }

    let context_style = AnsiColor::Cyan.on_default().effects(Effects::BOLD);
    let renderer = Renderer::styled()
        .error(AnsiColor::Red.on_default().effects(Effects::BOLD))
        .warning(AnsiColor::Yellow.on_default().effects(Effects::BOLD))
        .info(context_style)
        .line_num(context_style)
        .context(context_style)
        // The renderer doubles this value in its long-line layout. Keep an effectively
        // unbounded width below the overflow boundary when line wrapping is disabled.
        .term_width(width.map_or(usize::MAX / 4, |width| {
            width.saturating_sub(2).min(usize::MAX / 4)
        }));
    let rendered = renderer.render(&[group]);
    for line in rendered.lines() {
        if line.is_empty() {
            writeln!(stream)?;
        } else {
            writeln!(stream, "  {line}")?;
        }
    }
    Ok(())
}

/// Select explicit source windows before passing them to the renderer. In particular, unrelated
/// configuration lines must not become visible just because two annotations are close together.
fn source_elements<'a>(snippet: &'a SourceSnippet<'_>) -> Vec<Element<'a>> {
    let source = &snippet.source;
    let lines = SourceLines::new(source.text());
    let path = normalize_single_line(source.name());
    let origin = || {
        if path.is_empty() {
            Vec::new()
        } else {
            vec![Origin::path(path.clone()).into()]
        }
    };
    if source.line_start == 0 || source.line_start.checked_add(lines.last()).is_none() {
        return origin();
    }

    if !snippet.show_source {
        let valid = |annotation: &&SourceAnnotation<'_>| {
            source.text().get(annotation.range.clone()).is_some()
        };
        let annotation = snippet
            .annotations
            .iter()
            .filter(valid)
            .find(|annotation| annotation.kind == AnnotationKind::Primary)
            .or_else(|| snippet.annotations.iter().find(valid));
        let Some(annotation) = annotation else {
            return origin();
        };
        let line = lines.line_at(annotation.range.start);
        let Some(line_range) = lines.range(line, line) else {
            return origin();
        };
        let Some(prefix) = source.text().get(line_range.start..annotation.range.start) else {
            return origin();
        };
        return if path.is_empty() {
            Vec::new()
        } else {
            vec![
                Origin::path(path.clone())
                    .line(source.line_start + line)
                    .char_column(prefix.chars().count() + 1)
                    .into(),
            ]
        };
    }

    let mut windows = Vec::new();
    for (index, annotation) in snippet.annotations.iter().enumerate() {
        // `annotate-snippets` assumes valid UTF-8 boundaries and panics on out-of-bounds ranges.
        // Reject a malformed location instead of moving the underline to unrelated source text.
        let Some((first, last)) = lines.annotation_lines(&annotation.range) else {
            continue;
        };
        let trailing_eof = annotation.range.is_empty()
            && annotation.range.end == source.text().len()
            && source.text().ends_with('\n');
        windows.push(SourceWindow {
            first: if trailing_eof {
                first
            } else {
                first.saturating_sub(snippet.context_lines)
            },
            last: if trailing_eof {
                last
            } else {
                last.saturating_add(snippet.context_lines).min(lines.last())
            },
            annotations: vec![index],
            trailing_eof,
        });
    }
    if windows.is_empty() {
        return origin();
    }
    windows.sort_by_key(|window| window.first);

    let mut merged: Vec<SourceWindow> = Vec::new();
    for window in windows {
        if let Some(previous) = merged.last_mut()
            && previous.trailing_eof == window.trailing_eof
            && window.first <= previous.last.saturating_add(1)
        {
            previous.last = previous.last.max(window.last);
            previous.annotations.extend(window.annotations);
        } else {
            merged.push(window);
        }
    }

    let mut elements = Vec::new();
    for mut window in merged {
        let Some(range) = lines.range(window.first, window.last) else {
            continue;
        };
        let Some(text) = source.text().get(range.clone()) else {
            continue;
        };
        let normalized = NormalizedSource::new(text);
        let mut annotations = Vec::new();
        window.annotations.sort_unstable();
        for index in window.annotations {
            let annotation = &snippet.annotations[index];
            let visible_range = lines.visible_range(annotation.range.clone());
            let local = visible_range.start - range.start..visible_range.end - range.start;
            let mut rendered = annotation.kind.span(normalized.range(local));
            if let Some(label) = &annotation.label {
                rendered = rendered.label(normalize_single_line(label));
            }
            annotations.push(rendered);

            if snippet.context_lines > 0
                && !window.trailing_eof
                && let Some((first, last)) = lines.annotation_lines(&annotation.range)
            {
                if first > window.first
                    && let Some(context) = lines.content_range(window.first, first - 1)
                {
                    annotations.push(AnnotationKind::Visible.span(
                        normalized.range(context.start - range.start..context.end - range.start),
                    ));
                }
                if last < window.last
                    && let Some(context) = lines.content_range(last + 1, window.last)
                {
                    annotations.push(AnnotationKind::Visible.span(
                        normalized.range(context.start - range.start..context.end - range.start),
                    ));
                }
            }
        }
        let mut rendered = Snippet::source(normalized.text)
            .line_start(source.line_start + window.first)
            .annotations(annotations);
        if !path.is_empty() {
            rendered = rendered.path(path.clone());
        }
        elements.push(rendered.into());
    }
    if elements.is_empty() {
        origin()
    } else {
        elements
    }
}

struct SourceWindow {
    first: usize,
    last: usize,
    annotations: Vec<usize>,
    trailing_eof: bool,
}

struct SourceLines<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> SourceLines<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            starts: std::iter::once(0)
                .chain(text.match_indices('\n').map(|(index, _)| index + 1))
                .collect(),
        }
    }

    fn last(&self) -> usize {
        self.starts.len().saturating_sub(1)
    }

    fn line_at(&self, offset: usize) -> usize {
        self.starts
            .partition_point(|start| *start <= offset)
            .saturating_sub(1)
    }

    fn annotation_lines(&self, range: &Range<usize>) -> Option<(usize, usize)> {
        self.text.get(range.clone())?;
        let first = self.line_at(range.start);
        let last = self.line_at(if range.is_empty() {
            range.end
        } else {
            range.end - 1
        });
        Some((first, last))
    }

    fn range(&self, first: usize, last: usize) -> Option<Range<usize>> {
        let start = *self.starts.get(first)?;
        let end = self
            .starts
            .get(last.checked_add(1)?)
            .copied()
            .unwrap_or(self.text.len());
        Some(start..end)
    }

    fn content_range(&self, first: usize, last: usize) -> Option<Range<usize>> {
        let mut range = self.range(first, last)?;
        if self.text.get(range.clone())?.ends_with('\n') {
            range.end -= 1;
            if self.text.get(range.clone())?.ends_with('\r') {
                range.end -= 1;
            }
        }
        Some(range)
    }

    fn visible_range(&self, mut range: Range<usize>) -> Range<usize> {
        if !range.is_empty()
            && self.starts.binary_search(&range.end).is_ok()
            && let Some(line) =
                self.content_range(self.line_at(range.end - 1), self.line_at(range.end - 1))
        {
            // An exclusive end at the next line's beginning does not include that line. The
            // renderer should underline the preceding line's visible text, not its successor.
            range.end = line.end.max(range.start);
        }
        range
    }
}

/// Additional control characters not normalized by `annotate-snippets` itself. Replacing each
/// with one visible scalar keeps character columns meaningful; byte ranges are remapped below.
fn extra_control(character: char) -> bool {
    matches!(
        character,
        '\u{0080}'..='\u{009f}' | '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}' | '\u{2029}'
    )
}

fn normalize_extra_controls(text: &str) -> Cow<'_, str> {
    if text.chars().any(extra_control) {
        Cow::Owned(
            text.chars()
                .map(|character| {
                    if extra_control(character) {
                        '\u{fffd}'
                    } else {
                        character
                    }
                })
                .collect(),
        )
    } else {
        Cow::Borrowed(text)
    }
}

fn normalize_single_line(text: &str) -> String {
    annotate_snippets::normalize_untrusted_str(&normalize_extra_controls(text)).replace('\n', "␊")
}

struct NormalizedSource<'a> {
    text: Cow<'a, str>,
    // Original and rendered byte offsets immediately after each replacement.
    offsets: Vec<(usize, usize)>,
}

impl<'a> NormalizedSource<'a> {
    fn new(text: &'a str) -> Self {
        let Some(first) = text
            .char_indices()
            .find_map(|(index, character)| extra_control(character).then_some(index))
        else {
            return Self {
                text: Cow::Borrowed(text),
                offsets: Vec::new(),
            };
        };
        let mut normalized = String::with_capacity(text.len());
        normalized.push_str(&text[..first]);
        let mut offsets = Vec::new();
        for (index, character) in text[first..].char_indices() {
            if extra_control(character) {
                normalized.push('\u{fffd}');
                offsets.push((first + index + character.len_utf8(), normalized.len()));
            } else {
                normalized.push(character);
            }
        }
        Self {
            text: Cow::Owned(normalized),
            offsets,
        }
    }

    fn offset(&self, offset: usize) -> usize {
        let index = self
            .offsets
            .partition_point(|(original, _)| *original <= offset);
        index
            .checked_sub(1)
            .and_then(|index| self.offsets.get(index))
            .map_or(offset, |(original, rendered)| {
                rendered + (offset - original)
            })
    }

    fn range(&self, range: Range<usize>) -> Range<usize> {
        self.offset(range.start)..self.offset(range.end)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::ops::Range;

    use insta::{assert_debug_snapshot, assert_snapshot};
    use owo_colors::AnsiColors;

    use super::{SourceAnnotation, SourceFile, SourceLevel, SourceSnippet, write_snippets};
    use crate::{Diagnostic, ErrorOptions, Hints, Info, write_error_chain_with_options};

    fn range_of(text: &str, value: &str) -> Range<usize> {
        let start = text.find(value).expect("value must occur in the fixture");
        start..start + value.len()
    }

    fn render(snippets: &[SourceSnippet<'_>], width: Option<usize>) -> String {
        let mut output = String::new();
        write_snippets(&mut output, snippets, width, SourceLevel::Error)
            .expect("writing to a String is infallible");
        anstream::adapter::strip_str(&output).to_string()
    }

    #[test]
    fn source_windows_match_rendered_line_boundaries() {
        let source = SourceFile::new("lines.txt", "é-secret\r\nok\rtail\n");
        let crlf = source.text().find("\r\n").expect("CRLF in fixture");
        let bare_cr = source.text().rfind('\r').expect("bare CR in fixture");
        let end = source.text().len();
        assert_debug_snapshot!(
            [
                source.lines_for_span(0..crlf),
                source.lines_for_span(crlf..crlf),
                source.lines_for_span(crlf + 1..crlf + 1),
                source.lines_for_span(0..crlf + 2),
                source.lines_for_span(crlf + 2..crlf + 2),
                source.lines_for_span(bare_cr..bare_cr),
                source.lines_for_span(end..end),
                source.lines_for_span(1..2),
                source.lines_for_span(Range { start: 2, end: 1 }),
                source.lines_for_span(usize::MAX..usize::MAX),
            ],
            @r#"
        [
            Some(
                "é-secret\r\n",
            ),
            Some(
                "é-secret\r\n",
            ),
            Some(
                "é-secret\r\n",
            ),
            Some(
                "é-secret\r\n",
            ),
            Some(
                "ok\rtail\n",
            ),
            Some(
                "ok\rtail\n",
            ),
            Some(
                "",
            ),
            None,
            None,
            None,
        ]
        "#
        );
    }

    #[test]
    fn source_snippets_follow_their_error_and_info() {
        #[derive(Debug, thiserror::Error)]
        #[error("Invalid project configuration")]
        struct ProjectError {
            source_file: SourceFile,
            related_file: SourceFile,
            #[source]
            source: ParseError,
        }

        #[derive(Debug, thiserror::Error)]
        #[error("Expected a dependency string")]
        struct ParseError;

        fn diagnostic<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
            let error = error.downcast_ref::<ProjectError>()?;
            Some(
                Diagnostic::default().with_source(
                    Diagnostic::default()
                        .with_snippet(
                            SourceSnippet::new(error.source_file.clone()).with_annotation(
                                SourceAnnotation::primary(range_of(error.source_file.text(), "42"))
                                    .with_label("expected a string"),
                            ),
                        )
                        .with_info(
                            Info::new("The dependency group is included here").with_snippet(
                                SourceSnippet::new(error.related_file.clone()).with_annotation(
                                    SourceAnnotation::secondary(range_of(
                                        error.related_file.text(),
                                        "include-group",
                                    ))
                                    .with_label("included by this group"),
                                ),
                            ),
                        ),
                ),
            )
        }

        let error = ProjectError {
            source_file: SourceFile::new("pyproject.toml", "[dependency-groups]\ndev = [42]\n"),
            related_file: SourceFile::new(
                "workspace/pyproject.toml",
                "test = [{ include-group = \"dev\" }]\n",
            ),
            source: ParseError,
        };
        let mut output = String::new();
        write_error_chain_with_options(
            &error,
            &Hints::from("Use a quoted dependency specifier"),
            ErrorOptions::default()
                .with_width_override(80)
                .with_diagnostic(diagnostic)
                .with_stream(&mut output),
        )
        .expect("writing to a String is infallible");
        assert_snapshot!(anstream::adapter::strip_str(&output), @r#"
        error: Invalid project configuration
          cause: Expected a dependency string
           --> pyproject.toml:2:8
            |
          2 | dev = [42]
            |        ^^ expected a string
          info: The dependency group is included here
           --> workspace/pyproject.toml:1:11
            |
          1 | test = [{ include-group = "dev" }]
            |           ------------- included by this group

        hint: Use a quoted dependency specifier
        "#);
    }

    #[test]
    fn source_annotations_are_limited_to_selected_lines() {
        let text = "index = \"https://user:password@example.com\"\n\
                    first = [42]\n\
                    token = \"not-for-diagnostics\"\n\
                    second = [false]\n\
                    trailing = \"not-for-diagnostics\"\n";
        let snippet = SourceSnippet::new(SourceFile::new("uv.toml", text))
            .with_annotation(SourceAnnotation::primary(range_of(text, "42")).with_label("a number"))
            .with_annotation(
                SourceAnnotation::secondary(range_of(text, "false")).with_label("a boolean"),
            );
        assert_snapshot!(render(&[snippet], Some(80)), @"
         --> uv.toml:2:10
          |
        2 | first = [42]
          |          ^^ a number
          |
         ::: uv.toml:4:11
          |
        4 | second = [false]
          |           ----- a boolean
        ");
    }

    #[test]
    fn source_context_is_explicit() {
        let text = "before\nleft = [42]\nbetween\nright = [false]\nafter\n";
        let snippet = SourceSnippet::new(SourceFile::new("uv.toml", text))
            .with_context_lines(1)
            .with_annotation(SourceAnnotation::primary(range_of(text, "42")))
            .with_annotation(SourceAnnotation::secondary(range_of(text, "false")));
        assert_snapshot!(render(&[snippet], Some(80)), @"
         --> uv.toml:2:9
          |
        1 | before
        2 | left = [42]
          |         ^^
        3 | between
        4 | right = [false]
          |          -----
        5 | after
          |
        ");
    }

    #[test]
    fn source_multiple_annotations_and_files() {
        let text = "first = [\"one\", \"two\"]\n";
        let related = "second = \"three\"\n";
        let snippets = [
            SourceSnippet::new(SourceFile::new("pyproject.toml", text))
                .with_annotation(
                    SourceAnnotation::primary(range_of(text, "one")).with_label("first value"),
                )
                .with_annotation(
                    SourceAnnotation::secondary(range_of(text, "two")).with_label("related value"),
                ),
            SourceSnippet::new(SourceFile::new("uv.toml", related)).with_annotation(
                SourceAnnotation::secondary(range_of(related, "three"))
                    .with_label("declared in another file"),
            ),
        ];
        assert_snapshot!(render(&snippets, Some(80)), @r#"
         --> pyproject.toml:1:11
          |
        1 | first = ["one", "two"]
          |           ^^^    --- related value
          |           |
          |           first value
          |
         ::: uv.toml:1:11
          |
        1 | second = "three"
          |           ----- declared in another file
        "#);
    }

    #[test]
    fn source_invalid_ranges_do_not_invent_locations() {
        let text = "café = true\n";
        let invalid = SourceSnippet::new(SourceFile::new("invalid.toml", text))
            .with_annotation(SourceAnnotation::primary(4..5))
            .with_annotation(SourceAnnotation::primary(Range { start: 8, end: 2 }))
            .with_annotation(SourceAnnotation::primary(0..usize::MAX));
        let valid = SourceSnippet::new(SourceFile::new("valid.toml", text))
            .with_annotation(SourceAnnotation::primary(4..5))
            .with_annotation(SourceAnnotation::secondary(range_of(text, "true")));
        let missing = SourceSnippet::new(SourceFile::new("location-only.toml", text));
        let invalid_line =
            SourceSnippet::new(SourceFile::new("invalid-line.toml", text).with_line_start(0))
                .with_annotation(SourceAnnotation::primary(0..0));
        let overflowing_line = SourceSnippet::new(
            SourceFile::new("overflowing-line.toml", text).with_line_start(usize::MAX),
        )
        .with_annotation(SourceAnnotation::primary(0..0));
        assert_snapshot!(
            render(&[invalid, valid, missing, invalid_line, overflowing_line], Some(80)),
            @"
         --> invalid.toml
          |
         ::: valid.toml:1:8
          |
        1 | café = true
          |        ----
         ::: location-only.toml
         ::: invalid-line.toml
         ::: overflowing-line.toml
        "
        );
    }

    #[test]
    fn source_locations_can_omit_sensitive_text() {
        let source = SourceFile::new(
            "uv.toml",
            "token = \"not-for-diagnostics\"\r\nπ = \"private\"\r\n",
        )
        .with_line_start(8);
        let primary = range_of(source.text(), "private");
        let snippets = [
            SourceSnippet::new(source.clone())
                .with_annotation(SourceAnnotation::secondary(0..5))
                .with_annotation(SourceAnnotation::primary(primary))
                .without_source_text(),
            SourceSnippet::new(source.clone())
                .with_annotation(SourceAnnotation::secondary(0..5))
                .without_source_text(),
            SourceSnippet::new(source)
                .with_annotation(SourceAnnotation::primary(0..usize::MAX))
                .without_source_text(),
        ];
        assert_snapshot!(render(&snippets, Some(80)), @"
        --> uv.toml:9:6
        ::: uv.toml:8:1
        ::: uv.toml
        ");
    }

    #[test]
    fn source_eof_and_excerpts() {
        let snippets = [
            SourceSnippet::new(SourceFile::new("empty.toml", ""))
                .with_annotation(SourceAnnotation::primary(0..0).with_label("expected a value")),
            SourceSnippet::new(SourceFile::new("no-newline.toml", "value ="))
                .with_annotation(SourceAnnotation::primary(7..7).with_label("expected a value")),
            SourceSnippet::new(SourceFile::new("newline.toml", "value =\n"))
                .with_annotation(SourceAnnotation::primary(8..8).with_label("expected a value")),
            SourceSnippet::new(SourceFile::new("crlf.toml", "value =\r\n"))
                .with_annotation(SourceAnnotation::primary(9..9).with_label("expected a value")),
            SourceSnippet::new(
                SourceFile::new("excerpt.toml", "value = false\r\n").with_line_start(42),
            )
            .with_annotation(SourceAnnotation::primary(8..13).with_label("expected a string")),
        ];
        assert_snapshot!(render(&snippets, Some(80)), @"
          --> empty.toml:1:1
           |
         1 |
           | ^ expected a value
           |
          ::: no-newline.toml:1:8
           |
         1 | value =
           |        ^ expected a value
           |
          ::: newline.toml:2:1
           |
         2 |
           | ^ expected a value
           |
          ::: crlf.toml:2:1
           |
         2 |
           | ^ expected a value
           |
          ::: excerpt.toml:42:9
           |
        42 | value = false
           |         ^^^^^ expected a string
        ");
    }

    #[test]
    fn source_newline_boundaries_and_final_empty_line() {
        let text = "one\r\ntwo\r\n";
        let snippet = SourceSnippet::new(SourceFile::new("lines.toml", text))
            .with_annotation(SourceAnnotation::primary(0..5).with_label("first line"))
            .with_annotation(SourceAnnotation::secondary(5..8).with_label("second line"))
            .with_annotation(SourceAnnotation::primary(10..10).with_label("end of file"));
        assert_snapshot!(render(&[snippet], Some(80)), @"
         --> lines.toml:1:1
          |
        1 | one
          | ^^^ first line
        2 | two
          | --- second line
          |
         ::: lines.toml:3:1
          |
        3 |
          | ^ end of file
        ");
    }

    #[test]
    fn source_multiline_unicode_and_crlf() {
        let text = "π = [\r\n    \"café\",\r\n    \"🦀\",\r\n]\r\n";
        let first = range_of(text, "\"café\"");
        let last = range_of(text, "\"🦀\"");
        let snippet = SourceSnippet::new(SourceFile::new("unicode.toml", text))
            .with_annotation(
                SourceAnnotation::primary(first.start..last.end)
                    .with_label("these values conflict"),
            )
            .with_annotation(
                SourceAnnotation::secondary(range_of(text, "café"))
                    .with_label("first declared here"),
            );
        assert_snapshot!(render(&[snippet], Some(80)), @r#"
         --> unicode.toml:2:5
          |
        2 |       "café",
          |       ^---- first declared here
          |  _____|
          | |
        3 | |     "🦀",
          | |________^ these values conflict
        "#);
    }

    #[test]
    fn source_terminal_controls_are_visible_and_ranges_stay_aligned() {
        let text = "prefix\u{009b}\u{202e}\u{001b}[31m\t🦀target\n";
        let snippet = SourceSnippet::new(SourceFile::new("bad\u{009b}\n\u{001b}[31m.toml", text))
            .with_annotation(
                SourceAnnotation::primary(range_of(text, "target"))
                    .with_label("bad\u{001b}[31m\nlabel\u{009b}"),
            );
        assert_snapshot!(render(&[snippet], Some(80)), @"
         --> bad�␊␛[31m.toml:1:16
          |
        1 | prefix��␛[31m    🦀target
          |                    ^^^^^^ bad␛[31m␊label�
        ");
    }

    #[test]
    fn source_narrow_width_and_no_wrap() {
        let text = "dependencies = [\"first-package-with-a-long-name\", \"other-package-with-a-long-name\"]\n";
        let snippet = SourceSnippet::new(SourceFile::new("pyproject.toml", text)).with_annotation(
            SourceAnnotation::primary(range_of(text, "other-package-with-a-long-name"))
                .with_label("this dependency is not available"),
        );
        assert_snapshot!(render(std::slice::from_ref(&snippet), Some(32)), @r#"
         --> pyproject.toml:1:52
          |
        1 | ..., "other-package-with-a-long-name"]
          |       ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ this dependency is not available
        "#);
        assert_snapshot!(render(std::slice::from_ref(&snippet), Some(0)), @r#"
         --> pyproject.toml:1:52
          |
        1 | ..., "other...-name"]
          |       ^^^^^...^^^^^ this dependency is not available
        "#);
        // `COLUMNS` can contain any `usize`, including values that overflow renderer arithmetic.
        let _ = render(std::slice::from_ref(&snippet), Some(usize::MAX));
        assert_snapshot!(render(&[snippet], None), @r#"
         --> pyproject.toml:1:52
          |
        1 | dependencies = ["first-package-with-a-long-name", "other-package-with-a-long-name"]
          |                                                    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ this dependency is not available
        "#);
    }

    #[test]
    fn source_styles_follow_diagnostic_severity() {
        #[derive(Debug, thiserror::Error)]
        #[error("Invalid value")]
        struct SourceWarning;

        let snippet = SourceSnippet::new(SourceFile::new("uv.toml", "value = 42"))
            .with_annotation(SourceAnnotation::primary(8..10).with_label("expected a string"));
        let mut warning = String::new();
        write_snippets(
            &mut warning,
            std::slice::from_ref(&snippet),
            Some(80),
            SourceLevel::Warning,
        )
        .expect("writing to a String is infallible");
        let mut info = String::new();
        write_snippets(&mut info, &[snippet], Some(80), SourceLevel::Info)
            .expect("writing to a String is infallible");
        assert_debug_snapshot!(warning, @r#""   \u{1b}[1m\u{1b}[36m--> \u{1b}[0muv.toml:1:9\n    \u{1b}[1m\u{1b}[36m|\u{1b}[0m\n  \u{1b}[1m\u{1b}[36m1\u{1b}[0m \u{1b}[1m\u{1b}[36m|\u{1b}[0m value = 42\n    \u{1b}[1m\u{1b}[36m|\u{1b}[0m         \u{1b}[1m\u{1b}[33m^^\u{1b}[0m \u{1b}[1m\u{1b}[33mexpected a string\u{1b}[0m\n""#);
        assert_debug_snapshot!(info, @r#""   \u{1b}[1m\u{1b}[36m--> \u{1b}[0muv.toml:1:9\n    \u{1b}[1m\u{1b}[36m|\u{1b}[0m\n  \u{1b}[1m\u{1b}[36m1\u{1b}[0m \u{1b}[1m\u{1b}[36m|\u{1b}[0m value = 42\n    \u{1b}[1m\u{1b}[36m|\u{1b}[0m         \u{1b}[1m\u{1b}[36m^^\u{1b}[0m \u{1b}[1m\u{1b}[36mexpected a string\u{1b}[0m\n""#);

        let mut output = String::new();
        write_error_chain_with_options(
            &SourceWarning,
            &Hints::none(),
            ErrorOptions::default()
                .with_level("warning")
                .with_color(AnsiColors::Yellow)
                .with_width_override(80)
                .with_diagnostic(|_| {
                    Some(
                        Diagnostic::default().with_snippet(
                            SourceSnippet::new(SourceFile::new("uv.toml", "value = 42"))
                                .with_annotation(SourceAnnotation::primary(8..10)),
                        ),
                    )
                })
                .with_stream(&mut output),
        )
        .expect("writing to a String is infallible");
        assert_snapshot!(anstream::adapter::strip_str(&output), @"
        warning: Invalid value
           --> uv.toml:1:9
            |
          1 | value = 42
            |         ^^
        ");
    }

    #[test]
    fn source_debug_does_not_expose_contents() {
        let source = SourceFile::new("uv.toml", "token = \"not-for-diagnostics\"\n");
        assert_debug_snapshot!(source, @r#"
        SourceFile {
            name: "uv.toml",
            len: 30,
            line_start: 1,
        }
        "#);
    }

    #[test]
    fn source_all_small_ranges_are_safe() {
        for text in ["", "x\n", "x\r\ny", "é🦀\u{009b}\t\r\nz"] {
            for start in 0..=text.len() + 1 {
                for end in 0..=text.len() + 1 {
                    let snippet = SourceSnippet::new(SourceFile::new("input.toml", text))
                        .with_annotation(SourceAnnotation::primary(Range { start, end }));
                    let output = render(&[snippet], Some(0));
                    assert!(
                        !output.contains('\u{009b}'),
                        "raw C1 control in source range {start}..{end}"
                    );
                }
            }
        }
    }
}
