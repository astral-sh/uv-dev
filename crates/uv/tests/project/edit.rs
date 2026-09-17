#![expect(clippy::disallowed_types)]

#[cfg(feature = "test-git")]
mod conditional_imports {
    pub(crate) use uv_test::{READ_ONLY_GITHUB_TOKEN, decode_token};
}

#[cfg(feature = "test-git")]
use conditional_imports::*;

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};
use insta::assert_snapshot;
use serde_json::json;
use std::path::Path;
use url::Url;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[cfg(feature = "test-git-lfs")]
use uv_cache_key::{RepositoryUrl, cache_digest};
use uv_fs::Simplified;
use uv_static::EnvVars;

use uv_test::{apply_filters, uv_snapshot, venv_bin_path};

/// Add a PyPI requirement.
#[test]
fn add_registry() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio==3.7.0"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "anyio==3.7.0",
        ]
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "3.7.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-3.7.0.tar.gz", hash = "sha256:c8f99c47f03aec932b6cee4178beb10ce5b0aaf6d3e1ff52cc5e49fc3186af0a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-3.7.0-py3-none-any.whl", hash = "sha256:ea75fecadcfa9b11a8bfa2ff25ea52a2904950d4925ba758c97d97e32c314556", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "anyio" },
        ]

        [package.metadata]
        requires-dist = [{ name = "anyio", specifier = "==3.7.0" }]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 3 packages in [TIME]
    ");

    Ok(())
}

/// Add a Git requirement.
#[test]
#[cfg(feature = "test-git")]
fn add_git() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio==3.7.0"]
    "#})?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    // Adding with an ambiguous Git reference should treat it as a revision.
    uv_snapshot!(context.filters(), context.add().arg("uv-public-pypackage @ git+https://github.com/astral-test/uv-public-pypackage@0.0.1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage@0dacfd662c64cb4ceb16e6cf65a157a8b715b979)
    ");

    uv_snapshot!(context.filters(), context.add().arg("uv-public-pypackage @ git+https://github.com/astral-test/uv-public-pypackage").arg("--tag=0.0.1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Checked 4 packages in [TIME]
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "anyio==3.7.0",
            "uv-public-pypackage",
        ]

        [tool.uv.sources]
        uv-public-pypackage = { git = "https://github.com/astral-test/uv-public-pypackage", tag = "0.0.1" }
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "3.7.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-3.7.0.tar.gz", hash = "sha256:c8f99c47f03aec932b6cee4178beb10ce5b0aaf6d3e1ff52cc5e49fc3186af0a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-3.7.0-py3-none-any.whl", hash = "sha256:ea75fecadcfa9b11a8bfa2ff25ea52a2904950d4925ba758c97d97e32c314556", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "anyio" },
            { name = "uv-public-pypackage" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "anyio", specifier = "==3.7.0" },
            { name = "uv-public-pypackage", git = "https://github.com/astral-test/uv-public-pypackage?tag=0.0.1" },
        ]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "uv-public-pypackage"
        version = "0.1.0"
        source = { git = "https://github.com/astral-test/uv-public-pypackage?tag=0.0.1#0dacfd662c64cb4ceb16e6cf65a157a8b715b979" }
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 4 packages in [TIME]
    ");

    Ok(())
}

/// Add a Git requirement from a private repository, with credentials. The resolution should
/// succeed, but the `pyproject.toml` should omit the credentials.
#[test]
#[cfg(feature = "test-git")]
fn add_git_private_source() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let token = decode_token(READ_ONLY_GITHUB_TOKEN);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg(format!("uv-private-pypackage @ git+https://{token}@github.com/astral-test/uv-private-pypackage")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv-private-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-private-pypackage@d780faf0ac91257d4d5a4f0c5a0e4509608c0071)
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "uv-private-pypackage",
        ]

        [tool.uv.sources]
        uv-private-pypackage = { git = "https://github.com/astral-test/uv-private-pypackage" }
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "uv-private-pypackage" },
        ]

        [package.metadata]
        requires-dist = [{ name = "uv-private-pypackage", git = "https://github.com/astral-test/uv-private-pypackage" }]

        [[package]]
        name = "uv-private-pypackage"
        version = "0.1.0"
        source = { git = "https://github.com/astral-test/uv-private-pypackage#d780faf0ac91257d4d5a4f0c5a0e4509608c0071" }
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");

    Ok(())
}

/// Add a Git requirement from a private repository, with credentials. Since `--raw-sources` is
/// specified, the `pyproject.toml` should retain the credentials.
#[test]
#[cfg(feature = "test-git")]
fn add_git_private_raw() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let token = decode_token(READ_ONLY_GITHUB_TOKEN);
    let context = context.with_filter((&token, "***"));

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg(format!("uv-private-pypackage @ git+https://{token}@github.com/astral-test/uv-private-pypackage")).arg("--raw-sources"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv-private-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-private-pypackage@d780faf0ac91257d4d5a4f0c5a0e4509608c0071)
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters()
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "uv-private-pypackage @ git+https://***@github.com/astral-test/uv-private-pypackage",
        ]
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "uv-private-pypackage" },
        ]

        [package.metadata]
        requires-dist = [{ name = "uv-private-pypackage", git = "https://github.com/astral-test/uv-private-pypackage" }]

        [[package]]
        name = "uv-private-pypackage"
        version = "0.1.0"
        source = { git = "https://github.com/astral-test/uv-private-pypackage#d780faf0ac91257d4d5a4f0c5a0e4509608c0071" }
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");

    Ok(())
}

#[tokio::test]
#[cfg(feature = "test-git")]
async fn add_git_private_rate_limited_by_github_rest_api_403_response() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let token = decode_token(READ_ONLY_GITHUB_TOKEN);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&server)
        .await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context
        .add()
        .arg(format!("uv-private-pypackage @ git+https://{token}@github.com/astral-test/uv-private-pypackage"))
        .env(EnvVars::UV_GITHUB_FAST_PATH_URL, server.uri()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv-private-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-private-pypackage@d780faf0ac91257d4d5a4f0c5a0e4509608c0071)
    ");

    Ok(())
}

#[tokio::test]
#[cfg(feature = "test-git")]
async fn add_git_private_rate_limited_by_github_rest_api_429_response() -> Result<()> {
    use uv_client::DEFAULT_RETRIES;

    let context = uv_test::test_context!("3.12");
    let token = decode_token(READ_ONLY_GITHUB_TOKEN);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429))
        .expect(1 + u64::from(DEFAULT_RETRIES)) // Middleware retries on 429 by default
        .mount(&server)
        .await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context
        .add()
        .arg(format!("uv-private-pypackage @ git+https://{token}@github.com/astral-test/uv-private-pypackage"))
        .env(EnvVars::UV_GITHUB_FAST_PATH_URL, server.uri())
        .env(EnvVars::UV_TEST_NO_HTTP_RETRY_DELAY, "true"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv-private-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-private-pypackage@d780faf0ac91257d4d5a4f0c5a0e4509608c0071)
    ");

    Ok(())
}

#[test]
#[cfg(feature = "test-git")]
fn add_git_error() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");

    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");

    // Provide a tag without a Git source.
    uv_snapshot!(context.filters(), context.add().arg("flask").arg("--tag").arg("0.0.1"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `flask` did not resolve to a Git repository, but a Git reference (`--tag 0.0.1`) was provided.
    ");

    // Provide a tag with a non-Git source.
    uv_snapshot!(context.filters(), context.add().arg(format!("flask @ {}", server.file_url("flask-3.0.2-py3-none-any.whl"))).arg("--branch").arg("0.0.1"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `flask` did not resolve to a Git repository, but a Git reference (`--branch 0.0.1`) was provided.
    ");

    Ok(())
}

#[test]
#[cfg(feature = "test-git")]
fn add_git_branch() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("uv-public-pypackage @ git+https://github.com/astral-test/uv-public-pypackage").arg("--branch").arg("test-branch"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage@0dacfd662c64cb4ceb16e6cf65a157a8b715b979)
    ");

    Ok(())
}

#[test]
#[cfg(feature = "test-git")]
fn add_git_unnamed_partial_static_metadata_no_build() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        no-binary-package = ["dynamic-requires-python-tool"]
        no-build = true
    "#})?;

    // The static project name should allow the package-specific exception to apply; see
    // astral-sh/uv-dev#732.
    uv_snapshot!(context.filters(), context.add()
        .arg("git+https://github.com/astral-sh/uv-dynamic-requires-python-test@75a612dc87fc215e999a25a0efc376cbf9831afa#subdirectory=dynamic"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Building source distributions is disabled
    ");

    Ok(())
}

#[test]
#[cfg(feature = "test-git-lfs")]
fn add_git_lfs() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_git_lfs_config();

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = []
    "#})?;

    // Gather cache locations
    let git_cache = context.cache_dir.child("git-v0");
    let git_checkouts = git_cache.child("checkouts");
    let git_db = git_cache.child("db");
    let repo_url = RepositoryUrl::parse("https://github.com/astral-sh/test-lfs-repo")?;
    let lfs_db_bucket_objects = git_db
        .child(cache_digest(&repo_url))
        .child(".git")
        .child("lfs");
    let ok_checkout_file = git_checkouts
        .child(cache_digest(&repo_url.with_lfs(Some(true))))
        .child("261c828")
        .child(".ok");

    uv_snapshot!(context.filters(), context.add()
        .arg("--no-cache")
        .arg("test-lfs-repo @ git+https://github.com/astral-sh/test-lfs-repo")
        .arg("--rev").arg("261c828b8e05251f3a3e4f6b47b149d691c7efbb")
        .arg("--lfs"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + test-lfs-repo==0.1.0 (from git+https://github.com/astral-sh/test-lfs-repo@261c828b8e05251f3a3e4f6b47b149d691c7efbb#lfs=true)
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = [
            "test-lfs-repo",
        ]

        [tool.uv.sources]
        test-lfs-repo = { git = "https://github.com/astral-sh/test-lfs-repo", rev = "261c828b8e05251f3a3e4f6b47b149d691c7efbb", lfs = true }
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.13"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "test-lfs-repo" },
        ]

        [package.metadata]
        requires-dist = [{ name = "test-lfs-repo", git = "https://github.com/astral-sh/test-lfs-repo?lfs=true&rev=261c828b8e05251f3a3e4f6b47b149d691c7efbb" }]

        [[package]]
        name = "test-lfs-repo"
        version = "0.1.0"
        source = { git = "https://github.com/astral-sh/test-lfs-repo?lfs=true&rev=261c828b8e05251f3a3e4f6b47b149d691c7efbb#261c828b8e05251f3a3e4f6b47b149d691c7efbb" }
        "#
        );
    });

    // Change revision as an unnamed requirement
    uv_snapshot!(context.filters(), context.add()
        .arg("--no-cache")
        .arg("git+https://github.com/astral-sh/test-lfs-repo")
        .arg("--rev").arg("214e0b7be16b3c7e5f72f475c11bf26f48ea82d4")
        .arg("--lfs"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - test-lfs-repo==0.1.0 (from git+https://github.com/astral-sh/test-lfs-repo@261c828b8e05251f3a3e4f6b47b149d691c7efbb#lfs=true)
     + test-lfs-repo==0.1.0 (from git+https://github.com/astral-sh/test-lfs-repo@214e0b7be16b3c7e5f72f475c11bf26f48ea82d4#lfs=true)
    ");

    // Test LFS not found scenario resulting in an incomplete fetch cache

    // The filters below will remove any boilerplate before what we actually want to match.
    // They help handle slightly different output in uv-distribution/src/source/mod.rs between
    // calls to `git` and `git_metadata` functions which don't have guaranteed execution order.
    // In addition, we can get different error codes depending on where the failure occurs,
    // although we know the error code cannot be 0.
    let context = context
        .with_filter((r"exit_code: -?[1-9]\d*", "exit_code: [ERROR_CODE]"))
        .with_filter((
            "(?s)(----- stderr -----).*?The source distribution `[^`]+` is missing Git LFS artifacts.*",
            "$1\n[PREFIX]The source distribution `[DISTRIBUTION]` is missing Git LFS artifacts",
        ));

    uv_snapshot!(context.filters(), context.add()
        .env(EnvVars::UV_INTERNAL__TEST_LFS_DISABLED, "1")
        .arg("git+https://github.com/astral-sh/test-lfs-repo")
        .arg("--rev").arg("261c828b8e05251f3a3e4f6b47b149d691c7efbb")
        .arg("--lfs"), @"
    exit_code: [ERROR_CODE] (failure)
    ----- stderr -----
    [PREFIX]The source distribution `[DISTRIBUTION]` is missing Git LFS artifacts
    ");

    // There should be no .ok entry as LFS operations failed
    assert!(!ok_checkout_file.exists(), "Found unexpected .ok file.");

    // Test LFS recovery from an incomplete fetch cache
    uv_snapshot!(context.filters(), context.add()
        .arg("git+https://github.com/astral-sh/test-lfs-repo")
        .arg("--rev").arg("261c828b8e05251f3a3e4f6b47b149d691c7efbb")
        .arg("--lfs"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - test-lfs-repo==0.1.0 (from git+https://github.com/astral-sh/test-lfs-repo@214e0b7be16b3c7e5f72f475c11bf26f48ea82d4#lfs=true)
     + test-lfs-repo==0.1.0 (from git+https://github.com/astral-sh/test-lfs-repo@261c828b8e05251f3a3e4f6b47b149d691c7efbb#lfs=true)
    ");

    // Verify that we can import the module and access LFS content
    uv_snapshot!(context.filters(), context.python_command()
        .arg("-c")
        .arg("import test_lfs_repo.lfs_module"), @"
    exit_code: 0 (success)
    ");

    // Now let's delete some of the LFS entries from our db...
    fs_err::remove_file(&ok_checkout_file)?;
    fs_err::remove_dir_all(&lfs_db_bucket_objects)?;

    // Test LFS recovery from an incomplete db and non-fresh checkout
    uv_snapshot!(context.filters(), context.add()
        .arg("git+https://github.com/astral-sh/test-lfs-repo")
        .arg("--rev").arg("261c828b8e05251f3a3e4f6b47b149d691c7efbb")
        .arg("--reinstall")
        .arg("--lfs"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ test-lfs-repo==0.1.0 (from git+https://github.com/astral-sh/test-lfs-repo@261c828b8e05251f3a3e4f6b47b149d691c7efbb#lfs=true)
    ");

    // Verify that we can import the module and access LFS content
    uv_snapshot!(context.filters(), context.python_command()
        .arg("-c")
        .arg("import test_lfs_repo.lfs_module"), @"
    exit_code: 0 (success)
    ");

    // Verify our db and checkout recovered
    assert!(ok_checkout_file.exists());
    assert!(lfs_db_bucket_objects.exists());

    // Exercise the sdist cache
    uv_snapshot!(context.filters(), context.add()
        .arg("git+https://github.com/astral-sh/test-lfs-repo")
        .arg("--rev").arg("261c828b8e05251f3a3e4f6b47b149d691c7efbb")
        .arg("--lfs"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Checked 1 package in [TIME]
    ");

    Ok(())
}

/// Add a Git requirement using the `--raw-sources` API.
#[test]
#[cfg(feature = "test-git")]
fn add_git_raw() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio==3.7.0"]
    "#})?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    // Use an ambiguous tag reference, which would otherwise not resolve.
    uv_snapshot!(context.filters(), context.add().arg("uv-public-pypackage @ git+https://github.com/astral-test/uv-public-pypackage@0.0.1").arg("--raw-sources"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage@0dacfd662c64cb4ceb16e6cf65a157a8b715b979)
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "anyio==3.7.0",
            "uv-public-pypackage @ git+https://github.com/astral-test/uv-public-pypackage@0.0.1",
        ]
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "3.7.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-3.7.0.tar.gz", hash = "sha256:c8f99c47f03aec932b6cee4178beb10ce5b0aaf6d3e1ff52cc5e49fc3186af0a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-3.7.0-py3-none-any.whl", hash = "sha256:ea75fecadcfa9b11a8bfa2ff25ea52a2904950d4925ba758c97d97e32c314556", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "anyio" },
            { name = "uv-public-pypackage" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "anyio", specifier = "==3.7.0" },
            { name = "uv-public-pypackage", git = "https://github.com/astral-test/uv-public-pypackage?rev=0.0.1" },
        ]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "uv-public-pypackage"
        version = "0.1.0"
        source = { git = "https://github.com/astral-test/uv-public-pypackage?rev=0.0.1#0dacfd662c64cb4ceb16e6cf65a157a8b715b979" }
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 4 packages in [TIME]
    ");

    Ok(())
}

/// Add a Git requirement without the `git+` prefix.
#[test]
#[cfg(feature = "test-git")]
fn add_git_implicit() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio==3.7.0"]
    "#})?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    // Omit the `git+` prefix.
    uv_snapshot!(context.filters(), context.add().arg("uv-public-pypackage @ https://github.com/astral-test/uv-public-pypackage.git"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage.git@b270df1a2fb5d012294e9aaf05e7e0bab1e6a389)
    ");

    Ok(())
}

/// `--raw-sources` should be considered conflicting with sources-specific arguments, like `--tag`.
#[test]
#[cfg(feature = "test-git")]
fn add_raw_error() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Provide a tag without a Git source.
    uv_snapshot!(context.filters(), context.add().arg("uv-public-pypackage @ git+https://github.com/astral-test/uv-public-pypackage").arg("--tag").arg("0.0.1").arg("--raw-sources"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--tag <TAG>' cannot be used with '--raw'

    Usage: uv add --cache-dir [CACHE_DIR] --tag <TAG> --exclude-newer <EXCLUDE_NEWER> <PACKAGES|--requirements <REQUIREMENTS>>

    For more information, try '--help'.
    ");

    Ok(())
}

#[test]
fn reinstall_local_source_trees() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let project_1 = context.temp_dir.child("project1");
    project_1.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project1"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    let project_2 = context.temp_dir.child("project2");
    project_2.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project2"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().current_dir(&project_1).arg("../project2").arg("--editable"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + project2==0.1.0 (from file://[TEMP_DIR]/project2)
    ");

    // Running `uv add` should reinstall the project.
    uv_snapshot!(context.filters(), context.add().current_dir(&project_1).arg("../project2").arg("--editable"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ project2==0.1.0 (from file://[TEMP_DIR]/project2)
    ");

    Ok(())
}

#[test]
#[cfg(feature = "test-git")]
fn add_editable_error() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Provide `--editable` with a non-source tree.
    uv_snapshot!(context.filters(), context.add().arg(format!("flask @ {}", server.file_url("flask-3.0.2-py3-none-any.whl"))).arg("--editable"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `flask` did not resolve to a local directory, but the `--editable` flag was provided. Editable installs are only supported for local directories.
    ");

    Ok(())
}

/// Add an unnamed requirement.
#[test]
#[cfg(feature = "test-git")]
fn add_unnamed() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("git+https://github.com/astral-test/uv-public-pypackage").arg("--tag=0.0.1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage@0dacfd662c64cb4ceb16e6cf65a157a8b715b979)
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "uv-public-pypackage",
        ]

        [tool.uv.sources]
        uv-public-pypackage = { git = "https://github.com/astral-test/uv-public-pypackage", tag = "0.0.1" }
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "uv-public-pypackage" },
        ]

        [package.metadata]
        requires-dist = [{ name = "uv-public-pypackage", git = "https://github.com/astral-test/uv-public-pypackage?tag=0.0.1" }]

        [[package]]
        name = "uv-public-pypackage"
        version = "0.1.0"
        source = { git = "https://github.com/astral-test/uv-public-pypackage?tag=0.0.1#0dacfd662c64cb4ceb16e6cf65a157a8b715b979" }
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");

    Ok(())
}

/// Add and remove a development dependency.
#[test]
fn add_remove_dev() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio==3.7.0").arg("--dev"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [dependency-groups]
        dev = [
            "anyio==3.7.0",
        ]
        "#
        );
    });

    // `uv add` implies a full lock and sync, including development dependencies.
    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "3.7.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-3.7.0.tar.gz", hash = "sha256:c8f99c47f03aec932b6cee4178beb10ce5b0aaf6d3e1ff52cc5e49fc3186af0a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-3.7.0-py3-none-any.whl", hash = "sha256:ea75fecadcfa9b11a8bfa2ff25ea52a2904950d4925ba758c97d97e32c314556", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.dev-dependencies]
        dev = [
            { name = "anyio" },
        ]

        [package.metadata]

        [package.metadata.requires-dev]
        dev = [{ name = "anyio", specifier = "==3.7.0" }]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 3 packages in [TIME]
    ");

    // This should fail without --dev.
    uv_snapshot!(context.filters(), context.remove().arg("anyio"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The dependency `anyio` could not be found in `project.dependencies`

    hint: `anyio` is in the `dev` group (try: `uv remove anyio --group dev`)
    ");

    // Remove the dependency.
    uv_snapshot!(context.filters(), context.remove().arg("anyio").arg("--dev"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Uninstalled 3 packages in [TIME]
     - anyio==3.7.0
     - idna==3.6
     - sniffio==1.3.1
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [dependency-groups]
        dev = []
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.metadata]

        [package.metadata.requires-dev]
        dev = []
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");

    Ok(())
}

/// Add and remove an optional dependency.
#[test]
fn add_remove_optional() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio==3.7.0").arg("--optional=io"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        io = [
            "anyio==3.7.0",
        ]
        "#
        );
    });

    // `uv add` implies a full lock and sync, including development dependencies.
    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "3.7.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-3.7.0.tar.gz", hash = "sha256:c8f99c47f03aec932b6cee4178beb10ce5b0aaf6d3e1ff52cc5e49fc3186af0a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-3.7.0-py3-none-any.whl", hash = "sha256:ea75fecadcfa9b11a8bfa2ff25ea52a2904950d4925ba758c97d97e32c314556", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.optional-dependencies]
        io = [
            { name = "anyio" },
        ]

        [package.metadata]
        requires-dist = [{ name = "anyio", marker = "extra == 'io'", specifier = "==3.7.0" }]
        provides-extras = ["io"]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Install from the lockfile. At present, this will _uninstall_ the packages since `sync` does
    // not include extras by default.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 3 packages in [TIME]
     - anyio==3.7.0
     - idna==3.6
     - sniffio==1.3.1
    ");

    // This should fail without --optional.
    uv_snapshot!(context.filters(), context.remove().arg("anyio"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The dependency `anyio` could not be found in `project.dependencies`

    hint: `anyio` is an optional dependency (try: `uv remove anyio --optional io`)
    ");

    // Remove the dependency.
    uv_snapshot!(context.filters(), context.remove().arg("anyio").arg("--optional=io"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked in [TIME]
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        io = []
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.metadata]
        provides-extras = ["io"]
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");

    Ok(())
}

#[test]
fn add_remove_inline_optional() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
        optional-dependencies = { io = [
            "anyio==3.7.0",
        ] }
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("typing-extensions").arg("--optional=types"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + typing-extensions==4.10.0
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
        optional-dependencies = { io = [
            "anyio==3.7.0",
        ], types = [
            "typing-extensions>=4.10.0",
        ] }
        "#
        );
    });

    uv_snapshot!(context.filters(), context.remove().arg("typing-extensions").arg("--optional=types"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Uninstalled 1 package in [TIME]
     - typing-extensions==4.10.0
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
        optional-dependencies = { io = [
            "anyio==3.7.0",
        ], types = [] }
        "#
        );
    });

    Ok(())
}

/// Add and remove a workspace dependency.
#[test]
#[cfg(feature = "test-git")]
fn add_remove_workspace() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let workspace = context.temp_dir.child("pyproject.toml");
    workspace.write_str(indoc! {r#"
        [tool.uv.workspace]
        members = ["child1", "child2"]
    "#})?;

    let pyproject_toml = context.temp_dir.child("child1/pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "child1"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
    "#})?;
    context
        .temp_dir
        .child("child1")
        .child("src")
        .child("child1")
        .child("__init__.py")
        .touch()?;

    let pyproject_toml = context.temp_dir.child("child2/pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "child2"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
    "#})?;
    context
        .temp_dir
        .child("child2")
        .child("src")
        .child("child2")
        .child("__init__.py")
        .touch()?;

    // Adding a workspace package with a mismatched source should error.
    let mut add_cmd = context.add();
    add_cmd
        .arg("child2 @ git+https://github.com/astral-test/uv-public-pypackage")
        .arg("--package")
        .arg("child1");

    uv_snapshot!(context.filters(), add_cmd, @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Workspace dependency `child2` must refer to local directory, not a Git repository
    ");

    // Workspace packages should be detected automatically.
    let child1 = context.temp_dir.join("child1");
    let mut add_cmd = context.add();
    add_cmd.arg("child2").arg("--package").arg("child1");

    uv_snapshot!(context.filters(), add_cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + child1==0.1.0 (from file://[TEMP_DIR]/child1)
     + child2==0.1.0 (from file://[TEMP_DIR]/child2)
    ");

    let pyproject_toml = fs_err::read_to_string(child1.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "child1"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "child2",
        ]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"

        [tool.uv.sources]
        child2 = { workspace = true }
        "#
        );
    });

    // `uv add` implies a full lock and sync, including development dependencies.
    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        members = [
            "child1",
            "child2",
        ]

        [[package]]
        name = "child1"
        version = "0.1.0"
        source = { editable = "child1" }
        dependencies = [
            { name = "child2" },
        ]

        [package.metadata]
        requires-dist = [{ name = "child2", editable = "child2" }]

        [[package]]
        name = "child2"
        version = "0.1.0"
        source = { editable = "child2" }
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").current_dir(&child1), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 2 packages in [TIME]
    ");

    // Remove the dependency.
    uv_snapshot!(context.filters(), context.remove().arg("child2").current_dir(&child1), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 2 packages in [TIME]
    Installed 1 package in [TIME]
     ~ child1==0.1.0 (from file://[TEMP_DIR]/child1)
     - child2==0.1.0 (from file://[TEMP_DIR]/child2)
    ");

    let pyproject_toml = fs_err::read_to_string(child1.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "child1"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        members = [
            "child1",
            "child2",
        ]

        [[package]]
        name = "child1"
        version = "0.1.0"
        source = { editable = "child1" }

        [[package]]
        name = "child2"
        version = "0.1.0"
        source = { editable = "child2" }
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").current_dir(&child1), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");

    Ok(())
}

/// `uv add --dev` should update `dev-dependencies` (rather than `dependency-groups.dev`) if a
/// dependency already exists in `dev-dependencies`.
#[test]
fn update_existing_dev() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        dev-dependencies = ["anyio"]

        [dependency-groups]
        dev = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio==3.7.0").arg("--dev"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: The `tool.uv.dev-dependencies` field (used in `pyproject.toml`) is deprecated and will be removed in a future release; use `dependency-groups.dev` instead
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        dev-dependencies = [
            "anyio==3.7.0",
        ]

        [dependency-groups]
        dev = []
        "#
        );
    });

    Ok(())
}

/// `uv add --dev` should add to `dev-dependencies` (rather than `dependency-groups.dev`) if it
/// exists.
#[test]
fn add_existing_dev() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        dev-dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio==3.7.0").arg("--dev"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: The `tool.uv.dev-dependencies` field (used in `pyproject.toml`) is deprecated and will be removed in a future release; use `dependency-groups.dev` instead
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        dev-dependencies = [
            "anyio==3.7.0",
        ]
        "#
        );
    });

    Ok(())
}

/// `uv add --group dev` should update `dev-dependencies` (rather than `dependency-groups.dev`) if a
/// dependency already exists.
#[test]
fn update_existing_dev_group() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        dev-dependencies = ["anyio"]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio==3.7.0").arg("--group").arg("dev"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: The `tool.uv.dev-dependencies` field (used in `pyproject.toml`) is deprecated and will be removed in a future release; use `dependency-groups.dev` instead
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        dev-dependencies = [
            "anyio==3.7.0",
        ]
        "#
        );
    });

    Ok(())
}

/// `uv add --group dev` should add to `dependency-groups` even if `dev-dependencies` exists.
#[test]
fn add_existing_dev_group() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        dev-dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio==3.7.0").arg("--group").arg("dev"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: The `tool.uv.dev-dependencies` field (used in `pyproject.toml`) is deprecated and will be removed in a future release; use `dependency-groups.dev` instead
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        dev-dependencies = []

        [dependency-groups]
        dev = [
            "anyio==3.7.0",
        ]
        "#
        );
    });

    Ok(())
}

/// `uv remove --dev` should remove from both `dev-dependencies` and `dependency-groups.dev`.
#[test]
fn remove_both_dev() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        dev-dependencies = ["anyio"]

        [dependency-groups]
        dev = ["anyio>=3.7.0"]
    "#})?;

    uv_snapshot!(context.filters(), context.remove().arg("anyio").arg("--dev"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: The `tool.uv.dev-dependencies` field (used in `pyproject.toml`) is deprecated and will be removed in a future release; use `dependency-groups.dev` instead
    Resolved 1 package in [TIME]
    Checked in [TIME]
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        dev-dependencies = []

        [dependency-groups]
        dev = []
        "#
        );
    });

    Ok(())
}

