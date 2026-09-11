use std::borrow::Cow;
use std::fmt::{self, Write};
use std::ops::Range;
use std::sync::Arc;

use annotate_snippets::renderer::{AnsiColor, Effects};
use annotate_snippets::{AnnotationKind, Element, Group, Level, Origin, Patch, Renderer, Snippet};

use crate::SourceSuggestion;

/// Immutable source text retained when an error is produced.
///
/// Ranges refer to the bytes in [`Self::text`], after any decoding or redaction performed by the
/// producer. The renderer never reopens the file. The name should be safe to display to the user.
#[derive(Clone)]
pub struct SourceFile {
    name: Arc<str>,
    text: Arc<str>,
    /// All clones share the same immutable text and line boundaries.
    line_starts: Arc<[usize]>,
    line_start: usize,
}

impl SourceFile {
    /// Retain the display name and complete decoded contents of a source file.
    pub fn new(name: impl Into<Arc<str>>, text: impl Into<Arc<str>>) -> Self {
        let text: Arc<str> = text.into();
        let line_starts = source_line_starts(&text).into();
        Self {
            name: name.into(),
            text,
            line_starts,
            line_start: 1,
        }
    }

    /// Set the known, one-based starting line of an excerpt.
    ///
    /// The excerpt must start at a line boundary, and annotation ranges must be relative to the
    /// excerpt. An invalid line number causes the renderer to show only the source name.
    #[must_use]
    pub fn with_line_start(mut self, line_start: usize) -> Self {
        self.line_start = line_start;
        self
    }

    /// The user-facing source name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The exact decoded text used to calculate annotation ranges.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The one-based starting line of this source snapshot in its original input.
    pub fn line_start(&self) -> usize {
        self.line_start
    }

    /// The complete source lines that a zero-context annotation would display for this byte span.
    ///
    /// This uses the renderer's LF-delimited line boundaries, including any CR bytes. An invalid
    /// UTF-8 range returns `None`.
    #[cfg(test)]
    fn lines_for_span(&self, span: Range<usize>) -> Option<&str> {
        self.text().get(self.line_range_for_span(span)?)
    }

    /// The byte range of the complete lines returned by [`Self::lines_for_span`].
    #[cfg(test)]
    fn line_range_for_span(&self, span: Range<usize>) -> Option<Range<usize>> {
        let lines = self.lines();
        let (first, last) = lines.annotation_lines(&span)?;
        lines.range(first, last)
    }

    /// Reuse this retained snapshot's line index when resolving source positions.
    pub(crate) fn position_index(&self) -> SourcePositionIndex<'_> {
        SourcePositionIndex {
            lines: self.lines(),
            line_start: self.line_start(),
        }
    }

    fn lines(&self) -> SourceLines<'_> {
        SourceLines {
            text: self.text(),
            starts: Cow::Borrowed(&self.line_starts),
        }
    }
}

impl fmt::Debug for SourceFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Configuration files can contain credentials, so verbose error logging must not dump
        // their complete retained contents.
        let Self {
            name,
            text,
            line_starts: _,
            line_start,
        } = self;
        formatter
            .debug_struct("SourceFile")
            .field("name", name)
            .field("len", &text.len())
            .field("line_start", line_start)
            .finish()
    }
}

/// An annotation on a byte range in a [`SourceFile`].
#[derive(Clone, Debug)]
pub struct SourceAnnotation<'a> {
    range: Range<usize>,
    label: Option<Cow<'a, str>>,
    kind: AnnotationKind,
}

impl<'a> SourceAnnotation<'a> {
    /// Identify the source text responsible for the diagnostic.
    pub fn primary(range: Range<usize>) -> Self {
        Self {
            range,
            label: None,
            kind: AnnotationKind::Primary,
        }
    }

    /// Identify related source text that helps explain the diagnostic.
    pub fn secondary(range: Range<usize>) -> Self {
        Self {
            range,
            label: None,
            kind: AnnotationKind::Context,
        }
    }

    /// Describe why the source text is highlighted.
    #[must_use]
    pub fn with_label(mut self, label: impl Into<Cow<'a, str>>) -> Self {
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
pub struct SourceSnippet<'a> {
    source: SourceFile,
    annotations: Vec<SourceAnnotation<'a>>,
    context_lines: usize,
    show_source: bool,
}

impl<'a> SourceSnippet<'a> {
    /// Refer to a source without inventing a location within it.
    pub fn new(source: SourceFile) -> Self {
        Self {
            source,
            annotations: Vec::new(),
            context_lines: 0,
            show_source: true,
        }
    }

    /// Add a primary or secondary source annotation.
    #[must_use]
    pub fn with_annotation(mut self, annotation: SourceAnnotation<'a>) -> Self {
        self.annotations.push(annotation);
        self
    }

