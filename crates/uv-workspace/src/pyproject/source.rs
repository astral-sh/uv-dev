use serde::de::DeserializeOwned;
use toml::Spanned;
use toml::de::{DeTable, Deserializer};

use super::{PyProjectToml, PyProjectTomlSourcesWire, PyprojectTomlError, ToolUvSources};

type ParsedTomlTable<'source> = Spanned<DeTable<'source>>;

self_cell::self_cell! {
    struct SourceCell {
        owner: String,

        #[covariant]
        dependent: ParsedTomlTable,
    }
}

/// An owned TOML source that can be deserialized into independent project manifests.
///
/// Parsing retains the original TOML value types and source spans. Deserialization evaluates
/// requirements, URLs, and other context-sensitive fields for the current invocation.
pub struct PyProjectTomlSource(SourceCell);

impl PyProjectTomlSource {
    /// Parse a TOML document without interpreting its project-specific fields.
    pub fn parse(raw: String) -> Result<Self, toml::de::Error> {
        SourceCell::try_new(raw, |raw| DeTable::parse(raw)).map(Self)
    }

    /// Return the original source text.
    pub fn contents(&self) -> &str {
        self.0.borrow_owner()
    }

    /// Deserialize a fresh [`PyProjectToml`] with source-backed diagnostics.
    pub fn deserialize(&self) -> Result<PyProjectToml, PyprojectTomlError> {
        let pyproject = match self.deserialize_table::<PyProjectToml>() {
            Ok(pyproject) => pyproject,
            Err(error) => {
                // Source errors have precedence when both project and source fields are invalid.
                let sources = self
                    .deserialize_table::<PyProjectTomlSourcesWire>()?
                    .tool
                    .and_then(|tool| tool.uv)
                    .and_then(|uv| uv.sources);
                if let Some(sources) = sources {
                    ToolUvSources::try_from(sources)?;
                }
                return Err(PyprojectTomlError::Toml(error));
            }
        };

        Ok(PyProjectToml {
            raw: self.contents().to_owned(),
            ..pyproject
        })
    }

    fn deserialize_table<T: DeserializeOwned>(&self) -> Result<T, toml::de::Error> {
        let deserializer = Deserializer::from(self.0.borrow_dependent().clone());
        T::deserialize(deserializer).map_err(|mut error| {
            error.set_input(Some(self.contents()));
            error
        })
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::mem::discriminant;
    use std::sync::Arc;

    use anyhow::Result;
    use uv_redacted::DisplaySafeUrl;

    use super::PyProjectTomlSource;
    use crate::pyproject::{PyProjectToml, PyprojectTomlError};

    fn assert_full_eq(actual: PyProjectToml, expected: &PyProjectToml) {
        let PyProjectToml {
            project,
            tool,
            dependency_groups,
            raw,
            build_system,
        } = actual;
        assert_eq!(project, expected.project);
        assert_eq!(tool, expected.tool);
        assert_eq!(dependency_groups, expected.dependency_groups);
        assert_eq!(raw, expected.raw);
        assert_eq!(build_system.is_some(), expected.build_system.is_some());
    }

    fn assert_success_parity(raw: &str) -> Result<()> {
        let expected = PyProjectToml::from_string(raw.to_owned(), "pyproject.toml")?;
        let source = Arc::new(PyProjectTomlSource::parse(raw.to_owned())?);
        assert_eq!(source.contents(), raw);
        assert_full_eq(source.deserialize()?, &expected);
        assert_full_eq(source.deserialize()?, &expected);
        Ok(())
    }

    fn error_chain(error: &dyn Error) -> Vec<String> {
        let mut chain = Vec::new();
        let mut current = Some(error);
        while let Some(error) = current {
            chain.push(error.to_string());
            current = error.source();
        }
        chain
    }

    fn assert_error_parity(raw: &str) {
        let expected = PyProjectToml::from_string(raw.to_owned(), "pyproject.toml")
            .expect_err("invalid project manifest");
        let actual = PyProjectTomlSource::parse(raw.to_owned())
            .map_err(PyprojectTomlError::from)
            .and_then(|source| source.deserialize())
            .expect_err("invalid project manifest");
        assert_eq!(discriminant(&actual), discriminant(&expected));
        assert_eq!(error_chain(&actual), error_chain(&expected));
        if let (PyprojectTomlError::Toml(actual), PyprojectTomlError::Toml(expected)) =
            (&actual, &expected)
        {
            assert_eq!(actual.span(), expected.span());
            assert_eq!(actual.message(), expected.message());
        }
    }

    #[test]
    fn source_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<PyProjectTomlSource>();
    }