/// Do not allow add for groups in scripts.
#[test]
fn disallow_group_script_add() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let script = context.temp_dir.child("main.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.13"
        # dependencies = []
        #
        # ///
    "#})?;

    uv_snapshot!(context.filters(), context
        .add()
        .arg("--group")
        .arg("dev")
        .arg("anyio==3.7.0")
        .arg("--script")
        .arg("main.py"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--group <GROUP>' cannot be used with '--script <SCRIPT>'

    Usage: uv add --cache-dir [CACHE_DIR] --group <GROUP> --exclude-newer <EXCLUDE_NEWER> <PACKAGES|--requirements <REQUIREMENTS>>

    For more information, try '--help'.
    ");

    Ok(())
}

/// `uv remove --group dev` should remove from both `dev-dependencies` and `dependency-groups.dev`.
#[test]
fn remove_both_dev_group() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        dev-dependencies = ["anyio"]

        [dependency-groups]
        dev = ["anyio>=3.7.0"]
    "#})?;

    uv_snapshot!(context.filters(), context.remove().arg("anyio").arg("--group").arg("dev"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: The `tool.uv.dev-dependencies` field (used in `pyproject.toml`) is deprecated and will be removed in a future release; use `dependency-groups.dev` instead
    Resolved 1 package in [TIME]
    Checked in [TIME]
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        dev-dependencies = []

        [dependency-groups]
        dev = []
        "#
        );
    });

    Ok(())
}

/// Add a workspace dependency as an editable.
#[test]
fn add_workspace_editable() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let workspace = context.temp_dir.child("pyproject.toml");
    workspace.write_str(indoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv.workspace]
        members = ["child1", "child2"]
    "#})?;

    let pyproject_toml = context.temp_dir.child("child1/pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "child1"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
    "#})?;
    context
        .temp_dir
        .child("child1")
        .child("src")
        .child("child1")
        .child("__init__.py")
        .touch()?;

    let pyproject_toml = context.temp_dir.child("child2/pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "child2"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
    "#})?;
    context
        .temp_dir
        .child("child2")
        .child("src")
        .child("child2")
        .child("__init__.py")
        .touch()?;

    let child1 = context.temp_dir.join("child1");

    // `--no-editable` should add `editable = false`.
    let mut add_cmd = context.add();
    add_cmd
        .arg("child2")
        .arg("--no-editable")
        .current_dir(&child1);

    uv_snapshot!(context.filters(), add_cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + child1==0.1.0 (from file://[TEMP_DIR]/child1)
     + child2==0.1.0 (from file://[TEMP_DIR]/child2)
    ");

    let pyproject_toml = fs_err::read_to_string(child1.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "child1"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "child2",
        ]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"

        [tool.uv.sources]
        child2 = { workspace = true, editable = false }
        "#
        );
    });

    // `--editable` should not.
    let mut add_cmd = context.add();
    add_cmd.arg("child2").arg("--editable").current_dir(&child1);

    uv_snapshot!(context.filters(), add_cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 2 packages in [TIME]
    Uninstalled 2 packages in [TIME]
    Installed 2 packages in [TIME]
     ~ child1==0.1.0 (from file://[TEMP_DIR]/child1)
     ~ child2==0.1.0 (from file://[TEMP_DIR]/child2)
    ");

    let pyproject_toml = fs_err::read_to_string(child1.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "child1"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "child2",
        ]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"

        [tool.uv.sources]
        child2 = { workspace = true, editable = true }
        "#
        );
    });

    // `uv add` implies a full lock and sync, including development dependencies.
    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        members = [
            "child1",
            "child2",
            "parent",
        ]

        [[package]]
        name = "child1"
        version = "0.1.0"
        source = { editable = "child1" }
        dependencies = [
            { name = "child2" },
        ]

        [package.metadata]
        requires-dist = [{ name = "child2", editable = "child2" }]

        [[package]]
        name = "child2"
        version = "0.1.0"
        source = { editable = "child2" }

        [[package]]
        name = "parent"
        version = "0.1.0"
        source = { virtual = "." }
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").current_dir(&child1), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 2 packages in [TIME]
    ");

    Ok(())
}

/// Add a workspace dependency via its path.
#[test]
fn add_workspace_path() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let workspace = context.temp_dir.child("pyproject.toml");
    workspace.write_str(indoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv.workspace]
        members = ["child"]
    "#})?;

    let pyproject_toml = context.temp_dir.child("child/pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
    "#})?;
    context
        .temp_dir
        .child("child")
        .child("src")
        .child("child")
        .child("__init__.py")
        .touch()?;

    uv_snapshot!(context.filters(), context.add().arg("./child"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + child==0.1.0 (from file://[TEMP_DIR]/child)
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "child",
        ]

        [tool.uv.workspace]
        members = ["child"]

        [tool.uv.sources]
        child = { workspace = true }
        "#
        );
    });

    // `uv add` implies a full lock and sync, including development dependencies.
    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        members = [
            "child",
            "parent",
        ]

        [[package]]
        name = "child"
        version = "0.1.0"
        source = { editable = "child" }

        [[package]]
        name = "parent"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "child" },
        ]

        [package.metadata]
        requires-dist = [{ name = "child", editable = "child" }]
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");

    Ok(())
}

/// Add a path dependency, which should be implicitly added to the workspace.
#[test]
fn add_path_implicit_workspace() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let workspace = context.temp_dir.child("workspace");
    workspace.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    let child = workspace.child("packages").child("child");
    child.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
    "#})?;
    workspace
        .child("packages")
        .child("child")
        .child("src")
        .child("child")
        .child("__init__.py")
        .touch()?;

    uv_snapshot!(context.filters(), context.add().arg(Path::new("packages").join("child")).current_dir(workspace.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Added `packages/child` to workspace members
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + child==0.1.0 (from file://[TEMP_DIR]/workspace/packages/child)
    ");

    let pyproject_toml = fs_err::read_to_string(workspace.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "child",
        ]

        [tool.uv.workspace]
        members = [
            "packages/child",
        ]

        [tool.uv.sources]
        child = { workspace = true }
        "#
        );
    });

    // `uv add` implies a full lock and sync, including development dependencies.
    let lock = fs_err::read_to_string(workspace.join("uv.lock"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        members = [
            "child",
            "parent",
        ]

        [[package]]
        name = "child"
        version = "0.1.0"
        source = { editable = "packages/child" }

        [[package]]
        name = "parent"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "child" },
        ]

        [package.metadata]
        requires-dist = [{ name = "child", editable = "packages/child" }]
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").current_dir(workspace.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");

    Ok(())
}

/// Add a path dependency with `--no-workspace`, which should not be added to the workspace.
#[test]
fn add_path_no_workspace() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let workspace = context.temp_dir.child("workspace");
    workspace.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    let child = workspace.child("packages").child("child");
    child.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
    "#})?;
    workspace
        .child("packages")
        .child("child")
        .child("src")
        .child("child")
        .child("__init__.py")
        .touch()?;

    uv_snapshot!(context.filters(), context.add().arg(Path::new("packages").join("child")).current_dir(workspace.path()).arg("--no-workspace"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + child==0.1.0 (from file://[TEMP_DIR]/workspace/packages/child)
    ");

    let pyproject_toml = fs_err::read_to_string(workspace.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "child",
        ]

        [tool.uv.sources]
        child = { path = "packages/child" }
        "#
        );
    });

    // `uv add` implies a full lock and sync, including development dependencies.
    let lock = fs_err::read_to_string(workspace.join("uv.lock"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "child"
        version = "0.1.0"
        source = { directory = "packages/child" }

        [[package]]
        name = "parent"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "child" },
        ]

        [package.metadata]
        requires-dist = [{ name = "child", directory = "packages/child" }]
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").current_dir(workspace.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");

    Ok(())
}

/// Add a path dependency in an adjacent directory, which should not be added to the workspace.
#[test]
fn add_path_adjacent_directory() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let project = context.temp_dir.child("project");
    project.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    let dependency = context.temp_dir.child("dependency");
    dependency.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "dependency"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
    "#})?;
    dependency
        .child("src")
        .child("dependency")
        .child("__init__.py")
        .touch()?;

    uv_snapshot!(context.filters(), context.add().arg(dependency.path()).current_dir(project.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + dependency==0.1.0 (from file://[TEMP_DIR]/dependency)
    ");

    let pyproject_toml = fs_err::read_to_string(project.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "dependency",
        ]

        [tool.uv.sources]
        dependency = { path = "[TEMP_DIR]/dependency" }
        "#
        );
    });

    // `uv add` implies a full lock and sync, including development dependencies.
    let lock = fs_err::read_to_string(project.join("uv.lock"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "dependency"
        version = "0.1.0"
        source = { directory = "[TEMP_DIR]/dependency" }

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "dependency" },
        ]

        [package.metadata]
        requires-dist = [{ name = "dependency", directory = "[TEMP_DIR]/dependency" }]
        "#
        );
    });

    Ok(())
}

/// Check relative and absolute path handling with `uv add`.
///
/// When a user provides an absolute path or `file://` URL, it should be preserved as absolute
/// in pyproject.toml and uv.lock. Relative paths should remain relative.
///
/// See: <https://github.com/astral-sh/uv/issues/17307>
#[test]
fn add_relative_and_absolute_paths() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let project = context.temp_dir.child("project");
    project.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Create a dependency at a relative path (sibling directory).
    let relative_dep = context.temp_dir.child("relative_dep");
    relative_dep.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "relative-dep"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;
    relative_dep
        .child("src")
        .child("relative_dep")
        .child("__init__.py")
        .touch()?;

    // Create a dependency at an absolute path (using the full temp_dir path).
    let absolute_dep = context.temp_dir.child("absolute_dep");
    absolute_dep.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "absolute-dep"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;
    absolute_dep
        .child("src")
        .child("absolute_dep")
        .child("__init__.py")
        .touch()?;

    // Create a dependency that will be added via a file:// URL.
    let file_url_dep = context.temp_dir.child("file_url_dep");
    file_url_dep.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "file-url-dep"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;
    file_url_dep
        .child("src")
        .child("file_url_dep")
        .child("__init__.py")
        .touch()?;

    // Create a dependency that will be added via a file:// URL containing an expanded variable.
    let expanded_dep = context.temp_dir.child("expanded_dep");
    expanded_dep.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "expanded-dep"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
    "#})?;
    expanded_dep
        .child("src")
        .child("expanded_dep")
        .child("__init__.py")
        .touch()?;

    // Add the relative dependency using a relative path.
    uv_snapshot!(context.filters(), context.add().arg("../relative_dep").current_dir(project.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + relative-dep==0.1.0 (from file://[TEMP_DIR]/relative_dep)
    ");

    // Add the absolute dependency using an absolute path.
    uv_snapshot!(context.filters(), context.add().arg(absolute_dep.path()).current_dir(project.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + absolute-dep==0.1.0 (from file://[TEMP_DIR]/absolute_dep)
    ");

    // Add a dependency using a file:// URL (also absolute).
    let file_url = Url::from_file_path(file_url_dep.path()).unwrap();
    uv_snapshot!(context.filters(), context.add().arg(file_url.as_str()).current_dir(project.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + file-url-dep==0.1.0 (from file://[TEMP_DIR]/file_url_dep)
    ");

    // Expanded variables retain the portability behavior from #18680 and stay relative.
    uv_snapshot!(context.filters(), context.add().arg("file:///${PROJECT_ROOT}/../expanded_dep").current_dir(project.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + expanded-dep==0.1.0 (from file://[TEMP_DIR]/expanded_dep)
    ");

    // Check pyproject.toml - relative paths stay relative, absolute paths and file:// URLs
    // stay absolute.
    let pyproject_toml = fs_err::read_to_string(project.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "absolute-dep",
            "expanded-dep",
            "file-url-dep",
            "relative-dep",
        ]

        [tool.uv.sources]
        relative-dep = { path = "../relative_dep" }
        absolute-dep = { path = "[TEMP_DIR]/absolute_dep" }
        file-url-dep = { path = "[TEMP_DIR]/file_url_dep" }
        expanded-dep = { path = "../expanded_dep" }
        "#
        );
    });

    // Check uv.lock - relative paths stay relative, absolute paths stay absolute.
    let lock = fs_err::read_to_string(project.join("uv.lock"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "absolute-dep"
        version = "0.1.0"
        source = { directory = "[TEMP_DIR]/absolute_dep" }

        [[package]]
        name = "expanded-dep"
        version = "0.1.0"
        source = { directory = "../expanded_dep" }

        [[package]]
        name = "file-url-dep"
        version = "0.1.0"
        source = { directory = "[TEMP_DIR]/file_url_dep" }

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "absolute-dep" },
            { name = "expanded-dep" },
            { name = "file-url-dep" },
            { name = "relative-dep" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "absolute-dep", directory = "[TEMP_DIR]/absolute_dep" },
            { name = "expanded-dep", directory = "../expanded_dep" },
            { name = "file-url-dep", directory = "[TEMP_DIR]/file_url_dep" },
            { name = "relative-dep", directory = "../relative_dep" },
        ]

        [[package]]
        name = "relative-dep"
        version = "0.1.0"
        source = { directory = "../relative_dep" }
        "#
        );
    });

    Ok(())
}

/// Check relative and absolute archive path handling with `uv add`.
#[test]
fn add_relative_and_absolute_archives() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let project = context.temp_dir.child("project");
    project.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    let relative_archives = context.temp_dir.child("relative_archives");
    relative_archives.create_dir_all()?;
    let relative_archive = relative_archives.child("ok-1.0.0-py3-none-any.whl");
    fs_err::copy(
        context
            .workspace_root
            .join("test/links/ok-1.0.0-py3-none-any.whl"),
        relative_archive.path(),
    )?;

    let absolute_archives = context.temp_dir.child("absolute_archives");
    absolute_archives.create_dir_all()?;
    let absolute_archive = absolute_archives.child("tqdm-1000.0.0-py3-none-any.whl");
    fs_err::copy(
        context
            .workspace_root
            .join("test/links/tqdm-1000.0.0-py3-none-any.whl"),
        absolute_archive.path(),
    )?;

    uv_snapshot!(context.filters(), context.add().arg("../relative_archives/ok-1.0.0-py3-none-any.whl").arg("--no-sync").current_dir(project.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Resolved 2 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.add().arg(absolute_archive.path()).arg("--no-sync").current_dir(project.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Resolved 3 packages in [TIME]
    ");

    let pyproject_toml = fs_err::read_to_string(project.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "ok",
            "tqdm",
        ]

        [tool.uv.sources]
        ok = { path = "../relative_archives/ok-1.0.0-py3-none-any.whl" }
        tqdm = { path = "[TEMP_DIR]/absolute_archives/tqdm-1000.0.0-py3-none-any.whl" }
        "#
        );
    });

    Ok(())
}

/// Update a requirement, modifying the source and extras.
#[test]
#[cfg(feature = "test-git")]
fn update() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["requests==2.31.0"]
    "#})?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 5 packages in [TIME]
    Installed 5 packages in [TIME]
     + certifi==2024.2.2
     + charset-normalizer==3.3.2
     + idna==3.6
     + requests==2.31.0
     + urllib3==2.2.1
    ");

    // Enable an extra (note the version specifier should be preserved).
    uv_snapshot!(context.filters(), context.add().arg("requests[security]"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Checked 5 packages in [TIME]
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "requests[security]==2.31.0",
        ]
        "#
        );
    });

    // Enable extras using the CLI flag and add a marker.
    uv_snapshot!(context.filters(), context.add().arg("requests; python_version > '3.7'").args(["--extra=use_chardet_on_py3", "--extra=socks"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + chardet==5.2.0
     + pysocks==1.7.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "requests[security]==2.31.0",
            "requests[socks,use-chardet-on-py3]>=2.31.0 ; python_full_version >= '3.8'",
        ]
        "#
        );
    });

    // Change the source by specifying a version (note the extras, markers, and version should be
    // preserved).
    uv_snapshot!(context.filters(), context.add().arg("requests @ git+https://github.com/psf/requests").arg("--tag=v2.32.3"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - requests==2.31.0
     + requests==2.32.3 (from git+https://github.com/psf/requests@0e322af87745eff34caffe4df68456ebc20d9068)
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "requests[security]==2.31.0",
            "requests[socks,use-chardet-on-py3]>=2.31.0 ; python_full_version >= '3.8'",
        ]

        [tool.uv.sources]
        requests = { git = "https://github.com/psf/requests", tag = "v2.32.3" }
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "certifi"
        version = "2024.2.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/certifi-2024.2.2.tar.gz", hash = "sha256:e91f672a47ba5696e377a4c21f8166067bef36ae757b778af0fa4b291cbe259c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/certifi-2024.2.2-py3-none-any.whl", hash = "sha256:a51a951e128c84716d27bbad83ab60ef28c5e00b89980690f0fa1a4f18c3919c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "chardet"
        version = "5.2.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/chardet-5.2.0.tar.gz", hash = "sha256:14e333479486b6750563ed1c34b20d074979ea005dba771f12f09ec74e6c8554", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/chardet-5.2.0-py3-none-any.whl", hash = "sha256:98ceb96b16def39eee43463f1dce9eda8fa550e6c684967ec5ce24c95f193f65", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "charset-normalizer"
        version = "3.3.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/charset_normalizer-3.3.2.tar.gz", hash = "sha256:1f99fa75c8f89bf493bf79aa0018d58b9d483482e53db6b89cd57ad4b3c19f69", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/charset_normalizer-3.3.2-py3-none-any.whl", hash = "sha256:517568d9d94db8f21bd3f43e2f562112a955f912637d0ea38f2d9c9573b58427", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "requests", extra = ["socks", "use-chardet-on-py3"] },
        ]

        [package.metadata]
        requires-dist = [
            { name = "requests", extras = ["security"], git = "https://github.com/psf/requests?tag=v2.32.3" },
            { name = "requests", extras = ["socks", "use-chardet-on-py3"], marker = "python_full_version >= '3.8'", git = "https://github.com/psf/requests?tag=v2.32.3" },
        ]

        [[package]]
        name = "pysocks"
        version = "1.7.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/pysocks-1.7.1.tar.gz", hash = "sha256:cadb157861171f8bdf5d27eb0695ae281ca05f2a00e2965dffbb948baa098368", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/pysocks-1.7.1-py3-none-any.whl", hash = "sha256:660470e46c5861c60842b9c3ee874ddce6d3e5998a28dcf8deaa02324ecca4fe", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "requests"
        version = "2.32.3"
        source = { git = "https://github.com/psf/requests?tag=v2.32.3#0e322af87745eff34caffe4df68456ebc20d9068" }
        dependencies = [
            { name = "certifi" },
            { name = "charset-normalizer" },
            { name = "idna" },
            { name = "urllib3" },
        ]

        [package.optional-dependencies]
        socks = [
            { name = "pysocks" },
        ]
        use-chardet-on-py3 = [
            { name = "chardet" },
        ]

        [[package]]
        name = "urllib3"
        version = "2.2.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/urllib3-2.2.1.tar.gz", hash = "sha256:bc130c8be0c82b95aa705fee217ecec8588ffe784002bc2f0c7b344be12d2571", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/urllib3-2.2.1-py3-none-any.whl", hash = "sha256:1c9d3ba42fe5f5336585783bffa353950639e24f7edc0dd26f04839cc3c0cc32", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 7 packages in [TIME]
    ");

    Ok(())
}

/// Add and update a requirement, with different markers
#[test]
fn add_update_marker() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.8"
        dependencies = ["requests>=2.30; python_version >= '3.11'"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 5 packages in [TIME]
    Installed 5 packages in [TIME]
     + certifi==2024.2.2
     + charset-normalizer==3.3.2
     + idna==3.6
     + requests==2.31.0
     + urllib3==2.2.1
    ");

    // Restrict the `requests` version for Python <3.11
    uv_snapshot!(context.filters(), context.add().arg("requests>=2.0,<2.29; python_version < '3.11'"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    Checked 5 packages in [TIME]
    ");

    let pyproject_toml = context.read("pyproject.toml");

    // Should add a new line for the dependency since the marker does not match an existing one
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.8"
        dependencies = [
            "requests>=2.0,<2.29 ; python_full_version < '3.11'",
            "requests>=2.30; python_version >= '3.11'",
        ]
        "#
        );
    });

    // Change the restricted `requests` version for Python <3.11
    uv_snapshot!(context.filters(), context.add().arg("requests>=2.0,<2.20; python_version < '3.11'"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    Checked 5 packages in [TIME]
    ");

    let pyproject_toml = context.read("pyproject.toml");

    // Should mutate the existing dependency since the marker matches
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.8"
        dependencies = [
            "requests>=2.0,<2.20 ; python_full_version < '3.11'",
            "requests>=2.30; python_version >= '3.11'",
        ]
        "#
        );
    });

    // Restrict the `requests` version on Windows and Python >3.11
    uv_snapshot!(context.filters(), context.add().arg("requests>=2.31 ; sys_platform == 'win32' and python_version > '3.11'"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    Checked 5 packages in [TIME]
    ");

    let pyproject_toml = context.read("pyproject.toml");

    // Should add a new line for the dependency since the marker does not match an existing one
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.8"
        dependencies = [
            "requests>=2.0,<2.20 ; python_full_version < '3.11'",
            "requests>=2.30; python_version >= '3.11'",
            "requests>=2.31 ; python_full_version >= '3.12' and sys_platform == 'win32'",
        ]
        "#
        );
    });

    // Restrict the `requests` version on Windows
    uv_snapshot!(context.filters(), context.add().arg("requests>=2.10 ; sys_platform == 'win32'"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    Checked 5 packages in [TIME]
    ");

    let pyproject_toml = context.read("pyproject.toml");

    // Should add a new line for the dependency since the marker does not exactly match an existing
    // one — although it is a subset of the existing marker.
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.8"
        dependencies = [
            "requests>=2.0,<2.20 ; python_full_version < '3.11'",
            "requests>=2.10 ; sys_platform == 'win32'",
            "requests>=2.30; python_version >= '3.11'",
            "requests>=2.31 ; python_full_version >= '3.12' and sys_platform == 'win32'",
        ]
        "#
        );
    });

    // Remove `requests`
    uv_snapshot!(context.filters(), context.remove().arg("requests"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Uninstalled 5 packages in [TIME]
     - certifi==2024.2.2
     - charset-normalizer==3.3.2
     - idna==3.6
     - requests==2.31.0
     - urllib3==2.2.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    // Should remove all variants of `requests`
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.8"
        dependencies = []
        "#
        );
    });

    Ok(())
}

#[test]
#[cfg(feature = "test-git")]
fn update_source_replace_url() -> Result<()> {
    let artifacts = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&format!(
        indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "requests[security] @ {}"
        ]
    "#},
        artifacts.file_url("requests-2.31.0-py3-none-any.whl")
    ))?;

    // Change the source. The existing URL should be removed.
    uv_snapshot!(context.filters(), context.add().arg("requests @ git+https://github.com/psf/requests").arg("--tag=v2.32.3"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 5 packages in [TIME]
    Installed 5 packages in [TIME]
     + certifi==2024.2.2
     + charset-normalizer==3.3.2
     + idna==3.6
     + requests==2.32.3 (from git+https://github.com/psf/requests@0e322af87745eff34caffe4df68456ebc20d9068)
     + urllib3==2.2.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "requests[security]",
        ]

        [tool.uv.sources]
        requests = { git = "https://github.com/psf/requests", tag = "v2.32.3" }
        "#
        );
    });

    // Change the source again. The existing source should be replaced.
    uv_snapshot!(context.filters(), context.add().arg("requests @ git+https://github.com/psf/requests").arg("--tag=v2.32.2"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - requests==2.32.3 (from git+https://github.com/psf/requests@0e322af87745eff34caffe4df68456ebc20d9068)
     + requests==2.32.2 (from git+https://github.com/psf/requests@88dce9d854797c05d0ff296b70e0430535ef8aaf)
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "requests[security]",
        ]

        [tool.uv.sources]
        requests = { git = "https://github.com/psf/requests", tag = "v2.32.2" }
        "#
        );
    });

    Ok(())
}

/// If a source defined in `tool.uv.sources` but its name is not normalized, `uv add` should not
/// add the same source again.
#[test]
#[cfg(feature = "test-git")]
fn add_non_normalized_source() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "uv-public-pypackage"
        ]

        [tool.uv.sources]
        uv_public_pypackage = { git = "https://github.com/astral-test/uv-public-pypackage", tag = "0.0.1" }
        "#})?;

    uv_snapshot!(context.filters(), context.add().arg("uv-public-pypackage @ git+https://github.com/astral-test/uv-public-pypackage@0.0.1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage@0dacfd662c64cb4ceb16e6cf65a157a8b715b979)
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "uv-public-pypackage",
        ]

        [tool.uv.sources]
        uv-public-pypackage = { git = "https://github.com/astral-test/uv-public-pypackage", rev = "0.0.1" }
        "#
        );
    });

    Ok(())
}

/// Test updating an existing Git reference with branch/tag/rev options without re- specifying the
/// URL.
#[test]
#[cfg(feature = "test-git")]
fn add_update_git_reference_project() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("https://github.com/astral-test/uv-public-pypackage.git"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage.git@b270df1a2fb5d012294e9aaf05e7e0bab1e6a389)
    ");

    uv_snapshot!(context.filters(), context.add().arg("uv-public-pypackage").arg("--tag=0.0.1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage.git@b270df1a2fb5d012294e9aaf05e7e0bab1e6a389)
     + uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage.git@0dacfd662c64cb4ceb16e6cf65a157a8b715b979)
    ");

    uv_snapshot!(context.filters(), context.add().arg("uv-public-pypackage").arg("--branch=main"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage.git@0dacfd662c64cb4ceb16e6cf65a157a8b715b979)
     + uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage.git@b270df1a2fb5d012294e9aaf05e7e0bab1e6a389)
    ");

    uv_snapshot!(context.filters(), context.add().arg("uv-public-pypackage").arg("--rev=2005223fcad0e2c06daf2e14b93b790604868e1e"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage.git@b270df1a2fb5d012294e9aaf05e7e0bab1e6a389)
     + uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage.git@2005223fcad0e2c06daf2e14b93b790604868e1e)
    ");

    Ok(())
}

/// Test updating an existing Git reference with branch/tag/rev options without re-specifying the
/// URL in a script.
#[test]
#[cfg(feature = "test-git")]
fn add_update_git_reference_script() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {
        r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [ ]
        # ///

        import time
        time.sleep(5)
        "#
    })?;

    uv_snapshot!(context.filters(), context.add().arg("--script=script.py").arg("https://github.com/astral-test/uv-public-pypackage.git"),
        @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    "
    );

    let script_content = context.read("script.py");
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            script_content, @r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #  "uv-public-pypackage",
        # ]
        #
        # [tool.uv.sources]
        # uv-public-pypackage = { git = "https://github.com/astral-test/uv-public-pypackage.git" }
        # ///

        import time
        time.sleep(5)
        "#
        );
    });

    uv_snapshot!(context.filters(), context.add().arg("--script=script.py").arg("uv-public-pypackage").arg("--branch=test-branch"),
        @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    "
    );

    let script_content = context.read("script.py");
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            script_content, @r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #  "uv-public-pypackage",
        # ]
        #
        # [tool.uv.sources]
        # uv-public-pypackage = { git = "https://github.com/astral-test/uv-public-pypackage.git", branch = "test-branch" }
        # ///

        import time
        time.sleep(5)
        "#
        );
    });

    Ok(())
}

/// If a source defined in `tool.uv.sources` but its name is not normalized, `uv remove` should
/// remove the source.
#[test]
#[cfg(feature = "test-git")]
fn remove_non_normalized_source() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "uv-public-pypackage"
        ]

        [tool.uv.sources]
        uv_public_pypackage = { git = "https://github.com/astral-test/uv-public-pypackage", tag = "0.0.1" }
        "#})?;

    uv_snapshot!(context.filters(), context.remove().arg("uv-public-pypackage"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked in [TIME]
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
        "#
        );
    });

    Ok(())
}

/// Adding a dependency does not remove untracked dependencies from the environment.
#[test]
fn add_inexact() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio == 3.7.0"]
    "#})?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    // Manually remove a dependency.
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("iniconfig==2.0.0"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig==2.0.0",
        ]
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz", hash = "sha256:48c42a08c0ec1a24f2fe45f4efdefc9c19ac8e0aa8e82284503ccba80398bec3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "iniconfig" },
        ]

        [package.metadata]
        requires-dist = [{ name = "iniconfig", specifier = "==2.0.0" }]
        "#
        );
    });

    // Install from the lockfile without removing extraneous packages from the environment.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen").arg("--inexact"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");

    // Install from the lockfile, performing an exact sync.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 3 packages in [TIME]
     - anyio==3.7.0
     - idna==3.6
     - sniffio==1.3.1
    ");

    Ok(())
}

