use std::error::Error;
use std::ops::Range;
use std::sync::Arc;

use uv_errors::{Diagnostic, SourceFile};
use uv_toml::ParseError;

use crate::Pep723Error;

/// Resolve presentation for a PEP 723 metadata error without changing its source chain.
pub fn diagnostic_for_error<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
    let error = error.downcast_ref::<Pep723Error>()?;
    match error {
        Pep723Error::Toml(error) => error.diagnostic(),
        Pep723Error::UnclosedBlock
        | Pep723Error::UnclosedBlockTrailingContent
        | Pep723Error::DuplicateBlock
        | Pep723Error::MissingTag
        | Pep723Error::Io(_)
        | Pep723Error::Utf8(_)
        | Pep723Error::InvalidFilename(_) => None,
    }
}

/// Coordinates from normalized metadata to its original, commented script lines.
#[derive(Debug, Clone)]
pub(super) struct MetadataSourceMap {
    source_range: Range<usize>,
    line_start: usize,
    lines: Vec<MetadataLine>,
    metadata_len: usize,
    eof: usize,
}

#[derive(Debug, Clone)]
struct MetadataLine {
    metadata: Range<usize>,
    content: Range<usize>,
    line_end: usize,
}

impl MetadataSourceMap {
    /// Record the exact selected block. The extractor owns block precedence; this only verifies
    /// and maps the lines it selected, including normalized line endings and removed prefixes.
    pub(super) fn from_script(
        script: &[u8],
        opening: usize,
        line_count: usize,
        metadata: &str,
    ) -> Option<Self> {
        let script = std::str::from_utf8(script).ok()?;
        let mut physical_lines = script.get(opening..)?.split_inclusive('\n');
        let opening_line = physical_lines.next()?;
        if without_line_ending(opening_line) != "# /// script" {
            return None;
        }

        let source_start = opening.checked_add(opening_line.len())?;
        let mut source_offset = 0usize;
        let mut metadata_offset = 0usize;
        let mut lines = Vec::with_capacity(line_count);
        for _ in 0..line_count {
            let physical_line = physical_lines.next()?;
            let line = without_line_ending(physical_line);
            let content = line.strip_prefix('#')?;
            let content = if content.is_empty() {
                content
            } else {
                content.strip_prefix(' ')?
            };
            let metadata_end = metadata_offset.checked_add(content.len())?;
            if metadata.get(metadata_offset..metadata_end)? != content
                || metadata.as_bytes().get(metadata_end) != Some(&b'\n')
            {
                return None;
            }

            let content_start = source_offset.checked_add(line.len() - content.len())?;
            let content_end = content_start.checked_add(content.len())?;
            source_offset = source_offset.checked_add(physical_line.len())?;
            lines.push(MetadataLine {
                metadata: metadata_offset..metadata_end,
                content: content_start..content_end,
                line_end: source_offset,
            });
            metadata_offset = metadata_end.checked_add(1)?;
        }

        // An empty block is represented by one synthetic newline by the metadata extractor.
        let matches_metadata = if line_count == 0 {
            metadata == "\n"
        } else {
            metadata_offset == metadata.len()
        };
        if !matches_metadata {
            return None;
        }

        let closing_line = physical_lines.next()?;
        if without_line_ending(closing_line) != "# ///" {
            return None;
        }
        let source_end = source_start
            .checked_add(source_offset)?
            .checked_add(closing_line.len())?;
        let line_start = script
            .get(..source_start)?
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            .checked_add(1)?;

        Some(Self {
            source_range: source_start..source_end,
            line_start,
            lines,
            metadata_len: metadata.len(),
            eof: source_offset,
        })
    }

    pub(super) fn parse_error(
        &self,
        error: toml::de::Error,
        metadata: &str,
        script: &[u8],
        name: impl Into<Arc<str>>,
    ) -> ParseError {
        let span = error.span().and_then(|span| self.map_span(metadata, span));
        let text = script
            .get(self.source_range.clone())
            .and_then(|source| std::str::from_utf8(source).ok());
        if let (Some(span), Some(text)) = (span, text)
            && text.get(span.clone()).is_some()
        {
            ParseError::new_with_span(
                error,
                SourceFile::new(name, text).with_line_start(self.line_start),
                span,
            )
        } else {
            error.into()
        }
    }

    fn map_span(&self, metadata: &str, span: Range<usize>) -> Option<Range<usize>> {
        if metadata.len() != self.metadata_len || metadata.get(span.clone()).is_none() {
            return None;
        }
        if self.lines.is_empty() {
            return Some(self.eof..self.eof);
        }
        let start = self.map_start(span.start)?;
        let end = if span.is_empty() {
            start
        } else {
            self.map_end(span.end)?
        };
        (start <= end).then_some(start..end)
    }

    /// At a line boundary, a starting span skips that line's removed comment prefix.
    fn map_start(&self, offset: usize) -> Option<usize> {
        if offset == self.metadata_len {
            return Some(self.eof);
        }
        let line = self
            .lines
            .iter()
            .rev()
            .find(|line| line.metadata.start <= offset)?;
        if offset <= line.metadata.end {
            line.content.start.checked_add(offset - line.metadata.start)
        } else if offset == line.metadata.end.checked_add(1)? {
            Some(line.line_end)
        } else {
            None
        }
    }