    #[test]
    fn source_replays_all_project_fields() -> Result<()> {
        assert_success_parity(
            r#"
[build-system]
requires = ["setuptools>=64"]
build-backend = "setuptools.build_meta"

[project]
name = "source-example"
version = "1.2.3"
requires-python = ">=3.12"
dependencies = ["anyio>=4", "idna; python_version >= '3.12'"]

[project.optional-dependencies]
docs = ["sphinx>=7"]

[project.scripts]
source-example = "source_example:main"

[project.gui-scripts]
source-example-gui = "source_example:main"

[dependency-groups]
dev = ["ruff", { include-group = "test" }]
test = ["pytest"]
future = [{ include-group = "test", unknown = "value" }]

[tool.uv]
managed = true
package = true
default-groups = ["test"]
dev-dependencies = ["typing-extensions; python_version < '3.13'"]
override-dependencies = ["anyio==4.0.0", { package = { name = "httpx", version = "0.28.0" }, dependencies = ["idna==3.10"] }]
exclude-dependencies = ["old-package", { package = { name = "httpx", version = "0.28.0" }, dependencies = ["sniffio"] }]
constraint-dependencies = ["httpx<1"]
build-constraint-dependencies = ["setuptools>=64"]
environments = ["python_version >= '3.12'"]
required-environments = "sys_platform == 'linux'"
conflicts = [[{ extra = "docs" }, { group = "test" }]]

[tool.uv.dependency-groups]
test = { requires-python = ">=3.12" }

[tool.uv.workspace]
members = ["packages/*"]
exclude = ["packages/ignored"]

[tool.uv.sources]
httpx = { git = "https://github.com/encode/httpx", tag = "0.28.0", subdirectory = "src" }
anyio = [
    { url = "https://example.com/anyio-4.0.0-py3-none-any.whl", marker = "sys_platform == 'linux'" },
    { path = "../anyio", editable = false, package = false, marker = "sys_platform != 'linux'" },
]
idna = { index = "private" }
member = { workspace = true, editable = false }
external = { workspace = "../external" }

[[tool.uv.index]]
name = "private"
url = "https://example.com/simple"
explicit = true
publish-url = "https://example.com/legacy/"
authenticate = "always"
ignore-error-codes = [403]
cache-control = { api = "max-age=600", files = "max-age=3600" }
hash-algorithm = "sha256"
exclude-newer = "2025-01-01T00:00:00Z"

[tool.uv.build-backend]
module-name = "source_example"
"#,
        )
    }

    #[test]
    fn source_keeps_ignored_wide_integers() -> Result<()> {
        assert_success_parity(
            r#"
[project]
name = "example"
version = "1.0"

[tool.other]
wide-unsigned = 340282366920938463463374607431768211455
wide-signed = -170141183460469231731687303715884105728
"#,
        )
    }

    #[test]
    fn source_rejects_datetime_string_coercion() {
        assert_error_parity(
            r#"
[project]
name = 2026-09-11
version = "1.0"
"#,
        );
        assert_error_parity(
            r"
[tool.uv.workspace]
members = [2026-09-11]
",
        );
    }

    #[test]
    fn source_replays_syntax_and_normalized_duplicate_errors() {
        for raw in [
            "[project\n",
            r#"
[project]
name = "example"
version = "1.0"
[project.optional-dependencies]
foo-bar = ["anyio"]
foo_bar = ["idna"]
"#,
            r#"
[tool.uv.sources]
foo-bar = { path = "one" }
foo_bar = { path = "two" }
"#,
            r#"
[dependency-groups]
foo-bar = ["anyio"]
foo_bar = ["idna"]
"#,
            r#"
[tool.uv.dependency-groups]
foo-bar = { requires-python = ">=3.12" }
foo_bar = { requires-python = ">=3.13" }
"#,
        ] {
            assert_error_parity(raw);
        }
    }

    #[test]
    fn source_errors_have_precedence() {
        let raw = r#"
[project]
version = "1.0"

[tool.uv.sources]
example = [{ path = "one" }, { path = "two" }]
"#;
        let error = PyProjectToml::from_string(raw.to_owned(), "pyproject.toml")
            .expect_err("overlapping source markers");
        assert_eq!(
            discriminant(&error),
            discriminant(&PyprojectTomlError::Source(
                crate::pyproject::SourceError::MissingMarkers
            ))
        );
        assert_error_parity(raw);
    }

    #[test]
    fn source_lowers_file_urls_again() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("example-1.0-py3-none-any.whl");
        let url = DisplaySafeUrl::from_file_path(&path).expect("absolute fixture path");
        let raw = format!("[tool.uv]\ndev-dependencies = [\"example @ {url}\"]\n");
        let source = PyProjectTomlSource::parse(raw.clone())?;
        let first = source.deserialize()?;
        assert_full_eq(
            first.clone(),
            &PyProjectToml::from_string(raw.clone(), "pyproject.toml")?,
        );

        fs_err::create_dir(&path)?;
        let second = source.deserialize()?;
        assert_ne!(first.tool, second.tool);
        assert_full_eq(second, &PyProjectToml::from_string(raw, "pyproject.toml")?);
        Ok(())
    }
}