/// Remove a PyPI requirement.
#[test]
fn remove_registry() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio==3.7.0"]
    "#})?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    uv_snapshot!(context.filters(), context.remove().arg("anyio"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Uninstalled 3 packages in [TIME]
     - anyio==3.7.0
     - idna==3.6
     - sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    ");

    Ok(())
}

#[test]
fn add_preserves_indentation_in_pyproject_toml() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
          "anyio==3.7.0",
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("requests==2.31.0").arg("--no-sync"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
          "anyio==3.7.0",
          "requests==2.31.0",
        ]
        "#
        );
    });
    Ok(())
}

#[test]
fn add_puts_default_indentation_in_pyproject_toml_if_not_observed() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio==3.7.0"]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("requests==2.31.0").arg("--no-sync"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "anyio==3.7.0",
            "requests==2.31.0",
        ]
        "#
        );
    });
    Ok(())
}

/// Add a requirement without updating the lockfile.
#[test]
fn add_frozen() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Remove the virtual environment.
    fs_err::remove_dir_all(&context.venv)?;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio==3.7.0").arg("--frozen").env(EnvVars::VIRTUAL_ENV, "active"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "anyio==3.7.0",
        ]
        "#
        );
    });

    assert!(!context.temp_dir.join("uv.lock").exists());
    assert!(!context.venv.exists());

    Ok(())
}

/// Add a requirement without updating the environment.
#[test]
fn add_no_sync() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Remove the virtual environment.
    fs_err::remove_dir_all(&context.venv)?;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio==3.7.0").arg("--no-sync"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Resolved 4 packages in [TIME]
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "anyio==3.7.0",
        ]
        "#
        );
    });

    assert!(context.temp_dir.join("uv.lock").exists());
    assert!(!context.venv.exists());

    Ok(())
}

/// Editing without synchronization does not target either virtual environment.
#[test]
fn edit_no_sync_active_environment_mismatch() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    fs_err::remove_dir_all(&context.venv)?;

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    fs_err::copy(
        context
            .workspace_root
            .join("test/links/ok-1.0.0-py3-none-any.whl"),
        context.temp_dir.join("ok-1.0.0-py3-none-any.whl"),
    )?;

    uv_snapshot!(context.filters(), context
        .add()
        .arg("./ok-1.0.0-py3-none-any.whl")
        .arg("--no-sync")
        .arg("--offline")
        .env(EnvVars::VIRTUAL_ENV, "active"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Resolved 2 packages in [TIME]
    ");

    assert!(!context.venv.exists());

    uv_snapshot!(context.filters(), context
        .remove()
        .arg("ok")
        .arg("--no-sync")
        .arg("--offline")
        .env(EnvVars::VIRTUAL_ENV, "active"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Resolved 1 package in [TIME]
    ");

    assert!(!context.venv.exists());

    context
        .venv()
        .arg("active")
        .arg("--python")
        .arg("3.12")
        .assert()
        .success();

    // An explicitly requested active environment still determines the interpreter.
    uv_snapshot!(context.filters(), context
        .add()
        .arg("./ok-1.0.0-py3-none-any.whl")
        .arg("--no-sync")
        .arg("--offline")
        .arg("--active")
        .env(EnvVars::VIRTUAL_ENV, "active")
        .env(EnvVars::UV_PYTHON_SEARCH_PATH, ""), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    assert!(!context.venv.exists());

    uv_snapshot!(context.filters(), context
        .remove()
        .arg("ok")
        .arg("--no-sync")
        .arg("--offline")
        .arg("--active")
        .env(EnvVars::VIRTUAL_ENV, "active")
        .env(EnvVars::UV_PYTHON_SEARCH_PATH, ""), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");

    assert!(!context.venv.exists());

    // Commands that synchronize the project environment must retain the mismatch warning.
    uv_snapshot!(context.filters(), context
        .sync()
        .arg("--offline")
        .env(EnvVars::VIRTUAL_ENV, "active"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: `VIRTUAL_ENV=active` does not match the project environment path `.venv` and will be ignored; use `--active` to target the active environment instead
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Resolved 1 package in [TIME]
    Checked in [TIME]
    ");

    assert!(context.venv.exists());

    Ok(())
}

#[test]
fn add_reject_multiple_git_ref_flags() {
    let context = uv_test::test_context!("3.12");

    // --tag and --branch
    uv_snapshot!(context.filters(), context
        .add()
        .arg("foo")
        .arg("--tag")
        .arg("0.0.1")
        .arg("--branch")
        .arg("test"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--tag <TAG>' cannot be used with '--branch <BRANCH>'

    Usage: uv add --cache-dir [CACHE_DIR] --tag <TAG> --exclude-newer <EXCLUDE_NEWER> <PACKAGES|--requirements <REQUIREMENTS>>

    For more information, try '--help'.
    "
    );

    // --tag and --rev
    uv_snapshot!(context.filters(), context
        .add()
        .arg("foo")
        .arg("--tag")
        .arg("0.0.1")
        .arg("--rev")
        .arg("326b943"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--tag <TAG>' cannot be used with '--rev <REV>'

    Usage: uv add --cache-dir [CACHE_DIR] --tag <TAG> --exclude-newer <EXCLUDE_NEWER> <PACKAGES|--requirements <REQUIREMENTS>>

    For more information, try '--help'.
    "
    );

    // --tag and --tag
    uv_snapshot!(context.filters(), context
        .add()
        .arg("foo")
        .arg("--tag")
        .arg("0.0.1")
        .arg("--tag")
        .arg("0.0.2"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--tag <TAG>' cannot be used multiple times

    Usage: uv add [OPTIONS] <PACKAGES|--requirements <REQUIREMENTS>>

    For more information, try '--help'.
    "
    );
}

/// Avoiding persisting `add` calls when resolution fails.
#[test]
fn add_error() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("xyz"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to add dependencies
      cause: No solution found when resolving dependencies
      cause: Because xyz was not found in the package registry and your project depends on xyz, we can conclude that your project's requirements are unsatisfiable.

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    ");

    uv_snapshot!(context.filters(), context.add().arg("xyz").arg("--frozen"), @"
    exit_code: 0 (success)
    ");

    let lock = context.temp_dir.join("uv.lock");
    assert!(!lock.exists());

    Ok(())
}

/// Suggest avoiding dependencies for modules in the Python standard library.
#[test]
fn add_standard_library_error() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("pickle"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to add dependencies
      cause: No solution found when resolving dependencies
      cause: Because pickle was not found in the package registry and your project depends on pickle, we can conclude that your project's requirements are unsatisfiable.

    hint: The module `pickle` is included in the Python standard library and usually should not be added as a dependency

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    ");

    Ok(())
}

/// Avoid suggesting a standard-library alternative for unrelated resolution failures.
#[test]
fn add_standard_library_unrelated_resolution_error() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("typing").arg("xyz"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to add dependencies
      cause: No solution found when resolving dependencies
      cause: Because xyz was not found in the package registry and your project depends on xyz, we can conclude that your project's requirements are unsatisfiable.

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    ");

    Ok(())
}

/// Emit dedicated error message when adding Conda `environment.yml`
#[test]
fn add_environment_yml_error() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    let environment_yml = context.temp_dir.child("environment.yml");
    environment_yml.write_str(indoc! {r"
        name: test-env
        channels:
          - conda-forge
        dependencies:
          - python>=3.12
    "})?;

    uv_snapshot!(context.filters(), context.add().arg("-r").arg("environment.yml"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Conda environment files (i.e., `environment.yml`) are not supported
    ");

    Ok(())
}

/// Set a lower bound when adding unconstrained dependencies.
#[test]
fn add_lower_bound() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Adding `anyio` should include a lower-bound.
    uv_snapshot!(context.filters(), context.add().arg("anyio"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "anyio>=4.3.0",
        ]
        "#
        );
    });

    Ok(())
}

/// Avoid setting a lower bound when updating existing dependencies.
#[test]
fn add_lower_bound_existing() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio"]
    "#})?;

    // Adding `anyio` should _not_ set a lower-bound, since it's already present (even if
    // unconstrained).
    uv_snapshot!(context.filters(), context.add().arg("anyio"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "anyio",
        ]
        "#
        );
    });

    Ok(())
}

/// Avoid setting a lower bound with `--raw`.
#[test]
fn add_lower_bound_raw() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio"]
    "#})?;

    // Adding `anyio` should _not_ set a lower-bound when using `--raw`.
    uv_snapshot!(context.filters(), context.add().arg("anyio").arg("--raw"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "anyio",
        ]
        "#
        );
    });

    Ok(())
}

/// Set a lower bound when adding unconstrained dev dependencies.
#[test]
fn add_lower_bound_dev() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Adding `anyio` should include a lower-bound.
    uv_snapshot!(context.filters(), context.add().arg("anyio").arg("--dev"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [dependency-groups]
        dev = [
            "anyio>=4.3.0",
        ]
        "#
        );
    });

    Ok(())
}

/// Set a lower bound when adding unconstrained optional dependencies.
#[test]
fn add_lower_bound_optional() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Adding `anyio` should include a lower-bound.
    uv_snapshot!(context.filters(), context.add().arg("anyio").arg("--optional=io"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        io = [
            "anyio>=4.3.0",
        ]
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "4.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-4.3.0.tar.gz", hash = "sha256:13a6d97fa30ec110d85e3949a30c92306f0178135048329f54a335c3dade753a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-4.3.0-py3-none-any.whl", hash = "sha256:c4f443e7e5a2c003b1534688207e85dbd11960efb66d4d6a4e7693fdfc6f5b33", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.optional-dependencies]
        io = [
            { name = "anyio" },
        ]

        [package.metadata]
        requires-dist = [{ name = "anyio", marker = "extra == 'io'", specifier = ">=4.3.0" }]
        provides-extras = ["io"]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    Ok(())
}

/// Omit the local segment when adding dependencies (since `>=1.2.3+local` is invalid).
#[test]
fn add_lower_bound_local() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("local/local-simple.toml");
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Adding `a` should include a lower-bound, but no local segment.
    uv_snapshot!(context.filters(), context.add().arg("a").arg("--index").arg(server.index_url()).env_remove(EnvVars::UV_EXCLUDE_NEWER), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + a==1.2.3+foo
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "a>=1.2.3",
        ]

        [[tool.uv.index]]
        url = "http://[LOCALHOST]/simple/"
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [[package]]
        name = "a"
        version = "1.2.3+foo"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/a-1.2.3+foo.tar.gz", hash = "sha256:2fd6e9af06ed622c5b242c080167ffec11fd4c3360dc28d9700aba4c9f3d4a43", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.2.3+foo-py3-none-any.whl", hash = "sha256:2fd8eec176cad72b6c4372682485914aff65d215fb8cd71c85e9ed4511fa8bbe", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a" },
        ]

        [package.metadata]
        requires-dist = [{ name = "a", specifier = ">=1.2.3" }]
        "#
        );
    });

    Ok(())
}

/// Add dependencies to a non-project workspace root.
#[test]
fn add_non_project() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r"
        [tool.uv.workspace]
        members = []
    "})?;

    // Adding `iniconfig` should fail, since virtual workspace roots don't support production
    // dependencies.
    uv_snapshot!(context.filters(), context.add().arg("iniconfig"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Project is missing a `[project]` table; add a `[project]` table to use production dependencies, or run `uv add --dev` instead
    ");

    // Adding `iniconfig` as optional should fail, since virtual workspace roots don't support
    // optional dependencies.
    uv_snapshot!(context.filters(), context.add().arg("iniconfig").arg("--optional").arg("async"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Project is missing a `[project]` table; add a `[project]` table to use optional dependencies, or run `uv add --dev` instead
    ");

    // Adding `iniconfig` as a dev dependency should succeed.
    uv_snapshot!(context.filters(), context.add().arg("iniconfig").arg("--dev"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: No `requires-python` value found in the workspace. Defaulting to `>=3.12`.
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [tool.uv.workspace]
        members = []

        [dependency-groups]
        dev = [
            "iniconfig>=2.0.0",
        ]
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]

        [manifest.dependency-groups]
        dev = [{ name = "iniconfig", specifier = ">=2.0.0" }]

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz", hash = "sha256:48c42a08c0ec1a24f2fe45f4efdefc9c19ac8e0aa8e82284503ccba80398bec3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    Ok(())
}

#[test]
fn add_virtual_empty() -> Result<()> {
    // testing how `uv add` reacts to a pyproject with no `[project]` and nothing useful to it
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [tool.mycooltool]
        wow = "someconfig"
    "#})?;

    // Add normal dep (doesn't make sense)
    uv_snapshot!(context.filters(), context.add()
        .arg("sortedcontainers"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Project is missing a `[project]` table; add a `[project]` table to use production dependencies, or run `uv add --dev` instead
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [tool.mycooltool]
        wow = "someconfig"
        "#
        );
    });

    // Add dependency-group (can make sense!)
    uv_snapshot!(context.filters(), context.add()
        .arg("sortedcontainers")
        .arg("--group").arg("dev"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: No `requires-python` value found in the workspace. Defaulting to `>=3.12`.
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + sortedcontainers==2.4.0
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [tool.mycooltool]
        wow = "someconfig"

        [dependency-groups]
        dev = [
            "sortedcontainers>=2.4.0",
        ]
        "#
        );
    });

    Ok(())
}

#[test]
fn add_virtual_dependency_group() -> Result<()> {
    // testing basic `uv add --group` functionality
    // when the pyproject.toml is fully virtual (no `[project]`)
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [dependency-groups]
        foo = ["sortedcontainers"]
        bar = ["iniconfig"]
        dev = ["sniffio"]
    "#})?;

    // Add to existing group
    uv_snapshot!(context.filters(), context.add()
        .arg("sortedcontainers")
        .arg("--group").arg("dev"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: No `requires-python` value found in the workspace. Defaulting to `>=3.12`.
    Resolved 3 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + sniffio==1.3.1
     + sortedcontainers==2.4.0
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [dependency-groups]
        foo = ["sortedcontainers"]
        bar = ["iniconfig"]
        dev = [
            "sniffio",
            "sortedcontainers>=2.4.0",
        ]
        "#
        );
    });

    // Add to new group
    uv_snapshot!(context.filters(), context.add()
        .arg("sortedcontainers")
        .arg("--group").arg("baz"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: No `requires-python` value found in the workspace. Defaulting to `>=3.12`.
    Resolved 3 packages in [TIME]
    Checked 2 packages in [TIME]
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [dependency-groups]
        foo = ["sortedcontainers"]
        bar = ["iniconfig"]
        dev = [
            "sniffio",
            "sortedcontainers>=2.4.0",
        ]
        baz = [
            "sortedcontainers>=2.4.0",
        ]
        "#
        );
    });

    Ok(())
}

#[test]
fn add_empty_requirements_group() -> Result<()> {
    // Test that `uv add -r requirements.txt --group <name>` creates an empty group
    // when the requirements file is empty
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("")?;

    uv_snapshot!(context.filters(), context.add()
        .arg("-r").arg("requirements.txt")
        .arg("--group").arg("user"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Requirements file `requirements.txt` does not contain any dependencies
    Resolved 1 package in [TIME]
    Checked in [TIME]
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [dependency-groups]
        user = []
        "#
        );
    });

    Ok(())
}

#[test]
fn add_empty_requirements_optional() -> Result<()> {
    // Test that `uv add -r requirements.txt --optional <extra>` creates an empty extra
    // when the requirements file is empty
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("")?;

    uv_snapshot!(context.filters(), context.add()
        .arg("-r").arg("requirements.txt")
        .arg("--optional").arg("extra"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Requirements file `requirements.txt` does not contain any dependencies
    Resolved 1 package in [TIME]
    Checked in [TIME]
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        extra = []
        "#
        );
    });

    Ok(())
}

#[test]
fn remove_virtual_empty() -> Result<()> {
    // testing how `uv remove` reacts to a pyproject with no `[project]` and nothing useful to it
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [tool.mycooltool]
        wow = "someconfig"

        "#,
    )?;

    // Remove normal dep (doesn't make sense)
    uv_snapshot!(context.filters(), context.remove()
        .arg("sortedcontainers"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The dependency `sortedcontainers` could not be found in `project.dependencies`
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"

        [tool.mycooltool]
        wow = "someconfig"
        "#
        );
    });

    // Remove dependency-group (can make sense, but nothing there!)
    uv_snapshot!(context.filters(), context.remove()
        .arg("sortedcontainers")
        .arg("--group").arg("dev"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The dependency `sortedcontainers` could not be found in `dependency-groups.dev`
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"

        [tool.mycooltool]
        wow = "someconfig"
        "#
        );
    });

    Ok(())
}

#[test]
fn remove_virtual_dependency_group() -> Result<()> {
    // testing basic `uv remove --group` functionality
    // when the pyproject.toml is fully virtual (no `[project]`)
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [dependency-groups]
        foo = ["sortedcontainers"]
        bar = ["iniconfig"]
        dev = ["sniffio"]
    "#})?;

    // Remove from group
    uv_snapshot!(context.filters(), context.remove()
        .arg("sortedcontainers")
        .arg("--group").arg("foo"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: No `requires-python` value found in the workspace. Defaulting to `>=3.12`.
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [dependency-groups]
        foo = []
        bar = ["iniconfig"]
        dev = ["sniffio"]
        "#
        );
    });

    // Remove from non-existent group
    uv_snapshot!(context.filters(), context.remove()
        .arg("sortedcontainers")
        .arg("--group").arg("baz"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The dependency `sortedcontainers` could not be found in `dependency-groups.baz`
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [dependency-groups]
        foo = []
        bar = ["iniconfig"]
        dev = ["sniffio"]
        "#
        );
    });

    Ok(())
}

/// Add the same requirement multiple times.
#[test]
fn add_repeat() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "anyio>=4.3.0",
        ]
        "#
        );
    });

    uv_snapshot!(context.filters(), context.add().arg("anyio"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Checked 3 packages in [TIME]
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "anyio>=4.3.0",
        ]
        "#
        );
    });

    Ok(())
}

/// Add from requirement file.
#[test]
#[cfg(feature = "test-git")]
fn add_requirements_file() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_counts();

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("Flask==2.3.2\nanyio @ git+https://github.com/agronholm/anyio.git@4.4.0")?;

    uv_snapshot!(context.filters(), context.add().arg("-r").arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + anyio==4.4.0 (from git+https://github.com/agronholm/anyio.git@053e8f0a0f7b0f4a47a012eb5c6b1d9d84344e6a)
     + blinker==1.7.0
     + click==8.1.7
     + flask==2.3.2
     + idna==3.6
     + itsdangerous==2.1.2
     + jinja2==3.1.3
     + markupsafe==2.1.5
     + sniffio==1.3.1
     + werkzeug==3.0.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "anyio",
            "flask==2.3.2",
        ]

        [tool.uv.sources]
        anyio = { git = "https://github.com/agronholm/anyio.git", rev = "4.4.0" }
        "#
        );
    });

    // Passing stdin should succeed
    uv_snapshot!(context.filters(), context.add().arg("-r").arg("-").stdin(std::fs::File::open(requirements_txt)?), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Checked [N] packages in [TIME]
    ");

    // Passing a `setup.py` should fail.
    uv_snapshot!(context.filters(), context.add().arg("-r").arg("setup.py"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Adding requirements from a `setup.py` is not supported in `uv add`
    ");

    // Passing nothing should fail.
    uv_snapshot!(context.filters(), context.add(), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the following required arguments were not provided:
      <PACKAGES|--requirements <REQUIREMENTS>>

    Usage: uv add --cache-dir [CACHE_DIR] --exclude-newer <EXCLUDE_NEWER> <PACKAGES|--requirements <REQUIREMENTS>>

    For more information, try '--help'.
    ");

    Ok(())
}

/// Add a path dependency from a requirements file, respecting the lack of a `-e` flag.
#[test]
fn add_requirements_file_non_editable() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Create a peer package.
    let child = context.temp_dir.child("packages").child("child");
    child.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
    "#})?;
    child
        .child("src")
        .child("child")
        .child("__init__.py")
        .touch()?;

    // Without `-e`, the package should not be listed as editable.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("./packages/child")?;

    uv_snapshot!(context.filters(), context.add().arg("-r").arg("requirements.txt").arg("--no-workspace"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + child==0.1.0 (from file://[TEMP_DIR]/packages/child)
    ");

    let pyproject_toml_content = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml_content, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "child",
        ]

        [tool.uv.sources]
        child = { path = "packages/child" }
        "#
        );
    });

    Ok(())
}

/// Add a path dependency from a requirements file, respecting `-e` for editable.
#[test]
fn add_requirements_file_editable() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Create a peer package.
    let child = context.temp_dir.child("packages").child("child");
    child.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
    "#})?;
    child
        .child("src")
        .child("child")
        .child("__init__.py")
        .touch()?;

    // With `-e`, the package should be listed as editable.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("-e ./packages/child")?;

    uv_snapshot!(context.filters(), context.add().arg("-r").arg("requirements.txt").arg("--no-workspace"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + child==0.1.0 (from file://[TEMP_DIR]/packages/child)
    ");

    let pyproject_toml_content = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml_content, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "child",
        ]

        [tool.uv.sources]
        child = { path = "packages/child", editable = true }
        "#
        );
    });

    Ok(())
}

/// Add a path dependency from a requirements file, overriding the `-e` flag.
#[test]
fn add_requirements_file_editable_override() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Create a peer package.
    let child = context.temp_dir.child("packages").child("child");
    child.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
    "#})?;
    child
        .child("src")
        .child("child")
        .child("__init__.py")
        .touch()?;

    // With `-e`, the package should be listed as editable, but the `--no-editable` flag should
    // override it.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("-e ./packages/child")?;

    uv_snapshot!(context.filters(), context.add().arg("-r").arg("requirements.txt").arg("--no-workspace").arg("--no-editable"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + child==0.1.0 (from file://[TEMP_DIR]/packages/child)
    ");

    let pyproject_toml_content = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml_content, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "child",
        ]

        [tool.uv.sources]
        child = { path = "packages/child", editable = false }
        "#
        );
    });

    Ok(())
}

