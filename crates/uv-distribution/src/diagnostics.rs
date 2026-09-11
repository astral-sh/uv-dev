use std::error::Error;
use std::ops::Range;
use std::path::Path;
use std::str::FromStr;

use uv_errors::{Diagnostic, SourceAnnotation, SourceFile, SourceSnippet};
use uv_fs::Simplified;
use uv_normalize::PackageName;
use uv_pep508::MarkerTree;
use uv_toml::SourcePathSegment::{Index, Key};
use uv_toml::{SourceMap, SourcePathSegment};
use uv_workspace::pyproject::{PyProjectToml, Source};

use crate::LoweringError;

/// Resolve retained source locations without changing the lowering error or its source chain.
pub fn diagnostic_for_error<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
    let error = error.downcast_ref::<LoweringError>().or_else(|| {
        error
            .downcast_ref::<Box<LoweringError>>()
            .map(AsRef::as_ref)
    })?;
    match error {
        LoweringError::MissingIndex { diagnostic, .. } => {
            Some(Diagnostic::default().with_snippet(diagnostic.as_deref()?.clone()))
        }
        LoweringError::MissingWorkspaceSource(_)
        | LoweringError::NonWorkspaceSource(..)
        | LoweringError::UndeclaredWorkspacePackage(_)
        | LoweringError::InvalidWorkspaceSource(_)
        | LoweringError::MoreThanOneGitRef
        | LoweringError::GitUrlParse(_)
        | LoweringError::WorkspaceMember
        | LoweringError::InvalidUrl(_)
        | LoweringError::IndexCredentials(_)
        | LoweringError::InvalidVerbatimUrl(_)
        | LoweringError::ForbiddenFragment(_)
        | LoweringError::MissingGitSource(..)
        | LoweringError::WorkspaceFalse
        | LoweringError::Workspace(_)
        | LoweringError::WorkspaceSourceNotRoot { .. }
        | LoweringError::EditableFile(_)
        | LoweringError::PackagedFile(_)
        | LoweringError::GitFile(_)
        | LoweringError::GitDirectory(_)
        | LoweringError::ParsedUrl(_)
        | LoweringError::NonUtf8Path(_)
        | LoweringError::RelativeTo(_) => None,
    }
}

/// The occurrence in the original `tool.uv.sources` value, before scope filtering.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SourceOccurrence {
    pub(crate) index: usize,
    pub(crate) count: usize,
}

/// Capture a verified registry source only after lowering has found its index is missing.
pub(crate) fn missing_index_source(
    project_dir: &Path,
    pyproject: &PyProjectToml,
    package: &PackageName,
    occurrence: SourceOccurrence,
    source: &Source,
) -> Option<SourceSnippet<'static>> {
    let map = SourceMap::parse(&pyproject.raw).ok()?;
    let range = missing_index_span(&map, package, occurrence, source)?;
    let source = SourceFile::new(
        project_dir
            .join("pyproject.toml")
            .portable_display()
            .to_string(),
        pyproject.raw.as_str(),
    );
    // Inline source tables can share a line with credentials or arbitrary marker strings.
    Some(
        SourceSnippet::new(source)
            .with_annotation(SourceAnnotation::primary(range).with_label("undeclared index"))
            .without_source_text(),
    )
}

fn missing_index_span(
    map: &SourceMap<'_>,
    package: &PackageName,
    occurrence: SourceOccurrence,
    source: &Source,
) -> Option<Range<usize>> {
    let (index, marker, extra, group) = match source {
        Source::Registry {
            index,
            marker,
            extra,
            group,
        } => (index, marker, extra, group),
        Source::Git { .. }
        | Source::Url { .. }
        | Source::Path { .. }
        | Source::Workspace { .. } => return None,
    };
    let parent = [Key("tool"), Key("uv"), Key("sources")];
    let mut keys = map
        .keys(&parent)?
        .filter(|key| PackageName::from_str(key).is_ok_and(|name| &name == package));
    let key = keys.next()?;
    // Normalization must identify one authored key, not choose between ambiguous spellings.
    if keys.next().is_some() {
        return None;
    }

    let mut path = vec![Key("tool"), Key("uv"), Key("sources"), Key(key)];
    if let Some(count) = map.array_len(&path) {
        if occurrence.count != count || occurrence.index >= count {
            return None;
        }
        path.push(Index(occurrence.index));
    } else if occurrence.count != 1 || occurrence.index != 0 {
        return None;
    }

    // The selected semantic source is a registry source. A mismatched or unsupported table
    // cannot establish that the retained `index` belongs to this error.
    if !map
        .keys(&path)?
        .all(|key| matches!(key, "index" | "marker" | "extra" | "group"))
        || !source_field_matches(map, &path, "index", Some(index), None)
        || !source_field_matches(map, &path, "marker", Some(marker), Some(&MarkerTree::TRUE))
        || !source_field_matches(map, &path, "extra", extra.as_ref(), None)
        || !source_field_matches(map, &path, "group", group.as_ref(), None)
    {
        return None;
    }

    path.push(Key("index"));
    map.span(&path)
}

