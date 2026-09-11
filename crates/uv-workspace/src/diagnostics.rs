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
    field: Option<ProjectNameField>,
}

#[derive(Debug)]
struct ProjectNameField {
    key: Range<usize>,
    value: Range<usize>,
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
        let field = SourceMap::parse(source.text()).ok().and_then(|map| {
            let path = [Key("project"), Key("name")];
            let name = PackageName::from_str(map.string(&path)?).ok()?;
            // The typed member determines identity; only attach its matching declaration.
            if &name != expected_name {
                return None;
            }
            Some(ProjectNameField {
                key: map.key_span(&[Key("project")], "name")?,
                value: map.span(&path)?,
            })
        });
        Self { source, field }
    }

    fn snippet(&self, role: ProjectNameRole) -> SourceSnippet<'static> {
        let mut snippet = SourceSnippet::new(self.source.clone());
        if let Some(field) = &self.field {
            snippet = snippet.with_annotation(role.annotation(field.value.clone()));
            if self.is_standalone_assignment(field) {
                return snippet;
            }
        }
        snippet.without_source_text()
    }

    /// A validated project name is safe to show, but an inline table or trailing comment can
    /// contain unrelated credentials. Only expose a physical line containing this assignment
    /// and no other non-whitespace text.
    fn is_standalone_assignment(&self, field: &ProjectNameField) -> bool {
        let Some(window) = self.source.line_range_for_span(field.value.clone()) else {
            return false;
        };
        if self.source.line_range_for_span(field.key.clone()) != Some(window.clone()) {
            return false;
        }
        let text = self.source.text();
        text.get(window.start..field.key.start)
            .is_some_and(|prefix| prefix.trim().is_empty())
            && text
                .get(field.key.end..field.value.start)
                .is_some_and(|separator| separator.trim() == "=")
            && text
                .get(field.value.end..window.end)
                .is_some_and(|suffix| suffix.trim().is_empty())
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
    fn project_name_source_requires_exact_standalone_field() -> Result<()> {
        let name = PackageName::from_str("example")?;

        let escaped = ProjectNameSource::from_source(
            SourceFile::new(
                "pyproject.toml",
                "[project]\r\n\"na\\u006de\" = \"\\u0065xample\"\r\n",
            ),
            &name,
        );
        assert!(
            escaped
                .field
                .as_ref()
                .is_some_and(|field| escaped.is_standalone_assignment(field))
        );

        let multiline = ProjectNameSource::from_source(
            SourceFile::new(
                "pyproject.toml",
                "[project]\nname = \"\"\"\nexample\"\"\"\n",
            ),
            &name,
        );
        assert!(
            multiline
                .field
                .as_ref()
                .is_some_and(|field| !multiline.is_standalone_assignment(field))
        );

        let mismatched = ProjectNameSource::from_source(
            SourceFile::new("pyproject.toml", "[project]\nname = 'other'\n"),
            &name,
        );
        assert!(mismatched.field.is_none());

        Ok(())
    }
}