/// Add requirements from a file with a marker flag.
///
/// We test that:
/// * Adding requirements from a file applies the marker to all of them
/// * We combine the marker with existing markers
/// * We only sync the packages applicable under this marker
#[test]
fn add_requirements_file_with_marker_flag() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_win_txt = context.temp_dir.child("requirements.win.txt");
    requirements_win_txt.write_str("anyio>=2.31.0\niniconfig>=2; sys_platform != 'fantasy_os'\nnumpy>1.9; sys_platform == 'fantasy_os'")?;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    let base_pyproject_toml = indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#};
    pyproject_toml.write_str(base_pyproject_toml)?;

    // Add dependencies with a marker that does not apply for the current target.
    uv_snapshot!(context.filters(), context.add().arg("-r").arg("requirements.win.txt").arg("-m").arg("python_version == '3.11'"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked in [TIME]
    ");
    let edited_pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    // TODO(konsti): We should output `python_version == '3.12'` instead of lowering to
    // `python_full_version`.
    assert_snapshot!(
        edited_pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = [
        "anyio>=2.31.0 ; python_full_version == '3.11.*'",
        "iniconfig>=2 ; python_full_version == '3.11.*' and sys_platform != 'fantasy_os'",
        "numpy>1.9 ; python_full_version == '3.11.*' and sys_platform == 'fantasy_os'",
    ]
    "#
    );

    // Reset the project.
    pyproject_toml.write_str(base_pyproject_toml)?;
    fs_err::remove_file(context.temp_dir.join("uv.lock"))?;

    // Add dependencies with a marker that applies for the current target.
    uv_snapshot!(context.filters(), context.add().arg("-r").arg("requirements.win.txt").arg("-m").arg("python_version == '3.12'"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + iniconfig==2.0.0
     + sniffio==1.3.1
    ");
    let edited_pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    assert_snapshot!(
        edited_pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = [
        "anyio>=2.31.0 ; python_full_version == '3.12.*'",
        "iniconfig>=2 ; python_full_version == '3.12.*' and sys_platform != 'fantasy_os'",
        "numpy>1.9 ; python_full_version == '3.12.*' and sys_platform == 'fantasy_os'",
    ]
    "#
    );

    Ok(())
}

/// Add from requirement file, with additional, external constraints.
///
/// The constraints should be respected, but they should _not_ be recorded in the `pyproject.toml`
/// or `uv.lock` file.
#[test]
fn add_requirements_file_constraints() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_counts();

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    let requirements_in = context.temp_dir.child("requirements.in");
    requirements_in.write_str(indoc! {r"
            flask
            anyio
        "})?;

    // Write a set of valid, but outdated compiled requirements.
    //
    // For reference, these are generated by compiling:
    // ```txt
    // flask
    // anyio<4
    // click<7
    // ```
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {r"
            # This file was autogenerated by uv via the following command:
            #    uv pip compile --cache-dir [CACHE_DIR] requirements.in -o requirements.txt
            anyio==3.7.1
                # via -r requirements.in
            click==6.7
                # via
                #   -r requirements.in
                #   flask
            flask==1.1.4
                # via -r requirements.in
            idna==3.6
                # via anyio
            itsdangerous==1.1.0
                # via flask
            jinja2==2.11.3
                # via flask
            markupsafe==2.1.5
                # via jinja2
            sniffio==1.3.1
                # via anyio
            werkzeug==1.0.1
                # via flask
        "})?;

    // Pass the input requirements as constraints.
    uv_snapshot!(context.filters(), context.add().arg("-r").arg("requirements.in").arg("-c").arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + anyio==3.7.1
     + click==6.7
     + flask==1.1.4
     + idna==3.6
     + itsdangerous==1.1.0
     + jinja2==2.11.3
     + markupsafe==2.1.5
     + sniffio==1.3.1
     + werkzeug==1.0.1
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "anyio>=3.7.1",
            "flask>=1.1.4",
        ]
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "3.7.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-3.7.1.tar.gz", hash = "sha256:d6a7c4ae82d624240d7635e00e4336544892a573380600fea74ed050ea34ed74", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-3.7.1-py3-none-any.whl", hash = "sha256:e4ece7d3e3bffe08069efde0bb1ec9dcf99a7a6637201797d3fd541765a4f052", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "click"
        version = "6.7"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/click-6.7.tar.gz", hash = "sha256:2030996cf669b87cff9695021da19710ec3043449d6f8ade829565234d83abf2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/click-6.7-py3-none-any.whl", hash = "sha256:00bf8b64ac38c6f55afa5d042f3e10a718d80b4264cc2b5fdcf530e9038cbeed", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "flask"
        version = "1.1.4"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "click" },
            { name = "itsdangerous" },
            { name = "jinja2" },
            { name = "werkzeug" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/flask-1.1.4.tar.gz", hash = "sha256:fdf44d1f4eb8343fdc157e8699541bb19bc5978d2f7bbfed871dc5d1b4bd20fe", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/flask-1.1.4-py3-none-any.whl", hash = "sha256:162a8cc715eb5e6e8cc7f7b8d418ed31321fcfc14f3bab17091590fdc9a95c48", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "itsdangerous"
        version = "1.1.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/itsdangerous-1.1.0.tar.gz", hash = "sha256:edeaeb49163bb0fa797b5fe1126ec05b2d5598320f3bad8c71df875b85bf2691", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/itsdangerous-1.1.0-py3-none-any.whl", hash = "sha256:143a76824cb68484f66e9ec68cfd7875a1aaf69999f24096514774135cd3030b", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "jinja2"
        version = "2.11.3"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "markupsafe" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/jinja2-2.11.3.tar.gz", hash = "sha256:b2f1bf94452c0645a72b643710d31863f3f0ea16a36168335d9c231d46d668a4", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/jinja2-2.11.3-py3-none-any.whl", hash = "sha256:a9890098ccf38a1a0e6489210c4e01cb0ba4227c2d3cc327a4f1ca1f7d3611a8", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "markupsafe"
        version = "2.1.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/markupsafe-2.1.5.tar.gz", hash = "sha256:e38237d66e6760fe86fe38e4b6c70ff4eed7da50b15e8c0f2589f05850207f13", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/markupsafe-2.1.5-py3-none-any.whl", hash = "sha256:d0fe66b2745bbd943b48f3229667cf4506d88136363578b94e63d384e61d4984", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "anyio" },
            { name = "flask" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "anyio", specifier = ">=3.7.1" },
            { name = "flask", specifier = ">=1.1.4" },
        ]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "werkzeug"
        version = "1.0.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/werkzeug-1.0.1.tar.gz", hash = "sha256:63e00288ea20176f388f9e9bd5b1cb1552421b6c4389a1a929402365be7f2bef", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/werkzeug-1.0.1-py3-none-any.whl", hash = "sha256:bf524698ab1cb176ffa415cd2671f43c80c9f8deeb1f7b4b74ade61c12b0ba94", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Re-run with `--locked`.
    uv_snapshot!(context.filters(), context.lock().arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    ");

    Ok(())
}

/// Add a requirement to a dependency group.
#[test]
fn add_group() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio==3.7.0").arg("--group").arg("test"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    assert_snapshot!(pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = []

    [dependency-groups]
    test = [
        "anyio==3.7.0",
    ]
    "#
    );

    uv_snapshot!(context.filters(), context.add().arg("requests").arg("--group").arg("test"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + certifi==2024.2.2
     + charset-normalizer==3.3.2
     + requests==2.31.0
     + urllib3==2.2.1
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    assert_snapshot!(pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = []

    [dependency-groups]
    test = [
        "anyio==3.7.0",
        "requests>=2.31.0",
    ]
    "#
    );

    uv_snapshot!(context.filters(), context.add().arg("anyio==3.7.0").arg("--group").arg("second"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Checked 3 packages in [TIME]
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    assert_snapshot!(pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = []

    [dependency-groups]
    second = [
        "anyio==3.7.0",
    ]
    test = [
        "anyio==3.7.0",
        "requests>=2.31.0",
    ]
    "#
    );

    uv_snapshot!(context.filters(), context.add().arg("anyio==3.7.0").arg("--group").arg("alpha"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Checked 3 packages in [TIME]
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    assert_snapshot!(pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = []

    [dependency-groups]
    alpha = [
        "anyio==3.7.0",
    ]
    second = [
        "anyio==3.7.0",
    ]
    test = [
        "anyio==3.7.0",
        "requests>=2.31.0",
    ]
    "#
    );

    assert!(context.temp_dir.join("uv.lock").exists());

    Ok(())
}

/// Normalize group names when adding or removing.
#[test]
fn add_group_normalize() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [dependency-groups]
        cloud_export_to_parquet = [
            "anyio==3.7.0",
        ]
    "#})?;

    // Add with a non-normalized group name.
    uv_snapshot!(context.filters(), context.add().arg("iniconfig").arg("--group").arg("cloud_export_to_parquet"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + iniconfig==2.0.0
     + sniffio==1.3.1
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    assert_snapshot!(pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = []

    [dependency-groups]
    cloud_export_to_parquet = [
        "anyio==3.7.0",
        "iniconfig>=2.0.0",
    ]
    "#
    );

    // Add with a normalized group name (which doesn't match the `pyproject.toml`).
    uv_snapshot!(context.filters(), context.add().arg("typing-extensions").arg("--group").arg("cloud-export-to-parquet"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + typing-extensions==4.10.0
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    assert_snapshot!(pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = []

    [dependency-groups]
    cloud_export_to_parquet = [
        "anyio==3.7.0",
        "iniconfig>=2.0.0",
        "typing-extensions>=4.10.0",
    ]
    "#
    );

    // Remove with a non-normalized group name.
    uv_snapshot!(context.filters(), context.remove().arg("iniconfig").arg("--group").arg("cloud_export_to_parquet"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Uninstalled 5 packages in [TIME]
     - anyio==3.7.0
     - idna==3.6
     - iniconfig==2.0.0
     - sniffio==1.3.1
     - typing-extensions==4.10.0
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    assert_snapshot!(pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = []

    [dependency-groups]
    cloud_export_to_parquet = [
        "anyio==3.7.0",
        "typing-extensions>=4.10.0",
    ]
    "#
    );

    // Remove with a normalized group name (which doesn't match the `pyproject.toml`).
    uv_snapshot!(context.filters(), context.remove().arg("typing-extensions").arg("--group").arg("cloud-export-to-parquet"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Checked in [TIME]
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    assert_snapshot!(pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = []

    [dependency-groups]
    cloud_export_to_parquet = [
        "anyio==3.7.0",
    ]
    "#
    );

    Ok(())
}

/// Add a requirement to a dependency group (sorted before the other groups).
#[test]
fn add_group_before_commented_groups() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [dependency-groups]
        # This is our dev group
        dev = [
            "anyio==3.7.0",
        ]
        # This is our test group
        test = [
            "anyio==3.7.0",
            "requests>=2.31.0",
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio==3.7.0").arg("--group").arg("alpha"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    assert!(context.temp_dir.join("uv.lock").exists());

    assert_snapshot!(pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = []

    [dependency-groups]
    alpha = [
        "anyio==3.7.0",
    ]
    # This is our dev group
    dev = [
        "anyio==3.7.0",
    ]
    # This is our test group
    test = [
        "anyio==3.7.0",
        "requests>=2.31.0",
    ]
    "#
    );

    Ok(())
}

/// Add a requirement to dependency group (sorted between the other groups).
#[test]
fn add_group_between_commented_groups() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [dependency-groups]
        # This is our dev group
        dev = [
            "anyio==3.7.0",
        ]
        # This is our test group
        test = [
            "anyio==3.7.0",
            "requests>=2.31.0",
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio==3.7.0").arg("--group").arg("eta"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    assert!(context.temp_dir.join("uv.lock").exists());

    assert_snapshot!(pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = []

    [dependency-groups]
    # This is our dev group
    dev = [
        "anyio==3.7.0",
    ]
    eta = [
        "anyio==3.7.0",
    ]
    # This is our test group
    test = [
        "anyio==3.7.0",
        "requests>=2.31.0",
    ]
    "#
    );

    Ok(())
}

/// Add a requirement to a dependency group when existing dependency group
/// keys are not sorted.
#[test]
fn add_group_to_unsorted() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [dependency-groups]
        test = [
            "anyio==3.7.0",
            "requests>=2.31.0",
        ]
        second = [
            "anyio==3.7.0",
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio==3.7.0").arg("--group").arg("alpha"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    assert_snapshot!(pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = []

    [dependency-groups]
    test = [
        "anyio==3.7.0",
        "requests>=2.31.0",
    ]
    second = [
        "anyio==3.7.0",
    ]
    alpha = [
        "anyio==3.7.0",
    ]
    "#
    );

    assert!(context.temp_dir.join("uv.lock").exists());

    Ok(())
}

/// Remove a requirement from a dependency group.
#[test]
fn remove_group() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [dependency-groups]
        test = [
            "anyio==3.7.0",
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.remove().arg("anyio").arg("--group").arg("test"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked in [TIME]
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [dependency-groups]
        test = []
        "#
        );
    });

    uv_snapshot!(context.filters(), context.remove().arg("anyio").arg("--group").arg("test"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The dependency `anyio` could not be found in `dependency-groups.test`
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [dependency-groups]
        test = []
        "#
        );
    });

    uv_snapshot!(context.filters(), context.remove().arg("anyio").arg("--group").arg("test"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The dependency `anyio` could not be found in `dependency-groups.test`
    ");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio"]
    "#})?;

    uv_snapshot!(context.filters(), context.remove().arg("anyio").arg("--group").arg("test"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The dependency `anyio` could not be found in `dependency-groups.test`

    hint: `anyio` is a production dependency
    ");

    Ok(())
}

/// Add to a PEP 732 script.
#[test]
fn add_script() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "requests<3",
        #   "rich",
        # ]
        # ///

        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio").arg("--script").arg("script.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 11 packages in [TIME]
    ");

    let script_content = context.read("script.py");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            script_content, @r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "anyio>=4.3.0",
        #   "requests<3",
        #   "rich",
        # ]
        # ///

        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
        "#
        );
    });

    // Adding to a script without a lockfile shouldn't create a lockfile.
    assert!(!context.temp_dir.join("script.py.lock").exists());

    Ok(())
}

/// Test that `--bounds` is respected when adding to a script without a lockfile.
#[test]
fn add_script_bounds() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        print("Hello, world!")
    "#})?;

    // Add `anyio` with `--bounds minor` to the script.
    uv_snapshot!(context.filters(), context.add().arg("anyio").arg("--bounds").arg("minor").arg("--script").arg("script.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    let script_content = context.read("script.py");

    // The script should have bounds with minor version constraint (e.g., `>=4.3.0,<4.4.0`).
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            script_content, @r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = [
        #     "anyio>=4.3.0,<4.4.0",
        # ]
        # ///
        print("Hello, world!")
        "#
        );
    });

    // Adding to a script without a lockfile shouldn't create a lockfile.
    assert!(!context.temp_dir.join("script.py.lock").exists());

    Ok(())
}

#[test]
fn add_script_relative_path() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let project = context.temp_dir.child("project");
    project.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        print("Hello, world!")
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("./project").arg("--editable").arg("--script").arg("script.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");

    let script_content = context.read("script.py");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            script_content, @r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = [
        #     "project",
        # ]
        #
        # [tool.uv.sources]
        # project = { path = "project", editable = true }
        # ///
        print("Hello, world!")
        "#
        );
    });
    Ok(())
}

/// Respect inline settings when adding to a PEP 732 script.
#[test]
fn add_script_settings() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "requests>=2",
        #   "rich>=12",
        # ]
        #
        # [tool.uv]
        # resolution = "lowest-direct"
        # ///

        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
    "#})?;

    // Lock the script.
    uv_snapshot!(context.filters(), context.lock().arg("--script").arg("script.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    // Add `anyio` to the script.
    uv_snapshot!(context.filters(), context.add().arg("anyio>=3").arg("--script").arg("script.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    ");

    let script_content = context.read("script.py");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            script_content, @r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "anyio>=3",
        #   "requests>=2",
        #   "rich>=12",
        # ]
        #
        # [tool.uv]
        # resolution = "lowest-direct"
        # ///

        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
        "#
        );
    });

    let lock = context.read("script.py.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.11"

        [options]
        resolution-mode = "lowest-direct"
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        requirements = [
            { name = "anyio", specifier = ">=3" },
            { name = "requests", specifier = ">=2" },
            { name = "rich", specifier = ">=12" },
        ]

        [[package]]
        name = "anyio"
        version = "3.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-3.0.0.tar.gz", hash = "sha256:62fb42b0d181821ab3c819567d140c927006c2fbf64f4785b1259ed9bd7da21e", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-3.0.0-py3-none-any.whl", hash = "sha256:d2b073c2957df967f830ad0b7ddac8fc687bb90c47b329beb0e2acc253f896cb", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "commonmark"
        version = "0.9.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/commonmark-0.9.1.tar.gz", hash = "sha256:23cb950ad3f57034251b9a44444741c050ee92d6caa2240b598ca998fe82c44a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/commonmark-0.9.1-py3-none-any.whl", hash = "sha256:abe47cb5cae3422539272ad42c3084d77bda41cd9c43aedca15cf4f68cd98365", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "pygments"
        version = "2.17.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/pygments-2.17.2.tar.gz", hash = "sha256:74fb3e798e4ca65f3d50f25a592742c475802e21ba90feaea0a668e148bba565", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/pygments-2.17.2-py3-none-any.whl", hash = "sha256:21fec4bd796036ea6e3f2693c9749ca80863f36d1894ffb931199d24877422a0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "requests"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/requests-2.0.0.tar.gz", hash = "sha256:a5f62ac111bb1e8102e1ebb2c0eecba8f542dbb044e50b746feb5100310c71bf", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/requests-2.0.0-py3-none-any.whl", hash = "sha256:02d618663ec3aeaa847494c6dd6ddf247ff5f4b4bfde2ef5f0d8bb5e220d55f0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "rich"
        version = "12.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "commonmark" },
            { name = "pygments" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/rich-12.0.0.tar.gz", hash = "sha256:ae425d18518b114cfd7127aff81241ef275a519bdbf1d37787388ca16aadd3ff", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/rich-12.0.0-py3-none-any.whl", hash = "sha256:3536c3825d993c57e6dbdf76440db1d5bc642879ce6d4954e773211e838d5391", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    Ok(())
}

#[test]
fn add_script_trailing_comment_lines() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "requests<3",
        #   "rich",
        # ]
        # ///
        #
        # Additional description

        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio").arg("--script").arg("script.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 11 packages in [TIME]
    ");

    let script_content = context.read("script.py");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            script_content, @r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "anyio>=4.3.0",
        #   "requests<3",
        #   "rich",
        # ]
        # ///
        #
        # Additional description

        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
        "#
        );
    });

    // Adding to a script without a lockfile shouldn't create a lockfile.
    assert!(!context.temp_dir.join("script.py.lock").exists());

    Ok(())
}

/// Add to a script without an existing metadata table.
#[test]
fn add_script_without_metadata_table() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
    "#})?;

    uv_snapshot!(context.filters(), context.add().args(["rich", "requests<3"]).arg("--script").arg("script.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 9 packages in [TIME]
    ");

    let script_content = context.read("script.py");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            script_content, @r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = [
        #     "requests<3",
        #     "rich>=13.7.1",
        # ]
        # ///
        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
        "#
        );
    });
    Ok(())
}

/// Add to a script without an existing metadata table, but with a shebang.
#[test]
fn add_script_without_metadata_table_with_shebang() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        #!/usr/bin/env python3
        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
    "#})?;

    uv_snapshot!(context.filters(), context.add().args(["rich", "requests<3"]).arg("--script").arg("script.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 9 packages in [TIME]
    ");

    let script_content = context.read("script.py");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            script_content, @r#"
        #!/usr/bin/env python3
        # /// script
        # requires-python = ">=3.12"
        # dependencies = [
        #     "requests<3",
        #     "rich>=13.7.1",
        # ]
        # ///
        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
        "#
        );
    });
    Ok(())
}

/// Add to a script with a metadata table and a shebang.
#[test]
fn add_script_with_metadata_table_and_shebang() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        #!/usr/bin/env python3
        # /// script
        # requires-python = ">=3.12"
        # dependencies = []
        # ///
        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
    "#})?;

    uv_snapshot!(context.filters(), context.add().args(["rich", "requests<3"]).arg("--script").arg("script.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 9 packages in [TIME]
    ");

    let script_content = context.read("script.py");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            script_content, @r#"
        #!/usr/bin/env python3
        # /// script
        # requires-python = ">=3.12"
        # dependencies = [
        #     "requests<3",
        #     "rich>=13.7.1",
        # ]
        # ///
        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
        "#
        );
    });
    Ok(())
}

/// Add to a script without a metadata table, but with a docstring.
#[test]
fn add_script_without_metadata_table_with_docstring() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        """This is a script."""
        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
    "#})?;

    uv_snapshot!(context.filters(), context.add().args(["rich", "requests<3"]).arg("--script").arg("script.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 9 packages in [TIME]
    ");

    let script_content = context.read("script.py");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            script_content, @r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = [
        #     "requests<3",
        #     "rich>=13.7.1",
        # ]
        # ///
        """This is a script."""
        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
        "#
        );
    });
    Ok(())
}

/// Add to a script without a `.py` extension.
#[test]
fn add_extensionless_script() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let script = context.temp_dir.child("script");
    script.write_str(indoc! {r#"
        #!/usr/bin/env python3
        # /// script
        # requires-python = ">=3.12"
        # dependencies = []
        # ///
        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
    "#})?;

    uv_snapshot!(context.filters(), context.add().args(["rich", "requests<3"]).arg("--script").arg("script"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 9 packages in [TIME]
    ");

    let script_content = context.read("script");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            script_content, @r#"
        #!/usr/bin/env python3
        # /// script
        # requires-python = ">=3.12"
        # dependencies = [
        #     "requests<3",
        #     "rich>=13.7.1",
        # ]
        # ///
        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
        "#
        );
    });
    Ok(())
}

/// Add from a remote PEP 723 script via `-r`.
#[tokio::test]
async fn add_requirements_from_remote_script() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Create a mock server that serves a PEP 723 script.
    let server = MockServer::start().await;
    let script_content = indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = [
        #     "anyio>=4",
        #     "rich",
        # ]
        # ///
        import anyio
        from rich.pretty import pprint
        pprint("Hello, world!")
    "#};

    Mock::given(method("GET"))
        .and(path("/script"))
        .respond_with(ResponseTemplate::new(200).set_body_string(script_content))
        .mount(&server)
        .await;

    let script_url = format!("{}/script", server.uri());

    uv_snapshot!(context.filters(), context.add().arg("-r").arg(&script_url), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 7 packages in [TIME]
    Installed 7 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + markdown-it-py==3.0.0
     + mdurl==0.1.2
     + pygments==2.17.2
     + rich==13.7.1
     + sniffio==1.3.1
    ");

    let pyproject_toml_content = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml_content, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "anyio>=4",
            "rich>=13.7.1",
        ]
        "#
        );
    });

    Ok(())
}

/// Remove a dependency that is present in multiple places.
#[test]
fn remove_repeated() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let anyio_local = context.workspace_root.join("test/packages/anyio_local");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio"]

        [project.optional-dependencies]
        foo = ["anyio"]

        [tool.uv]
        dev-dependencies = ["anyio"]

        [tool.uv.sources]
        anyio = {{ path = "{anyio_local}" }}
    "#,
        anyio_local = anyio_local.portable_display(),
    })?;

    uv_snapshot!(context.filters(), context.remove().arg("anyio"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: The `tool.uv.dev-dependencies` field (used in `pyproject.toml`) is deprecated and will be removed in a future release; use `dependency-groups.dev` instead
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.3.0+foo (from file://[WORKSPACE]/test/packages/anyio_local)
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        foo = ["anyio"]

        [tool.uv]
        dev-dependencies = ["anyio"]

        [tool.uv.sources]
        anyio = { path = "[WORKSPACE]/test/packages/anyio_local" }
        "#
        );
    });

    uv_snapshot!(context.filters(), context.remove().arg("anyio").arg("--optional").arg("foo"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: The `tool.uv.dev-dependencies` field (used in `pyproject.toml`) is deprecated and will be removed in a future release; use `dependency-groups.dev` instead
    Resolved 2 packages in [TIME]
    Checked 1 package in [TIME]
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        foo = []

        [tool.uv]
        dev-dependencies = ["anyio"]

        [tool.uv.sources]
        anyio = { path = "[WORKSPACE]/test/packages/anyio_local" }
        "#
        );
    });

    uv_snapshot!(context.filters(), context.remove().arg("anyio").arg("--dev"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: The `tool.uv.dev-dependencies` field (used in `pyproject.toml`) is deprecated and will be removed in a future release; use `dependency-groups.dev` instead
    Resolved 1 package in [TIME]
    Uninstalled 1 package in [TIME]
     - anyio==4.3.0+foo (from file://[WORKSPACE]/test/packages/anyio_local)
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        foo = []

        [tool.uv]
        dev-dependencies = []
        "#
        );
    });
    Ok(())
}

/// Add to (and remove from) a PEP 732 script with a lockfile.
#[test]
fn add_remove_script_lock() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "requests<3",
        #   "rich",
        # ]
        # ///

        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
    "#})?;

    // Explicitly lock the script.
    uv_snapshot!(context.filters(), context.lock().arg("--script").arg("script.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 9 packages in [TIME]
    ");

    let lock = context.read("script.py.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.11"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        requirements = [
            { name = "requests", specifier = "<3" },
            { name = "rich" },
        ]

        [[package]]
        name = "certifi"
        version = "2024.2.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/certifi-2024.2.2.tar.gz", hash = "sha256:e91f672a47ba5696e377a4c21f8166067bef36ae757b778af0fa4b291cbe259c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/certifi-2024.2.2-py3-none-any.whl", hash = "sha256:a51a951e128c84716d27bbad83ab60ef28c5e00b89980690f0fa1a4f18c3919c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "charset-normalizer"
        version = "3.3.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/charset_normalizer-3.3.2.tar.gz", hash = "sha256:1f99fa75c8f89bf493bf79aa0018d58b9d483482e53db6b89cd57ad4b3c19f69", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/charset_normalizer-3.3.2-py3-none-any.whl", hash = "sha256:517568d9d94db8f21bd3f43e2f562112a955f912637d0ea38f2d9c9573b58427", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "markdown-it-py"
        version = "3.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "mdurl" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/markdown_it_py-3.0.0.tar.gz", hash = "sha256:9da074ee4cbdb8bae512b0dd3d24668030a5563e3092a8820f84d169c91d7054", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/markdown_it_py-3.0.0-py3-none-any.whl", hash = "sha256:ccf6685e82b431f4e3a8742f14143f99b51076d5f6755ec8fdf1deb2f7e61fd1", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "mdurl"
        version = "0.1.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/mdurl-0.1.2.tar.gz", hash = "sha256:f011e4fc1812d50c7263e03f2a1c85f330773012611e167c40612f57f9c5a78b", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/mdurl-0.1.2-py3-none-any.whl", hash = "sha256:4562cf0c7d3b8ad3a9159954291743a108b557ee3cb6da5e405a182593283e55", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "pygments"
        version = "2.17.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/pygments-2.17.2.tar.gz", hash = "sha256:74fb3e798e4ca65f3d50f25a592742c475802e21ba90feaea0a668e148bba565", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/pygments-2.17.2-py3-none-any.whl", hash = "sha256:21fec4bd796036ea6e3f2693c9749ca80863f36d1894ffb931199d24877422a0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "requests"
        version = "2.31.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "certifi" },
            { name = "charset-normalizer" },
            { name = "idna" },
            { name = "urllib3" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/requests-2.31.0.tar.gz", hash = "sha256:221003429db202ebdbf13d86243ec4c1a397306221796b6e1a0c78c6c99930ab", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/requests-2.31.0-py3-none-any.whl", hash = "sha256:a75969d96235bdbe3ee8cd8e73ef08fedd0c44cd5262d1b5a7a99a4360716226", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "rich"
        version = "13.7.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "markdown-it-py" },
            { name = "pygments" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/rich-13.7.1.tar.gz", hash = "sha256:8e4f50ed64b8c95c6c0c11db5ded04add7162caa4f9e25b3c38a6da820edcb14", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/rich-13.7.1-py3-none-any.whl", hash = "sha256:aab91bc07f6736693c9edeb64d7fe0914dd7262021a32951f3230c67e67a93c2", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "urllib3"
        version = "2.2.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/urllib3-2.2.1.tar.gz", hash = "sha256:bc130c8be0c82b95aa705fee217ecec8588ffe784002bc2f0c7b344be12d2571", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/urllib3-2.2.1-py3-none-any.whl", hash = "sha256:1c9d3ba42fe5f5336585783bffa353950639e24f7edc0dd26f04839cc3c0cc32", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Adding to a locked script should update the lockfile.
    uv_snapshot!(context.filters(), context.add().arg("anyio").arg("--script").arg("script.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 11 packages in [TIME]
    ");

    let script_content = context.read("script.py");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            script_content, @r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "anyio>=4.3.0",
        #   "requests<3",
        #   "rich",
        # ]
        # ///

        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
        "#
        );
    });

    let lock = context.read("script.py.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.11"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        requirements = [
            { name = "anyio", specifier = ">=4.3.0" },
            { name = "requests", specifier = "<3" },
            { name = "rich" },
        ]

        [[package]]
        name = "anyio"
        version = "4.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-4.3.0.tar.gz", hash = "sha256:13a6d97fa30ec110d85e3949a30c92306f0178135048329f54a335c3dade753a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-4.3.0-py3-none-any.whl", hash = "sha256:c4f443e7e5a2c003b1534688207e85dbd11960efb66d4d6a4e7693fdfc6f5b33", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "certifi"
        version = "2024.2.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/certifi-2024.2.2.tar.gz", hash = "sha256:e91f672a47ba5696e377a4c21f8166067bef36ae757b778af0fa4b291cbe259c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/certifi-2024.2.2-py3-none-any.whl", hash = "sha256:a51a951e128c84716d27bbad83ab60ef28c5e00b89980690f0fa1a4f18c3919c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "charset-normalizer"
        version = "3.3.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/charset_normalizer-3.3.2.tar.gz", hash = "sha256:1f99fa75c8f89bf493bf79aa0018d58b9d483482e53db6b89cd57ad4b3c19f69", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/charset_normalizer-3.3.2-py3-none-any.whl", hash = "sha256:517568d9d94db8f21bd3f43e2f562112a955f912637d0ea38f2d9c9573b58427", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "markdown-it-py"
        version = "3.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "mdurl" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/markdown_it_py-3.0.0.tar.gz", hash = "sha256:9da074ee4cbdb8bae512b0dd3d24668030a5563e3092a8820f84d169c91d7054", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/markdown_it_py-3.0.0-py3-none-any.whl", hash = "sha256:ccf6685e82b431f4e3a8742f14143f99b51076d5f6755ec8fdf1deb2f7e61fd1", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "mdurl"
        version = "0.1.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/mdurl-0.1.2.tar.gz", hash = "sha256:f011e4fc1812d50c7263e03f2a1c85f330773012611e167c40612f57f9c5a78b", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/mdurl-0.1.2-py3-none-any.whl", hash = "sha256:4562cf0c7d3b8ad3a9159954291743a108b557ee3cb6da5e405a182593283e55", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "pygments"
        version = "2.17.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/pygments-2.17.2.tar.gz", hash = "sha256:74fb3e798e4ca65f3d50f25a592742c475802e21ba90feaea0a668e148bba565", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/pygments-2.17.2-py3-none-any.whl", hash = "sha256:21fec4bd796036ea6e3f2693c9749ca80863f36d1894ffb931199d24877422a0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "requests"
        version = "2.31.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "certifi" },
            { name = "charset-normalizer" },
            { name = "idna" },
            { name = "urllib3" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/requests-2.31.0.tar.gz", hash = "sha256:221003429db202ebdbf13d86243ec4c1a397306221796b6e1a0c78c6c99930ab", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/requests-2.31.0-py3-none-any.whl", hash = "sha256:a75969d96235bdbe3ee8cd8e73ef08fedd0c44cd5262d1b5a7a99a4360716226", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "rich"
        version = "13.7.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "markdown-it-py" },
            { name = "pygments" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/rich-13.7.1.tar.gz", hash = "sha256:8e4f50ed64b8c95c6c0c11db5ded04add7162caa4f9e25b3c38a6da820edcb14", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/rich-13.7.1-py3-none-any.whl", hash = "sha256:aab91bc07f6736693c9edeb64d7fe0914dd7262021a32951f3230c67e67a93c2", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "urllib3"
        version = "2.2.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/urllib3-2.2.1.tar.gz", hash = "sha256:bc130c8be0c82b95aa705fee217ecec8588ffe784002bc2f0c7b344be12d2571", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/urllib3-2.2.1-py3-none-any.whl", hash = "sha256:1c9d3ba42fe5f5336585783bffa353950639e24f7edc0dd26f04839cc3c0cc32", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Removing from a locked script should update the lockfile.
    uv_snapshot!(context.filters(), context.remove().arg("anyio").arg("--script").arg("script.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 9 packages in [TIME]
    ");

    let script_content = context.read("script.py");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            script_content, @r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "requests<3",
        #   "rich",
        # ]
        # ///

        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
        "#
        );
    });

    let lock = context.read("script.py.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.11"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [manifest]
        requirements = [
            { name = "requests", specifier = "<3" },
            { name = "rich" },
        ]

        [[package]]
        name = "certifi"
        version = "2024.2.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/certifi-2024.2.2.tar.gz", hash = "sha256:e91f672a47ba5696e377a4c21f8166067bef36ae757b778af0fa4b291cbe259c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/certifi-2024.2.2-py3-none-any.whl", hash = "sha256:a51a951e128c84716d27bbad83ab60ef28c5e00b89980690f0fa1a4f18c3919c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "charset-normalizer"
        version = "3.3.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/charset_normalizer-3.3.2.tar.gz", hash = "sha256:1f99fa75c8f89bf493bf79aa0018d58b9d483482e53db6b89cd57ad4b3c19f69", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/charset_normalizer-3.3.2-py3-none-any.whl", hash = "sha256:517568d9d94db8f21bd3f43e2f562112a955f912637d0ea38f2d9c9573b58427", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "markdown-it-py"
        version = "3.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "mdurl" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/markdown_it_py-3.0.0.tar.gz", hash = "sha256:9da074ee4cbdb8bae512b0dd3d24668030a5563e3092a8820f84d169c91d7054", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/markdown_it_py-3.0.0-py3-none-any.whl", hash = "sha256:ccf6685e82b431f4e3a8742f14143f99b51076d5f6755ec8fdf1deb2f7e61fd1", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "mdurl"
        version = "0.1.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/mdurl-0.1.2.tar.gz", hash = "sha256:f011e4fc1812d50c7263e03f2a1c85f330773012611e167c40612f57f9c5a78b", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/mdurl-0.1.2-py3-none-any.whl", hash = "sha256:4562cf0c7d3b8ad3a9159954291743a108b557ee3cb6da5e405a182593283e55", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "pygments"
        version = "2.17.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/pygments-2.17.2.tar.gz", hash = "sha256:74fb3e798e4ca65f3d50f25a592742c475802e21ba90feaea0a668e148bba565", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/pygments-2.17.2-py3-none-any.whl", hash = "sha256:21fec4bd796036ea6e3f2693c9749ca80863f36d1894ffb931199d24877422a0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "requests"
        version = "2.31.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "certifi" },
            { name = "charset-normalizer" },
            { name = "idna" },
            { name = "urllib3" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/requests-2.31.0.tar.gz", hash = "sha256:221003429db202ebdbf13d86243ec4c1a397306221796b6e1a0c78c6c99930ab", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/requests-2.31.0-py3-none-any.whl", hash = "sha256:a75969d96235bdbe3ee8cd8e73ef08fedd0c44cd5262d1b5a7a99a4360716226", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "rich"
        version = "13.7.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "markdown-it-py" },
            { name = "pygments" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/rich-13.7.1.tar.gz", hash = "sha256:8e4f50ed64b8c95c6c0c11db5ded04add7162caa4f9e25b3c38a6da820edcb14", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/rich-13.7.1-py3-none-any.whl", hash = "sha256:aab91bc07f6736693c9edeb64d7fe0914dd7262021a32951f3230c67e67a93c2", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "urllib3"
        version = "2.2.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/urllib3-2.2.1.tar.gz", hash = "sha256:bc130c8be0c82b95aa705fee217ecec8588ffe784002bc2f0c7b344be12d2571", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/urllib3-2.2.1-py3-none-any.whl", hash = "sha256:1c9d3ba42fe5f5336585783bffa353950639e24f7edc0dd26f04839cc3c0cc32", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    Ok(())
}

/// Remove from a PEP 723 script.
#[test]
fn remove_script() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "requests<3",
        #   "rich",
        #   "anyio",
        # ]
        # ///

        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
    "#})?;

    uv_snapshot!(context.filters(), context.remove().arg("anyio").arg("--script").arg("script.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Updated `script.py`
    ");

    let script_content = context.read("script.py");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            script_content, @r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "requests<3",
        #   "rich",
        # ]
        # ///

        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
        "#
        );
    });
    Ok(())
}

/// `uv remove --dev` cannot be used with a PEP 723 script.
#[test]
fn remove_dev_script_conflict() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("script.py").write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = ["anyio"]
        # ///
    "#})?;

    uv_snapshot!(context.filters(), context.remove().arg("anyio").arg("--dev").arg("--script").arg("script.py"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--dev' cannot be used with '--script <SCRIPT>'

    Usage: uv remove --cache-dir [CACHE_DIR] --dev --exclude-newer <EXCLUDE_NEWER> <PACKAGES>...

    For more information, try '--help'.
    ");

    Ok(())
}

/// Remove last dependency PEP 723 script
#[test]
fn remove_last_dep_script() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "rich",
        # ]
        # ///

        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
    "#})?;

    uv_snapshot!(context.filters(), context.remove().arg("rich").arg("--script").arg("script.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Updated `script.py`
    ");

    let script_content = context.read("script.py");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            script_content, @r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = []
        # ///

        import requests
        from rich.pretty import pprint

        resp = requests.get("https://peps.python.org/api/peps.json")
        data = resp.json()
        pprint([(k, v["title"]) for k, v in data.items()][:10])
        "#
        );
    });
    Ok(())
}

/// Add a Git requirement to PEP 723 script.
#[test]
#[cfg(feature = "test-git")]
fn add_git_to_script() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let script = context.temp_dir.child("script.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "anyio",
        # ]
        # ///

        import anyio
        import uv_public_pypackage
    "#})?;

    uv_snapshot!(context.filters(), context
        .add()
        .arg("uv-public-pypackage @ git+https://github.com/astral-test/uv-public-pypackage")
        .arg("--tag=0.0.1")
        .arg("--script")
        .arg("script.py"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    let script_content = context.read("script.py");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            script_content, @r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "anyio",
        #   "uv-public-pypackage",
        # ]
        #
        # [tool.uv.sources]
        # uv-public-pypackage = { git = "https://github.com/astral-test/uv-public-pypackage", tag = "0.0.1" }
        # ///

        import anyio
        import uv_public_pypackage
        "#
        );
    });

    // Ensure that the script runs without error.
    context.run().arg("script.py").assert().success();

    Ok(())
}

#[test]
fn add_include_default_groups() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [dependency-groups]
        foo = ["anyio"]

        [tool.uv]
        default-groups = ["foo"]
    "#})?;

    // add should install default groups.
    uv_snapshot!(context.filters(), context.add().arg("typing-extensions"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
     + typing-extensions==4.10.0
    ");

    let pyproject_toml = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "typing-extensions>=4.10.0",
        ]

        [dependency-groups]
        foo = ["anyio"]

        [tool.uv]
        default-groups = ["foo"]
        "#
        );
    });

    assert!(context.temp_dir.join("uv.lock").exists());

    Ok(())
}

#[test]
fn remove_include_default_groups() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "typing-extensions>=4.10.0",
        ]

        [dependency-groups]
        dev = ["anyio"]
    "#})?;

    // remove should install default groups.
    uv_snapshot!(context.filters(), context.remove().arg("typing-extensions"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let pyproject_toml = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [dependency-groups]
        dev = ["anyio"]
        "#
        );
    });

    assert!(context.temp_dir.join("uv.lock").exists());

    Ok(())
}

