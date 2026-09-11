use std::error::Error;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use uv_errors::{Diagnostic, Info, SourceAnnotation, SourceFile, SourceSnippet};
use uv_fs::Simplified;
use uv_pep508::{Pep508Error, Pep508ErrorSource, split_scheme};
use uv_pypi_types::VerbatimParsedUrl;
use uv_redacted::DisplaySafeUrl;

use crate::{MakeEditableError, RequirementsTxtFileError, RequirementsTxtParserError};

/// Resolve user-facing source locations for a requirements-file error in a source chain.
pub fn diagnostic_for_error<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
    error
        .downcast_ref::<RequirementsTxtFileError>()
        .or_else(|| {
            error
                .downcast_ref::<Box<RequirementsTxtFileError>>()
                .map(AsRef::as_ref)
        })?
        .diagnostic()
}

/// Retain the decoded text used by the parser, rather than reading the file again during rendering.
pub(super) fn source_file(path: &Path, text: impl Into<Arc<str>>) -> SourceFile {
    let name = if path == Path::new("-") {
        "<stdin>".to_string()
    } else if path.starts_with("http://") || path.starts_with("https://") {
        DisplaySafeUrl::parse(&path.to_string_lossy()).map_or_else(
            |_| "<remote requirements>".to_string(),
            |url| url.to_string(),
        )
    } else {
        path.portable_display().to_string()
    };
    SourceFile::new(name, text)
}

impl RequirementsTxtFileError {
    fn diagnostic(&self) -> Option<Diagnostic<'_>> {
        let source_file = self.source_file.as_ref()?;

        let diagnostic = match self.error.as_ref() {
            RequirementsTxtParserError::NonEditable {
                source: MakeEditableError::Registry,
                start,
                end,
                ..
            } => Diagnostic::new("Unsupported editable requirement").with_snippet(snippet(
                source_file,
                *start..*end,
                "not editable",
            )),
            RequirementsTxtParserError::NoBinary { start, end, .. } => Diagnostic::new(
                "Invalid specifier for `--no-binary`",
            )
            .with_snippet(snippet(source_file, *start..*end, "invalid package name")),
            RequirementsTxtParserError::OnlyBinary { start, end, .. } => Diagnostic::new(
                "Invalid specifier for `--only-binary`",
            )
            .with_snippet(snippet(source_file, *start..*end, "invalid package name")),
            RequirementsTxtParserError::UnnamedConstraint { start, end } => {
                Diagnostic::new("Unnamed requirements are not allowed as constraints").with_info(
                    Info::new("The constraints file was included here").with_snippet(
                        secondary_snippet(source_file, *start..*end, "constraints included here"),
                    ),
                )
            }
            RequirementsTxtParserError::Parser {
                message,
                line,
                column,
            } => Diagnostic::new(message.as_str()).with_snippet(snippet(
                source_file,
                point_span(source_file.text(), *line, *column)?,
                "invalid requirements syntax",
            )),
            RequirementsTxtParserError::UnsupportedRequirement { source, start, end } => {
                Diagnostic::new("Unsupported requirement").with_source(pep508_diagnostic(
                    source_file,
                    source,
                    *start..*end,
                    "unsupported requirement",
                )?)
            }
            RequirementsTxtParserError::Pep508 { source, start, end } => {
                Diagnostic::new("Couldn't parse requirement").with_source(pep508_diagnostic(
                    source_file,
                    source,
                    *start..*end,
                    "invalid requirement",
                )?)
            }
            RequirementsTxtParserError::ParsedUrl { source, start, end } => {
                Diagnostic::new("Couldn't parse URL").with_source(pep508_diagnostic(
                    source_file,
                    source,
                    *start..*end,
                    "invalid URL",
                )?)
            }
            RequirementsTxtParserError::Subfile { start, end, .. } => {
                Diagnostic::new("Failed to parse included requirements file").with_info(
                    Info::new("The file was included here").with_snippet(secondary_snippet(
                        source_file,
                        *start..*end,
                        "included here",
                    )),
                )
            }
            // These variants already use typed URL presentation to redact credentials. A raw
            // source excerpt must not turn a redacted URL back into its original spelling.
            RequirementsTxtParserError::Url { .. }
            | RequirementsTxtParserError::FileUrl { .. }
            | RequirementsTxtParserError::VerbatimUrl { .. }
            | RequirementsTxtParserError::NonEditable {
                source: MakeEditableError::Url(_),
                ..
            }
            | RequirementsTxtParserError::Io(_)
            | RequirementsTxtParserError::UrlConversion(_)
            | RequirementsTxtParserError::UnsupportedUrl(_)
            | RequirementsTxtParserError::MissingRequirementPrefix(_)
            | RequirementsTxtParserError::NonUnicodeUrl { .. } => return None,
            #[cfg(feature = "http")]
            RequirementsTxtParserError::Reqwest(..)
            | RequirementsTxtParserError::ClientBuild(..)
            | RequirementsTxtParserError::InvalidUrl(..) => return None,
        };
        Some(diagnostic)
    }
}

