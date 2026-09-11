use std::ops::Range;

use toml_edit::{Array, ArrayOfTables, Document, Item, TomlError};

/// A step through the syntax tree of a TOML document.
#[derive(Debug, Clone, Copy)]
pub enum SourcePathSegment<'a> {
    /// A decoded table key.
    Key(&'a str),
    /// The zero-based occurrence in an array or array of tables.
    Index(usize),
}

/// Source locations for semantic fields in a TOML document.
///
/// Paths refer to parsed keys and array occurrences, not matching text. Spans are half-open
/// UTF-8 byte ranges into the original document passed to [`Self::parse`].
#[derive(Debug)]
pub struct SourceMap<'a> {
    document: Document<&'a str>,
}

impl<'a> SourceMap<'a> {
    /// Parse an immutable document, retaining its original source spans.
    pub fn parse(source: &'a str) -> Result<Self, TomlError> {
        Ok(Self {
            document: Document::parse(source)?,
        })
    }

    /// Return the range of a value, table, or array occurrence.
    pub fn span(&self, path: &[SourcePathSegment<'_>]) -> Option<Range<usize>> {
        self.item(path)?.span()
    }

    /// Return the range of a key within a table or inline table.
    pub fn key_span(&self, parent: &[SourcePathSegment<'_>], key: &str) -> Option<Range<usize>> {
        self.item(parent)?
            .as_table_like()?
            .get_key_value(key)?
            .0
            .span()
    }

    /// Iterate over decoded keys in source order.
    ///
    /// Callers with normalized semantic names can resolve the original spelling before looking
    /// up a value's span.
    pub fn keys(&self, path: &[SourcePathSegment<'_>]) -> Option<impl Iterator<Item = &str>> {
        Some(self.item(path)?.as_table_like()?.iter().map(|(key, _)| key))
    }

    /// Return a decoded string value.
    pub fn string(&self, path: &[SourcePathSegment<'_>]) -> Option<&str> {
        self.item(path)?.as_str()
    }

    /// Return the number of values or tables in an array.
    pub fn array_len(&self, path: &[SourcePathSegment<'_>]) -> Option<usize> {
        let item = self.item(path)?;
        item.as_array()
            .map(Array::len)
            .or_else(|| item.as_array_of_tables().map(ArrayOfTables::len))
    }

    fn item(&self, path: &[SourcePathSegment<'_>]) -> Option<&Item> {
        let mut item = self.document.as_item();
        for segment in path {
            item = match segment {
                SourcePathSegment::Key(key) => item.get(key)?,
                SourcePathSegment::Index(index) => item.get(*index)?,
            };
        }
        Some(item)
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_debug_snapshot;

    use super::SourceMap;
    use super::SourcePathSegment::{Index, Key};

    #[test]
    fn spans_follow_keys_and_array_occurrences() {
        let source = "# café\n[dependency-groups]\n\"Dev.Tools\" = [\"same\", { include-group = \"same\" }, \"same\"]\n";
        let map = SourceMap::parse(source).unwrap();
        let group = [Key("dependency-groups"), Key("Dev.Tools")];
        let first = [Key("dependency-groups"), Key("Dev.Tools"), Index(0)];
        let include = [
            Key("dependency-groups"),
            Key("Dev.Tools"),
            Index(1),
            Key("include-group"),
        ];
        let last = [Key("dependency-groups"), Key("Dev.Tools"), Index(2)];
        let ranges = [
            map.key_span(&[Key("dependency-groups")], "Dev.Tools"),
            map.span(&first),
            map.span(&include),
            map.span(&last),
        ];
        let values = ranges
            .clone()
            .map(|range| range.and_then(|range| source.get(range)));
        assert_debug_snapshot!((map.array_len(&group), ranges, values), @r#"
        (
            Some(
                3,
            ),
            [
                Some(
                    28..39,
                ),
                Some(
                    43..49,
                ),
                Some(
                    69..75,
                ),
                Some(
                    79..85,
                ),
            ],
            [
                Some(
                    "\"Dev.Tools\"",
                ),
                Some(
                    "\"same\"",
                ),
                Some(
                    "\"same\"",
                ),
                Some(
                    "\"same\"",
                ),
            ],
        )
        "#);
    }

    #[test]
    fn spans_handle_dotted_keys_and_array_tables() {
        let source = "tool.uv.sources.pkg = { path = 'src', marker = \"sys_platform == 'linux'\" }\n[[tool.uv.index]]\nname = 'first'\n[[tool.uv.index]]\nname = 'second'\n";
        let map = SourceMap::parse(source).unwrap();
        let marker = [
            Key("tool"),
            Key("uv"),
            Key("sources"),
            Key("pkg"),
            Key("marker"),
        ];
        let index = [Key("tool"), Key("uv"), Key("index"), Index(1), Key("name")];
        let ranges = [map.span(&marker), map.span(&index)];
        let values = ranges
            .clone()
            .map(|range| range.and_then(|range| source.get(range)));
        let keys = map
            .keys(&[Key("tool"), Key("uv")])
            .map(Iterator::collect::<Vec<_>>);
        assert_debug_snapshot!((keys, map.string(&index), ranges, values), @r#"
        (
            Some(
                [
                    "sources",
                    "index",
                ],
            ),
            Some(
                "second",
            ),
            [
                Some(
                    47..72,
                ),
                Some(
                    133..141,
                ),
            ],
            [
                Some(
                    "\"sys_platform == 'linux'\"",
                ),
                Some(
                    "'second'",
                ),
            ],
        )
        "#);
    }
}