/// Revert changes to the `pyproject.toml` and `uv.lock` when the `add` operation fails.
#[test]
fn fail_to_add_revert_project() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Add a dependency on a package that declares static metadata (so can always resolve), but
    // can't be installed.
    let pyproject_toml = context.temp_dir.child("child/pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig"]

        [build-system]
        requires = ["setuptools"]
        build-backend = "setuptools.build_meta"
    "#})?;
    context
        .temp_dir
        .child("child")
        .child("setup.py")
        .write_str("1/0")?;

    uv_snapshot!(context.filters(), context.add().arg("./child").arg("--no-workspace"), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    error: Failed to add dependencies
      cause: Failed to build `child @ file://[TEMP_DIR]/child`
      cause: The build backend returned an error
      cause: Call to `setuptools.build_meta.build_wheel` failed (exit status: 1)

             [stderr]
             Traceback (most recent call last):
               File "<string>", line 14, in <module>
               File "[CACHE_DIR]/builds-v0/[TMP]/[PYTHON-LIB]/site-packages/setuptools/build_meta.py", line 325, in get_requires_for_build_wheel
                 return self._get_build_requires(config_settings, requirements=['wheel'])
                        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
               File "[CACHE_DIR]/builds-v0/[TMP]/[PYTHON-LIB]/site-packages/setuptools/build_meta.py", line 295, in _get_build_requires
                 self.run_setup()
               File "[CACHE_DIR]/builds-v0/[TMP]/[PYTHON-LIB]/site-packages/setuptools/build_meta.py", line 311, in run_setup
                 exec(code, locals())
               File "<string>", line 1, in <module>
             ZeroDivisionError: division by zero

    hint: `child` was included because `parent` (v0.1.0) depends on `child`

    hint: Build failures usually indicate a problem with the package or the build environment

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    "#);

    let pyproject_toml = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
        "#
        );
    });

    // The lockfile should not exist, even though resolution succeeded.
    assert!(!context.temp_dir.join("uv.lock").exists());

    Ok(())
}

/// Revert changes to the `pyproject.toml` and `uv.lock` when the `add` operation fails.
///
/// In this case, the project has an existing lockfile.
#[test]
fn fail_to_edit_revert_project() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("iniconfig"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    let before = fs_err::read_to_string(context.temp_dir.join("uv.lock"))?;

    // Add a dependency on a package that declares static metadata (so can always resolve), but
    // can't be installed.
    let pyproject_toml = context.temp_dir.child("child/pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig"]

        [build-system]
        requires = ["setuptools"]
        build-backend = "setuptools.build_meta"
    "#})?;
    context
        .temp_dir
        .child("child")
        .child("setup.py")
        .write_str("1/0")?;

    uv_snapshot!(context.filters(), context.add().arg("./child").arg("--no-workspace"), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    error: Failed to add dependencies
      cause: Failed to build `child @ file://[TEMP_DIR]/child`
      cause: The build backend returned an error
      cause: Call to `setuptools.build_meta.build_wheel` failed (exit status: 1)

             [stderr]
             Traceback (most recent call last):
               File "<string>", line 14, in <module>
               File "[CACHE_DIR]/builds-v0/[TMP]/[PYTHON-LIB]/site-packages/setuptools/build_meta.py", line 325, in get_requires_for_build_wheel
                 return self._get_build_requires(config_settings, requirements=['wheel'])
                        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
               File "[CACHE_DIR]/builds-v0/[TMP]/[PYTHON-LIB]/site-packages/setuptools/build_meta.py", line 295, in _get_build_requires
                 self.run_setup()
               File "[CACHE_DIR]/builds-v0/[TMP]/[PYTHON-LIB]/site-packages/setuptools/build_meta.py", line 311, in run_setup
                 exec(code, locals())
               File "<string>", line 1, in <module>
             ZeroDivisionError: division by zero

    hint: `child` was included because `parent` (v0.1.0) depends on `child`

    hint: Build failures usually indicate a problem with the package or the build environment

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    "#);

    let pyproject_toml = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig>=2.0.0",
        ]
        "#
        );
    });

    // The lockfile should exist, but be unchanged.
    let after = fs_err::read_to_string(context.temp_dir.join("uv.lock"))?;
    assert_eq!(before, after);

    Ok(())
}

/// Revert changes to the root `pyproject.toml` and `uv.lock` when the `add` operation fails.
#[test]
fn fail_to_add_revert_workspace_root() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Add a dependency on a package that declares static metadata (so can always resolve), but
    // can't be installed.
    let pyproject_toml = context.temp_dir.child("child/pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig"]

        [build-system]
        requires = ["setuptools"]
        build-backend = "setuptools.build_meta"
    "#})?;
    context
        .temp_dir
        .child("child")
        .child("setup.py")
        .write_str("1/0")?;

    // Add a dependency on a package that declares static metadata (so can always resolve), but
    // can't be installed.
    let pyproject_toml = context.temp_dir.child("broken").child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "broken"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig"]

        [build-system]
        requires = ["setuptools"]
        build-backend = "setuptools.build_meta"
    "#})?;
    context
        .temp_dir
        .child("broken")
        .child("setup.py")
        .write_str("1/0")?;

    uv_snapshot!(context.filters(), context.add().arg("./broken"), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    Added `broken` to workspace members
    Resolved 3 packages in [TIME]
    error: Failed to add dependencies
      cause: Failed to build `broken @ file://[TEMP_DIR]/broken`
      cause: The build backend returned an error
      cause: Call to `setuptools.build_meta.build_editable` failed (exit status: 1)

             [stderr]
             Traceback (most recent call last):
               File "<string>", line 14, in <module>
               File "[CACHE_DIR]/builds-v0/[TMP]/[PYTHON-LIB]/site-packages/setuptools/build_meta.py", line 448, in get_requires_for_build_editable
                 return self.get_requires_for_build_wheel(config_settings)
                        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
               File "[CACHE_DIR]/builds-v0/[TMP]/[PYTHON-LIB]/site-packages/setuptools/build_meta.py", line 325, in get_requires_for_build_wheel
                 return self._get_build_requires(config_settings, requirements=['wheel'])
                        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
               File "[CACHE_DIR]/builds-v0/[TMP]/[PYTHON-LIB]/site-packages/setuptools/build_meta.py", line 295, in _get_build_requires
                 self.run_setup()
               File "[CACHE_DIR]/builds-v0/[TMP]/[PYTHON-LIB]/site-packages/setuptools/build_meta.py", line 311, in run_setup
                 exec(code, locals())
               File "<string>", line 1, in <module>
             ZeroDivisionError: division by zero

    hint: `broken` was included because `parent` (v0.1.0) depends on `broken`

    hint: Build failures usually indicate a problem with the package or the build environment

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    "#);

    let pyproject_toml = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
        "#
        );
    });

    // The lockfile should not exist, even though resolution succeeded.
    assert!(!context.temp_dir.join("uv.lock").exists());

    Ok(())
}

/// Revert changes to the root `pyproject.toml` and `uv.lock` when the `add` operation fails.
#[test]
fn fail_to_add_revert_workspace_member() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["child"]

        [tool.uv.workspace]
        members = ["child"]

        [tool.uv.sources]
        child = { workspace = true }
    "#})?;

    // Add a workspace dependency.
    let project = context.temp_dir.child("child");
    project.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig"]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
    "#})?;
    project
        .child("src")
        .child("child")
        .child("__init__.py")
        .touch()?;

    // Add a dependency on a package that declares static metadata (so can always resolve), but
    // can't be installed.
    let pyproject_toml = context.temp_dir.child("broken/pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "broken"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig"]

        [build-system]
        requires = ["setuptools"]
        build-backend = "setuptools.build_meta"
    "#})?;
    context
        .temp_dir
        .child("broken")
        .child("setup.py")
        .write_str("1/0")?;

    uv_snapshot!(context.filters(), context.add().current_dir(&project).arg("../broken"), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    Added `broken` to workspace members
    Resolved 4 packages in [TIME]
    error: Failed to add dependencies
      cause: Failed to build `broken @ file://[TEMP_DIR]/broken`
      cause: The build backend returned an error
      cause: Call to `setuptools.build_meta.build_editable` failed (exit status: 1)

             [stderr]
             Traceback (most recent call last):
               File "<string>", line 14, in <module>
               File "[CACHE_DIR]/builds-v0/[TMP]/[PYTHON-LIB]/site-packages/setuptools/build_meta.py", line 448, in get_requires_for_build_editable
                 return self.get_requires_for_build_wheel(config_settings)
                        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
               File "[CACHE_DIR]/builds-v0/[TMP]/[PYTHON-LIB]/site-packages/setuptools/build_meta.py", line 325, in get_requires_for_build_wheel
                 return self._get_build_requires(config_settings, requirements=['wheel'])
                        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
               File "[CACHE_DIR]/builds-v0/[TMP]/[PYTHON-LIB]/site-packages/setuptools/build_meta.py", line 295, in _get_build_requires
                 self.run_setup()
               File "[CACHE_DIR]/builds-v0/[TMP]/[PYTHON-LIB]/site-packages/setuptools/build_meta.py", line 311, in run_setup
                 exec(code, locals())
               File "<string>", line 1, in <module>
             ZeroDivisionError: division by zero

    hint: `broken` was included because `child` (v0.1.0) depends on `broken`

    hint: Build failures usually indicate a problem with the package or the build environment

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    "#);

    let pyproject_toml = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["child"]

        [tool.uv.workspace]
        members = ["child"]

        [tool.uv.sources]
        child = { workspace = true }
        "#
        );
    });

    let pyproject_toml =
        fs_err::read_to_string(context.temp_dir.join("child").join("pyproject.toml"))?;
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig"]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
        "#
        );
    });

    // The lockfile should not exist, even though resolution succeeded.
    assert!(!context.temp_dir.join("uv.lock").exists());

    Ok(())
}

/// Ensure that the added dependencies are sorted if the dependency list was already sorted prior
/// to the operation.
#[test]
fn sorted_dependencies() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = [
        "CacheControl[filecache]>=0.14,<0.15",
        "iniconfig",
    ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().args(["typing-extensions", "anyio"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 7 packages in [TIME]
    Installed 7 packages in [TIME]
     + anyio==4.3.0
     + cachecontrol==0.14.0
     + filelock==3.8.0
     + idna==3.6
     + iniconfig==2.0.0
     + sniffio==1.3.1
     + typing-extensions==4.10.0
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "anyio>=4.3.0",
            "CacheControl[filecache]>=0.14,<0.15",
            "iniconfig",
            "typing-extensions>=4.10.0",
        ]
        "#
        );
    });
    Ok(())
}

/// Ensure that if the dependencies are sorted naively (i.e. by the whole
/// requirement specifier), that added dependencies are sorted in the same way.
#[test]
fn naive_sorted_dependencies() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_counts();

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = [
        "pytest-mock>=3.14",
        "pytest>=8.1.1",
    ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().args(["pytest-randomly"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + iniconfig==2.0.0
     + packaging==24.0
     + pluggy==1.4.0
     + pytest==8.1.1
     + pytest-mock==3.14.0
     + pytest-randomly==3.15.0
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "pytest-mock>=3.14",
            "pytest-randomly>=3.15.0",
            "pytest>=8.1.1",
        ]
        "#
        );
    });
    Ok(())
}

/// Ensure that the added dependencies are case sensitive sorted if the dependency list was already
/// case sensitive sorted prior to the operation.
#[test]
fn case_sensitive_sorted_dependencies() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = [
        "CacheControl[filecache]>=0.14,<0.15",
        "PyYAML",
        "iniconfig",
    ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().args(["typing-extensions", "anyio"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 9 packages in [TIME]
    Prepared 8 packages in [TIME]
    Installed 8 packages in [TIME]
     + anyio==4.3.0
     + cachecontrol==0.14.0
     + filelock==3.8.0
     + idna==3.6
     + iniconfig==2.0.0
     + pyyaml==6.0.2
     + sniffio==1.3.1
     + typing-extensions==4.10.0
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "CacheControl[filecache]>=0.14,<0.15",
            "PyYAML",
            "anyio>=4.3.0",
            "iniconfig",
            "typing-extensions>=4.10.0",
        ]
        "#
        );
    });
    Ok(())
}

/// Ensure that if the dependencies are sorted naively (i.e. by the whole
/// requirement specifier), that added dependencies are sorted in the same way.
#[test]
fn case_sensitive_naive_sorted_dependencies() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_counts();

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = [
        "Typing-extensions>=4.10.0",
        "pytest-mock>=3.14",
        "pytest>=8.1.1",
    ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().args(["pytest-randomly"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + iniconfig==2.0.0
     + packaging==24.0
     + pluggy==1.4.0
     + pytest==8.1.1
     + pytest-mock==3.14.0
     + pytest-randomly==3.15.0
     + typing-extensions==4.10.0
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "Typing-extensions>=4.10.0",
            "pytest-mock>=3.14",
            "pytest-randomly>=3.15.0",
            "pytest>=8.1.1",
        ]
        "#
        );
    });
    Ok(())
}

/// Ensure that sorting is based on the name, rather than the combined name-and-specifiers.
#[test]
fn sorted_dependencies_name_specifiers() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12.0"
        dependencies = [
            "pytest>=8",
            "typing-extensions>=4.10.0",
        ]
    "#})?;

    uv_snapshot!(context.filters(), universal_windows_filters=true, context.add().args(["pytest-mock"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + iniconfig==2.0.0
     + packaging==24.0
     + pluggy==1.4.0
     + pytest==8.1.1
     + pytest-mock==3.14.0
     + typing-extensions==4.10.0
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12.[X]"
        dependencies = [
            "pytest>=8",
            "pytest-mock>=3.14.0",
            "typing-extensions>=4.10.0",
        ]
        "#
        );
    });

    uv_snapshot!(context.filters(), universal_windows_filters=true, context.add().args(["pytest-randomly"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 9 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + pytest-randomly==3.15.0
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12.[X]"
        dependencies = [
            "pytest>=8",
            "pytest-mock>=3.14.0",
            "pytest-randomly>=3.15.0",
            "typing-extensions>=4.10.0",
        ]
        "#
        );
    });

    Ok(())
}

#[test]
fn sorted_dependencies_with_include_group() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_counts();

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"

    [dependency-groups]
    dev = [
        { include-group = "coverage" },
        "pytest-mock>=3.14",
        "pytest>=8.1.1",
    ]
    coverage = [
        "coverage>=7.4.4",
    ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().args(["--dev", "pytest-randomly"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + coverage==7.6.10
     + iniconfig==2.0.0
     + packaging==24.0
     + pluggy==1.4.0
     + pytest==8.1.1
     + pytest-mock==3.14.0
     + pytest-randomly==3.15.0
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [dependency-groups]
        dev = [
            { include-group = "coverage" },
            "pytest-mock>=3.14",
            "pytest-randomly>=3.15.0",
            "pytest>=8.1.1",
        ]
        coverage = [
            "coverage>=7.4.4",
        ]
        "#
        );
    });
    Ok(())
}

#[test]
fn sorted_dependencies_new_dependency_after_include_group() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_counts();

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"

    [dependency-groups]
    dev = [
        { include-group = "coverage" },
    ]
    coverage = [
        "coverage>=7.4.4",
    ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().args(["--dev", "pytest"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + coverage==7.6.10
     + iniconfig==2.0.0
     + packaging==24.0
     + pluggy==1.4.0
     + pytest==8.1.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [dependency-groups]
        dev = [
            { include-group = "coverage" },
            "pytest>=8.1.1",
        ]
        coverage = [
            "coverage>=7.4.4",
        ]
        "#
        );
    });
    Ok(())
}

#[test]
fn sorted_dependencies_include_group_kept_at_bottom() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_counts();

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"

    [dependency-groups]
    dev = [
        "pytest>=8.1.1",
        { include-group = "coverage" },
    ]
    coverage = [
        "coverage>=7.4.4",
    ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().args(["--dev", "pytest-randomly"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + coverage==7.6.10
     + iniconfig==2.0.0
     + packaging==24.0
     + pluggy==1.4.0
     + pytest==8.1.1
     + pytest-randomly==3.15.0
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [dependency-groups]
        dev = [
            "pytest>=8.1.1",
            "pytest-randomly>=3.15.0",
            { include-group = "coverage" },
        ]
        coverage = [
            "coverage>=7.4.4",
        ]
        "#
        );
    });
    Ok(())
}

/// Ensure that the custom ordering of the dependencies is preserved
/// after adding a package.
#[test]
fn custom_dependencies() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = [
        "yarl",
        "CacheControl[filecache]>=0.14,<0.15",
        "mwparserfromhell",
        "pywikibot",
        "sentry-sdk",
    ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("pydantic").arg("--frozen"), @"
    exit_code: 0 (success)
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "yarl",
            "CacheControl[filecache]>=0.14,<0.15",
            "mwparserfromhell",
            "pywikibot",
            "sentry-sdk",
            "pydantic",
        ]
        "#
        );
    });
    Ok(())
}

/// Regression test for: <https://github.com/astral-sh/uv/issues/7259>
#[test]
fn update_offset() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig",
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().args(["typing-extensions", "iniconfig"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + iniconfig==2.0.0
     + typing-extensions==4.10.0
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig",
            "typing-extensions>=4.10.0",
        ]
        "#
        );
    });

    Ok(())
}

/// Check hint for <https://github.com/astral-sh/uv/issues/7329>
/// if there is a broken cyclic dependency on a local package.
#[test]
fn add_shadowed_name() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "dagster"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Pinned constrained, check for a direct dependency loop.
    uv_snapshot!(context.filters(), context.add().arg("dagster-webserver==1.6.13"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to add dependencies
      cause: No solution found when resolving dependencies
      cause: Because dagster-webserver>=1.6.13 depends on your project and your project depends on dagster-webserver==1.6.13, we can conclude that your project's requirements are unsatisfiable.

    hint: The package `dagster-webserver` depends on the package `dagster` but the name is shadowed by your project. Consider changing the name of the project.

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    ");

    // Constraint with several available versions, check for an indirect dependency loop.
    uv_snapshot!(context.filters(), context.add().arg("dagster-webserver>=1.6.11,<1.7.0"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to add dependencies
      cause: No solution found when resolving dependencies
      cause: Because dagster-webserver<=1.6.11 depends on your project and dagster-webserver==1.6.12 depends on your project, we can conclude that dagster-webserver<=1.6.12 depends on your project.
             And because dagster-webserver>=1.6.13 depends on your project and your project depends on dagster-webserver, we can conclude that your project's requirements are unsatisfiable.

    hint: The package `dagster-webserver` depends on the package `dagster` but the name is shadowed by your project. Consider changing the name of the project.

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    ");

    Ok(())
}

/// Warn when a user provides an index via `--index-url` or `--extra-index-url`.
#[test]
fn add_warn_index_url() -> Result<()> {
    let index = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let incompatible_index = uv_test::packse::PackseServer::new("packages/edit-test-index.toml");
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("idna").arg("--index-url").arg(index.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Indexes specified via `--index-url` will not be persisted to the `pyproject.toml` file; use `--default-index` instead.
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + idna==3.6
    ");

    let pyproject_toml = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "idna>=3.6",
        ]
        "#
        );
    });

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "idna" },
        ]

        [package.metadata]
        requires-dist = [{ name = "idna", specifier = ">=3.6" }]
        "#
        );
    });

    uv_snapshot!(context.filters(), context.add().arg("iniconfig").arg("--extra-index-url").arg(incompatible_index.index_url()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    warning: Indexes specified via `--extra-index-url` will not be persisted to the `pyproject.toml` file; use `--index` instead.
    error: Failed to add dependencies
      cause: No solution found when resolving dependencies
      cause: Because only idna==2.7 is available and your project depends on idna>=3.6, we can conclude that your project's requirements are unsatisfiable.

    hint: `idna` was found on http://[LOCALHOST]/simple/, but not at the requested version (idna>=3.6). A compatible version may be available on a subsequent index (e.g., http://[LOCALHOST]/simple/). By default, uv will only consider versions that are published on the first index that contains a given package, to avoid dependency confusion attacks. If all indexes are equally trusted, use `--index-strategy unsafe-best-match` to consider all versions from all indexes, regardless of the order in which they were defined.

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    ");

    Ok(())
}

/// Don't warn if the user provides an index via `index-url` in `pyproject.toml`.
#[test]
fn add_no_warn_index_url() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
        [tool.uv]
        index-url = "{index_url}"
    "#,
        index_url = server.index_url(),
    })?;

    uv_snapshot!(context.filters(), context.add().arg("iniconfig"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    let pyproject_toml = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig>=2.0.0",
        ]
        [tool.uv]
        index-url = "http://[LOCALHOST]/simple/"
        "#
        );
    });

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz", hash = "sha256:48c42a08c0ec1a24f2fe45f4efdefc9c19ac8e0aa8e82284503ccba80398bec3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "iniconfig" },
        ]

        [package.metadata]
        requires-dist = [{ name = "iniconfig", specifier = ">=2.0.0" }]
        "#
        );
    });

    Ok(())
}