    /// Opt in to showing this many surrounding lines for each annotation.
    #[must_use]
    pub fn with_context_lines(mut self, context_lines: usize) -> Self {
        self.context_lines = context_lines;
        self
    }

    /// Show a location without exposing the source text.
    ///
    /// The first valid primary annotation determines the location, or the first valid annotation
    /// when there is no primary annotation. Invalid ranges still fall back to the source name.
    #[must_use]
    pub fn without_source_text(mut self) -> Self {
        self.show_source = false;
        self
    }
}

pub(crate) enum SourceLevel {
    Error,
    Warning,
    Info,
    Hint,
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
            Self::Hint => Level::HELP,
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
    write_group(stream, group, width)
}

/// Render an explicitly authorized edit preview, or just its source locations.
pub(crate) fn write_suggestion(
    stream: &mut impl Write,
    suggestion: &SourceSuggestion,
    width: Option<usize>,
) -> fmt::Result {
    if !suggestion.show_source() || !suggestion.preview_line_numbers_fit() {
        let source = suggestion.source();
        let name = normalize_single_line(source.name());
        if name.is_empty() {
            return Ok(());
        }
        let positions = source.position_index();
        let locations = suggestion.edits().iter().filter_map(|edit| {
            let position = positions.position(edit.range.start)?;
            Some(
                Origin::path(name.clone())
                    .line(position.line)
                    .char_column(position.character_column + 1),
            )
        });
        return write_group(
            stream,
            Group::with_level(SourceLevel::Hint.level()).elements(locations),
            width,
        );
    }

    let snippet = suggestion.snippet();
    let Some(view) = source_view(&snippet) else {
        return Ok(());
    };
    let SourceViewKind::Windows(windows) = view.kind else {
        return write_snippets(stream, &[snippet], width, SourceLevel::Hint);
    };

    let mut group = Group::with_level(SourceLevel::Hint.level());
    for window in windows {
        let normalized = NormalizedSource::new(window.text);
        let patches = window
            .annotations
            .into_iter()
            .filter_map(|annotation| {
                let edit = suggestion.edits().get(annotation.index)?;
                Some(Patch::new(
                    normalized.range(annotation.range),
                    normalize_extra_controls(&edit.replacement),
                ))
            })
            .collect::<Vec<_>>();
        if patches.is_empty() {
            continue;
        }
        let mut rendered = Snippet::source(normalized.text)
            .line_start(window.line_start)
            .patches(patches);
        if let Some(name) = &view.name {
            rendered = rendered.path(name.clone());
        }
        group = group.element(rendered);
    }
    write_group(stream, group, width)
}

fn write_group(stream: &mut impl Write, group: Group<'_>, width: Option<usize>) -> fmt::Result {
    if group.is_empty() {
        return Ok(());
    }

    let context_style = AnsiColor::Cyan.on_default().effects(Effects::BOLD);
    let renderer = Renderer::styled()
        .error(AnsiColor::Red.on_default().effects(Effects::BOLD))
        .warning(AnsiColor::Yellow.on_default().effects(Effects::BOLD))
        .info(context_style)
        .help(context_style)
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

fn source_elements<'a>(snippet: &'a SourceSnippet<'_>) -> Vec<Element<'a>> {
    let Some(view) = source_view(snippet) else {
        return Vec::new();
    };
    let origin = || {
        view.name
            .as_ref()
            .map(|name| Origin::path(name.clone()).into())
            .into_iter()
            .collect()
    };
    let windows = match view.kind {
        SourceViewKind::Origin => return origin(),
        SourceViewKind::Location(position) => {
            return view
                .name
                .as_ref()
                .map(|name| {
                    Origin::path(name.clone())
                        .line(position.line)
                        .char_column(position.character_column + 1)
                        .into()
                })
                .into_iter()
                .collect();
        }
        SourceViewKind::Windows(windows) => windows,
    };

    let mut elements = Vec::new();
    for window in windows {
        let normalized = NormalizedSource::new(window.text);
        let mut annotations = Vec::new();
        for annotation in window.annotations {
            let mut rendered = annotation
                .kind
                .span(normalized.range(annotation.display_range));
            if let Some(label) = annotation.label {
                rendered = rendered.label(normalize_single_line(label));
            }
            annotations.push(rendered);
            annotations.extend(
                annotation
                    .visible_context
                    .into_iter()
                    .map(|range| AnnotationKind::Visible.span(normalized.range(range))),
            );
        }
        let mut rendered = Snippet::source(normalized.text)
            .line_start(window.line_start)
            .annotations(annotations);
        if let Some(name) = &view.name {
            rendered = rendered.path(name.clone());
        }
        elements.push(rendered.into());
    }
    elements
}

/// The explicitly selected, displayable part of one source snapshot.
pub(crate) struct SourceView<'a> {
    pub(crate) name: Option<String>,
    pub(crate) kind: SourceViewKind<'a>,
}

pub(crate) enum SourceViewKind<'a> {
    Origin,
    Location(SourcePosition),
    Windows(Vec<SourceWindowView<'a>>),
}

/// A position in the decoded input, with a one-based line and zero-based columns.
pub(crate) struct SourcePosition {
    pub(crate) line: usize,
    pub(crate) byte_column: usize,
    pub(crate) character_column: usize,
}

pub(crate) struct SourceWindowView<'a> {
    pub(crate) text: &'a str,
    pub(crate) line_start: usize,
    pub(crate) annotations: Vec<SourceAnnotationView<'a>>,
}