fn snippet(
    source_file: &SourceFile,
    range: Range<usize>,
    label: &'static str,
) -> SourceSnippet<'static> {
    let show_source = can_show_source(source_file.text(), &range);
    let snippet = SourceSnippet::new(source_file.clone())
        .with_annotation(SourceAnnotation::primary(range).with_label(label));
    if show_source {
        snippet
    } else {
        snippet.without_source_text()
    }
}

fn secondary_snippet(
    source_file: &SourceFile,
    range: Range<usize>,
    label: &'static str,
) -> SourceSnippet<'static> {
    let show_source = can_show_source(source_file.text(), &range);
    let snippet = SourceSnippet::new(source_file.clone())
        .with_annotation(SourceAnnotation::secondary(range).with_label(label));
    if show_source {
        snippet
    } else {
        snippet.without_source_text()
    }
}

/// Source locations are derived from parser spans, never from the text of a message. This separate
/// privacy guard conservatively omits source lines that might expose URL credentials or signatures.
fn can_show_source(text: &str, range: &Range<usize>) -> bool {
    let Some(lines) = source_lines(text, range) else {
        return false;
    };
    !may_contain_url(lines)
}

fn may_contain_url(text: &str) -> bool {
    text.contains('@')
        || text.contains("://")
        || text
            .split_whitespace()
            .any(|word| split_scheme(word).is_some())
}

/// Return only the physical lines touched by an already-known byte span.
fn source_lines<'a>(text: &'a str, range: &Range<usize>) -> Option<&'a str> {
    text.get(range.clone())?;
    let start = text[..range.start]
        .rfind(['\r', '\n'])
        .map_or(0, |offset| offset + 1);
    let last = if range.is_empty() {
        range.end
    } else {
        text[..range.end].char_indices().next_back()?.0
    };
    let end = text[last..]
        .find(['\r', '\n'])
        .map_or(text.len(), |offset| last + offset);
    text.get(start..end)
}

/// Override only the presentation of the PEP 508 source. Its own `Display` includes an underline
/// relative to the requirement substring, which would duplicate the actual file location.
fn pep508_diagnostic(
    source_file: &SourceFile,
    error: &Pep508Error<VerbatimParsedUrl>,
    statement: Range<usize>,
    label: &'static str,
) -> Option<Diagnostic<'static>> {
    if matches!(&error.message, Pep508ErrorSource::UrlError(_))
        || may_contain_url(&error.input)
        || !can_show_source(source_file.text(), &statement)
    {
        return None;
    }
    let range =
        mapped_pep508_span(source_file.text(), error, statement.clone()).unwrap_or(statement);
    Some(
        Diagnostic::new(error.message.to_string()).with_snippet(snippet(source_file, range, label)),
    )
}

/// A leaf span is relative to the exact input passed to PEP 508 parsing. A transformed input has
/// no direct mapping, so the caller falls back to the containing requirements-file statement.
fn mapped_pep508_span(
    text: &str,
    error: &Pep508Error<VerbatimParsedUrl>,
    statement: Range<usize>,
) -> Option<Range<usize>> {
    let input = text.get(statement.clone())?;
    if input != error.input {
        return None;
    }

    let start = statement.start.checked_add(error.start)?;
    let end = if error.start == input.len() && error.len <= 1 {
        // The PEP 508 parser allows a one-character underline just beyond its input.
        start
    } else {
        start.checked_add(error.len)?
    };
    if start > statement.end || end > statement.end || text.get(start..end).is_none() {
        return None;
    }
    Some(start..end)
}