/// Add an index provided via `--index`.
#[test]
fn add_index() -> Result<()> {
    let index = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let pytorch_index = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let replacement_index = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12").with_exclude_newer("2025-01-30T00:00Z");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        constraint-dependencies = ["markupsafe<3"]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("iniconfig==2.0.0").arg("--index").arg(index.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    let pyproject_toml = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig==2.0.0",
        ]

        [tool.uv]
        constraint-dependencies = ["markupsafe<3"]

        [[tool.uv.index]]
        url = "http://[LOCALHOST]/simple/"
        "#
        );
    });

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2025-01-30T00:00:00Z"

        [manifest]
        constraints = [{ name = "markupsafe", specifier = "<3" }]

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz", hash = "sha256:48c42a08c0ec1a24f2fe45f4efdefc9c19ac8e0aa8e82284503ccba80398bec3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "iniconfig" },
        ]

        [package.metadata]
        requires-dist = [{ name = "iniconfig", specifier = "==2.0.0" }]
        "#
        );
    });

    // Adding a subsequent index should put it _above_ the existing index.
    uv_snapshot!(context.filters(), context.add().arg("jinja2").arg("--index").arg(format!("pytorch={}", pytorch_index.index_url())), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + jinja2==3.1.3
     + markupsafe==2.1.5
    ");

    let pyproject_toml = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig==2.0.0",
            "jinja2>=3.1.3",
        ]

        [tool.uv]
        constraint-dependencies = ["markupsafe<3"]

        [[tool.uv.index]]
        name = "pytorch"
        url = "http://[LOCALHOST]/simple/"

        [tool.uv.sources]
        jinja2 = { index = "pytorch" }

        [[tool.uv.index]]
        url = "http://[LOCALHOST]/simple/"
        "#
        );
    });

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2025-01-30T00:00:00Z"

        [manifest]
        constraints = [{ name = "markupsafe", specifier = "<3" }]

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz", hash = "sha256:48c42a08c0ec1a24f2fe45f4efdefc9c19ac8e0aa8e82284503ccba80398bec3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "jinja2"
        version = "3.1.3"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "markupsafe" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/jinja2-3.1.3.tar.gz", hash = "sha256:046923c85a8464827b55a7cf5541e7f08077b19245732698df3d2d6d653d6001", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/jinja2-3.1.3-py3-none-any.whl", hash = "sha256:84ee4127f133eb82e1b98c1fdcf7abbdfbe4efe8d15a1b4954c0f5655103cd71", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "markupsafe"
        version = "2.1.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/markupsafe-2.1.5.tar.gz", hash = "sha256:e38237d66e6760fe86fe38e4b6c70ff4eed7da50b15e8c0f2589f05850207f13", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/markupsafe-2.1.5-py3-none-any.whl", hash = "sha256:d0fe66b2745bbd943b48f3229667cf4506d88136363578b94e63d384e61d4984", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "iniconfig" },
            { name = "jinja2" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "iniconfig", specifier = "==2.0.0" },
            { name = "jinja2", specifier = ">=3.1.3", index = "http://[LOCALHOST]/simple/" },
        ]
        "#
        );
    });

    // Adding a subsequent index with the same name should replace it.
    uv_snapshot!(context.filters(), context.add().arg("jinja2").arg("--index").arg(format!("pytorch={}", replacement_index.index_url())), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Checked 3 packages in [TIME]
    ");

    let pyproject_toml = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig==2.0.0",
            "jinja2>=3.1.3",
        ]

        [tool.uv]
        constraint-dependencies = ["markupsafe<3"]

        [tool.uv.sources]
        jinja2 = { index = "pytorch" }

        [[tool.uv.index]]
        name = "pytorch"
        url = "http://[LOCALHOST]/simple/"

        [[tool.uv.index]]
        url = "http://[LOCALHOST]/simple/"
        "#
        );
    });

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2025-01-30T00:00:00Z"

        [manifest]
        constraints = [{ name = "markupsafe", specifier = "<3" }]

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz", hash = "sha256:48c42a08c0ec1a24f2fe45f4efdefc9c19ac8e0aa8e82284503ccba80398bec3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "jinja2"
        version = "3.1.3"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "markupsafe" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/jinja2-3.1.3.tar.gz", hash = "sha256:046923c85a8464827b55a7cf5541e7f08077b19245732698df3d2d6d653d6001", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/jinja2-3.1.3-py3-none-any.whl", hash = "sha256:84ee4127f133eb82e1b98c1fdcf7abbdfbe4efe8d15a1b4954c0f5655103cd71", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "markupsafe"
        version = "2.1.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/markupsafe-2.1.5.tar.gz", hash = "sha256:e38237d66e6760fe86fe38e4b6c70ff4eed7da50b15e8c0f2589f05850207f13", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/markupsafe-2.1.5-py3-none-any.whl", hash = "sha256:d0fe66b2745bbd943b48f3229667cf4506d88136363578b94e63d384e61d4984", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "iniconfig" },
            { name = "jinja2" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "iniconfig", specifier = "==2.0.0" },
            { name = "jinja2", specifier = ">=3.1.3", index = "http://[LOCALHOST]/simple/" },
        ]
        "#
        );
    });

    // Adding a subsequent index with the same URL should bump it to the top.
    uv_snapshot!(context.filters(), context.add().arg("typing-extensions").arg("--index").arg(index.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + typing-extensions==4.12.2
    ");

    let pyproject_toml = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig==2.0.0",
            "jinja2>=3.1.3",
            "typing-extensions>=4.12.2",
        ]

        [tool.uv]
        constraint-dependencies = ["markupsafe<3"]

        [tool.uv.sources]
        jinja2 = { index = "pytorch" }

        [[tool.uv.index]]
        url = "http://[LOCALHOST]/simple/"

        [[tool.uv.index]]
        name = "pytorch"
        url = "http://[LOCALHOST]/simple/"
        "#
        );
    });

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2025-01-30T00:00:00Z"

        [manifest]
        constraints = [{ name = "markupsafe", specifier = "<3" }]

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz", hash = "sha256:48c42a08c0ec1a24f2fe45f4efdefc9c19ac8e0aa8e82284503ccba80398bec3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "jinja2"
        version = "3.1.3"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "markupsafe" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/jinja2-3.1.3.tar.gz", hash = "sha256:046923c85a8464827b55a7cf5541e7f08077b19245732698df3d2d6d653d6001", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/jinja2-3.1.3-py3-none-any.whl", hash = "sha256:84ee4127f133eb82e1b98c1fdcf7abbdfbe4efe8d15a1b4954c0f5655103cd71", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "markupsafe"
        version = "2.1.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/markupsafe-2.1.5.tar.gz", hash = "sha256:e38237d66e6760fe86fe38e4b6c70ff4eed7da50b15e8c0f2589f05850207f13", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/markupsafe-2.1.5-py3-none-any.whl", hash = "sha256:d0fe66b2745bbd943b48f3229667cf4506d88136363578b94e63d384e61d4984", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "iniconfig" },
            { name = "jinja2" },
            { name = "typing-extensions" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "iniconfig", specifier = "==2.0.0" },
            { name = "jinja2", specifier = ">=3.1.3", index = "http://[LOCALHOST]/simple/" },
            { name = "typing-extensions", specifier = ">=4.12.2" },
        ]

        [[package]]
        name = "typing-extensions"
        version = "4.12.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/typing_extensions-4.12.2.tar.gz", hash = "sha256:aab23f7f64c40de03caff00b39de163a0f65a62a877175a1cfc1e8a4f510250c", upload-time = "2024-06-07T18:52:15.995Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/typing_extensions-4.12.2-py3-none-any.whl", hash = "sha256:7c5b8656c1d3100f66c2edb19bd37e771b9ec94535ccc66d2e33b4a15edb7ea5", upload-time = "2024-06-07T18:52:15.995Z" },
        ]
        "#
        );
    });

    // Adding a subsequent index with the same URL should bump it to the top, but retain the name.
    uv_snapshot!(context.filters(), context.add().arg("typing-extensions").arg("--index").arg(replacement_index.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Checked 4 packages in [TIME]
    ");

    let pyproject_toml = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig==2.0.0",
            "jinja2>=3.1.3",
            "typing-extensions>=4.12.2",
        ]

        [tool.uv]
        constraint-dependencies = ["markupsafe<3"]

        [tool.uv.sources]
        jinja2 = { index = "pytorch" }

        [[tool.uv.index]]
        name = "pytorch"
        url = "http://[LOCALHOST]/simple/"

        [[tool.uv.index]]
        url = "http://[LOCALHOST]/simple/"
        "#
        );
    });

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2025-01-30T00:00:00Z"

        [manifest]
        constraints = [{ name = "markupsafe", specifier = "<3" }]

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz", hash = "sha256:48c42a08c0ec1a24f2fe45f4efdefc9c19ac8e0aa8e82284503ccba80398bec3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "jinja2"
        version = "3.1.3"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "markupsafe" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/jinja2-3.1.3.tar.gz", hash = "sha256:046923c85a8464827b55a7cf5541e7f08077b19245732698df3d2d6d653d6001", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/jinja2-3.1.3-py3-none-any.whl", hash = "sha256:84ee4127f133eb82e1b98c1fdcf7abbdfbe4efe8d15a1b4954c0f5655103cd71", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "markupsafe"
        version = "2.1.5"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/markupsafe-2.1.5.tar.gz", hash = "sha256:e38237d66e6760fe86fe38e4b6c70ff4eed7da50b15e8c0f2589f05850207f13", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/markupsafe-2.1.5-py3-none-any.whl", hash = "sha256:d0fe66b2745bbd943b48f3229667cf4506d88136363578b94e63d384e61d4984", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "iniconfig" },
            { name = "jinja2" },
            { name = "typing-extensions" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "iniconfig", specifier = "==2.0.0" },
            { name = "jinja2", specifier = ">=3.1.3", index = "http://[LOCALHOST]/simple/" },
            { name = "typing-extensions", specifier = ">=4.12.2" },
        ]

        [[package]]
        name = "typing-extensions"
        version = "4.12.2"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/typing_extensions-4.12.2.tar.gz", hash = "sha256:aab23f7f64c40de03caff00b39de163a0f65a62a877175a1cfc1e8a4f510250c", upload-time = "2024-06-07T18:52:15.995Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/typing_extensions-4.12.2-py3-none-any.whl", hash = "sha256:7c5b8656c1d3100f66c2edb19bd37e771b9ec94535ccc66d2e33b4a15edb7ea5", upload-time = "2024-06-07T18:52:15.995Z" },
        ]
        "#
        );
    });

    Ok(())
}

/// Add an index provided via `--default-index`.
#[test]
fn add_default_index_url() -> Result<()> {
    let index = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let replacement = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("iniconfig").arg("--default-index").arg(index.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    let pyproject_toml = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig>=2.0.0",
        ]

        [[tool.uv.index]]
        url = "http://[LOCALHOST]/simple/"
        default = true
        "#
        );
    });

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz", hash = "sha256:48c42a08c0ec1a24f2fe45f4efdefc9c19ac8e0aa8e82284503ccba80398bec3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "iniconfig" },
        ]

        [package.metadata]
        requires-dist = [{ name = "iniconfig", specifier = ">=2.0.0" }]
        "#
        );
    });

    // Adding another `--default-index` replaces the current default.
    uv_snapshot!(context.filters(), context.add().arg("typing-extensions").arg("--default-index").arg(replacement.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + typing-extensions==4.10.0
    ");

    let pyproject_toml = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig>=2.0.0",
            "typing-extensions>=4.10.0",
        ]

        [[tool.uv.index]]
        url = "http://[LOCALHOST]/simple/"
        default = true
        "#
        );
    });

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz", hash = "sha256:48c42a08c0ec1a24f2fe45f4efdefc9c19ac8e0aa8e82284503ccba80398bec3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "iniconfig" },
            { name = "typing-extensions" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "iniconfig", specifier = ">=2.0.0" },
            { name = "typing-extensions", specifier = ">=4.10.0" },
        ]

        [[package]]
        name = "typing-extensions"
        version = "4.10.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/typing_extensions-4.10.0.tar.gz", hash = "sha256:adefbbc2f75a47edb1f4491a2e99a45438ec3dd0c2670276b321cad2ca522246", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/typing_extensions-4.10.0-py3-none-any.whl", hash = "sha256:0626263fe1dcda7bc3ee7b2872064b534d4831718055d22d4f32f9e474a867a4", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    Ok(())
}

#[tokio::test]
async fn add_index_credentials() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let proxy = crate::pypi_proxy::start().await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Provide credentials for the index via the environment variable.
    uv_snapshot!(context.filters(), context.add().arg("iniconfig==2.0.0").env(EnvVars::UV_DEFAULT_INDEX, proxy.authenticated_url("public", "heron", "/basic-auth/simple")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    let pyproject_toml = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig==2.0.0",
        ]

        [[tool.uv.index]]
        url = "http://[LOCALHOST]/basic-auth/simple"
        default = true
        "#
        );
    });

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/basic-auth/simple" }
        sdist = { url = "http://[LOCALHOST]/basic-auth/files/packages/d7/4b/cbd8e699e64a6f16ca3a8220661b5f83792b3017d0f79807cb8708d33913/iniconfig-2.0.0.tar.gz", hash = "sha256:2d91e135bf72d31a410b17c16da610a82cb55f6b0477d1a902134b24a455b8b3", size = 4646, upload-time = "2023-01-07T11:08:11.254Z" }
        wheels = [
            { url = "http://[LOCALHOST]/basic-auth/files/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:b6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374", size = 5892, upload-time = "2023-01-07T11:08:09.864Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "iniconfig" },
        ]

        [package.metadata]
        requires-dist = [{ name = "iniconfig", specifier = "==2.0.0" }]
        "#
        );
    });

    Ok(())
}

#[tokio::test]
async fn existing_index_credentials() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let proxy = crate::pypi_proxy::start().await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc!(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        # Set an internal index as the default, without credentials.
        [[tool.uv.index]]
        name = "internal"
        url = "{proxy_uri}/basic-auth/simple"
        default = true
    "#,
        proxy_uri = proxy.uri()
    ))?;

    // Provide credentials for the index via the environment variable.
    uv_snapshot!(context.filters(), context.add().arg("iniconfig==2.0.0").env(EnvVars::UV_DEFAULT_INDEX, proxy.authenticated_url("public", "heron", "/basic-auth/simple")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    let pyproject_toml = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig==2.0.0",
        ]

        # Set an internal index as the default, without credentials.
        [[tool.uv.index]]
        name = "internal"
        url = "http://[LOCALHOST]/basic-auth/simple"
        default = true
        "#
        );
    });

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/basic-auth/simple" }
        sdist = { url = "http://[LOCALHOST]/basic-auth/files/packages/d7/4b/cbd8e699e64a6f16ca3a8220661b5f83792b3017d0f79807cb8708d33913/iniconfig-2.0.0.tar.gz", hash = "sha256:2d91e135bf72d31a410b17c16da610a82cb55f6b0477d1a902134b24a455b8b3", size = 4646, upload-time = "2023-01-07T11:08:11.254Z" }
        wheels = [
            { url = "http://[LOCALHOST]/basic-auth/files/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:b6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374", size = 5892, upload-time = "2023-01-07T11:08:09.864Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "iniconfig" },
        ]

        [package.metadata]
        requires-dist = [{ name = "iniconfig", specifier = "==2.0.0" }]
        "#
        );
    });

    Ok(())
}

/// Add an index with a trailing slash.
#[test]
fn add_index_with_trailing_slash() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12").with_exclude_newer("2025-01-30T00:00Z");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        constraint-dependencies = ["markupsafe<3"]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("iniconfig==2.0.0").arg("--index").arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    let pyproject_toml = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig==2.0.0",
        ]

        [tool.uv]
        constraint-dependencies = ["markupsafe<3"]

        [[tool.uv.index]]
        url = "http://[LOCALHOST]/simple/"
        "#
        );
    });

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2025-01-30T00:00:00Z"

        [manifest]
        constraints = [{ name = "markupsafe", specifier = "<3" }]

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz", hash = "sha256:48c42a08c0ec1a24f2fe45f4efdefc9c19ac8e0aa8e82284503ccba80398bec3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "iniconfig" },
        ]

        [package.metadata]
        requires-dist = [{ name = "iniconfig", specifier = "==2.0.0" }]
        "#
        );
    });

    Ok(())
}

/// Add an index without a trailing slash.
#[test]
fn add_index_without_trailing_slash() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let index_url = server.index_url();
    let index_url_without_trailing_slash = index_url.trim_end_matches('/');
    let context = uv_test::test_context!("3.12").with_exclude_newer("2025-01-30T00:00Z");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        constraint-dependencies = ["markupsafe<3"]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("iniconfig==2.0.0").arg("--index").arg(index_url_without_trailing_slash), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    let pyproject_toml = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig==2.0.0",
        ]

        [tool.uv]
        constraint-dependencies = ["markupsafe<3"]

        [[tool.uv.index]]
        url = "http://[LOCALHOST]/simple"
        "#
        );
    });

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2025-01-30T00:00:00Z"

        [manifest]
        constraints = [{ name = "markupsafe", specifier = "<3" }]

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple" }
        sdist = { url = "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz", hash = "sha256:48c42a08c0ec1a24f2fe45f4efdefc9c19ac8e0aa8e82284503ccba80398bec3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "iniconfig" },
        ]

        [package.metadata]
        requires-dist = [{ name = "iniconfig", specifier = "==2.0.0" }]
        "#
        );
    });

    Ok(())
}

/// Add an index with an existing relative path.
#[test]
fn add_index_with_existing_relative_path_index() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let project = context.temp_dir.child("project");
    project.create_dir_all()?;
    let pyproject_toml = project.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [[tool.uv.index]]
        name = "local"
        url = "./links-alias"
        format = "flat"
    "#})?;

    // Create a non-empty flat index.
    let packages = project.child("test-index");
    packages.create_dir_all()?;
    packages.child("placeholder").touch()?;
    uv_fs::create_symlink(packages.path(), project.child("links-alias").path())?;

    let index = format!("local={}", packages.path().display());
    uv_snapshot!(context.filters(), context.add().arg("iniconfig").arg("--frozen").arg("--project").arg(project.path()).arg("--index").arg(index), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    ");

    let pyproject_toml = fs_err::read_to_string(project.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig",
        ]

        [[tool.uv.index]]
        name = "local"
        url = "[TEMP_DIR]/project/test-index"
        format = "flat"

        [tool.uv.sources]
        iniconfig = { index = "local" }
        "#);
    });

    Ok(())
}

#[test]
fn add_index_with_relative_path_for_project() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let project = context.temp_dir.child("project");
    project.create_dir_all()?;
    let pyproject_toml = project.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    let packages = project.child("test-index");
    packages.create_dir_all()?;
    packages.child("placeholder").touch()?;

    uv_snapshot!(context.filters(), context.add().arg("iniconfig").arg("--frozen").arg("--project").arg(project.path()).arg("--index").arg("local=./project/test-index"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    ");

    let pyproject_toml = fs_err::read_to_string(project.join("pyproject.toml"))?;
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig",
        ]

        [tool.uv.sources]
        iniconfig = { index = "local" }

        [[tool.uv.index]]
        name = "local"
        url = "test-index"
        "#);
    });

    Ok(())
}

/// Add an index with an existing relative path to a script outside the working directory.
#[test]
fn add_index_with_existing_relative_path_in_script() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let scripts = context.temp_dir.child("scripts");
    scripts.create_dir_all()?;
    let script = scripts.child("main.py");
    script.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = []
        #
        # [[tool.uv.index]]
        # name = "local"
        # url = "../links"
        # format = "flat"
        # ///
    "#})?;

    let packages = context.temp_dir.child("links");
    packages.create_dir_all()?;
    let wheel_src = context
        .workspace_root
        .join("test/links/ok-1.0.0-py3-none-any.whl");
    fs_err::copy(&wheel_src, packages.child("ok-1.0.0-py3-none-any.whl"))?;

    uv_snapshot!(context.filters(), context.add().arg("iniconfig").arg("--frozen").arg("--script").arg(script.path()).arg("--index").arg("local=./links"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: `--frozen` is a no-op for Python scripts with inline metadata, which always run in isolation
    ");

    let script = fs_err::read_to_string(script.path())?;
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(script, @r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = [
        #     "iniconfig",
        # ]
        #
        # [[tool.uv.index]]
        # name = "local"
        # url = "../links"
        # format = "flat"
        #
        # [tool.uv.sources]
        # iniconfig = { index = "local" }
        # ///
        "#);
    });

    Ok(())
}

/// Add an index with a non-existent relative path.
#[test]
fn add_index_with_non_existent_relative_path() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("iniconfig").arg("--index").arg("./test-index"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Directory not found for index: file://[TEMP_DIR]/test-index
    ");

    Ok(())
}

/// Add an index with a non-existent relative path with the same name as a defined index.
#[tokio::test]
async fn add_index_with_non_existent_relative_path_with_same_name_as_index() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let proxy = crate::pypi_proxy::start().await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc!(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [[tool.uv.index]]
        name = "test-index"
        url = "{proxy_uri}/simple"
    "#,
        proxy_uri = proxy.uri()
    ))?;

    uv_snapshot!(context.filters(), context.add().arg("iniconfig").arg("--index").arg("./test-index"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Directory not found for index: file://[TEMP_DIR]/test-index
    ");

    Ok(())
}

/// Add a dependency using a configured index selected by name.
#[test]
fn add_index_by_name() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    // `explicit` and `default` are supported together; use both to test overriding behaviour.
    let initial = indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [[tool.uv.index]]
        name = "internal"
        url = "https://example.invalid/simple"
        explicit = true
        default = true
    "#};
    pyproject_toml.write_str(initial)?;

    // Without preview, selecting a configured index by name emits a warning.
    uv_snapshot!(context.filters(), context.add()
        .arg("iniconfig")
        .arg("--index").arg("internal")
        .arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Referencing an index by name is experimental and may change without warning. Pass `--preview-features index-by-name` to disable this warning.
    ");

    pyproject_toml.write_str(initial)?;

    // Enabling preview suppresses the warning.
    uv_snapshot!(context.filters(), context.add()
        .arg("iniconfig")
        .arg("--index").arg("internal")
        .arg("--preview-features").arg("index-by-name")
        .arg("--frozen"), @"
    exit_code: 0 (success)
    ");

    // Preserve the existing index configuration and pin the dependency to it.
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(context.read("pyproject.toml"), @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig",
        ]

        [[tool.uv.index]]
        name = "internal"
        url = "https://example.invalid/simple"
        explicit = true
        default = true

        [tool.uv.sources]
        iniconfig = { index = "internal" }
        "#);
    });

    Ok(())
}

/// Keep a named local index relative to its project when invoked from another directory.
#[test]
fn add_index_by_name_with_relative_path() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let project = context.temp_dir.child("project");
    project.create_dir_all()?;
    let pyproject_toml = project.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [[tool.uv.index]]
        name = "local"
        url = "wheels"
        format = "flat"
    "#})?;

    let packages = project.child("wheels");
    packages.create_dir_all()?;
    packages.child("placeholder").touch()?;

    // Base the relative index URL at the project, not the invocation directory.
    uv_snapshot!(context.filters(), context.add()
        .arg("iniconfig")
        .arg("--index").arg("local")
        .arg("--preview-features").arg("index-by-name")
        .arg("--project").arg(project.path())
        .arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    ");

    let pyproject_toml = fs_err::read_to_string(pyproject_toml.path())?;

    // Preserve the relative URL spelling and pin the dependency to the configured index.
    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig",
        ]

        [[tool.uv.index]]
        name = "local"
        url = "wheels"
        format = "flat"

        [tool.uv.sources]
        iniconfig = { index = "local" }
        "#);
    });

    Ok(())
}

#[tokio::test]
async fn add_index_empty_directory() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let proxy = crate::pypi_proxy::start().await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc!(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [[tool.uv.index]]
        name = "test-index"
        url = "{proxy_uri}/simple"
    "#,
        proxy_uri = proxy.uri()
    ))?;

    let packages = context.temp_dir.child("test-index");
    packages.create_dir_all()?;

    uv_snapshot!(context.filters(), context.add().arg("iniconfig").arg("--index").arg("./test-index"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Index directory `file://[TEMP_DIR]/test-index` is empty, skipping
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    Ok(())
}

#[test]
fn add_index_with_ambiguous_relative_path() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filter((r"\./|\.\\", r"[PREFIX]"));

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    #[cfg(unix)]
    uv_snapshot!(context.filters(), context.add().arg("iniconfig").arg("--index").arg("test-index"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: Relative paths passed to `--index` or `--default-index` should be disambiguated from index names (use `[PREFIX]test-index`). Support for ambiguous values will be removed in the future
    error: Directory not found for index: file://[TEMP_DIR]/test-index
    ");

    #[cfg(windows)]
    uv_snapshot!(context.filters(), context.add().arg("iniconfig").arg("--index").arg("test-index"), @r"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: Relative paths passed to `--index` or `--default-index` should be disambiguated from index names (use `[PREFIX]test-index` or `[PREFIX]test-index`). Support for ambiguous values will be removed in the future
    error: Directory not found for index: file://[TEMP_DIR]/test-index
    ");

    Ok(())
}

/// Add a PyPI requirement.
#[test]
fn add_group_comment() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "myproject"
        version = "0.1.0"
        description = "Add your description here"
        requires-python = ">=3.11"
        [dependency-groups]
        # These are our dev dependencies
        dev = [
            "typing-extensions",
        ]
        # These are our test dependencies
        test = [
            "iniconfig"
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("--group").arg("dev").arg("sniffio"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + sniffio==1.3.1
     + typing-extensions==4.10.0
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "myproject"
        version = "0.1.0"
        description = "Add your description here"
        requires-python = ">=3.11"
        [dependency-groups]
        # These are our dev dependencies
        dev = [
            "sniffio>=1.3.1",
            "typing-extensions",
        ]
        # These are our test dependencies
        test = [
            "iniconfig"
        ]
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.11"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz", hash = "sha256:48c42a08c0ec1a24f2fe45f4efdefc9c19ac8e0aa8e82284503ccba80398bec3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "myproject"
        version = "0.1.0"
        source = { virtual = "." }

        [package.dev-dependencies]
        dev = [
            { name = "sniffio" },
            { name = "typing-extensions" },
        ]
        test = [
            { name = "iniconfig" },
        ]

        [package.metadata]

        [package.metadata.requires-dev]
        dev = [
            { name = "sniffio", specifier = ">=1.3.1" },
            { name = "typing-extensions" },
        ]
        test = [{ name = "iniconfig" }]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "typing-extensions"
        version = "4.10.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/typing_extensions-4.10.0.tar.gz", hash = "sha256:adefbbc2f75a47edb1f4491a2e99a45438ec3dd0c2670276b321cad2ca522246", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/typing_extensions-4.10.0-py3-none-any.whl", hash = "sha256:0626263fe1dcda7bc3ee7b2872064b534d4831718055d22d4f32f9e474a867a4", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 2 packages in [TIME]
    ");

    Ok(())
}

#[test]
fn add_index_comments() -> Result<()> {
    let existing = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let replacement = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [[tool.uv.index]]
        name = "internal"
        url = "{existing}"  # This is a test index.
        default = true
    "#,
        existing = existing.index_url(),
    })?;

    // Preserve the comment on the index URL, despite replacing it.
    uv_snapshot!(context.filters(), context.add().arg("iniconfig==2.0.0").env(EnvVars::UV_DEFAULT_INDEX, replacement.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    let pyproject_toml = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig==2.0.0",
        ]

        [[tool.uv.index]]
        name = "internal"
        url = "http://[LOCALHOST]/simple/"  # This is a test index.
        default = true
        "#
        );
    });

    let lock = fs_err::read_to_string(context.temp_dir.join("uv.lock"))?;

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz", hash = "sha256:48c42a08c0ec1a24f2fe45f4efdefc9c19ac8e0aa8e82284503ccba80398bec3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "iniconfig" },
        ]

        [package.metadata]
        requires-dist = [{ name = "iniconfig", specifier = "==2.0.0" }]
        "#
        );
    });

    Ok(())
}

/// Accidentally add a dependency on the project itself.
#[test]
fn add_self() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "anyio"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio==3.7.0"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Requirement name `anyio` matches project name `anyio`, but self-dependencies are not permitted without the `--dev` or `--optional` flags. If your project name (`anyio`) is shadowing that of a third-party dependency, consider renaming the project.
    ");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "anyio"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        types = ["typing-extensions>=4"]
    "#})?;

    // However, recursive extras are fine.
    uv_snapshot!(context.filters(), context.add().arg("anyio[types]").arg("--optional").arg("all"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + typing-extensions==4.10.0
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "anyio"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        types = ["typing-extensions>=4"]
        all = [
            "anyio[types]",
        ]

        [tool.uv.sources]
        anyio = { workspace = true }
        "#
        );
    });

    // And recursive development dependencies
    uv_snapshot!(context.filters(), context.add().arg("anyio[types]").arg("--dev"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Checked 1 package in [TIME]
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "anyio"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        types = ["typing-extensions>=4"]
        all = [
            "anyio[types]",
        ]

        [tool.uv.sources]
        anyio = { workspace = true }

        [dependency-groups]
        dev = [
            "anyio[types]",
        ]
        "#
        );
    });

    Ok(())
}

#[test]
fn add_preserves_end_of_line_comments() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            # comment 0
            "anyio==3.7.0", # comment 1
            # comment 2
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("requests==2.31.0"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 7 packages in [TIME]
    Installed 7 packages in [TIME]
     + anyio==3.7.0
     + certifi==2024.2.2
     + charset-normalizer==3.3.2
     + idna==3.6
     + requests==2.31.0
     + sniffio==1.3.1
     + urllib3==2.2.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            # comment 0
            "anyio==3.7.0", # comment 1
            # comment 2
            "requests==2.31.0",
        ]
        "#
        );
    });
    Ok(())
}

#[test]
fn add_preserves_end_of_line_comment_on_non_last_deps() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [ # comment
            "anyio==3.7.0", # comment 1
            "sniffio==1.3.1",
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("requests==2.31.0"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 7 packages in [TIME]
    Installed 7 packages in [TIME]
     + anyio==3.7.0
     + certifi==2024.2.2
     + charset-normalizer==3.3.2
     + idna==3.6
     + requests==2.31.0
     + sniffio==1.3.1
     + urllib3==2.2.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [ # comment
            "anyio==3.7.0", # comment 1
            "requests==2.31.0",
            "sniffio==1.3.1",
        ]
        "#
        );
    });
    Ok(())
}

#[test]
fn add_preserves_end_of_line_comment_on_updated_optional_dependency() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        typing = [
            "pandas-stubs>=2.0.2",
            "narwhals>=1.42.0" # narwhals are toothed whales native to the Arctic
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("narwhals>=1.42").arg("--optional=typing").arg("--frozen"), @"
    exit_code: 0 (success)
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        typing = [
            "pandas-stubs>=2.0.2",
            "narwhals>=1.42", # narwhals are toothed whales native to the Arctic
        ]
        "#
        );
    });

    Ok(())
}

