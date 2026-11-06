use std::collections::BTreeMap;
use std::env::current_dir;
use std::path::Path;

use uv_distribution_types::IndexUrl;
use uv_lock::{Lock, PublishLock};
use uv_pep508::VerbatimUrl;
use uv_pypi_types::{PyProjectToml, ResolutionMetadata};
use uv_redacted::SafeUrl;

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
    let error = PublishLock::from_lock(&lock, Path::new("."), &metadata, false)
        .expect_err("the package selected by the override must satisfy the published requirement");
    insta::assert_snapshot!(error, @"Cannot export `tool` from `uv.lock`: the resolution does not satisfy `helper>=2`");
    Ok(())
}

/// The original requirements of a registry dependency may be absent from the workspace lock.
#[test]
fn publish_rejects_transitive_overrides() -> Result<(), Box<dyn std::error::Error>> {
    let lock = Lock::from_toml(
        r#"
version = 1
revision = 5
requires-python = ">=3.12"

[manifest]
overrides = [{ name = "leaf", specifier = "==1" }]

[[package]]
name = "tool"
version = "1.0"
source = { editable = "." }
dependencies = [{ name = "middle" }]

[[package]]
name = "middle"
version = "1.0"
source = { registry = "https://pypi.org/simple" }
dependencies = [{ name = "leaf" }]
wheels = [{ url = "https://example.com/packages/middle-1.0-py3-none-any.whl", hash = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" }]

[[package]]
name = "leaf"
version = "1.0"
source = { registry = "https://pypi.org/simple" }
wheels = [{ url = "https://example.com/packages/leaf-1.0-py3-none-any.whl", hash = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" }]
"#,
    )?;
    let pyproject = PyProjectToml::from_toml(
        r#"
[project]
name = "tool"
version = "1.0"
requires-python = ">=3.12"
dependencies = ["middle>=1"]
"#,
        "pyproject.toml",
    )?;
    let metadata = ResolutionMetadata::parse_pyproject_toml(pyproject, None)?;
    let error = PublishLock::from_lock(&lock, Path::new("."), &metadata, false)
        .expect_err("the registry dependency's original requirements may differ");
    insta::assert_snapshot!(error, @"Cannot export `tool` from `uv.lock`: dependency overrides, exclusions, or metadata overrides can change its published requirements");
    Ok(())
}

#[test]
fn publish_rejects_signed_registry_artifact() -> Result<(), Box<dyn std::error::Error>> {
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

[[package]]
name = "helper"
version = "1.0"
source = { registry = "https://example.com/simple" }
wheels = [{ url = "https://example.com/packages/helper-1.0-py3-none-any.whl?signature=secret", hash = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" }]
"#,
    )?;
    let pyproject = PyProjectToml::from_toml(
        r#"
[project]
name = "tool"
version = "1.0"
requires-python = ">=3.12"
dependencies = ["helper"]
"#,
        "pyproject.toml",
    )?;
    let metadata = ResolutionMetadata::parse_pyproject_toml(pyproject, None)?;
    let error = PublishLock::from_lock(&lock, Path::new("."), &metadata, true)
        .expect_err("signed URLs cannot be distributed in the lock");
    insta::assert_snapshot!(error, @"Cannot export artifact `helper-1.0-py3-none-any.whl` for `helper` with a query string in its URL");
    Ok(())
}

#[test]
fn publish_rejects_signed_registry_sdist() -> Result<(), Box<dyn std::error::Error>> {
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

[[package]]
name = "helper"
version = "1.0"
source = { registry = "https://example.com/simple" }
sdist = { url = "https://example.com/packages/helper-1.0.tar.gz?signature=secret", hash = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" }
"#,
    )?;
    let pyproject = PyProjectToml::from_toml(
        r#"
[project]
name = "tool"
version = "1.0"
requires-python = ">=3.12"
dependencies = ["helper"]
"#,
        "pyproject.toml",
    )?;
    let metadata = ResolutionMetadata::parse_pyproject_toml(pyproject, None)?;
    let error = PublishLock::from_lock(&lock, Path::new("."), &metadata, true)
        .expect_err("signed URLs cannot be distributed in the lock");
    insta::assert_snapshot!(error, @"Cannot export artifact `helper-1.0.tar.gz` for `helper` with a query string in its URL");
    Ok(())
}

#[test]
fn publish_rejects_index_queries() -> Result<(), Box<dyn std::error::Error>> {
    let url = SafeUrl::parse("https://example.com/simple?signature=secret")?;
    let index = IndexUrl::from(VerbatimUrl::from_url(url));
    let error = PublishLock::check_publish_index(&index, true)
        .expect_err("the embedded index URL must be durable");
    insta::assert_snapshot!(error, @"Cannot export an index URL with a query string");
    Ok(())
}

#[test]
fn publish_requires_workspace_artifacts_to_finalize() -> Result<(), Box<dyn std::error::Error>> {
    let lock = Lock::from_toml(
        r#"
version = 1
revision = 5
requires-python = ">=3.12"

[manifest]
members = ["helper", "tool"]

[[package]]
name = "tool"
version = "1.0"
source = { editable = "." }
dependencies = [{ name = "helper" }]

[[package]]
name = "helper"
version = "1.0"
source = { editable = "packages/helper" }
"#,
    )?;
    let pyproject = PyProjectToml::from_toml(
        r#"
[project]
name = "tool"
version = "1.0"
requires-python = ">=3.12"
dependencies = ["helper"]
"#,
        "pyproject.toml",
    )?;
    let metadata = ResolutionMetadata::parse_pyproject_toml(pyproject, None)?;
    let prepared = PublishLock::from_lock(&lock, &current_dir()?, &metadata, false)?;
    let index = IndexUrl::from(VerbatimUrl::from_url(SafeUrl::parse(
        "https://pypi.org/simple",
    )?));
    let published = BTreeMap::from([("helper".parse()?, Vec::new())]);
    let error = prepared
        .finalize(Some(&index), &published)
        .expect_err("every workspace entry needs at least one registry artifact");
    insta::assert_snapshot!(error, @"No published files found for `helper==1.0`");
    Ok(())
}
