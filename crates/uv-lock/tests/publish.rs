use std::path::Path;

use uv_lock::{Lock, PylockToml};
use uv_pypi_types::{PyProjectToml, ResolutionMetadata};

/// Overrides may change a resolved package without changing the distribution's requirements.
#[test]
fn publish_rejects_incompatible_resolved_version() -> Result<(), Box<dyn std::error::Error>> {
    let lock = Lock::from_toml(
        r#"
version = 1
revision = 5
requires-python = ">=3.12"

[[package]]
name = "tool"
version = "1.0"
source = { editable = "." }
dependencies = [{ name = "helper" }]
[package.metadata]
requires-dist = [{ name = "helper", specifier = ">=2" }]

[[package]]
name = "helper"
version = "1.0"
source = { registry = "https://pypi.org/simple" }
wheels = [{ url = "https://example.com/packages/helper-1.0-py3-none-any.whl", hash = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" }]
"#,
    )?;
    let pyproject = PyProjectToml::from_toml(
        r#"
[project]
name = "tool"
version = "1.0"
requires-python = ">=3.12"
dependencies = ["helper>=2"]
"#,
        "pyproject.toml",
    )?;
    let metadata = ResolutionMetadata::parse_pyproject_toml(pyproject, None)?;
    let error = PylockToml::from_publish_lock(&lock, Path::new("."), &metadata, false)
        .expect_err("the package selected by the override must satisfy the published requirement");
    insta::assert_snapshot!(error, @"Cannot export `tool` from `uv.lock`: the resolution does not satisfy `helper>=2`");
    Ok(())
}