#[test]
fn add_direct_url_subdirectory() -> Result<()> {
    let direct_artifacts =
        uv_test::packse::PackseServer::new("packages/lock-direct-artifacts.toml");
    let root_url = direct_artifacts.file_url("root-0.0.1.tar.gz");
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg(format!("root @ {root_url}#subdirectory=packages/root")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + root==0.0.1 (from http://[LOCALHOST]/files/root-0.0.1.tar.gz#subdirectory=packages/root)
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "root",
        ]

        [tool.uv.sources]
        root = { url = "http://[LOCALHOST]/files/root-0.0.1.tar.gz", subdirectory = "packages/root" }
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "4.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-4.3.0.tar.gz", hash = "sha256:13a6d97fa30ec110d85e3949a30c92306f0178135048329f54a335c3dade753a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-4.3.0-py3-none-any.whl", hash = "sha256:c4f443e7e5a2c003b1534688207e85dbd11960efb66d4d6a4e7693fdfc6f5b33", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "root" },
        ]

        [package.metadata]
        requires-dist = [{ name = "root", url = "http://[LOCALHOST]/files/root-0.0.1.tar.gz", subdirectory = "packages/root" }]

        [[package]]
        name = "root"
        version = "0.0.1"
        source = { url = "http://[LOCALHOST]/files/root-0.0.1.tar.gz", subdirectory = "packages/root" }
        dependencies = [
            { name = "anyio" },
        ]
        sdist = { hash = "sha256:33240cb91b02e5410728c950f15b43103b3c15fb129edbec6bcc818b34cb72b2" }

        [package.metadata]
        requires-dist = [{ name = "anyio" }]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 4 packages in [TIME]
    ");

    Ok(())
}

#[test]
fn add_direct_url_subdirectory_raw() -> Result<()> {
    let direct_artifacts =
        uv_test::packse::PackseServer::new("packages/lock-direct-artifacts.toml");
    let root_url = direct_artifacts.file_url("root-0.0.1.tar.gz");
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg(format!("root @ {root_url}#subdirectory=packages/root")).arg("--raw-sources"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + root==0.0.1 (from http://[LOCALHOST]/files/root-0.0.1.tar.gz#subdirectory=packages/root)
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "root @ http://[LOCALHOST]/files/root-0.0.1.tar.gz#subdirectory=packages/root",
        ]
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "anyio"
        version = "4.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "idna" },
            { name = "sniffio" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/anyio-4.3.0.tar.gz", hash = "sha256:13a6d97fa30ec110d85e3949a30c92306f0178135048329f54a335c3dade753a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/anyio-4.3.0-py3-none-any.whl", hash = "sha256:c4f443e7e5a2c003b1534688207e85dbd11960efb66d4d6a4e7693fdfc6f5b33", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "idna"
        version = "3.6"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/idna-3.6.tar.gz", hash = "sha256:9aae8f72192b28db0d56fcef130afe490d1538a8d1bf1700e6d219521421525f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/idna-3.6-py3-none-any.whl", hash = "sha256:e80025850eafa8760055fd6f2f6e83f84bf13d4a844fe81abb2b499e3a3e8af0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "root" },
        ]

        [package.metadata]
        requires-dist = [{ name = "root", url = "http://[LOCALHOST]/files/root-0.0.1.tar.gz", subdirectory = "packages/root" }]

        [[package]]
        name = "root"
        version = "0.0.1"
        source = { url = "http://[LOCALHOST]/files/root-0.0.1.tar.gz", subdirectory = "packages/root" }
        dependencies = [
            { name = "anyio" },
        ]
        sdist = { hash = "sha256:33240cb91b02e5410728c950f15b43103b3c15fb129edbec6bcc818b34cb72b2" }

        [package.metadata]
        requires-dist = [{ name = "anyio" }]

        [[package]]
        name = "sniffio"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/sniffio-1.3.1.tar.gz", hash = "sha256:ce520d2eb3c2be02f0c148dab5ba304e8705e1c7e7b4bec8a9146c464a597a6a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/sniffio-1.3.1-py3-none-any.whl", hash = "sha256:2743fa2a853c508a2310882c0b4104631e0b0fcb855e00a912b7e3f27e6b3f05", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 4 packages in [TIME]
    ");

    Ok(())
}

#[test]
fn add_preserves_open_bracket_comment() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [ # comment 0
            # comment 1
            "anyio==3.7.0", # comment 2
            # comment 3
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("requests==2.31.0"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    Prepared 7 packages in [TIME]
    Installed 7 packages in [TIME]
     + anyio==3.7.0
     + certifi==2024.2.2
     + charset-normalizer==3.3.2
     + idna==3.6
     + requests==2.31.0
     + sniffio==1.3.1
     + urllib3==2.2.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [ # comment 0
            # comment 1
            "anyio==3.7.0", # comment 2
            # comment 3
            "requests==2.31.0",
        ]
        "#
        );
    });
    Ok(())
}

#[test]
fn add_preserves_empty_comment() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            # First line.
            # Second line.
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio==3.7.0"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            # First line.
            # Second line.
            "anyio==3.7.0",
        ]
        "#
        );
    });

    Ok(())
}

#[test]
fn add_preserves_trailing_comment() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "idna",
            "iniconfig",  # Use iniconfig.
            # First line.
            # Second line.
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio==3.7.0"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + iniconfig==2.0.0
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "anyio==3.7.0",
            "idna",
            "iniconfig",  # Use iniconfig.
            # First line.
            # Second line.
        ]
        "#
        );
    });

    uv_snapshot!(context.filters(), context.add().arg("typing-extensions"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + typing-extensions==4.10.0
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "anyio==3.7.0",
            "idna",
            "iniconfig",  # Use iniconfig.
            # First line.
            # Second line.
            "typing-extensions>=4.10.0",
        ]
        "#
        );
    });

    Ok(())
}

#[test]
fn add_preserves_trailing_depth() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
          "idna",
          "iniconfig",# Use iniconfig.
            # First line.
            # Second line.
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio==3.7.0"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + iniconfig==2.0.0
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
          "anyio==3.7.0",
          "idna",
          "iniconfig",# Use iniconfig.
          # First line.
          # Second line.
        ]
        "#
        );
    });

    Ok(())
}

#[test]
fn add_preserves_first_own_line_comment() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            # comment
            "sniffio==1.3.1",
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("charset-normalizer"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + charset-normalizer==3.3.2
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "charset-normalizer>=3.3.2",
            # comment
            "sniffio==1.3.1",
        ]
        "#
        );
    });
    Ok(())
}

#[test]
fn add_preserves_first_line_bracket_comment() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [ # comment
            "sniffio==1.3.1",
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("charset-normalizer"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + charset-normalizer==3.3.2
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [ # comment
            "charset-normalizer>=3.3.2",
            "sniffio==1.3.1",
        ]
        "#
        );
    });
    Ok(())
}

#[test]
fn add_no_indent() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
"sniffio==1.3.1"
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("charset-normalizer"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + charset-normalizer==3.3.2
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
                [project]
                name = "project"
                version = "0.1.0"
                requires-python = ">=3.12"
                dependencies = [
            "charset-normalizer>=3.3.2",
            "sniffio==1.3.1",
        ]
        "#
        );
    });
    Ok(())
}

/// Accept requirements, not just package names, in `uv remove`.
#[test]
fn remove_requirement() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["flask"]
    "#})?;

    uv_snapshot!(context.filters(), context.remove().arg("flask[dotenv]"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked in [TIME]
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
        "#
        );
    });

    Ok(())
}

/// Remove all dependencies with remaining comments
#[test]
fn remove_all_with_comments() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "duct",
            "minilog",
            # foo
            # bar
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.remove().arg("duct").arg("minilog"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked in [TIME]
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            # foo
            # bar
        ]
        "#
        );
    });

    Ok(())
}

/// Removing a dependency should preserve end-of-line comments on nearby lines.
///
/// See: <https://github.com/astral-sh/uv/issues/18555>
#[test]
fn remove_preserves_nearby_end_of_line_comments() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig>=2.0.0", # this comment is clearly essential
            "typing-extensions>=4.0.0",
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.remove().arg("typing-extensions"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig>=2.0.0", # this comment is clearly essential
        ]
        "#
        );
    });

    Ok(())
}

/// Removing multiple adjacent matching dependencies should preserve comment order.
///
/// See: <https://github.com/astral-sh/uv/issues/18555>
#[test]
fn remove_preserves_comment_order_for_multiple_adjacent_matches() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig>=2.0.0", # comment on iniconfig
            "typing-extensions>=4.0.0 ; python_version < '3.11'", # comment on first typing-extensions
            "typing-extensions>=4.0.0 ; python_version >= '3.11'",
            "sniffio>=1.3.0",
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.remove().arg("typing-extensions"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + iniconfig==2.0.0
     + sniffio==1.3.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig>=2.0.0", # comment on iniconfig
            # comment on first typing-extensions
            "sniffio>=1.3.0",
        ]
        "#
        );
    });

    Ok(())
}

/// If multiple indexes are provided on the CLI, the first-provided index should take precedence
/// during resolution, and should appear first in the `pyproject.toml` file.
///
/// See: <https://github.com/astral-sh/uv/issues/14817>
#[test]
fn multiple_index_cli() -> Result<()> {
    let first_index = uv_test::packse::PackseServer::new("packages/edit-test-index.toml");
    let second_index = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context
        .add()
        .arg("requests")
        .arg("--index")
        .arg(first_index.index_url())
        .arg("--index")
        .arg(second_index.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + requests==2.5.4.1
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "requests>=2.5.4.1",
        ]

        [[tool.uv.index]]
        url = "http://[LOCALHOST]/simple/"

        [[tool.uv.index]]
        url = "http://[LOCALHOST]/simple/"
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "requests" },
        ]

        [package.metadata]
        requires-dist = [{ name = "requests", specifier = ">=2.5.4.1" }]

        [[package]]
        name = "requests"
        version = "2.5.4.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/requests-2.5.4.1.tar.gz", hash = "sha256:0a4477484466dcd583827c7b144886241c8b7947b0bbaf93df723755cfcb64d9", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/requests-2.5.4.1-py3-none-any.whl", hash = "sha256:148448464c5e3a6bee3edee69a72524fadc17c386ad63f9151f67bbd9646721c", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");

    Ok(())
}

/// If an index is repeated by the CLI and an environment variable, the CLI value should take
/// precedence.
///
/// The index that appears in the `pyproject.toml` should also be consistent with the index that
/// appears in the `uv.lock`.
///
/// See: <https://github.com/astral-sh/uv/issues/11312>
#[test]
fn repeated_index_cli_environment_variable() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let index_url = server.index_url();
    let index_url_without_trailing_slash = index_url.trim_end_matches('/');
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context
        .add()
        .arg("iniconfig")
        // Without a trailing slash.
        .arg("--index")
        .arg(index_url_without_trailing_slash)
        // With a trailing slash.
        .env(EnvVars::UV_DEFAULT_INDEX, &index_url), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig>=2.0.0",
        ]

        [[tool.uv.index]]
        url = "http://[LOCALHOST]/simple"
        default = true
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple" }
        sdist = { url = "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz", hash = "sha256:48c42a08c0ec1a24f2fe45f4efdefc9c19ac8e0aa8e82284503ccba80398bec3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "iniconfig" },
        ]

        [package.metadata]
        requires-dist = [{ name = "iniconfig", specifier = ">=2.0.0" }]
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");

    Ok(())
}

/// If an index is repeated on the CLI, the first-provided index should take precedence.
/// Newlines in `UV_INDEX` should be treated as separators.
///
/// The index that appears in the `pyproject.toml` should also be consistent with the index that
/// appears in the `uv.lock`.
///
/// See: <https://github.com/astral-sh/uv/issues/11312>
#[test]
fn repeated_index_cli_environment_variable_newline() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let index_url = server.index_url();
    let index_url_without_trailing_slash = index_url.trim_end_matches('/');
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context
        .add()
        .env(EnvVars::UV_INDEX, format!("{index_url_without_trailing_slash}\n{index_url}"))
        .arg("iniconfig"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig>=2.0.0",
        ]

        [[tool.uv.index]]
        url = "http://[LOCALHOST]/simple"
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple" }
        sdist = { url = "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz", hash = "sha256:48c42a08c0ec1a24f2fe45f4efdefc9c19ac8e0aa8e82284503ccba80398bec3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "iniconfig" },
        ]

        [package.metadata]
        requires-dist = [{ name = "iniconfig", specifier = ">=2.0.0" }]
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");

    Ok(())
}

/// If an index is repeated on the CLI, the first-provided index should take precedence.
///
/// The index that appears in the `pyproject.toml` should also be consistent with the index that
/// appears in the `uv.lock`.
///
/// See: <https://github.com/astral-sh/uv/issues/11312>
#[test]
fn repeated_index_cli() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let index_url = server.index_url();
    let index_url_without_trailing_slash = index_url.trim_end_matches('/');
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context
        .add()
        .arg("iniconfig")
        // Without a trailing slash.
        .arg("--index")
        .arg(index_url_without_trailing_slash)
        // With a trailing slash.
        .arg("--index")
        .arg(&index_url), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig>=2.0.0",
        ]

        [[tool.uv.index]]
        url = "http://[LOCALHOST]/simple"
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple" }
        sdist = { url = "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz", hash = "sha256:48c42a08c0ec1a24f2fe45f4efdefc9c19ac8e0aa8e82284503ccba80398bec3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "iniconfig" },
        ]

        [package.metadata]
        requires-dist = [{ name = "iniconfig", specifier = ">=2.0.0" }]
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");

    Ok(())
}

/// If an index is repeated on the CLI, the first-provided index should take precedence.
///
/// The index that appears in the `pyproject.toml` should also be consistent with the index that
/// appears in the `uv.lock`.
///
/// See: <https://github.com/astral-sh/uv/issues/11312>
#[test]
fn repeated_index_cli_reversed() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let index_url = server.index_url();
    let index_url_without_trailing_slash = index_url.trim_end_matches('/');
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context
        .add()
        .arg("iniconfig")
        // With a trailing slash.
        .arg("--index")
        .arg(&index_url)
        // Without a trailing slash.
        .arg("--index")
        .arg(index_url_without_trailing_slash), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig>=2.0.0",
        ]

        [[tool.uv.index]]
        url = "http://[LOCALHOST]/simple/"
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz", hash = "sha256:48c42a08c0ec1a24f2fe45f4efdefc9c19ac8e0aa8e82284503ccba80398bec3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "iniconfig" },
        ]

        [package.metadata]
        requires-dist = [{ name = "iniconfig", specifier = ">=2.0.0" }]
        "#
        );
    });

    // Install from the lockfile.
    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");

    Ok(())
}

#[test]
fn add_with_build_constraints() -> Result<()> {
    let context = uv_test::test_context!("3.9");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.8"
    dependencies = []

    [tool.uv]
    build-constraint-dependencies = ["setuptools==1"]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("requests==1.2"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to add dependencies
      cause: Failed to download and build `requests==1.2.0`
      cause: Failed to resolve requirements from `setup.py` build
      cause: No solution found when resolving: `setuptools>=40.8.0`
      cause: Because you require setuptools>=40.8.0 and setuptools==1, we can conclude that your requirements are unsatisfiable.

    hint: `requests` (v1.2.0) was included because `project` (v0.1.0) depends on `requests==1.2`

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    ");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.8"
    dependencies = []

    [tool.uv]
    build-constraint-dependencies = ["setuptools>=40"]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("requests==1.2"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + requests==1.2.0
    ");

    Ok(())
}

#[test]
#[cfg(feature = "test-git")]
fn add_unsupported_git_scheme() {
    let context = uv_test::test_context!("3.12");

    context.init().arg(".").assert().success();

    uv_snapshot!(context.filters(), context.add().arg("git+fantasy://ferris/dreams/of/urls@7701ffcbae245819b828dc5f885a5201158897ef"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse: `git+fantasy://ferris/dreams/of/urls@7701ffcbae245819b828dc5f885a5201158897ef`
      cause: Unsupported Git URL scheme `fantasy:` in `fantasy://ferris/dreams/of/urls` (expected one of `https:`, `ssh:`, or `file:`)
             git+fantasy://ferris/dreams/of/urls@7701ffcbae245819b828dc5f885a5201158897ef
             ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    ");
}

#[tokio::test]
async fn add_index_url_in_keyring() -> Result<()> {
    let keyring_context = uv_test::test_context!("3.12");

    // Install our keyring plugin
    keyring_context
        .pip_install()
        .arg(
            keyring_context
                .workspace_root
                .join("test")
                .join("packages")
                .join("keyring_stub"),
        )
        .arg(
            keyring_context
                .workspace_root
                .join("test")
                .join("packages")
                .join("keyring_test_plugin"),
        )
        .assert()
        .success();

    let context = uv_test::test_context!("3.12");
    let proxy = crate::pypi_proxy::start().await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc!(
        r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = []
        [tool.uv]
        keyring-provider = "subprocess"
        [[tool.uv.index]]
        name = "proxy"
        url = "{proxy_uri}/basic-auth/simple"
        default = true
        "#,
        proxy_uri = proxy.uri()
    ))?;

    uv_snapshot!(context.filters(), context.add().arg("anyio")
        .env(EnvVars::index_username("PROXY"), "public")
        .env(EnvVars::KEYRING_TEST_CREDENTIALS, format!(r#"{{"{}": {{"public": "heron"}}}}"#, proxy.url("/basic-auth/simple")))
        .env(EnvVars::PATH, venv_bin_path(&keyring_context.venv)), @"
    exit_code: 0 (success)
    ----- stderr -----
    Keyring request for public@http://[LOCALHOST]/basic-auth/simple
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    "
    );

    context.assert_command("import anyio").success();
    Ok(())
}

#[tokio::test]
async fn add_full_url_in_keyring() -> Result<()> {
    let keyring_context = uv_test::test_context!("3.12");

    // Install our keyring plugin
    keyring_context
        .pip_install()
        .arg(
            keyring_context
                .workspace_root
                .join("test")
                .join("packages")
                .join("keyring_stub"),
        )
        .arg(
            keyring_context
                .workspace_root
                .join("test")
                .join("packages")
                .join("keyring_test_plugin"),
        )
        .assert()
        .success();

    let context = uv_test::test_context!("3.12");
    let proxy = crate::pypi_proxy::start().await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc!(
        r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = []
        [tool.uv]
        keyring-provider = "subprocess"
        [[tool.uv.index]]
        name = "proxy"
        url = "{proxy_uri}/basic-auth/simple"
        default = true
        "#,
        proxy_uri = proxy.uri()
    ))?;

    uv_snapshot!(context.filters(), context.add().arg("anyio")
        .env(EnvVars::index_username("PROXY"), "public")
        .env(EnvVars::KEYRING_TEST_CREDENTIALS, format!(r#"{{"{}": {{"public": "heron"}}}}"#, proxy.url("/basic-auth/simple/anyio")))
        .env(EnvVars::PATH, venv_bin_path(&keyring_context.venv)), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Keyring request for public@http://[LOCALHOST]/basic-auth/simple
    Keyring request for public@[LOCALHOST]
    Keyring request for public@http://[LOCALHOST]
    error: Failed to add dependencies
      cause: No solution found when resolving dependencies
      cause: Because anyio was not found in the package registry and your project depends on anyio, we can conclude that your project's requirements are unsatisfiable.

    hint: An index URL (http://[LOCALHOST]/basic-auth/simple) could not be queried due to a lack of valid authentication credentials (401 Unauthorized)

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    "
    );
    Ok(())
}

/// If uv receives an authentication failure from a configured index, it
/// should not fall back to the default index.
#[tokio::test]
async fn add_stop_index_search_early_on_auth_failure() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let proxy = crate::pypi_proxy::start().await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc!(
        r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = []
        [[tool.uv.index]]
        name = "my-index"
        url = "{proxy_uri}/basic-auth/simple"
        "#,
        proxy_uri = proxy.uri()
    ))?;

    uv_snapshot!(context.filters(), context.add().arg("anyio"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to add dependencies
      cause: No solution found when resolving dependencies
      cause: Because anyio was not found in the package registry and your project depends on anyio, we can conclude that your project's requirements are unsatisfiable.

    hint: An index URL (http://[LOCALHOST]/basic-auth/simple) could not be queried due to a lack of valid authentication credentials (401 Unauthorized)

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    "
    );
    Ok(())
}

/// uv should continue searching the default index if it receives an
/// authentication failure that is specified in `ignore-error-codes`.
#[tokio::test]
async fn add_ignore_error_codes() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let proxy = crate::pypi_proxy::start().await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc!(
        r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = []
        [[tool.uv.index]]
        name = "my-index"
        url = "{proxy_uri}/basic-auth/simple"
        ignore-error-codes = [401, 403]
        "#,
        proxy_uri = proxy.uri()
    ))?;

    uv_snapshot!(context.filters(), context.add().arg("anyio"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    "
    );

    context.assert_command("import anyio").success();
    Ok(())
}

/// uv should only fall through on 404s if an empty list is specified
/// in `ignore-error-codes`, even for indexes that normally ignore 403s.
#[tokio::test]
async fn add_empty_ignore_error_codes() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = []

        [[tool.uv.index]]
        name = "my-index"
        url = "{server_url}"
        ignore-error-codes = []
        "#,
        server_url = server.uri(),
    })?;

    // The empty `ignore-error-codes` list means 403 errors should NOT be ignored.
    uv_snapshot!(context.filters(), context.add().arg("anyio"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to add dependencies
      cause: No solution found when resolving dependencies
      cause: Because anyio was not found in the package registry and your project depends on anyio, we can conclude that your project's requirements are unsatisfiable.

    hint: An index (http://[LOCALHOST]/) returned a 403 Forbidden error. Check that the index URL is correct and the credentials are valid.

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    "
    );
    Ok(())
}

/// If a package was available from an index that returned a 403, uv should suggest that the index
/// might use 403s for packages that are not found.
#[tokio::test]
async fn lock_forbidden_index_with_available_package() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/anyio/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"
            {
                "name": "anyio",
                "files": [{
                    "filename": "anyio-4.3.0-py3-none-any.whl",
                    "url": "/anyio-4.3.0-py3-none-any.whl",
                    "hashes": {
                        "sha256": "048e05d0f6caeed70d731f3db756d35dcc1f35747c8c403364a8332c630441b8"
                    },
                    "core-metadata": true,
                    "requires-python": ">=3.8",
                    "upload-time": "2024-02-19T08:36:26Z"
                }]
            }
            "#,
            "application/vnd.pypi.simple.v1+json",
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/anyio-4.3.0-py3-none-any.whl.metadata"))
        .respond_with(ResponseTemplate::new(200).set_body_string(indoc! {"
            Metadata-Version: 2.3
            Name: anyio
            Version: 4.3.0
            Requires-Dist: idna>=2.8
        "}))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/idna/"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = ["anyio"]

        [[tool.uv.index]]
        name = "my-index"
        url = "{server_url}"
        ignore-error-codes = []
        default = true
        "#,
        server_url = server.uri(),
    })?;

    uv_snapshot!(context.filters(), context.lock(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because idna was not found in the package registry and all versions of anyio depend on idna>=2.8, we can conclude that all versions of anyio cannot be used.
             And because your project depends on anyio, we can conclude that your project's requirements are unsatisfiable.

    hint: An index (http://[LOCALHOST]/) returned a 403 Forbidden error, but uv received a successful response from another request to the index. If the failing package is not present on this index, consider adding `ignore-error-codes = [403]` to the index's `[[tool.uv.index]]` entry to continue searching across indexes.
    ");
    Ok(())
}

/// uv should not report a credential error on a missing package for pytorch since
/// pytorch returns 403s to indicate not found.
#[test]
fn add_missing_package_on_pytorch() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = []

        [tool.uv.sources]
        fakepkg = { index = "pytorch" }

        [[tool.uv.index]]
        name = "pytorch"
        url = "https://download.pytorch.org/whl/cpu"
        "#
    })?;

    uv_snapshot!(context.add().arg("fakepkg"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to add dependencies
      cause: No solution found when resolving dependencies
      cause: Because fakepkg was not found in the package registry and your project depends on fakepkg, we can conclude that your project's requirements are unsatisfiable.

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    "
    );
    Ok(())
}

/// Test HTTP errors other than 401s and 403s.
#[tokio::test]
async fn add_unexpected_error_code() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = []
        "#
    })?;

    uv_snapshot!(context.filters(), context.add().arg("anyio").arg("--index").arg(server.uri())
        .env(EnvVars::UV_TEST_NO_HTTP_RETRY_DELAY, "true")
        .env(EnvVars::UV_HTTP_RETRIES, "1"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Request failed after 1 retry in [TIME]
      cause: Failed to fetch: `http://[LOCALHOST]/anyio/`
      cause: HTTP status server error (503 Service Unavailable) for url (http://[LOCALHOST]/anyio/)
    "
    );
    Ok(())
}

/// uv should fail to parse `pyproject.toml` if `ignore-error-codes`
/// contains an invalid status code number.
#[tokio::test]
async fn add_invalid_ignore_error_code() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let proxy = crate::pypi_proxy::start().await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc!(
        r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = []
        [[tool.uv.index]]
        name = "my-index"
        url = "{proxy_uri}/basic-auth/simple"
        ignore-error-codes = [401, 403, 1234]
        "#,
        proxy_uri = proxy.uri()
    ))?;

    uv_snapshot!(context.filters(), context.add().arg("anyio"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: Failed to parse `pyproject.toml` during settings discovery:
      TOML parse error at line 9, column 22
        |
      9 | ignore-error-codes = [401, 403, 1234]
        |                      ^^^^^^^^^^^^^^^^
      1234 is not a valid HTTP status code

    error: Failed to parse: `pyproject.toml`
      cause: TOML parse error at line 9, column 22
               |
             9 | ignore-error-codes = [401, 403, 1234]
               |                      ^^^^^^^^^^^^^^^^
             1234 is not a valid HTTP status code
    "
    );

    Ok(())
}

/// uv should fail to parse `pyproject.toml` if `require-python`
/// contains an invalid specifier and try to return a helpful hint.
#[test]
fn add_invalid_requires_python() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = "3.12"
        dependencies = []
        "#
    })?;

    uv_snapshot!(context.add().arg("anyio"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse: `pyproject.toml`
      cause: TOML parse error at line 4, column 19
               |
             4 | requires-python = "3.12"
               |                   ^^^^^^
             Failed to parse version: Unexpected end of version specifier, expected operator. Did you mean `==3.12`?:
             3.12
             ^^^^
    "#);

    Ok(())
}

/// In authentication "always", the normal authentication flow should still work.
#[tokio::test]
async fn add_auth_policy_always_with_credentials() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let proxy = crate::pypi_proxy::start().await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc!(
        r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = []

        [[tool.uv.index]]
        name = "my-index"
        url = "{proxy_uri}/basic-auth/simple"
        authenticate = "always"
        default = true
        "#,
        proxy_uri = proxy.uri()
    ))?;

    uv_snapshot!(context.filters(), context.add().arg("anyio")
        .env(EnvVars::UV_INDEX_MY_INDEX_USERNAME, "public")
        .env(EnvVars::UV_INDEX_MY_INDEX_PASSWORD, "heron"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    "
    );

    context.assert_command("import anyio").success();
    Ok(())
}

/// In authentication "always", unauthenticated requests to a registry that
/// doesn't require credentials will fail.
#[test]
fn add_auth_policy_always_without_credentials() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = []

        [[tool.uv.index]]
        name = "my-index"
        url = "{index_url}"
        authenticate = "always"
        default = true
        "#,
        index_url = server.index_url(),
    })?;

    uv_snapshot!(context.filters(), context.add().arg("anyio"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch: `http://[LOCALHOST]/simple/anyio/`
      cause: Missing credentials for http://[LOCALHOST]/simple/anyio/
    "
    );

    uv_snapshot!(context.filters(), context.pip_install().arg("black"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch: `http://[LOCALHOST]/simple/black/`
      cause: Missing credentials for http://[LOCALHOST]/simple/black/
    "
    );
    Ok(())
}

/// In authentication "always", authenticated requests with a username but
/// no discoverable password will fail.
#[test]
fn add_auth_policy_always_with_username_no_password() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let mut index_url = Url::parse(&server.index_url())?;
    let _ = index_url.set_username("public");
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = []

        [[tool.uv.index]]
        name = "my-index"
        url = "{index_url}"
        authenticate = "always"
        default = true
        "#,
        index_url = index_url.as_str(),
    })?;

    uv_snapshot!(context.filters(), context.add().arg("anyio"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch: `http://[LOCALHOST]/simple/anyio/`
      cause: Incomplete credentials for http://[LOCALHOST]/simple/anyio/
    "
    );
    Ok(())
}

