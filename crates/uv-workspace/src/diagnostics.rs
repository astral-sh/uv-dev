use std::error::Error;
use std::ops::Range;
use std::str::FromStr;

use uv_errors::{Diagnostic, Info, SourceAnnotation, SourceFile, SourceSnippet};
use uv_fs::Simplified;
use uv_normalize::PackageName;
use uv_toml::SourceMap;
use uv_toml::SourcePathSegment::Key;

use crate::{WorkspaceError, WorkspaceMember};

/// Resolve source presentation for workspace discovery errors.
pub fn diagnostic_for_error<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
    error.downcast_ref::<WorkspaceError>()?.diagnostic()
}

#[derive(Debug)]
pub(crate) struct DuplicatePackageDiagnostic {
    name: PackageName,
    first: ProjectNameSource,
    second: ProjectNameSource,
}

impl DuplicatePackageDiagnostic {
    pub(crate) fn new(first: &WorkspaceMember, second: &WorkspaceMember) -> Self {
        Self {
            name: first.project().name.clone(),
            first: ProjectNameSource::new(first),
            second: ProjectNameSource::new(second),
        }
    }

    pub(crate) fn diagnostic(&self) -> Diagnostic<'_> {
        Diagnostic::new(format!(
            "Two workspace members are both named `{}`",
            self.name
        ))
        .with_snippet(self.second.snippet(ProjectNameRole::Duplicate))
        .with_info(
            Info::new("The name was first declared here")
                .with_snippet(self.first.snippet(ProjectNameRole::First)),
        )
    }
}

#[derive(Debug)]
struct ProjectNameSource {
    source: SourceFile,
    span: Option<Range<usize>>,
}

#[derive(Clone, Copy)]
enum ProjectNameRole {
    First,
    Duplicate,
}

impl ProjectNameRole {
    fn annotation(self, range: Range<usize>) -> SourceAnnotation<'static> {
        match self {
            Self::First => SourceAnnotation::secondary(range).with_label("first declared here"),
            Self::Duplicate => SourceAnnotation::primary(range).with_label("duplicate name"),
        }
    }
}

impl ProjectNameSource {
    fn new(member: &WorkspaceMember) -> Self {
        Self::from_source(
            SourceFile::new(
                member
                    .root()
                    .join("pyproject.toml")
                    .portable_display()
                    .to_string(),
                member.pyproject_toml().raw.as_str(),
            ),
            &member.project().name,
        )
    }

    fn from_source(source: SourceFile, expected_name: &PackageName) -> Self {
        let span = SourceMap::parse(source.text()).ok().and_then(|map| {
            let path = [Key("project"), Key("name")];
            let name = PackageName::from_str(map.string(&path)?).ok()?;
            // The typed member determines identity; only attach its matching declaration.
            if &name != expected_name {
                return None;
            }
            map.span(&path)
        });
        Self { source, span }
    }

    fn snippet(&self, role: ProjectNameRole) -> SourceSnippet<'static> {
        let mut snippet = SourceSnippet::new(self.source.clone());
        if let Some(span) = &self.span {
            snippet = snippet.with_annotation(role.annotation(span.clone()));
        }
        snippet
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use anyhow::Result;

    use uv_errors::SourceFile;
    use uv_normalize::PackageName;

    use super::ProjectNameSource;

    #[test]
    fn project_name_source_uses_exact_matching_field() -> Result<()> {
        let name = PackageName::from_str("example")?;
        let sources = [
            "[project]\r\n\"na\\u006de\" = \"\\u0065xample\"\r\n",
            "[project]\nname = \"\"\"\nexample\"\"\"\n",
            "project = { name = 'example', version = '0.1.0' }\n",
            "[project]\nname = 'example' # shared by both members\n",
            "[project]\nname = 'other'\n",
        ];
        let values = sources.map(|source| {
            let name_source =
                ProjectNameSource::from_source(SourceFile::new("pyproject.toml", source), &name);
            name_source.span.and_then(|span| source.get(span))
        });
        insta::assert_debug_snapshot!(values, @r#"
        [
            Some(
                "\"\\u0065xample\"",
            ),
            Some(
                "\"\"\"\nexample\"\"\"",
            ),
            Some(
                "'example'",
            ),
            Some(
                "'example'",
            ),
            None,
        ]
        "#);

        Ok(())
    }
}
