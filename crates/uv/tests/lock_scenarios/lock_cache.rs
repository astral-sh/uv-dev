use anyhow::Result;
use assert_fs::prelude::*;

use uv_static::EnvVars;
use uv_test::packse::PackseServer;
use uv_test::packse::scenario::Scenario;
use uv_test::uv_snapshot;

/// A conflict enables phase saving before marker forks select different stable and pre-release
/// versions. Cached candidate choices must not leak between those forks.
#[test]
fn conflict_before_prerelease_forks() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let scenario = toml::from_str::<Scenario>(
        r#"
        name = "conflict-before-prerelease-forks"

        [root]
        requires = ["a", "sentinel==1.0.0", "forker"]

        [expected]
        satisfiable = true

        [packages.a.versions."1.0.0"]

        [packages.a.versions."2.0.0"]
        requires = ["sentinel==2.0.0"]

        [packages.sentinel.versions."1.0.0"]

        [packages.sentinel.versions."2.0.0"]

        [packages.forker.versions."1.0.0"]
        requires = [
            "target>=2.0.0b1 ; sys_platform == 'linux'",
            "target==1.0.0 ; sys_platform != 'linux'",
        ]

        [packages.target.versions."1.0.0"]

        [packages.target.versions."2.0.0b1"]
        "#,
    )?;
    let server = PackseServer::from_scenario(&scenario);

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["a", "sentinel==1.0.0", "forker"]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.export()
        .arg("--frozen")
        .arg("--no-header")
        .arg("--no-annotate")
        .arg("--no-hashes")
        .arg("--no-emit-project"), @"
    exit_code: 0 (success)
    ----- stdout -----
    a==1.0.0
    forker==1.0.0
    sentinel==1.0.0
    target==1.0.0 ; sys_platform != 'linux'
    target==2.0.0b1 ; sys_platform == 'linux'
    ");

    Ok(())
}

/// Once a dependency conflict enables phase saving, an explicitly selected index must still win
/// over a newer candidate on the default index.
#[test]
fn conflict_preserves_explicit_index() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let default = conflicting_source_scenario()?;
    let private_scenario = toml::from_str::<Scenario>(
        r#"
        name = "conflict-preserves-explicit-index-private"

        [root]
        requires = ["target"]

        [expected]
        satisfiable = true

        [packages.target.versions."1.0.0"]
        "#,
    )?;
    let private = PackseServer::from_scenario(&private_scenario);

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&format!(
            r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["a", "sentinel==1.0.0", "target"]

        [tool.uv.sources]
        target = {{ index = "private" }}

        [[tool.uv.index]]
        name = "private"
        url = "{}"
        explicit = true
        "#,
            private.index_url()
        ))?;

    uv_snapshot!(context.filters(), context.lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(default.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.export()
        .arg("--frozen")
        .arg("--no-header")
        .arg("--no-annotate")
        .arg("--no-hashes")
        .arg("--no-emit-project"), @"
    exit_code: 0 (success)
    ----- stdout -----
    a==1.0.0
    sentinel==1.0.0
    target==1.0.0
    ");

    Ok(())
}

/// Direct URLs must bypass saved index candidates even after backtracking activates caching.
#[test]
fn conflict_preserves_direct_url() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = conflicting_source_scenario()?;

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&format!(
            r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["a", "sentinel==1.0.0", "target"]

        [tool.uv.sources]
        target = {{ url = "{}" }}
        "#,
            server.file_url("target-1.0.0-py3-none-any.whl")
        ))?;

    uv_snapshot!(context.filters(), context.lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.export()
        .arg("--frozen")
        .arg("--no-header")
        .arg("--no-annotate")
        .arg("--no-hashes")
        .arg("--no-emit-project"), @"
    exit_code: 0 (success)
    ----- stdout -----
    a==1.0.0
    sentinel==1.0.0
    target @ http://[LOCALHOST]/files/target-1.0.0-py3-none-any.whl
    ");

    Ok(())
}

/// Conflicts must not let saved candidates from one required Python environment contaminate
/// another environment's fork.
#[test]
fn conflict_before_required_python_forks() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let scenario = toml::from_str::<Scenario>(
        r#"
        name = "conflict-before-required-python-forks"

        [root]
        requires = ["a", "sentinel==1.0.0", "target"]

        [expected]
        satisfiable = true

        [packages.a.versions."1.0.0"]

        [packages.a.versions."2.0.0"]
        requires = ["sentinel==2.0.0"]

        [packages.sentinel.versions."1.0.0"]

        [packages.sentinel.versions."2.0.0"]

        [packages.target.versions."1.0.0"]
        requires_python = ">=3.12,<3.13"

        [packages.target.versions."2.0.0"]
        requires_python = ">=3.13"
        "#,
    )?;
    let server = PackseServer::from_scenario(&scenario);

    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["a", "sentinel==1.0.0", "target"]

        [tool.uv]
        required-environments = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.export()
        .arg("--frozen")
        .arg("--no-header")
        .arg("--no-annotate")
        .arg("--no-hashes")
        .arg("--no-emit-project"), @"
    exit_code: 0 (success)
    ----- stdout -----
    a==1.0.0
    sentinel==1.0.0
    target==1.0.0 ; python_full_version < '3.13'
    target==2.0.0 ; python_full_version >= '3.13'
    ");

    Ok(())
}

fn conflicting_source_scenario() -> Result<PackseServer> {
    let scenario = toml::from_str::<Scenario>(
        r#"
        name = "conflict-preserves-explicit-source"

        [root]
        requires = ["a", "sentinel==1.0.0", "target"]

        [expected]
        satisfiable = true

        [packages.a.versions."1.0.0"]

        [packages.a.versions."2.0.0"]
        requires = ["sentinel==2.0.0"]

        [packages.sentinel.versions."1.0.0"]

        [packages.sentinel.versions."2.0.0"]

        [packages.target.versions."1.0.0"]

        [packages.target.versions."9.0.0"]
        "#,
    )?;

    Ok(PackseServer::from_scenario(&scenario))
}