/// Match a typed value, using the default only when the field is absent.
fn source_field_matches<T: FromStr + PartialEq>(
    map: &SourceMap<'_>,
    parent: &[SourcePathSegment<'_>],
    key: &'static str,
    expected: Option<&T>,
    default: Option<&T>,
) -> bool {
    let mut path = parent.to_vec();
    path.push(Key(key));
    let Some(_) = map.span(&path) else {
        return expected == default;
    };
    map.string(&path)
        .and_then(|value| T::from_str(value).ok())
        .is_some_and(|value| expected == Some(&value))
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::path::Path;
    use std::str::FromStr;

    use anyhow::{Context, Result};
    use uv_distribution_types::IndexName;
    use uv_errors::{ErrorOptions, Hinted, write_error_chain_with_options};
    use uv_normalize::{ExtraName, GroupName, PackageName};
    use uv_pep508::MarkerTree;
    use uv_toml::SourceMap;
    use uv_workspace::pyproject::{PyProjectToml, Source};

    use crate::LoweringError;

    use super::{SourceOccurrence, diagnostic_for_error, missing_index_source, missing_index_span};

    fn registry_source(
        index: &str,
        marker: Option<&str>,
        extra: Option<&str>,
        group: Option<&str>,
    ) -> Result<Source> {
        Ok(Source::Registry {
            index: IndexName::from_str(index)?,
            marker: marker
                .map(MarkerTree::from_str)
                .transpose()?
                .unwrap_or(MarkerTree::TRUE),
            extra: extra.map(ExtraName::from_str).transpose()?,
            group: group.map(GroupName::from_str).transpose()?,
        })
    }

    #[test]
    fn missing_index_spans_follow_original_occurrences() -> Result<()> {
        let package = PackageName::from_str("demo-pkg")?;
        let cases = [
            (
                "# café\r\n[tool.uv.sources.\"Demo.Pkg\"]\r\nindex = \"priv\\u0061te\"\r\n",
                SourceOccurrence { index: 0, count: 1 },
                registry_source("private", None, None, None)?,
            ),
            (
                "[tool.uv.sources]\n\"Demo.Pkg\" = [\n  { index = 'private', extra = 'unused' },\n  { index = 'private', group = 'Dev.Tools', marker = \"python_version >= '3.12'\" },\n]\n",
                SourceOccurrence { index: 1, count: 2 },
                registry_source(
                    "private",
                    Some("python_version >= '3.12'"),
                    None,
                    Some("dev-tools"),
                )?,
            ),
            (
                "[[tool.uv.sources.\"Demo_Pkg\"]]\nindex = 'private'\nmarker = \"python_version < '3.12'\"\n[[tool.uv.sources.\"Demo_Pkg\"]]\nindex = 'private'\nmarker = \"python_version >= '3.12'\"\n",
                SourceOccurrence { index: 1, count: 2 },
                registry_source("private", Some("python_version >= '3.12'"), None, None)?,
            ),
        ];
        let mut locations = Vec::new();
        for (text, occurrence, source) in cases {
            let map = SourceMap::parse(text)?;
            let range = missing_index_span(&map, &package, occurrence, &source)
                .context("the source occurrence should identify its index")?;
            let value = text.get(range.clone()).map(str::to_string);
            locations.push((range, value));
        }
        insta::assert_debug_snapshot!(locations, @r#"
        [
            (
                47..61,
                Some(
                    "\"priv\\u0061te\"",
                ),
            ),
            (
                88..97,
                Some(
                    "'private'",
                ),
            ),
            (
                123..132,
                Some(
                    "'private'",
                ),
            ),
        ]
        "#);
        Ok(())
    }

    #[test]
    fn missing_index_spans_reject_mismatched_sources() -> Result<()> {
        let package = PackageName::from_str("demo-pkg")?;
        let expected = registry_source(
            "private",
            Some("python_version >= '3.12'"),
            Some("dev-tools"),
            None,
        )?;
        let occurrence = SourceOccurrence { index: 0, count: 1 };
        let cases = [
            (
                "matching normalized key and scope",
                "[tool.uv.sources]\n\"Demo.Pkg\" = { index = 'private', marker = \"python_version >= '3.12'\", extra = 'Dev.Tools' }\n",
                occurrence,
            ),
            (
                "different index",
                "[tool.uv.sources]\ndemo-pkg = { index = 'other', marker = \"python_version >= '3.12'\", extra = 'dev-tools' }\n",
                occurrence,
            ),
            (
                "omitted marker",
                "[tool.uv.sources]\ndemo-pkg = { index = 'private', extra = 'dev-tools' }\n",
                occurrence,
            ),
            (
                "invalid marker type",
                "[tool.uv.sources]\ndemo-pkg = { index = 'private', marker = true, extra = 'dev-tools' }\n",
                occurrence,
            ),
            (
                "different extra",
                "[tool.uv.sources]\ndemo-pkg = { index = 'private', marker = \"python_version >= '3.12'\", extra = 'other' }\n",
                occurrence,
            ),
            (
                "group instead of extra",
                "[tool.uv.sources]\ndemo-pkg = { index = 'private', marker = \"python_version >= '3.12'\", group = 'dev-tools' }\n",
                occurrence,
            ),
            (
                "different original count",
                "[tool.uv.sources]\ndemo-pkg = [{ index = 'private', marker = \"python_version >= '3.12'\", extra = 'dev-tools' }]\n",
                SourceOccurrence { index: 0, count: 2 },
            ),
            (
                "single source with array occurrence",
                "[tool.uv.sources]\ndemo-pkg = { index = 'private', marker = \"python_version >= '3.12'\", extra = 'dev-tools' }\n",
                SourceOccurrence { index: 1, count: 2 },
            ),
            (
                "competing source type",
                "[tool.uv.sources]\ndemo-pkg = { index = 'private', url = 'https://user:sentinel-secret@example.com/private.whl', marker = \"python_version >= '3.12'\", extra = 'dev-tools' }\n",
                occurrence,
            ),
            (
                "ambiguous normalized keys",
                "[tool.uv.sources]\n\"Demo.Pkg\" = { index = 'private', marker = \"python_version >= '3.12'\", extra = 'dev-tools' }\ndemo_pkg = { index = 'private', marker = \"python_version >= '3.12'\", extra = 'dev-tools' }\n",
                occurrence,
            ),
        ];
        let mut locations = Vec::new();
        for (case, text, occurrence) in cases {
            let map = SourceMap::parse(text)?;
            locations.push((
                case,
                missing_index_span(&map, &package, occurrence, &expected),
            ));
        }
        insta::assert_debug_snapshot!(locations, @r#"
        [
            (
                "matching normalized key and scope",
                Some(
                    41..50,
                ),
            ),
            (
                "different index",
                None,
            ),
            (
                "omitted marker",
                None,
            ),
            (
                "invalid marker type",
                None,
            ),
            (
                "different extra",
                None,
            ),
            (
                "group instead of extra",
                None,
            ),
            (
                "different original count",
                None,
            ),
            (
                "single source with array occurrence",
                None,
            ),
            (
                "competing source type",
                None,
            ),
            (
                "ambiguous normalized keys",
                None,
            ),
        ]
        "#);
        Ok(())
    }

    #[test]
    fn missing_index_source_is_location_only() -> Result<()> {
        let pyproject = PyProjectToml::from_string(
            "[tool.uv.sources]\n\"Demo.Pkg\" = [{ url = 'https://user:sentinel-secret@example.com/demo_pkg-1.0.0-py3-none-any.whl', marker = \"sys_platform == 'sentinel-secret'\" }, { index = 'private', marker = \"sys_platform != 'sentinel-secret'\" }]\n".to_string(),
            Path::new("pyproject.toml"),
        )?;
        let package = PackageName::from_str("demo-pkg")?;
        let source = pyproject
            .tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.sources.as_ref())
            .and_then(|sources| sources.inner().get(&package))
            .and_then(|sources| sources.iter().nth(1))
            .context("the second registry source should be retained")?;
        let snippet = missing_index_source(
            Path::new("."),
            &pyproject,
            &package,
            SourceOccurrence { index: 1, count: 2 },
            source,
        )
        .context("the registry source should have a verified location")?;
        let error = LoweringError::MissingIndex {
            package,
            index: IndexName::from_str("private")?,
            hint: Some("Declare the index in the project configuration".to_string()),
            diagnostic: Some(Box::new(snippet)),
        };
        assert!(diagnostic_for_error(&error).is_some());
        assert!(error.source().is_none());
        let hints = error.hints();
        let mut output = String::new();
        write_error_chain_with_options(
            &error,
            &hints,
            ErrorOptions::default()
                .with_width_override(usize::MAX)
                .with_diagnostic(diagnostic_for_error)
                .with_stream(&mut output),
        )?;
        insta::assert_snapshot!(anstream::adapter::strip_str(&output), @"
        error: Package `demo-pkg` references an undeclared index: `private`
           --> ./pyproject.toml:2:157

        hint: Declare the index in the project configuration
        ");
        Ok(())
    }
}