/// In authentication "never", even if the correct credentials are supplied
/// in the URL, no authenticated requests will be allowed.
#[tokio::test]
async fn add_auth_policy_never_with_url_credentials() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let proxy = crate::pypi_proxy::start().await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc!(
        r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = []

        [[tool.uv.index]]
        name = "my-index"
        url = "{proxy_auth_uri}/basic-auth/simple"
        authenticate = "never"
        default = true
        "#,
        proxy_auth_uri = proxy.authenticated_uri("public", "heron")
    ))?;

    uv_snapshot!(context.filters(), context.add().arg("anyio"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to fetch: `http://[LOCALHOST]/basic-auth/files/packages/14/fd/2f20c40b45e4fb4324834aea24bd4afdf1143390242c0b33774da0e2e34f/anyio-4.3.0-py3-none-any.whl`
      cause: HTTP status client error (401 Unauthorized) for url (http://[LOCALHOST]/basic-auth/files/packages/14/fd/2f20c40b45e4fb4324834aea24bd4afdf1143390242c0b33774da0e2e34f/anyio-4.3.0-py3-none-any.whl)
    "
    );

    Ok(())
}

/// In authentication "never", client errors that are configured to be ignored should allow the
/// resolver to try another version.
#[tokio::test]
async fn add_auth_policy_never_with_url_credentials_ignored() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let proxy = crate::pypi_proxy::start().await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc!(
        r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = []

        [[tool.uv.index]]
        name = "my-index"
        url = "{proxy_auth_uri}/basic-auth/simple"
        authenticate = "never"
        ignore-error-codes = [401]
        default = true
        "#,
        proxy_auth_uri = proxy.authenticated_uri("public", "heron")
    ))?;

    uv_snapshot!(context.filters(), context.add().arg("anyio"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to add dependencies
      cause: No solution found when resolving dependencies
      cause: Because anyio==4.3.0 could not be fetched from the network (`401 Unauthorized`) and only anyio==4.3.0 is available, we can conclude that all versions of anyio cannot be used.
             And because your project depends on anyio, we can conclude that your project's requirements are unsatisfiable.

    hint: Metadata for `anyio` (v4.3.0) could not be fetched; the server returned: `401 Unauthorized`

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    "
    );

    Ok(())
}

/// In authentication "never", even if the correct credentials are supplied
/// via env vars, no authenticated requests will be allowed.
#[tokio::test]
async fn add_auth_policy_never_with_env_var_credentials() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let proxy = crate::pypi_proxy::start().await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc!(
        r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = []

        [[tool.uv.index]]
        name = "my-index"
        url = "{proxy_uri}/basic-auth/simple"
        authenticate = "never"
        default = true
        "#,
        proxy_uri = proxy.uri()
    ))?;

    uv_snapshot!(context.filters(), context.add().arg("anyio")
        .env(EnvVars::UV_INDEX_MY_INDEX_USERNAME, "public")
        .env(EnvVars::UV_INDEX_MY_INDEX_PASSWORD, "heron"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to add dependencies
      cause: No solution found when resolving dependencies
      cause: Because anyio was not found in the package registry and your project depends on anyio, we can conclude that your project's requirements are unsatisfiable.

    hint: An index URL (http://[LOCALHOST]/basic-auth/simple) could not be queried due to a lack of valid authentication credentials (401 Unauthorized)

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    "
    );

    Ok(())
}

/// In authentication "never", the normal flow for unauthenticated requests should
/// still work.
#[test]
fn add_auth_policy_never_without_credentials() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.11, <4"
        dependencies = []

        [[tool.uv.index]]
        name = "my-index"
        url = "{index_url}"
        authenticate = "never"
        default = true
        "#,
        index_url = server.index_url(),
    })?;

    uv_snapshot!(context.filters(), context.add().arg("anyio"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    "
    );

    context.assert_command("import anyio").success();
    Ok(())
}

/// If uv receives a 302 redirect to a cross-origin server, it should not forward
/// credentials. In the absence of a netrc entry for the new location,
/// it should fail.
#[tokio::test]
async fn add_redirect_cross_origin() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filter((r"127\.0\.0\.1:\d*", "[LOCALHOST]"));
    let proxy = crate::pypi_proxy::start().await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = []
        "#
    })?;

    let redirect_server = MockServer::start().await;
    let proxy_base = proxy.url("/basic-auth/simple/");

    Mock::given(method("GET"))
        .respond_with(move |req: &wiremock::Request| {
            let redirect_url = redirect_url_to_base(req, &proxy_base);
            ResponseTemplate::new(302).insert_header("Location", &redirect_url)
        })
        .mount(&redirect_server)
        .await;

    let mut redirect_url = Url::parse(&redirect_server.uri())?;
    let _ = redirect_url.set_username("public");
    let _ = redirect_url.set_password(Some("heron"));

    uv_snapshot!(context.filters(), context.add().arg("--default-index").arg(redirect_url.as_str()).arg("anyio"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to add dependencies
      cause: No solution found when resolving dependencies
      cause: Because anyio was not found in the package registry and your project depends on anyio, we can conclude that your project's requirements are unsatisfiable.

    hint: An index URL (http://[LOCALHOST]/) could not be queried due to a lack of valid authentication credentials (401 Unauthorized)

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    "
    );

    Ok(())
}

/// If uv receives a 302 redirect to a cross-origin server with credentials
/// in the location, use those credentials for the redirect request.
#[tokio::test]
async fn add_redirect_cross_origin_credentials_in_location() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filter((r"127\.0\.0\.1:\d*", "[LOCALHOST]"));
    let proxy = crate::pypi_proxy::start().await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = []
        "#
    })?;

    let redirect_server = MockServer::start().await;
    let proxy_base = proxy.authenticated_url("public", "heron", "/basic-auth/simple/");

    Mock::given(method("GET"))
        .respond_with(move |req: &wiremock::Request| {
            // Responds with credentials in the location
            let redirect_url = redirect_url_to_base(req, &proxy_base);
            ResponseTemplate::new(302).insert_header("Location", &redirect_url)
        })
        .mount(&redirect_server)
        .await;

    let redirect_url = Url::parse(&redirect_server.uri())?;

    uv_snapshot!(context.filters(), context.add().arg("--default-index").arg(redirect_url.as_str()).arg("anyio"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    "
    );

    Ok(())
}

/// uv currently fails to look up keyring credentials on a cross-origin redirect.
#[tokio::test]
async fn add_redirect_with_keyring_cross_origin() -> Result<()> {
    let keyring_context = uv_test::test_context!("3.12");

    // Install our keyring plugin
    keyring_context
        .pip_install()
        .arg(
            keyring_context
                .workspace_root
                .join("test")
                .join("packages")
                .join("keyring_stub"),
        )
        .arg(
            keyring_context
                .workspace_root
                .join("test")
                .join("packages")
                .join("keyring_test_plugin"),
        )
        .assert()
        .success();

    let context = uv_test::test_context!("3.12").with_filter((r"127\.0\.0\.1:\d*", "[LOCALHOST]"));
    let proxy = crate::pypi_proxy::start().await;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        keyring-provider = "subprocess"
        "#,
    })?;

    let redirect_server = MockServer::start().await;
    let proxy_base = proxy.url("/basic-auth/simple/");

    Mock::given(method("GET"))
        .respond_with(move |req: &wiremock::Request| {
            let redirect_url = redirect_url_to_base(req, &proxy_base);
            ResponseTemplate::new(302).insert_header("Location", &redirect_url)
        })
        .mount(&redirect_server)
        .await;

    let mut redirect_url = Url::parse(&redirect_server.uri())?;
    let _ = redirect_url.set_username("public");

    uv_snapshot!(context.filters(), context.add().arg("--default-index")
        .arg(redirect_url.as_str())
        .arg("anyio")
        .env(EnvVars::KEYRING_TEST_CREDENTIALS, format!(r#"{{"{host}": {{"public": "heron"}}}}"#, host = proxy.host_port()))
        .env(EnvVars::PATH, venv_bin_path(&keyring_context.venv)), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Keyring request for public@http://[LOCALHOST]/
    Keyring request for public@[LOCALHOST]
    Keyring request for public@http://[LOCALHOST]
    error: Failed to add dependencies
      cause: No solution found when resolving dependencies
      cause: Because anyio was not found in the package registry and your project depends on anyio, we can conclude that your project's requirements are unsatisfiable.

    hint: An index URL (http://[LOCALHOST]/) could not be queried due to a lack of valid authentication credentials (401 Unauthorized)

    hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
    "
    );

    Ok(())
}

/// If uv receives a cross-origin 302 redirect, it should use credentials from netrc
/// for the new location.
#[tokio::test]
async fn pip_install_redirect_with_netrc_cross_origin() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filter((r"127\.0\.0\.1:\d*", "[LOCALHOST]"));
    let proxy = crate::pypi_proxy::start().await;

    let netrc = context.temp_dir.child(".netrc");
    netrc.write_str(&format!(
        "machine {} login public password heron",
        proxy.host()
    ))?;

    let redirect_server = MockServer::start().await;
    let proxy_base = proxy.url("/basic-auth/simple/");

    Mock::given(method("GET"))
        .respond_with(move |req: &wiremock::Request| {
            let redirect_url = redirect_url_to_base(req, &proxy_base);
            ResponseTemplate::new(302).insert_header("Location", &redirect_url)
        })
        .mount(&redirect_server)
        .await;

    let mut redirect_url = Url::parse(&redirect_server.uri())?;
    let _ = redirect_url.set_username("public");

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("anyio")
        .arg("--index-url")
        .arg(redirect_url.as_str())
        .env(EnvVars::NETRC, netrc.to_str().unwrap())
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    "
    );

    context.assert_command("import anyio").success();

    Ok(())
}

fn redirect_url_to_base(req: &wiremock::Request, base: &str) -> String {
    let last_path_segment = req
        .url
        .path_segments()
        .expect("path has segments")
        .rfind(|segment| !segment.is_empty())
        .expect("path has a package segment");
    format!("{base}{last_path_segment}/")
}

/// Test the error message when adding a package with multiple existing references in
/// `pyproject.toml`.
#[test]
fn add_ambiguous() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "foo"
        version = "0.1.0"
        requires-python = ">=3.12.0"
        dependencies = [
            "anyio>=4.0.0",
            "anyio>=4.1.0",
        ]
        [dependency-groups]
        bar = ["anyio>=4.1.0", "anyio>=4.2.0"]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Cannot perform ambiguous update; found multiple entries for `anyio`:
    - `anyio>=4.0.0`
    - `anyio>=4.1.0`
    ");

    uv_snapshot!(context.filters(), context.add().arg("--group").arg("bar").arg("anyio"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Cannot perform ambiguous update; found multiple entries for `anyio`:
    - `anyio>=4.1.0`
    - `anyio>=4.2.0`
    ");

    Ok(())
}

/// Normalize extra names when adding or removing.
#[test]
fn add_optional_normalize() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.optional-dependencies]
        cloud_export_to_parquet = [
            "anyio==3.7.0",
        ]
    "#})?;

    // Add with a non-normalized group name.
    uv_snapshot!(context.filters(), context.add().arg("iniconfig").arg("--optional").arg("cloud_export_to_parquet"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==3.7.0
     + idna==3.6
     + iniconfig==2.0.0
     + sniffio==1.3.1
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    assert_snapshot!(pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = []

    [project.optional-dependencies]
    cloud_export_to_parquet = [
        "anyio==3.7.0",
        "iniconfig>=2.0.0",
    ]
    "#
    );

    // Add with a normalized group name (which doesn't match the `pyproject.toml`).
    uv_snapshot!(context.filters(), context.add().arg("typing-extensions").arg("--optional").arg("cloud-export-to-parquet"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + typing-extensions==4.10.0
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    assert_snapshot!(pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = []

    [project.optional-dependencies]
    cloud_export_to_parquet = [
        "anyio==3.7.0",
        "iniconfig>=2.0.0",
        "typing-extensions>=4.10.0",
    ]
    "#
    );

    // Remove with a non-normalized group name.
    uv_snapshot!(context.filters(), context.remove().arg("iniconfig").arg("--optional").arg("cloud_export_to_parquet"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    Uninstalled 5 packages in [TIME]
     - anyio==3.7.0
     - idna==3.6
     - iniconfig==2.0.0
     - sniffio==1.3.1
     - typing-extensions==4.10.0
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    assert_snapshot!(pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = []

    [project.optional-dependencies]
    cloud_export_to_parquet = [
        "anyio==3.7.0",
        "typing-extensions>=4.10.0",
    ]
    "#
    );

    // Remove with a normalized group name (which doesn't match the `pyproject.toml`).
    uv_snapshot!(context.filters(), context.remove().arg("typing-extensions").arg("--optional").arg("cloud-export-to-parquet"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Checked in [TIME]
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());

    assert_snapshot!(pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = []

    [project.optional-dependencies]
    cloud_export_to_parquet = [
        "anyio==3.7.0",
    ]
    "#
    );

    Ok(())
}

/// Test `uv add` with different kinds of bounds and constraints.
#[test]
fn add_bounds() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Set bounds in `uv.toml`
    let uv_toml = context.temp_dir.child("uv.toml");
    uv_toml.write_str(indoc! {r#"
        add-bounds = "exact"
    "#})?;
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("idna"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + idna==3.6
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());
    assert_snapshot!(
        pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = [
        "idna==3.6",
    ]
    "#
    );

    fs_err::remove_file(uv_toml)?;

    // Set bounds in `pyproject.toml`
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv]
        add-bounds = "major"
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + anyio==4.3.0
     + sniffio==1.3.1
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());
    assert_snapshot!(
        pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = [
        "anyio>=4.3.0,<5.0.0",
    ]

    [tool.uv]
    add-bounds = "major"
    "#
    );

    // Existing constraints take precedence over the bounds option
    uv_snapshot!(context.filters(), context.add().arg("anyio").arg("--bounds").arg("minor"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Checked 3 packages in [TIME]
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());
    assert_snapshot!(
        pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = [
        "anyio>=4.3.0,<5.0.0",
    ]

    [tool.uv]
    add-bounds = "major"
    "#
    );

    // Explicit constraints take precedence over the bounds option
    uv_snapshot!(context.filters(), context.add().arg("anyio==4.2").arg("idna").arg("--bounds").arg("minor"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - anyio==4.3.0
     + anyio==4.2.0
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());
    assert_snapshot!(
        pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = [
        "anyio==4.2",
        "idna>=3.6,<3.7",
    ]

    [tool.uv]
    add-bounds = "major"
    "#
    );

    // Set bounds on the CLI.
    uv_snapshot!(context.filters(), context.add().arg("sniffio").arg("--bounds").arg("minor"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Checked 3 packages in [TIME]
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());
    assert_snapshot!(
        pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = [
        "anyio==4.2",
        "idna>=3.6,<3.7",
        "sniffio>=1.3.1,<1.4.0",
    ]

    [tool.uv]
    add-bounds = "major"
    "#
    );

    Ok(())
}

/// Hint that we're using an explicit bound over the preferred bounds.
#[test]
fn add_bounds_requirement_over_bounds_kind() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Set bounds in `uv.toml`
    let uv_toml = context.temp_dir.child("uv.toml");
    uv_toml.write_str(indoc! {r#"
        add-bounds = "exact"
    "#})?;
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("anyio==4.2").arg("idna").arg("--bounds").arg("minor"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    note: Using explicit requirement `anyio==4.2` over bounds preference `minor`
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.2.0
     + idna==3.6
     + sniffio==1.3.1
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());
    assert_snapshot!(
        pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = [
        "anyio==4.2",
        "idna>=3.6,<3.7",
    ]
    "#
    );

    Ok(())
}

/// Add a path dependency with `--workspace` flag to add it to workspace members. The root already
/// contains a workspace definition, so the package should be added to the workspace members.
#[test]
fn add_path_with_existing_workspace() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let workspace_toml = context.temp_dir.child("pyproject.toml");
    workspace_toml.write_str(indoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"

        [tool.uv.workspace]
        members = ["project"]
    "#})?;

    // Create a project within the workspace.
    let project_dir = context.temp_dir.child("project");
    project_dir.create_dir_all()?;

    let project_toml = project_dir.child("pyproject.toml");
    project_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Create a dependency package outside the workspace members.
    let dep_dir = context.temp_dir.child("dep");
    dep_dir.create_dir_all()?;

    let dep_toml = dep_dir.child("pyproject.toml");
    dep_toml.write_str(indoc! {r#"
        [project]
        name = "dep"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Add the dependency from the project directory. It should automatically be added as a
    // workspace member, since it's in the same directory as the workspace.
    uv_snapshot!(context.filters(), context
        .add()
        .current_dir(&project_dir)
        .arg("../dep"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Added `dep` to workspace members
    Resolved 3 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + dep==0.1.0 (from file://[TEMP_DIR]/dep)
    ");

    let pyproject_toml = context.read("pyproject.toml");
    assert_snapshot!(
        pyproject_toml, @r#"
    [project]
    name = "parent"
    version = "0.1.0"
    requires-python = ">=3.12"

    [tool.uv.workspace]
    members = [
        "project",
        "dep",
    ]
    "#
    );

    let pyproject_toml = apply_filters(context.read("project/pyproject.toml"), context.filters());
    assert_snapshot!(
        pyproject_toml, @r#"
    [project]
    name = "project"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = [
        "dep",
    ]

    [tool.uv.sources]
    dep = { workspace = true }
    "#
    );

    Ok(())
}

/// Add a path dependency with `--workspace` flag to add it to workspace members. The root doesn't
/// contain a workspace definition, so `uv add` should create one.
#[test]
fn add_path_with_workspace() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let workspace_toml = context.temp_dir.child("pyproject.toml");
    workspace_toml.write_str(indoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;

    // Create a dependency package outside the workspace members.
    let dep_dir = context.temp_dir.child("dep");
    dep_dir.create_dir_all()?;

    let dep_toml = dep_dir.child("pyproject.toml");
    dep_toml.write_str(indoc! {r#"
        [project]
        name = "dep"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Add the dependency with `--workspace` flag from the project directory.
    uv_snapshot!(context.filters(), context
        .add()
        .arg("./dep")
        .arg("--workspace"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Added `dep` to workspace members
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + dep==0.1.0 (from file://[TEMP_DIR]/dep)
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());
    assert_snapshot!(
        pyproject_toml, @r#"
    [project]
    name = "parent"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = [
        "dep",
    ]

    [tool.uv.workspace]
    members = [
        "dep",
    ]

    [tool.uv.sources]
    dep = { workspace = true }
    "#
    );

    Ok(())
}

/// Add a path dependency within the workspace directory without --workspace flag.
/// It should automatically be added as a workspace member.
#[test]
fn add_path_within_workspace_defaults_to_workspace() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let workspace_toml = context.temp_dir.child("pyproject.toml");
    workspace_toml.write_str(indoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv.workspace]
        members = []
    "#})?;

    let dep_dir = context.temp_dir.child("dep");
    dep_dir.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "dep"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Add the dependency without --workspace flag - it should still be added as workspace member
    // since it's within the workspace directory.
    uv_snapshot!(context.filters(), context
        .add()
        .arg("./dep"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Added `dep` to workspace members
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + dep==0.1.0 (from file://[TEMP_DIR]/dep)
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());
    assert_snapshot!(
        pyproject_toml, @r#"
    [project]
    name = "parent"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = [
        "dep",
    ]

    [tool.uv.workspace]
    members = [
        "dep",
    ]

    [tool.uv.sources]
    dep = { workspace = true }
    "#
    );

    Ok(())
}

/// Add a path dependency within the workspace directory with --no-workspace flag.
/// It should be added as a direct path dependency.
#[test]
fn add_path_with_no_workspace() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let workspace_toml = context.temp_dir.child("pyproject.toml");
    workspace_toml.write_str(indoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv.workspace]
        members = []
    "#})?;

    let dep_dir = context.temp_dir.child("dep");
    dep_dir.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "dep"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Add the dependency with --no-workspace flag - it should be added as direct path dependency.
    uv_snapshot!(context.filters(), context
        .add()
        .arg("./dep")
        .arg("--no-workspace"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + dep==0.1.0 (from file://[TEMP_DIR]/dep)
    ");

    let pyproject_toml = apply_filters(context.read("pyproject.toml"), context.filters());
    assert_snapshot!(
        pyproject_toml, @r#"
    [project]
    name = "parent"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = [
        "dep",
    ]

    [tool.uv.workspace]
    members = []

    [tool.uv.sources]
    dep = { path = "dep" }
    "#
    );

    Ok(())
}

/// Add a path dependency outside the workspace directory.
/// It should be added as a direct path dependency, not a workspace member.
#[test]
fn add_path_outside_workspace_no_default() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Create a workspace directory
    let workspace_dir = context.temp_dir.child("workspace");
    workspace_dir.create_dir_all()?;

    let workspace_toml = workspace_dir.child("pyproject.toml");
    workspace_toml.write_str(indoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv.workspace]
        members = []
    "#})?;

    // Create a dependency outside the workspace
    let dep_dir = context.temp_dir.child("external_dep");
    dep_dir.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "dep"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;

    // Add the dependency without --workspace flag - it should be a direct path dependency
    // since it's outside the workspace directory.
    uv_snapshot!(context.filters(), context
        .add()
        .current_dir(&workspace_dir)
        .arg("../external_dep"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + dep==0.1.0 (from file://[TEMP_DIR]/external_dep)
    ");

    let pyproject_toml = apply_filters(fs_err::read_to_string(workspace_toml)?, context.filters());
    assert_snapshot!(
        pyproject_toml, @r#"
    [project]
    name = "parent"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = [
        "dep",
    ]

    [tool.uv.workspace]
    members = []

    [tool.uv.sources]
    dep = { path = "../external_dep" }
    "#
    );

    Ok(())
}

/// Existing sources should survive the in-memory project refresh before re-locking.
#[test]
fn add_preserves_existing_sources_during_staged_update() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "preserved-source-dependency",
        ]

        [tool.uv.sources]
        preserved-source-dependency = { path = "preserved-source-dependency" }
    "#})?;

    context
        .temp_dir
        .child("preserved-source-dependency")
        .child("pyproject.toml")
        .write_str(indoc! {r#"
            [project]
            name = "preserved-source-dependency"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = []

            [build-system]
            requires = ["hatchling"]
            build-backend = "hatchling.build"
        "#})?;
    context
        .temp_dir
        .child("preserved-source-dependency")
        .child("src")
        .child("preserved_source_dependency")
        .child("__init__.py")
        .touch()?;

    context
        .temp_dir
        .child("added-source-dependency")
        .child("pyproject.toml")
        .write_str(indoc! {r#"
            [project]
            name = "added-source-dependency"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = []

            [build-system]
            requires = ["hatchling"]
            build-backend = "hatchling.build"
        "#})?;
    context
        .temp_dir
        .child("added-source-dependency")
        .child("src")
        .child("added_source_dependency")
        .child("__init__.py")
        .touch()?;

    uv_snapshot!(context.filters(), context
        .add()
        .arg("./added-source-dependency")
        .arg("--no-workspace")
        .arg("--no-sync"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    Ok(())
}

/// Existing sources should survive the in-memory project refresh before re-locking.
#[test]
fn remove_preserves_existing_sources_during_staged_update() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "preserved-source-dependency",
            "removed-source-dependency",
        ]

        [tool.uv.sources]
        preserved-source-dependency = { path = "preserved-source-dependency" }
        removed-source-dependency = { path = "removed-source-dependency" }
    "#})?;

    context
        .temp_dir
        .child("preserved-source-dependency")
        .child("pyproject.toml")
        .write_str(indoc! {r#"
            [project]
            name = "preserved-source-dependency"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = []

            [build-system]
            requires = ["hatchling"]
            build-backend = "hatchling.build"
        "#})?;
    context
        .temp_dir
        .child("preserved-source-dependency")
        .child("src")
        .child("preserved_source_dependency")
        .child("__init__.py")
        .touch()?;

    context
        .temp_dir
        .child("removed-source-dependency")
        .child("pyproject.toml")
        .write_str(indoc! {r#"
            [project]
            name = "removed-source-dependency"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = []

            [build-system]
            requires = ["hatchling"]
            build-backend = "hatchling.build"
        "#})?;
    context
        .temp_dir
        .child("removed-source-dependency")
        .child("src")
        .child("removed_source_dependency")
        .child("__init__.py")
        .touch()?;

    uv_snapshot!(context.filters(), context
        .remove()
        .arg("removed-source-dependency")
        .arg("--no-sync"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    Ok(())
}

/// See: <https://github.com/astral-sh/uv/issues/14961>
#[test]
fn add_multiline_indentation() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [dependency-groups]
        dev = ["ruff", "typing-extensions"]
    "#})?;

    uv_snapshot!(context.filters(), context.add().arg("iniconfig").arg("--dev"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + iniconfig==2.0.0
     + ruff==0.3.4
     + typing-extensions==4.10.0
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [dependency-groups]
        dev = [
            "iniconfig>=2.0.0",
            "ruff",
            "typing-extensions",
        ]
        "#
        );
    });

    Ok(())
}

/// Add a requirement without installing the project.
#[test]
fn add_no_install_project() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
    "#})?;
    context
        .temp_dir
        .child("project")
        .child("src")
        .child("project")
        .child("__init__.py")
        .touch()?;

    uv_snapshot!(context.filters(), context.add().arg("iniconfig").arg("--no-install-project"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    let pyproject_toml = context.read("pyproject.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            pyproject_toml, @r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = [
            "iniconfig>=2.0.0",
        ]

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
        "#
        );
    });

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "iniconfig"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/iniconfig-2.0.0.tar.gz", hash = "sha256:48c42a08c0ec1a24f2fe45f4efdefc9c19ac8e0aa8e82284503ccba80398bec3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl", hash = "sha256:8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { editable = "." }
        dependencies = [
            { name = "iniconfig" },
        ]

        [package.metadata]
        requires-dist = [{ name = "iniconfig", specifier = ">=2.0.0" }]
        "#
        );
    });

    uv_snapshot!(context.filters(), context.add().arg("typing-extensions").env(EnvVars::UV_NO_INSTALL_PROJECT, "1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + typing-extensions==4.10.0
    ");

    uv_snapshot!(context.filters(), context.add().arg("typing-extensions").arg("--frozen").env(EnvVars::UV_NO_INSTALL_PROJECT, "1"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument `UV_NO_INSTALL_PROJECT` (environment variable) cannot be used with `--frozen`
    ");

    uv_snapshot!(context.filters(), context.add().arg("typing-extensions").arg("--frozen").env(EnvVars::UV_ONLY_INSTALL_PROJECT, "1"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument `UV_ONLY_INSTALL_PROJECT` (environment variable) cannot be used with `--frozen`
    ");

    Ok(())
}

#[test]
#[cfg(feature = "test-git-lfs")]
fn add_git_lfs_error() -> Result<()> {
    let context = uv_test::test_context!("3.13").with_git_lfs_config();

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = []
    "#})?;

    // Request lfs (via arg) without a Git source.
    uv_snapshot!(context.filters(), context.add().arg("typing-extensions").arg("--lfs"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `typing-extensions` did not resolve to a Git repository, but a Git extension (`--lfs`) was provided.
    ");

    // Request lfs (via env var) without a Git source.
    uv_snapshot!(context.filters(), context.add().arg("typing-extensions").env(EnvVars::UV_GIT_LFS, "true"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + typing-extensions==4.10.0
    ");

    // Request lfs (both arg and env var) without a Git source.
    uv_snapshot!(context.filters(), context.add().arg("typing-extensions").env(EnvVars::UV_GIT_LFS, "true").arg("--lfs"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `typing-extensions` did not resolve to a Git repository, but a Git extension (`--lfs`) was provided.
    ");

    // Request lfs from arg and disable lfs from env var (should be ignored) without a Git source.
    uv_snapshot!(context.filters(), context.add().arg("typing-extensions").env(EnvVars::UV_GIT_LFS, "false").arg("--lfs"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `typing-extensions` did not resolve to a Git repository, but a Git extension (`--lfs`) was provided.
    ");

    Ok(())
}

/// Ensure that `uv add` aborts when malware is detected in a dependency.
#[tokio::test]
async fn add_malware_detected() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    let project = indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#};
    pyproject_toml.write_str(
        &project.replace("dependencies = []", "dependencies = [\"iniconfig==2.0.0\"]"),
    )?;
    context.lock().assert().success();
    context.rewrite_lock_registry_sources("https://pypi.org/simple")?;
    pyproject_toml.write_str(project)?;

    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/querybatch"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"vulns": [{"id": "MAL-2026-1234"}]}]
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v1/vulns/MAL-2026-1234"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "MAL-2026-1234",
            "modified": "2026-01-01T00:00:00Z",
        })))
        .mount(&server)
        .await;

    uv_snapshot!(context.filters(), context
        .add()
        .arg("iniconfig==2.0.0")
        .env(EnvVars::UV_INTERNAL__TEST_DEFAULT_INDEX, "https://pypi.org/simple")
        .arg("--preview-features").arg("malware-check")
        .env(EnvVars::UV_MALWARE_CHECK, "1")
        .env(EnvVars::UV_MALWARE_CHECK_URL, server.uri()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    warning: Malware detected in locked dependencies:
      - `iniconfig==2.0.0`: MAL-2026-1234 (https://osv.dev/vulnerability/MAL-2026-1234)
    error: Malware detected in one or more dependencies that would be installed; aborting sync. Set `UV_MALWARE_CHECK=0` to bypass this check.
    ");

    Ok(())
}