/// Translate the parser's one-based Unicode-codepoint coordinates back into decoded UTF-8 bytes.
fn point_span(text: &str, line: usize, column: usize) -> Option<Range<usize>> {
    let mut current_line = 1;
    let mut current_column = 1;
    let mut characters = text.char_indices().peekable();
    while let Some((offset, character)) = characters.next() {
        if (current_line, current_column) == (line, column) {
            let end = match character {
                '\r' | '\n' => offset,
                character => offset + character.len_utf8(),
            };
            return Some(offset..end);
        }
        match character {
            '\r' => {
                if characters.peek().is_some_and(|(_, next)| *next == '\n') {
                    characters.next();
                }
                current_line += 1;
                current_column = 1;
            }
            '\n' => {
                current_line += 1;
                current_column = 1;
            }
            _ => current_column += 1,
        }
    }
    ((current_line, current_column) == (line, column)).then_some(text.len()..text.len())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use anyhow::{Context, Result};
    use uv_pep508::{Pep508Error, Pep508ErrorSource};

    use crate::RequirementsTxtRequirement;

    use super::{can_show_source, mapped_pep508_span, point_span};

    #[test]
    fn pep508_span_uses_original_decoded_bytes() -> Result<()> {
        let requirement = "flask==1.0.x";
        let prefix = "# café\r\n";
        let text = format!("{prefix}{requirement} \\\r\n    --hash=sha256:abc\r\n");
        let error = RequirementsTxtRequirement::parse(requirement, Path::new("."), false)
            .err()
            .context("the requirement should be invalid")?;
        let span = prefix.len()..prefix.len() + requirement.len();

        insta::assert_debug_snapshot!(mapped_pep508_span(&text, &error, span), @"
        Some(
            14..21,
        )
        ");
        Ok(())
    }

    #[test]
    fn pep508_span_rejects_unmapped_or_invalid_input() {
        let error = |input: &str, start, len| Pep508Error {
            message: Pep508ErrorSource::String("invalid requirement".to_string()),
            input: input.to_string(),
            start,
            len,
        };
        let text = "# café\nflask";
        let start = "# café\n".len();
        let span = start..text.len();

        insta::assert_debug_snapshot!((
            mapped_pep508_span(text, &error("different input", 0, 1), span.clone()),
            mapped_pep508_span(text, &error("flask", usize::MAX, 1), span.clone()),
            mapped_pep508_span(text, &error("flask", 1, usize::MAX), span.clone()),
            mapped_pep508_span(text, &error("flask", 5, 1), span),
            mapped_pep508_span("é", &error("é", 1, 1), 0..2),
        ), @"
        (
            None,
            None,
            None,
            Some(
                13..13,
            ),
            None,
        )
        ");
    }

    #[test]
    fn point_span_uses_unicode_codepoints_and_crlf() {
        let text = "é\r\nflask\r\n";
        insta::assert_debug_snapshot!((
            point_span(text, 1, 1),
            point_span(text, 1, 2),
            point_span(text, 2, 1),
            point_span(text, 3, 1),
            point_span(text, 4, 1),
        ), @"
        (
            Some(
                0..2,
            ),
            Some(
                2..2,
            ),
            Some(
                4..5,
            ),
            Some(
                11..11,
            ),
            None,
        )
        ");
    }

    #[test]
    fn source_filter_uses_only_annotated_lines() {
        let prefix = "--index-url https://user:password@example.com/simple\n";
        let requirement = "flask==1.0.x";
        let text = format!("{prefix}{requirement}\n");
        insta::assert_debug_snapshot!((
            can_show_source(&text, &(0..prefix.len())),
            can_show_source(&text, &(prefix.len() + 5..prefix.len() + requirement.len())),
            can_show_source("-r https://user:password@example.com/requirements.txt", &(0..2)),
            can_show_source("-r constraints.txt", &(0..18)),
        ), @"
        (
            false,
            true,
            false,
            true,
        )
        ");
    }
}