    /// An ending span at the same boundary includes the previous newline, not the next prefix.
    fn map_end(&self, offset: usize) -> Option<usize> {
        if offset == 0 {
            return self.map_start(offset);
        }
        let line = self.lines.iter().find(|line| {
            offset >= line.metadata.start && offset <= line.metadata.end.saturating_add(1)
        })?;
        if offset <= line.metadata.end {
            line.content.start.checked_add(offset - line.metadata.start)
        } else {
            Some(line.line_end)
        }
    }
}

fn without_line_ending(line: &str) -> &str {
    line.strip_suffix("\r\n")
        .or_else(|| line.strip_suffix('\n'))
        .unwrap_or(line)
}

#[cfg(test)]
mod tests {
    use insta::{assert_debug_snapshot, assert_snapshot};

    use crate::ScriptTag;

    use super::MetadataSourceMap;

    #[test]
    fn script_span_mapping() {
        let script = "#!/usr/bin/env python3\r\n# /// script\r\n# title = 'café'\r\n#\r\n# dependencies = [\r\n#   'bad ???',\r\n# ]\r\n# ///\r\nprint('not metadata')\r\n";
        let tag = ScriptTag::parse(script.as_bytes())
            .expect("valid script block")
            .expect("script metadata");
        let source = tag.source.as_ref().expect("metadata source map");
        let text = &script[source.source_range.clone()];
        let word = tag.metadata.find("café").expect("Unicode value");
        let dependencies = tag.metadata.find("dependencies").expect("dependency array");
        let bad_requirement = tag.metadata.find("bad ???").expect("invalid requirement");
        let mapped = source
            .map_span(&tag.metadata, dependencies..bad_requirement + 7)
            .expect("multiline source span");

        assert_snapshot!(&text[mapped], @r"
        dependencies = [
        #   'bad ???
        ");
        assert_debug_snapshot!((
            source.line_start,
            source.map_span(&tag.metadata, word..word + "café".len()).map(|span| &text[span]),
            source.map_span(&tag.metadata, tag.metadata.len()..tag.metadata.len()).map(|span| &text[span.start..]),
            source.map_span(&tag.metadata, word + 4..word + 5),
            source.map_span(&tag.metadata, tag.metadata.len()..tag.metadata.len() + 1),
        ), @r##"
        (
            3,
            Some(
                "café",
            ),
            Some(
                "# ///\r\n",
            ),
            None,
            None,
        )
        "##);

        for line in &source.lines {
            assert_eq!(
                source.map_span(&tag.metadata, line.metadata.clone()),
                Some(line.content.clone()),
            );
            assert_eq!(
                source.map_span(&tag.metadata, line.metadata.end..line.metadata.end + 1),
                Some(line.content.end..line.line_end),
            );
        }
    }

    #[test]
    fn script_span_mapping_keeps_block_precedence() {
        let script = "# /// script\n#\n# ///\n#\n# ///\nprint('after')\n";
        let tag = ScriptTag::parse(script.as_bytes())
            .expect("valid script block")
            .expect("script metadata");
        let source = tag.source.as_ref().expect("metadata source map");
        let text = &script[source.source_range.clone()];
        let slash = tag.metadata.find("///").expect("embedded delimiter");
        assert_debug_snapshot!((
            &tag.metadata,
            source.map_span(&tag.metadata, slash..slash + 3).map(|span| &text[span]),
            source.map_span(&tag.metadata, tag.metadata.len()..tag.metadata.len()).map(|span| &text[span.start..]),
        ), @r##"
        (
            "\n///\n\n",
            Some(
                "///",
            ),
            Some(
                "# ///\n",
            ),
        )
        "##);
    }

    #[test]
    fn script_span_mapping_empty_and_mismatched_input() {
        let script = "# /// script\n# ///";
        let tag = ScriptTag::parse(script.as_bytes())
            .expect("valid script block")
            .expect("script metadata");
        let source = tag.source.as_ref().expect("metadata source map");
        assert_debug_snapshot!((
            source.map_span(&tag.metadata, 0..1),
            source.map_span(&tag.metadata, 1..1),
            MetadataSourceMap::from_script(script.as_bytes(), 0, 0, "different"),
            MetadataSourceMap::from_script(b"# /// script\r", 0, 0, "\n"),
        ), @"
        (
            Some(
                0..0,
            ),
            Some(
                0..0,
            ),
            None,
            None,
        )
        ");
    }

    #[test]
    fn script_source_does_not_affect_equality() {
        let script = "# /// script\n# dependencies = []\n# ///\n";
        let crlf = script.replace('\n', "\r\n");
        assert_eq!(
            ScriptTag::parse(script.as_bytes()).expect("LF script"),
            ScriptTag::parse(crlf.as_bytes()).expect("CRLF script"),
        );
    }
}
