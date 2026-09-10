use std::error::Error;
use std::fmt;
use std::ops::Range;

use uv_errors::{Diagnostic, SourceAnnotation, SourceFile, SourceSnippet};

/// A TOML parse error with optional source provenance retained by the caller.
///
/// Its ordinary display and source chain are the TOML error's own. Rich renderers can use
/// [`Self::diagnostic`] without removing input from, or replacing, the original error.
#[derive(Debug)]
pub struct ParseError {
    error: Box<toml::de::Error>,
    document: Option<SourceFile>,
}

impl ParseError {
    /// Retain the decoded document from the boundary that produced this error.
    pub fn new(error: toml::de::Error, document: SourceFile) -> Self {
        Self {
            error: Box::new(error),
            document: Some(document),
        }
    }

    /// Describe the error using its retained source, if its span is available and valid.
    pub fn diagnostic(&self) -> Option<Diagnostic<'_>> {
        diagnostic_for_error(&self.error, self.document.as_ref()?)
    }
}

impl From<toml::de::Error> for ParseError {
    fn from(error: toml::de::Error) -> Self {
        Self {
            error: Box::new(error),
            document: None,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, formatter)
    }
}

impl Error for ParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.error.source()
    }
}

/// Describe a TOML error using the exact decoded input passed to the parser.
///
/// A TOML span uses byte offsets into the complete document. Only the first physical line touched
/// by the span is displayed: multiline highlights are clipped to that line, and surrounding lines
/// are not included because configuration files may contain credentials. This bounds the source
/// context; it does not redact values within the selected line. An unavailable or invalid span
/// leaves the parser's ordinary display untouched, including any deserialization key path.
pub fn diagnostic_for_error<'a>(
    error: &'a toml::de::Error,
    document: &SourceFile,
) -> Option<Diagnostic<'a>> {
    let excerpt = line_excerpt(document.text(), error.span()?)?;
    let source = SourceFile::new(document.name(), excerpt.text).with_line_start(excerpt.line_start);
    let snippet = SourceSnippet::new(source)
        .with_context_lines(0)
        .with_annotation(SourceAnnotation::primary(excerpt.range));
    Some(Diagnostic::new(error.message()).with_snippet(snippet))
}

#[derive(Debug)]
struct LineExcerpt<'a> {
    text: &'a str,
    line_start: usize,
    range: Range<usize>,
}

/// Translate a valid UTF-8 byte span to a single complete physical line.
fn line_excerpt(input: &str, range: Range<usize>) -> Option<LineExcerpt<'_>> {
    if range.start > range.end || !input.is_char_boundary(range.end) {
        return None;
    }
    let before = input.get(..range.start)?;
    let after = input.get(range.start..)?;
    let start = before.rfind('\n').map_or(0, |index| index + 1);
    let end = after
        .find('\n')
        .map_or(input.len(), |index| range.start + index + 1);
    Some(LineExcerpt {
        text: &input[start..end],
        line_start: before.bytes().filter(|byte| *byte == b'\n').count() + 1,
        range: (range.start - start)..(range.end.min(end) - start),
    })
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::ops::Range;

    use insta::{assert_debug_snapshot, assert_snapshot};
    use serde::Deserialize;
    use uv_errors::SourceFile;

    use super::{ParseError, diagnostic_for_error, line_excerpt};

    #[test]
    fn first_line_excerpt() {
        let input = "token = 'secret'\r\nitems = [\r\n  'first',\r\n]\r\nother = 'secret'\r\n";
        let start = input.find('[').expect("array in test input");
        let end = input.find(']').expect("array terminator in test input") + 1;
        assert_debug_snapshot!(line_excerpt(input, start..end), @r#"
        Some(
            LineExcerpt {
                text: "items = [\r\n",
                line_start: 2,
                range: 8..11,
            },
        )
        "#);
    }

    #[test]
    fn utf8_and_eof_excerpt() {
        let input = "name = 'café'\nitems = [";
        assert_debug_snapshot!(line_excerpt(input, 11..13), @r#"
        Some(
            LineExcerpt {
                text: "name = 'café'\n",
                line_start: 1,
                range: 11..13,
            },
        )
        "#);
        assert_debug_snapshot!(line_excerpt(input, input.len()..input.len()), @r#"
        Some(
            LineExcerpt {
                text: "items = [",
                line_start: 2,
                range: 9..9,
            },
        )
        "#);
        assert_debug_snapshot!(line_excerpt("items = [\n", 10..10), @r#"
        Some(
            LineExcerpt {
                text: "",
                line_start: 2,
                range: 0..0,
            },
        )
        "#);
    }

    #[test]
    fn invalid_excerpt() {
        assert!(line_excerpt("café", 3..4).is_none());
        assert!(line_excerpt("café", 4..5).is_none());
        assert!(line_excerpt("abc", Range { start: 2, end: 1 }).is_none());
        assert!(line_excerpt("abc", 3..4).is_none());
    }

    #[test]
    fn parse_error_keeps_original_fallback() {
        #[derive(Deserialize)]
        struct Settings {
            #[serde(rename = "count")]
            _count: usize,
        }

        let input = "count = 'invalid'";
        let error = toml::from_str::<Settings>(input)
            .err()
            .expect("invalid integer in test input");
        let error = ParseError::new(error, SourceFile::new("uv.toml", input));
        assert!(error.source().is_none());
        assert!(error.diagnostic().is_some());
        assert_debug_snapshot!(error.error.span(), @"
        Some(
            8..17,
        )
        ");
        assert_snapshot!(error, @r#"
        TOML parse error at line 1, column 9
          |
        1 | count = 'invalid'
          |         ^^^^^^^^^
        invalid type: string "invalid", expected usize
        "#);

        let error = <toml::de::Error as serde::de::Error>::custom("invalid settings");
        assert!(diagnostic_for_error(&error, &SourceFile::new("uv.toml", input)).is_none());
        assert!(ParseError::from(error).diagnostic().is_none());
    }
}