impl SourceWindowView<'_> {
    /// Index this selected window once when resolving multiple annotation positions.
    pub(crate) fn position_index(&self) -> SourcePositionIndex<'_> {
        SourcePositionIndex {
            lines: SourceLines::new(self.text),
            line_start: self.line_start,
        }
    }
}

pub(crate) struct SourcePositionIndex<'a> {
    lines: SourceLines<'a>,
    line_start: usize,
}

impl SourcePositionIndex<'_> {
    /// Resolve a byte offset relative to the selected window.
    pub(crate) fn position(&self, offset: usize) -> Option<SourcePosition> {
        self.lines.position(offset, self.line_start)
    }
}

pub(crate) struct SourceAnnotationView<'a> {
    /// The occurrence in the original snippet's annotation list.
    index: usize,
    /// The original, half-open byte range relative to the selected window.
    pub(crate) range: Range<usize>,
    display_range: Range<usize>,
    pub(crate) label: Option<&'a str>,
    pub(crate) kind: AnnotationKind,
    visible_context: Vec<Range<usize>>,
}

/// Select explicit source windows before handing source text to any renderer. In particular,
/// unrelated configuration lines must not become visible merely because annotations are nearby.
pub(crate) fn source_view<'a>(snippet: &'a SourceSnippet<'_>) -> Option<SourceView<'a>> {
    let source = &snippet.source;
    let positions = source.position_index();
    let lines = &positions.lines;
    let name = normalize_single_line(source.name());
    let name = (!name.is_empty()).then_some(name);
    let origin = |name: Option<String>| {
        name.map(|name| SourceView {
            name: Some(name),
            kind: SourceViewKind::Origin,
        })
    };
    if source.line_start == 0 || source.line_start.checked_add(lines.last()).is_none() {
        return origin(name);
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
        let Some(position) =
            annotation.and_then(|annotation| positions.position(annotation.range.start))
        else {
            return origin(name);
        };
        return name.map(|name| SourceView {
            name: Some(name),
            kind: SourceViewKind::Location(position),
        });
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
        return origin(name);
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

    let mut windows = Vec::new();
    for mut window in merged {
        let Some(range) = lines.range(window.first, window.last) else {
            continue;
        };
        let Some(text) = source.text().get(range.clone()) else {
            continue;
        };
        let mut annotations = Vec::new();
        window.annotations.sort_unstable();
        for index in window.annotations {
            let annotation = &snippet.annotations[index];
            let visible_range = lines.visible_range(annotation.range.clone());
            let mut visible_context = Vec::new();
            if snippet.context_lines > 0
                && !window.trailing_eof
                && let Some((first, last)) = lines.annotation_lines(&annotation.range)
            {
                if first > window.first
                    && let Some(context) = lines.content_range(window.first, first - 1)
                {
                    visible_context.push(context.start - range.start..context.end - range.start);
                }
                if last < window.last
                    && let Some(context) = lines.content_range(last + 1, window.last)
                {
                    visible_context.push(context.start - range.start..context.end - range.start);
                }
            }
            annotations.push(SourceAnnotationView {
                index,
                range: annotation.range.start - range.start..annotation.range.end - range.start,
                display_range: visible_range.start - range.start..visible_range.end - range.start,
                label: annotation.label.as_deref(),
                kind: annotation.kind,
                visible_context,
            });
        }
        windows.push(SourceWindowView {
            text,
            line_start: source.line_start + window.first,
            annotations,
        });
    }
    if windows.is_empty() {
        origin(name)
    } else {
        Some(SourceView {
            name,
            kind: SourceViewKind::Windows(windows),
        })
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
    starts: Cow<'a, [usize]>,
}

impl<'a> SourceLines<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            starts: Cow::Owned(source_line_starts(text)),
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

    fn position(&self, offset: usize, line_start: usize) -> Option<SourcePosition> {
        self.text.get(..offset)?;
        let line = self.line_at(offset);
        let start = *self.starts.get(line)?;
        let prefix = self.text.get(start..offset)?;
        Some(SourcePosition {
            line: line_start.checked_add(line)?,
            byte_column: offset - start,
            character_column: prefix.chars().count(),
        })
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

fn source_line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(index, _)| index + 1))
        .collect()
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
    use std::borrow::Cow;
    use std::error::Error;
    use std::ops::Range;
    use std::sync::Arc;

    use insta::{assert_debug_snapshot, assert_snapshot};
    use owo_colors::AnsiColors;

    use super::{
        SourceAnnotation, SourceFile, SourceLevel, SourcePosition, SourceSnippet, SourceWindowView,
        write_snippets,
    };
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

    fn coordinates(position: Option<SourcePosition>) -> Option<(usize, usize, usize)> {
        position.map(|position| {
            (
                position.line,
                position.byte_column,
                position.character_column,
            )
        })
    }

    #[test]
    fn source_clones_share_an_immutable_line_index() {
        let source = SourceFile::new("lines.txt", "first\r\né🦀\nlast");
        let excerpt = source.clone().with_line_start(41);
        assert!(Arc::ptr_eq(&source.line_starts, &excerpt.line_starts));

        assert_eq!(excerpt.line_range_for_span(7..13), Some(7..14));
        let starts = source.line_starts.as_ref();
        assert_eq!(starts, &[0, 7, 14]);

        let source_positions = source.position_index();
        let excerpt_positions = excerpt.position_index();
        assert!(matches!(&source_positions.lines.starts, Cow::Borrowed(_)));
        assert!(matches!(&excerpt_positions.lines.starts, Cow::Borrowed(_)));
        assert!(std::ptr::eq(starts, source_positions.lines.starts.as_ref()));
        assert!(std::ptr::eq(
            starts,
            excerpt_positions.lines.starts.as_ref()
        ));
        assert_debug_snapshot!(
            (
                coordinates(source_positions.position(13)),
                coordinates(excerpt_positions.position(13)),
                excerpt.lines_for_span(7..13),
            ),
            @r#"
        (
            Some(
                (
                    2,
                    6,
                    2,
                ),
            ),
            Some(
                (
                    42,
                    6,
                    2,
                ),
            ),
            Some(
                "é🦀\n",
            ),
        )
        "#
        );
    }

    #[test]
    fn source_position_indexes_preserve_excerpt_coordinates() {
        let text = "é\r\nβ🦀\rtail\n";
        let source = SourceFile::new("excerpt.txt", text).with_line_start(10);
        let positions = source.position_index();
        let window = SourceWindowView {
            text: &text[4..],
            line_start: 11,
            annotations: Vec::new(),
        };
        let window_positions = window.position_index();
        for offset in 0..=window.text.len() + 1 {
            assert_eq!(
                coordinates(window_positions.position(offset)),
                coordinates(positions.position(offset + 4)),
                "window offset {offset}"
            );
        }

        assert_debug_snapshot!(
            [0, 1, 2, 3, 4, 6, 10, 11, 15, 16, 17]
                .map(|offset| coordinates(positions.position(offset))),
            @"
        [
            Some(
                (
                    10,
                    0,
                    0,
                ),
            ),
            None,
            Some(
                (
                    10,
                    2,
                    1,
                ),
            ),
            Some(
                (
                    10,
                    3,
                    2,
                ),
            ),
            Some(
                (
                    11,
                    0,
                    0,
                ),
            ),
            Some(
                (
                    11,
                    2,
                    1,
                ),
            ),
            Some(
                (
                    11,
                    6,
                    2,
                ),
            ),
            Some(
                (
                    11,
                    7,
                    3,
                ),
            ),
            Some(
                (
                    11,
                    11,
                    7,
                ),
            ),
            Some(
                (
                    12,
                    0,
                    0,
                ),
            ),
            None,
        ]
        "
        );
        assert!(
            source
                .clone()
                .with_line_start(usize::MAX)
                .position_index()
                .position(4)
                .is_none()
        );
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
        assert_debug_snapshot!(
            [
                source.line_range_for_span(crlf + 1..crlf + 1),
                source.line_range_for_span(bare_cr..bare_cr),
                source.line_range_for_span(end..end),
                source.line_range_for_span(1..2),
            ],
            @"
        [
            Some(
                0..11,
            ),
            Some(
                11..19,
            ),
            Some(
                19..19,
            ),
            None,
        ]
        "
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
        assert_eq!(source.line_range_for_span(0..0), Some(0..30));
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
