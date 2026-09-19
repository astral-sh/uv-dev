//! DO NOT EDIT
//!
//! Generated with `cargo dev generate-scenario-tests`
//! Scenarios from <test/scenarios>
//!
#![cfg(all(feature = "test-python", feature = "test-pypi"))]
#![expect(clippy::needless_raw_string_hashes)]
#![expect(clippy::doc_markdown)]

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use insta::assert_snapshot;

use uv_static::EnvVars;
use uv_test::packse::PackseServer;
use uv_test::uv_snapshot;

/// There are two packages, `a` and `b`. We select `a` with `a==2.0.0` first, and then `b`, but `a==2.0.0` conflicts with all new versions of `b`, so we backtrack through versions of `b`.
///
/// We need to detect this conflict and prioritize `b` over `a` instead of backtracking down to the too old version of `b==1.0.0` that doesn't depend on `a` anymore.
///
/// ```text
/// wrong-backtracking-basic
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a
/// │   │   ├── satisfied by a-1.0.0
/// │   │   └── satisfied by a-2.0.0
/// │   └── requires b
/// │       ├── satisfied by b-1.0.0
/// │       ├── satisfied by b-2.0.0
/// │       ├── satisfied by b-2.0.1
/// │       ├── satisfied by b-2.0.2
/// │       ├── satisfied by b-2.0.3
/// │       ├── satisfied by b-2.0.4
/// │       ├── satisfied by b-2.0.5
/// │       ├── satisfied by b-2.0.6
/// │       ├── satisfied by b-2.0.7
/// │       ├── satisfied by b-2.0.8
/// │       └── satisfied by b-2.0.9
/// ├── a
/// │   ├── a-1.0.0
/// │   └── a-2.0.0
/// ├── b
/// │   ├── b-1.0.0
/// │   │   └── requires too-old
/// │   │       └── satisfied by too-old-1.0.0
/// │   ├── b-2.0.0
/// │   │   └── requires a==1.0.0
/// │   │       └── satisfied by a-1.0.0
/// │   ├── b-2.0.1
/// │   │   └── requires a==1.0.0
/// │   │       └── satisfied by a-1.0.0
/// │   ├── b-2.0.2
/// │   │   └── requires a==1.0.0
/// │   │       └── satisfied by a-1.0.0
/// │   ├── b-2.0.3
/// │   │   └── requires a==1.0.0
/// │   │       └── satisfied by a-1.0.0
/// │   ├── b-2.0.4
/// │   │   └── requires a==1.0.0
/// │   │       └── satisfied by a-1.0.0
/// │   ├── b-2.0.5
/// │   │   └── requires a==1.0.0
/// │   │       └── satisfied by a-1.0.0
/// │   ├── b-2.0.6
/// │   │   └── requires a==1.0.0
/// │   │       └── satisfied by a-1.0.0
/// │   ├── b-2.0.7
/// │   │   └── requires a==1.0.0
/// │   │       └── satisfied by a-1.0.0
/// │   ├── b-2.0.8
/// │   │   └── requires a==1.0.0
/// │   │       └── satisfied by a-1.0.0
/// │   └── b-2.0.9
/// │       └── requires a==1.0.0
/// │           └── satisfied by a-1.0.0
/// └── too-old
///     └── too-old-1.0.0
/// ```
#[test]
fn wrong_backtracking_basic() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("backtracking/wrong-backtracking-basic.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a''',
          '''b''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:957f99ff1d65ce0d7883d50f4e67ed8d4b42e76d2c2b5e62384ff0ba538647b5", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:f936eedc194aa91ca01a4c6c9981136ca6c75ce6df47e3951b12522881dce809", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "2.0.9"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "a" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/b-2.0.9.tar.gz", hash = "sha256:8a0dca91cfb1e865caa23018dc01a32afc0ede285eb653a34e87929401af0152", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-2.0.9-py3-none-any.whl", hash = "sha256:fb91402b66338aaf9408407aa3681dcdd0984b9774ecf46632bc3761198399fa", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a" },
            { name = "b" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a" },
            { name = "b" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// There are three packages, `a`, `b` and `b-inner`. Unlike wrong-backtracking-basic, `b` depends on `b-inner` and `a` and `b-inner` conflict, to add a layer of indirection.
///
/// We select `a` with `a==2.0.0` first, then `b`, and then `b-inner`, but `a==2.0.0` conflicts with all new versions of `b-inner`, so we backtrack through versions of `b-inner`.
///
/// We need to detect this conflict and prioritize `b` and `b-inner` over `a` instead of backtracking down to the too old version of `b-inner==1.0.0` that doesn't depend on `a` anymore.
///
/// ```text
/// wrong-backtracking-indirect
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a
/// │   │   ├── satisfied by a-1.0.0
/// │   │   └── satisfied by a-2.0.0
/// │   └── requires b
/// │       └── satisfied by b-1.0.0
/// ├── a
/// │   ├── a-1.0.0
/// │   └── a-2.0.0
/// ├── b
/// │   └── b-1.0.0
/// │       └── requires b-inner
/// │           ├── satisfied by b-inner-1.0.0
/// │           ├── satisfied by b-inner-2.0.0
/// │           ├── satisfied by b-inner-2.0.1
/// │           ├── satisfied by b-inner-2.0.2
/// │           ├── satisfied by b-inner-2.0.3
/// │           ├── satisfied by b-inner-2.0.4
/// │           ├── satisfied by b-inner-2.0.5
/// │           ├── satisfied by b-inner-2.0.6
/// │           ├── satisfied by b-inner-2.0.7
/// │           ├── satisfied by b-inner-2.0.8
/// │           └── satisfied by b-inner-2.0.9
/// ├── b-inner
/// │   ├── b-inner-1.0.0
/// │   │   └── requires too-old
/// │   │       └── satisfied by too-old-1.0.0
/// │   ├── b-inner-2.0.0
/// │   │   └── requires a==1.0.0
/// │   │       └── satisfied by a-1.0.0
/// │   ├── b-inner-2.0.1
/// │   │   └── requires a==1.0.0
/// │   │       └── satisfied by a-1.0.0
/// │   ├── b-inner-2.0.2
/// │   │   └── requires a==1.0.0
/// │   │       └── satisfied by a-1.0.0
/// │   ├── b-inner-2.0.3
/// │   │   └── requires a==1.0.0
/// │   │       └── satisfied by a-1.0.0
/// │   ├── b-inner-2.0.4
/// │   │   └── requires a==1.0.0
/// │   │       └── satisfied by a-1.0.0
/// │   ├── b-inner-2.0.5
/// │   │   └── requires a==1.0.0
/// │   │       └── satisfied by a-1.0.0
/// │   ├── b-inner-2.0.6
/// │   │   └── requires a==1.0.0
/// │   │       └── satisfied by a-1.0.0
/// │   ├── b-inner-2.0.7
/// │   │   └── requires a==1.0.0
/// │   │       └── satisfied by a-1.0.0
/// │   ├── b-inner-2.0.8
/// │   │   └── requires a==1.0.0
/// │   │       └── satisfied by a-1.0.0
/// │   └── b-inner-2.0.9
/// │       └── requires a==1.0.0
/// │           └── satisfied by a-1.0.0
/// └── too-old
///     └── too-old-1.0.0
/// ```
#[test]
fn wrong_backtracking_indirect() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("backtracking/wrong-backtracking-indirect.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a''',
          '''b''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [[package]]
        name = "a"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/a-2.0.0.tar.gz", hash = "sha256:9610291c2bd57390019f58ca72d0dd4584bb9e7073fa347633ed8bc7267fccfe", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-2.0.0-py3-none-any.whl", hash = "sha256:833374310e0a15880f3be9e6d082f527c9ac70129b2054d733da9b754315361f", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "b-inner" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/b-1.0.0.tar.gz", hash = "sha256:dc8e65ac8e153f517377e576ac880386219fa74cad98ef9dc8ccc7ffaaebb55e", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-1.0.0-py3-none-any.whl", hash = "sha256:e64d7b65e2bc771f36c53ea70d805ebf643322fa2d1761a0dc45b75d0374e2fb", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b-inner"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "too-old" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/b_inner-1.0.0.tar.gz", hash = "sha256:9593374b380761095c60460348d2134a778370ccb51d1bb8a9893c7d51934c8c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b_inner-1.0.0-py3-none-any.whl", hash = "sha256:dc550a3821df74da8f99f92aeff395eae8f050034fe8403a13043d42dca06a95", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a" },
            { name = "b" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a" },
            { name = "b" },
        ]

        [[package]]
        name = "too-old"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/too_old-1.0.0.tar.gz", hash = "sha256:91e0cdc85c04e313e2a5cf8b4ad6459e61594f62d91b04ad658ae44d48b1644a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/too_old-1.0.0-py3-none-any.whl", hash = "sha256:7efb79d455d0a679335ce5abee7d3bf298cac8c6e0aa19654b7c033d603c93ef", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// This test ensures that multiple non-conflicting but also
/// non-overlapping dependency specifications with the same package name
/// are allowed and supported.
///
/// At time of writing, this provokes a fork in the resolver, but it
/// arguably shouldn't since the requirements themselves do not conflict
/// with one another. However, this does impact resolution. Namely, it
/// leaves the `a>=1` fork free to choose `a==2.0.0` since it behaves as if
/// the `a<2` constraint doesn't exist.
///
///
/// ```text
/// fork-allows-non-conflicting-non-overlapping-dependencies
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a>=1 ; sys_platform == 'linux'
/// │   │   ├── satisfied by a-1.0.0
/// │   │   └── satisfied by a-2.0.0
/// │   └── requires a<2 ; sys_platform == 'darwin'
/// │       └── satisfied by a-1.0.0
/// └── a
///     ├── a-1.0.0
///     └── a-2.0.0
/// ```
#[test]
fn fork_allows_non_conflicting_non_overlapping_dependencies() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/allows-non-conflicting-non-overlapping-dependencies.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a>=1 ; sys_platform == 'linux'''',
          '''a<2 ; sys_platform == 'darwin'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "sys_platform == 'darwin'",
            "sys_platform == 'linux'",
            "sys_platform != 'darwin' and sys_platform != 'linux'",
        ]

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:957f99ff1d65ce0d7883d50f4e67ed8d4b42e76d2c2b5e62384ff0ba538647b5", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:f936eedc194aa91ca01a4c6c9981136ca6c75ce6df47e3951b12522881dce809", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a", marker = "sys_platform == 'darwin' or sys_platform == 'linux'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a", marker = "sys_platform == 'darwin'", specifier = "<2" },
            { name = "a", marker = "sys_platform == 'linux'", specifier = ">=1" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// This test ensures that multiple non-conflicting dependency
/// specifications with the same package name are allowed and supported.
///
/// This test exists because the universal resolver forks itself based on
/// duplicate dependency specifications by looking at package name. So at
/// first glance, a case like this could perhaps cause an errant fork.
/// While it's difficult to test for "does not create a fork" (at time of
/// writing, the implementation does not fork), we can at least check that
/// this case is handled correctly without issue. Namely, forking should
/// only occur when there are duplicate dependency specifications with
/// disjoint marker expressions.
///
///
/// ```text
/// fork-allows-non-conflicting-repeated-dependencies
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a>=1
/// │   │   ├── satisfied by a-1.0.0
/// │   │   └── satisfied by a-2.0.0
/// │   └── requires a<2
/// │       └── satisfied by a-1.0.0
/// └── a
///     ├── a-1.0.0
///     └── a-2.0.0
/// ```
#[test]
fn fork_allows_non_conflicting_repeated_dependencies() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/allows-non-conflicting-repeated-dependencies.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a>=1''',
          '''a<2''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:957f99ff1d65ce0d7883d50f4e67ed8d4b42e76d2c2b5e62384ff0ba538647b5", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:f936eedc194aa91ca01a4c6c9981136ca6c75ce6df47e3951b12522881dce809", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a", specifier = "<2" },
            { name = "a", specifier = ">=1" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// An extremely basic test of universal resolution. In this case, the resolution
/// should contain two distinct versions of `a` depending on `sys_platform`.
///
///
/// ```text
/// fork-basic
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a>=2 ; sys_platform == 'linux'
/// │   │   └── satisfied by a-2.0.0
/// │   └── requires a<2 ; sys_platform == 'darwin'
/// │       └── satisfied by a-1.0.0
/// └── a
///     ├── a-1.0.0
///     └── a-2.0.0
/// ```
#[test]
fn fork_basic() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/basic.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a>=2 ; sys_platform == 'linux'''',
          '''a<2 ; sys_platform == 'darwin'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "sys_platform == 'darwin'",
            "sys_platform == 'linux'",
            "sys_platform != 'darwin' and sys_platform != 'linux'",
        ]

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'darwin'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:957f99ff1d65ce0d7883d50f4e67ed8d4b42e76d2c2b5e62384ff0ba538647b5", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:f936eedc194aa91ca01a4c6c9981136ca6c75ce6df47e3951b12522881dce809", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "a"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-2.0.0.tar.gz", hash = "sha256:9610291c2bd57390019f58ca72d0dd4584bb9e7073fa347633ed8bc7267fccfe", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-2.0.0-py3-none-any.whl", hash = "sha256:833374310e0a15880f3be9e6d082f527c9ac70129b2054d733da9b754315361f", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'darwin'" },
            { name = "a", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'linux'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a", marker = "sys_platform == 'darwin'", specifier = "<2" },
            { name = "a", marker = "sys_platform == 'linux'", specifier = ">=2" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// We have a conflict after forking. This scenario exists to test the error message.
///
///
/// ```text
/// conflict-in-fork
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a>=2 ; sys_platform == 'os1'
/// │   │   └── satisfied by a-2.0.0
/// │   └── requires a<2 ; sys_platform == 'os2'
/// │       └── satisfied by a-1.0.0
/// ├── a
/// │   ├── a-1.0.0
/// │   │   ├── requires b
/// │   │   │   └── satisfied by b-1.0.0
/// │   │   └── requires c
/// │   │       └── satisfied by c-1.0.0
/// │   └── a-2.0.0
/// ├── b
/// │   └── b-1.0.0
/// │       └── requires d==1
/// │           └── satisfied by d-1.0.0
/// ├── c
/// │   └── c-1.0.0
/// │       └── requires d==2
/// │           └── satisfied by d-2.0.0
/// └── d
///     ├── d-1.0.0
///     └── d-2.0.0
/// ```
#[test]
fn conflict_in_fork() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/conflict-in-fork.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a>=2 ; sys_platform == 'os1'''',
          '''a<2 ; sys_platform == 'os2'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies for split (markers: sys_platform == 'os2')
      cause: Because all versions of c depend on d==2 and all versions of b depend on d==1, we can conclude that all versions of b and all versions of c are incompatible.
             And because a<=1.0.0 depends on b, we can conclude that a<=1.0.0 and all versions of c are incompatible.
             And because a<=1.0.0 depends on c and your project depends on a{sys_platform == 'os2'}<2, we can conclude that your project's requirements are unsatisfiable.

    hint: The resolution failed for an environment that is not the current one, consider limiting the environments with `tool.uv.environments`.
    "
    );

    Ok(())
}

/// This test ensures that conflicting dependency specifications lead to an
/// unsatisfiable result.
///
/// In particular, this is a case that should not fork even though there
/// are conflicting requirements because their marker expressions are
/// overlapping. (Well, there aren't any marker expressions here, which
/// means they are both unconditional.)
///
///
/// ```text
/// fork-conflict-unsatisfiable
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a>=2
/// │   │   ├── satisfied by a-2.0.0
/// │   │   └── satisfied by a-3.0.0
/// │   └── requires a<2
/// │       └── satisfied by a-1.0.0
/// └── a
///     ├── a-1.0.0
///     ├── a-2.0.0
///     └── a-3.0.0
/// ```
#[test]
fn fork_conflict_unsatisfiable() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/conflict-unsatisfiable.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a>=2''',
          '''a<2''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because your project depends on a>=2 and a<2, we can conclude that your project's requirements are unsatisfiable.
    "
    );

    Ok(())
}

/// Two independent parents in an earlier fork initially choose version `2` of
/// their shared dependencies. A delayed sibling first requires `shared-a==1`,
/// then reaches `shared-b==1` through a second dependency chain. The completed
/// fork must accumulate both agreements on its original checkpoint. Satisfying
/// the later `shared-b` agreement must not restore `switchable-a==2` and recreate
/// the duplicate eliminated by the first agreement.
///
///
/// ```text
/// coordinated-agreement-accumulation
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires later-entry ; python_full_version >= '3.13'
/// │   │   └── satisfied by later-entry-1.0.0
/// │   ├── requires switchable-a ; python_full_version < '3.13'
/// │   │   ├── satisfied by switchable-a-1.0.0
/// │   │   └── satisfied by switchable-a-2.0.0
/// │   └── requires switchable-b ; python_full_version < '3.13'
/// │       ├── satisfied by switchable-b-1.0.0
/// │       └── satisfied by switchable-b-2.0.0
/// ├── later-entry
/// │   └── later-entry-1.0.0
/// │       └── requires start-one
/// │           └── satisfied by start-one-1.0.0
/// ├── shared-a
/// │   ├── shared-a-1.0.0
/// │   └── shared-a-2.0.0
/// ├── shared-b
/// │   ├── shared-b-1.0.0
/// │   └── shared-b-2.0.0
/// ├── stage-a
/// │   └── stage-a-1.0.0
/// │       ├── requires shared-a==1.0.0
/// │       │   └── satisfied by shared-a-1.0.0
/// │       └── requires stage-b-one
/// │           └── satisfied by stage-b-one-1.0.0
/// ├── stage-b-one
/// │   └── stage-b-one-1.0.0
/// │       └── requires stage-b-two
/// │           └── satisfied by stage-b-two-1.0.0
/// ├── stage-b-three
/// │   └── stage-b-three-1.0.0
/// │       └── requires shared-b==1.0.0
/// │           └── satisfied by shared-b-1.0.0
/// ├── stage-b-two
/// │   └── stage-b-two-1.0.0
/// │       └── requires stage-b-three
/// │           └── satisfied by stage-b-three-1.0.0
/// ├── start-four
/// │   └── start-four-1.0.0
/// │       └── requires stage-a
/// │           └── satisfied by stage-a-1.0.0
/// ├── start-one
/// │   └── start-one-1.0.0
/// │       └── requires start-two
/// │           └── satisfied by start-two-1.0.0
/// ├── start-three
/// │   └── start-three-1.0.0
/// │       └── requires start-four
/// │           └── satisfied by start-four-1.0.0
/// ├── start-two
/// │   └── start-two-1.0.0
/// │       └── requires start-three
/// │           └── satisfied by start-three-1.0.0
/// ├── switchable-a
/// │   ├── switchable-a-1.0.0
/// │   │   └── requires shared-a==1.0.0
/// │   │       └── satisfied by shared-a-1.0.0
/// │   └── switchable-a-2.0.0
/// │       └── requires shared-a==2.0.0
/// │           └── satisfied by shared-a-2.0.0
/// └── switchable-b
///     ├── switchable-b-1.0.0
///     │   └── requires shared-b==1.0.0
///     │       └── satisfied by shared-b-1.0.0
///     └── switchable-b-2.0.0
///         └── requires shared-b==2.0.0
///             └── satisfied by shared-b-2.0.0
/// ```
#[test]
fn coordinated_agreement_accumulation() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/coordinated/coordinated-agreement-accumulation.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''switchable-a ; python_full_version < '3.13'''',
          '''switchable-b ; python_full_version < '3.13'''',
          '''later-entry ; python_full_version >= '3.13'''',
        ]
        requires-python = ">=3.12, <3.14"
        [tool.uv]
        fork-strategy = "fewest"
        environments = [
          '''python_full_version == '3.12.*'''',
          '''python_full_version == '3.13.*'''',
        ]
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 14 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12, <3.14"
        resolution-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]
        supported-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]

        [options]
        fork-strategy = "fewest"

        [[package]]
        name = "later-entry"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "start-one" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/later_entry-1.0.0.tar.gz", hash = "sha256:0d76dedae8ca69f2eb43c6dad256cc4ab5b43f90fff683cd32d296a6ea8d6a18", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/later_entry-1.0.0-py3-none-any.whl", hash = "sha256:81af3d22ffa59437ee3a1468db173f77ee03ddbb53ed3e2f0f2cdcc65644a189", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "later-entry", marker = "python_full_version >= '3.13'" },
            { name = "switchable-a", marker = "python_full_version < '3.13'" },
            { name = "switchable-b", marker = "python_full_version < '3.13'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "later-entry", marker = "python_full_version >= '3.13'" },
            { name = "switchable-a", marker = "python_full_version < '3.13'" },
            { name = "switchable-b", marker = "python_full_version < '3.13'" },
        ]

        [[package]]
        name = "shared-a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/shared_a-1.0.0.tar.gz", hash = "sha256:ab5cd85c53970d7431d7ffba2c7cd6401399a6d7829f15d491e5bbfce39e005b", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared_a-1.0.0-py3-none-any.whl", hash = "sha256:31a1838c977f8bd08eecf1c67185f114a5cf5d668e97d835af2412cd8d46c1a8", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "shared-b"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/shared_b-1.0.0.tar.gz", hash = "sha256:c42108896ea8f684bea39b436426c9db11dbb08726793f14e30657e9d21615ae", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared_b-1.0.0-py3-none-any.whl", hash = "sha256:302da71881eb0a1ff18d2163a14bbb3255e31a4a40e70655c8448351309adb40", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "stage-a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "shared-a" },
            { name = "stage-b-one" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/stage_a-1.0.0.tar.gz", hash = "sha256:8ea2f1182ba3054f969e180d8ff2b3a81fb751d00ea207113ece1d77f178d2ff", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/stage_a-1.0.0-py3-none-any.whl", hash = "sha256:d2d52720cbcb35d1ecfc24b336842d36a6355b7de3a858a99ff609f79c6355cf", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "stage-b-one"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "stage-b-two" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/stage_b_one-1.0.0.tar.gz", hash = "sha256:684e28d33573c19292970adeb21c582aeee800ab53ec8fb12e27c17d8b618f2e", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/stage_b_one-1.0.0-py3-none-any.whl", hash = "sha256:3504c86ed0906cd0ec182a12283e3c99360a32ca6d57147e3e78f07ae4f36aae", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "stage-b-three"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "shared-b" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/stage_b_three-1.0.0.tar.gz", hash = "sha256:f2e9bcf552320a6fd321439265ab52d74c0b91ed4b88332c1e502d3d50f19a0a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/stage_b_three-1.0.0-py3-none-any.whl", hash = "sha256:7debf85011b0fe9081357e75b10d6a37f3561c51f36f3ee097792e1c41587b70", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "stage-b-two"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "stage-b-three" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/stage_b_two-1.0.0.tar.gz", hash = "sha256:542148c9d1c25f89e7e0254ae33663ff7b469709fbd3d99ae9f3ddb36c2f447b", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/stage_b_two-1.0.0-py3-none-any.whl", hash = "sha256:57f735647af2f79618b997e6f546e5862642d1778db99e1b4570b64781c35ee6", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "start-four"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "stage-a" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/start_four-1.0.0.tar.gz", hash = "sha256:5259472e93888c2c69e99c537b6e66c7b3aaaad226d3103cebb3d141b6f791ef", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/start_four-1.0.0-py3-none-any.whl", hash = "sha256:460d781dd55f4ce732c91f39ba657f5dfb16b5c37c50136a0bc7d3638bf68de3", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "start-one"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "start-two" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/start_one-1.0.0.tar.gz", hash = "sha256:22d25d454e961e26fef41b983e49015cab8b4c146c3ce9f16d93c0c87575f127", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/start_one-1.0.0-py3-none-any.whl", hash = "sha256:5f447eabf4147fa28087ad0d108371ea9b8b3845be40d386011c39fea68b4a38", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "start-three"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "start-four" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/start_three-1.0.0.tar.gz", hash = "sha256:1eec91e54802646ec54fd02a65431923d2557ed8dcda28f955c6d1a217ebbff2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/start_three-1.0.0-py3-none-any.whl", hash = "sha256:a2393e533950f17a819923c5cdc5fe184cf8c84749f18d51bd36d06dd4c298eb", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "start-two"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "start-three" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/start_two-1.0.0.tar.gz", hash = "sha256:3f6155f68efda504903194ae10121704405c2d5d1e03ea36c181d49f4f8b73ac", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/start_two-1.0.0-py3-none-any.whl", hash = "sha256:81f078a796a83e7263d79b81b0b80f30e1d8e321c98e1a335725eba2ced5f0fd", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "switchable-a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "shared-a" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/switchable_a-1.0.0.tar.gz", hash = "sha256:2b4b679289012946ed69b0eddc5128b0fa6ef849eb9ec94bb5222a24a64c58bd", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/switchable_a-1.0.0-py3-none-any.whl", hash = "sha256:915f35f479cd194fcbc870796ae6df875a127a94292401894f9ae7bbc62ea267", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "switchable-b"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "shared-b" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/switchable_b-1.0.0.tar.gz", hash = "sha256:91bdb2c42b9626de899ea3aa37d67d573bb8cb377979bd5b01bdd8ca804d8b17", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/switchable_b-1.0.0-py3-none-any.whl", hash = "sha256:107ee0f05cf2a5be1336eb15bc10eb5e2060e067e129fbac11f052282b42b224", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// The earlier fork can eliminate the first duplicate by downgrading
/// `switchable-a`. A delayed sibling then selects `leaf==2` before introducing
/// `shared-b==1`. Downgrading `switchable-b` would make `shared-b` consistent, but
/// would also introduce `leaf==1` and create a duplicate of `leaf`. The second
/// agreement must be rejected because it does not improve the total number of
/// duplicate versions. The first accepted agreement must remain in effect.
///
///
/// ```text
/// coordinated-agreement-non-improvement
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires later-entry ; python_full_version >= '3.13'
/// │   │   └── satisfied by later-entry-1.0.0
/// │   ├── requires switchable-a ; python_full_version < '3.13'
/// │   │   ├── satisfied by switchable-a-1.0.0
/// │   │   └── satisfied by switchable-a-2.0.0
/// │   └── requires switchable-b ; python_full_version < '3.13'
/// │       ├── satisfied by switchable-b-1.0.0
/// │       └── satisfied by switchable-b-2.0.0
/// ├── later-entry
/// │   └── later-entry-1.0.0
/// │       └── requires start-one
/// │           └── satisfied by start-one-1.0.0
/// ├── leaf
/// │   ├── leaf-1.0.0
/// │   └── leaf-2.0.0
/// ├── shared-a
/// │   ├── shared-a-1.0.0
/// │   └── shared-a-2.0.0
/// ├── shared-b
/// │   ├── shared-b-1.0.0
/// │   └── shared-b-2.0.0
/// ├── stage-a
/// │   └── stage-a-1.0.0
/// │       ├── requires shared-a==1.0.0
/// │       │   └── satisfied by shared-a-1.0.0
/// │       └── requires trade-gate
/// │           └── satisfied by trade-gate-1.0.0
/// ├── start-five
/// │   └── start-five-1.0.0
/// │       └── requires stage-a
/// │           └── satisfied by stage-a-1.0.0
/// ├── start-four
/// │   └── start-four-1.0.0
/// │       └── requires start-five
/// │           └── satisfied by start-five-1.0.0
/// ├── start-one
/// │   └── start-one-1.0.0
/// │       └── requires start-two
/// │           └── satisfied by start-two-1.0.0
/// ├── start-three
/// │   └── start-three-1.0.0
/// │       └── requires start-four
/// │           └── satisfied by start-four-1.0.0
/// ├── start-two
/// │   └── start-two-1.0.0
/// │       └── requires start-three
/// │           └── satisfied by start-three-1.0.0
/// ├── switchable-a
/// │   ├── switchable-a-1.0.0
/// │   │   └── requires shared-a==1.0.0
/// │   │       └── satisfied by shared-a-1.0.0
/// │   └── switchable-a-2.0.0
/// │       └── requires shared-a==2.0.0
/// │           └── satisfied by shared-a-2.0.0
/// ├── switchable-b
/// │   ├── switchable-b-1.0.0
/// │   │   ├── requires leaf==1.0.0
/// │   │   │   └── satisfied by leaf-1.0.0
/// │   │   └── requires shared-b==1.0.0
/// │   │       └── satisfied by shared-b-1.0.0
/// │   └── switchable-b-2.0.0
/// │       ├── requires leaf==2.0.0
/// │       │   └── satisfied by leaf-2.0.0
/// │       └── requires shared-b==2.0.0
/// │           └── satisfied by shared-b-2.0.0
/// ├── trade-delay-one
/// │   └── trade-delay-one-1.0.0
/// │       └── requires trade-delay-two
/// │           └── satisfied by trade-delay-two-1.0.0
/// ├── trade-delay-three
/// │   └── trade-delay-three-1.0.0
/// │       └── requires shared-b==1.0.0
/// │           └── satisfied by shared-b-1.0.0
/// ├── trade-delay-two
/// │   └── trade-delay-two-1.0.0
/// │       └── requires trade-delay-three
/// │           └── satisfied by trade-delay-three-1.0.0
/// └── trade-gate
///     └── trade-gate-1.0.0
///         ├── requires leaf==2.0.0
///         │   └── satisfied by leaf-2.0.0
///         └── requires trade-delay-one
///             └── satisfied by trade-delay-one-1.0.0
/// ```
#[test]
fn coordinated_agreement_non_improvement() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/coordinated/coordinated-agreement-non-improvement.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''switchable-a ; python_full_version < '3.13'''',
          '''switchable-b ; python_full_version < '3.13'''',
          '''later-entry ; python_full_version >= '3.13'''',
        ]
        requires-python = ">=3.12, <3.14"
        [tool.uv]
        fork-strategy = "fewest"
        environments = [
          '''python_full_version == '3.12.*'''',
          '''python_full_version == '3.13.*'''',
        ]
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 18 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12, <3.14"
        resolution-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]
        supported-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]

        [options]
        fork-strategy = "fewest"

        [[package]]
        name = "later-entry"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "start-one" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/later_entry-1.0.0.tar.gz", hash = "sha256:0d76dedae8ca69f2eb43c6dad256cc4ab5b43f90fff683cd32d296a6ea8d6a18", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/later_entry-1.0.0-py3-none-any.whl", hash = "sha256:81af3d22ffa59437ee3a1468db173f77ee03ddbb53ed3e2f0f2cdcc65644a189", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "leaf"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/leaf-2.0.0.tar.gz", hash = "sha256:bc5de13acb59a7f406cce765be6be6cbab785a7c9c9894dfc880580568cefc19", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/leaf-2.0.0-py3-none-any.whl", hash = "sha256:0eed3aff23491c8f604e933140e005dde855457c5a75508f39d416852a38d552", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "later-entry", marker = "python_full_version >= '3.13'" },
            { name = "switchable-a", marker = "python_full_version < '3.13'" },
            { name = "switchable-b", marker = "python_full_version < '3.13'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "later-entry", marker = "python_full_version >= '3.13'" },
            { name = "switchable-a", marker = "python_full_version < '3.13'" },
            { name = "switchable-b", marker = "python_full_version < '3.13'" },
        ]

        [[package]]
        name = "shared-a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/shared_a-1.0.0.tar.gz", hash = "sha256:ab5cd85c53970d7431d7ffba2c7cd6401399a6d7829f15d491e5bbfce39e005b", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared_a-1.0.0-py3-none-any.whl", hash = "sha256:31a1838c977f8bd08eecf1c67185f114a5cf5d668e97d835af2412cd8d46c1a8", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "shared-b"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "python_full_version >= '3.13'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/shared_b-1.0.0.tar.gz", hash = "sha256:c42108896ea8f684bea39b436426c9db11dbb08726793f14e30657e9d21615ae", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared_b-1.0.0-py3-none-any.whl", hash = "sha256:302da71881eb0a1ff18d2163a14bbb3255e31a4a40e70655c8448351309adb40", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "shared-b"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "python_full_version < '3.13'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/shared_b-2.0.0.tar.gz", hash = "sha256:c14a44d12edd41387c48de54f142b7bf69e858f55bbf9e9fdc46fd4316d19ae9", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared_b-2.0.0-py3-none-any.whl", hash = "sha256:99f58fa1a17a9bc4b114cdf962677f3e92c9db1536a5cf9a68aaa95e199fdd89", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "stage-a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "shared-a" },
            { name = "trade-gate" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/stage_a-1.0.0.tar.gz", hash = "sha256:750d2449087e500b63ee350fd14cfaa263a1b3f76eccf7c94b02f74e09efc2a3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/stage_a-1.0.0-py3-none-any.whl", hash = "sha256:2d8b5ab830bb0501985e36ce8c8f39d6bb8e59079b867e82237ca92fcb32907a", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "start-five"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "stage-a" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/start_five-1.0.0.tar.gz", hash = "sha256:c18f1540ce8094d38d94aeeb97936a7d7fbe656a3f0cd41646e20de01d746b25", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/start_five-1.0.0-py3-none-any.whl", hash = "sha256:79205ec72a9a5dd3f87900d8049e3d1196d2c05248af05729f62aff78f469b37", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "start-four"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "start-five" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/start_four-1.0.0.tar.gz", hash = "sha256:1f8df4f971b8c4f4af7e499da174fe8e59645ee2bb521ec57dbb752cca49311a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/start_four-1.0.0-py3-none-any.whl", hash = "sha256:8457b106ec43027590bf6bcd99e32085334851efbc4299ffd20eb8c3fc97a7c0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "start-one"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "start-two" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/start_one-1.0.0.tar.gz", hash = "sha256:22d25d454e961e26fef41b983e49015cab8b4c146c3ce9f16d93c0c87575f127", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/start_one-1.0.0-py3-none-any.whl", hash = "sha256:5f447eabf4147fa28087ad0d108371ea9b8b3845be40d386011c39fea68b4a38", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "start-three"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "start-four" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/start_three-1.0.0.tar.gz", hash = "sha256:1eec91e54802646ec54fd02a65431923d2557ed8dcda28f955c6d1a217ebbff2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/start_three-1.0.0-py3-none-any.whl", hash = "sha256:a2393e533950f17a819923c5cdc5fe184cf8c84749f18d51bd36d06dd4c298eb", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "start-two"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "start-three" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/start_two-1.0.0.tar.gz", hash = "sha256:3f6155f68efda504903194ae10121704405c2d5d1e03ea36c181d49f4f8b73ac", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/start_two-1.0.0-py3-none-any.whl", hash = "sha256:81f078a796a83e7263d79b81b0b80f30e1d8e321c98e1a335725eba2ced5f0fd", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "switchable-a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "shared-a" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/switchable_a-1.0.0.tar.gz", hash = "sha256:2b4b679289012946ed69b0eddc5128b0fa6ef849eb9ec94bb5222a24a64c58bd", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/switchable_a-1.0.0-py3-none-any.whl", hash = "sha256:915f35f479cd194fcbc870796ae6df875a127a94292401894f9ae7bbc62ea267", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "switchable-b"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "leaf" },
            { name = "shared-b", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        sdist = { url = "http://[LOCALHOST]/files/switchable_b-2.0.0.tar.gz", hash = "sha256:e2e1d41fb96e7525fe47f6b813a521596ee54feae5e9ccd5293e0b5111b1582d", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/switchable_b-2.0.0-py3-none-any.whl", hash = "sha256:aaf1cdf12dc9bf606b402c44793525370956f15f019dc8317b305e02ab022df1", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "trade-delay-one"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "trade-delay-two" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/trade_delay_one-1.0.0.tar.gz", hash = "sha256:c95da5071a848848d38031c63c8efdad0710439bd9e8d5373b8a16782a2131dd", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/trade_delay_one-1.0.0-py3-none-any.whl", hash = "sha256:579c354eddf30b043a1c5ecc0c4d21773724509943f73e05e48269bf38535508", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "trade-delay-three"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "shared-b", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        sdist = { url = "http://[LOCALHOST]/files/trade_delay_three-1.0.0.tar.gz", hash = "sha256:f6a22e010a04efee423c24756888fccf297284da184ba3ef55d28f37e9362c9b", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/trade_delay_three-1.0.0-py3-none-any.whl", hash = "sha256:aacffb55fa0ef514307b0a47da333c61697f06cae729d64e92f67b13ac827077", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "trade-delay-two"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "trade-delay-three" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/trade_delay_two-1.0.0.tar.gz", hash = "sha256:829ce9dd69a745736ce9c6297ca1d8983a524a5ad4ec63ccfeb32eb9f584e008", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/trade_delay_two-1.0.0-py3-none-any.whl", hash = "sha256:40be011b2776cae33e5405fe417c3d78082efa025d0d83e9f9b41e0f2bd5c176", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "trade-gate"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "leaf" },
            { name = "trade-delay-one" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/trade_gate-1.0.0.tar.gz", hash = "sha256:db5f1aedbea54a54984125162877bbf529b1ff67f1838f51db3b06ad84d54fdf", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/trade_gate-1.0.0-py3-none-any.whl", hash = "sha256:35f1c71b14f769e7de7a227bad74f4c9b89ffe96322019077206c44993036261", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// An earlier fork finishes with `switchable==2` and `shared==2`. A delayed sibling
/// initially selects `flaky==2`, which requires `shared==1` and `leaf==1`.
/// Coordination can therefore replace the earlier parent with `switchable==1`.
/// A separate dependency chain eventually requires `leaf==2`, forcing the live
/// sibling to backtrack to `flaky==1` and `shared==2`. The earlier fork must then
/// replace its `shared==1` agreement with `shared==2`, without retaining the
/// incompatibilities learned from the withdrawn agreement or the sibling's
/// retracted `shared==1` decision.
///
/// The final lock contains `shared==2` in both environments. A resolver trace is
/// also needed to distinguish agreement replacement from never coordinating.
///
///
/// ```text
/// coordinated-agreement-replacement
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires later-entry ; python_full_version >= '3.13'
/// │   │   └── satisfied by later-entry-1.0.0
/// │   └── requires switchable ; python_full_version < '3.13'
/// │       ├── satisfied by switchable-1.0.0
/// │       └── satisfied by switchable-2.0.0
/// ├── conflict-five
/// │   └── conflict-five-1.0.0
/// │       └── requires leaf==2.0.0
/// │           └── satisfied by leaf-2.0.0
/// ├── conflict-four
/// │   └── conflict-four-1.0.0
/// │       └── requires conflict-five
/// │           └── satisfied by conflict-five-1.0.0
/// ├── conflict-one
/// │   └── conflict-one-1.0.0
/// │       └── requires conflict-two
/// │           └── satisfied by conflict-two-1.0.0
/// ├── conflict-three
/// │   └── conflict-three-1.0.0
/// │       └── requires conflict-four
/// │           └── satisfied by conflict-four-1.0.0
/// ├── conflict-two
/// │   └── conflict-two-1.0.0
/// │       └── requires conflict-three
/// │           └── satisfied by conflict-three-1.0.0
/// ├── flaky
/// │   ├── flaky-1.0.0
/// │   │   ├── requires leaf==2.0.0
/// │   │   │   └── satisfied by leaf-2.0.0
/// │   │   └── requires shared==2.0.0
/// │   │       └── satisfied by shared-2.0.0
/// │   └── flaky-2.0.0
/// │       ├── requires leaf==1.0.0
/// │       │   └── satisfied by leaf-1.0.0
/// │       └── requires shared==1.0.0
/// │           └── satisfied by shared-1.0.0
/// ├── later-entry
/// │   └── later-entry-1.0.0
/// │       └── requires start-one
/// │           └── satisfied by start-one-1.0.0
/// ├── later-root
/// │   └── later-root-1.0.0
/// │       ├── requires conflict-one
/// │       │   └── satisfied by conflict-one-1.0.0
/// │       └── requires flaky>=1.0.0,<=2.0.0
/// │           ├── satisfied by flaky-1.0.0
/// │           └── satisfied by flaky-2.0.0
/// ├── leaf
/// │   ├── leaf-1.0.0
/// │   └── leaf-2.0.0
/// ├── shared
/// │   ├── shared-1.0.0
/// │   └── shared-2.0.0
/// ├── start-one
/// │   └── start-one-1.0.0
/// │       └── requires start-two
/// │           └── satisfied by start-two-1.0.0
/// ├── start-three
/// │   └── start-three-1.0.0
/// │       └── requires later-root
/// │           └── satisfied by later-root-1.0.0
/// ├── start-two
/// │   └── start-two-1.0.0
/// │       └── requires start-three
/// │           └── satisfied by start-three-1.0.0
/// └── switchable
///     ├── switchable-1.0.0
///     │   └── requires shared==1.0.0
///     │       └── satisfied by shared-1.0.0
///     └── switchable-2.0.0
///         └── requires shared==2.0.0
///             └── satisfied by shared-2.0.0
/// ```
#[test]
fn coordinated_agreement_replacement() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/coordinated/coordinated-agreement-replacement.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''switchable ; python_full_version < '3.13'''',
          '''later-entry ; python_full_version >= '3.13'''',
        ]
        requires-python = ">=3.12, <3.14"
        [tool.uv]
        fork-strategy = "fewest"
        environments = [
          '''python_full_version == '3.12.*'''',
          '''python_full_version == '3.13.*'''',
        ]
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 15 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12, <3.14"
        resolution-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]
        supported-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]

        [options]
        fork-strategy = "fewest"

        [[package]]
        name = "conflict-five"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "leaf" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/conflict_five-1.0.0.tar.gz", hash = "sha256:bb2a56c743437ed9b58ab0fe4158bfcfe4e28f18aab0a5142801e22477bb25be", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/conflict_five-1.0.0-py3-none-any.whl", hash = "sha256:2d8ee608a045ecb4892562012e227c492581959ff837a7413159eacf7b22d92b", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "conflict-four"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "conflict-five" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/conflict_four-1.0.0.tar.gz", hash = "sha256:0393b49b14a41fdfa51fdc5660ddc4e47f4e0e547219c7aab59add2ed692f119", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/conflict_four-1.0.0-py3-none-any.whl", hash = "sha256:69402caf2ab3b26c97bdcd75c7647dd1d558d529b68dfd2892c0816136c397e4", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "conflict-one"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "conflict-two" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/conflict_one-1.0.0.tar.gz", hash = "sha256:a82a37ca7389207222cde57673c887005a04be4d9df34326693736a8c55d07cc", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/conflict_one-1.0.0-py3-none-any.whl", hash = "sha256:200ec100c51a20cd445c337ac40d412a7f483b801803f585ca117810897d1b11", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "conflict-three"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "conflict-four" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/conflict_three-1.0.0.tar.gz", hash = "sha256:c7014f0e450ef8c2c609aee9558d992ae862726c6d66a8689c06a291ad148432", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/conflict_three-1.0.0-py3-none-any.whl", hash = "sha256:b299b9e96162f27cd34ed14b5fb20d1ab9d64d43907f16e6b7fba81537d70276", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "conflict-two"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "conflict-three" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/conflict_two-1.0.0.tar.gz", hash = "sha256:e0271489b445a0572c6cbab20d4c44e2b6a650ff1cd8d9f8861bc1388944845d", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/conflict_two-1.0.0-py3-none-any.whl", hash = "sha256:8acad3db9c99c801dceec54a161b64c0024e8dfa94abced8784127108ac720a4", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "flaky"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "leaf" },
            { name = "shared" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/flaky-1.0.0.tar.gz", hash = "sha256:4676b6abb2f92a49223a91c49944f44cd13b0b1c4dc7f7438d18ff70459190b4", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/flaky-1.0.0-py3-none-any.whl", hash = "sha256:66cdac3ab80dc00f2399847371a28f8d52bbc9082cd5cf6114986b8ca7a70a1e", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "later-entry"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "start-one" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/later_entry-1.0.0.tar.gz", hash = "sha256:0d76dedae8ca69f2eb43c6dad256cc4ab5b43f90fff683cd32d296a6ea8d6a18", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/later_entry-1.0.0-py3-none-any.whl", hash = "sha256:81af3d22ffa59437ee3a1468db173f77ee03ddbb53ed3e2f0f2cdcc65644a189", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "later-root"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "conflict-one" },
            { name = "flaky" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/later_root-1.0.0.tar.gz", hash = "sha256:377905791c707afb37b110e815d501fe67ad59c01eed467c8d1c4393f56c161a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/later_root-1.0.0-py3-none-any.whl", hash = "sha256:66900ac6ce68e770a96d9f14903481de2b1ff27dc2787ec5480c3abbde00bf40", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "leaf"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/leaf-2.0.0.tar.gz", hash = "sha256:bc5de13acb59a7f406cce765be6be6cbab785a7c9c9894dfc880580568cefc19", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/leaf-2.0.0-py3-none-any.whl", hash = "sha256:0eed3aff23491c8f604e933140e005dde855457c5a75508f39d416852a38d552", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "later-entry", marker = "python_full_version >= '3.13'" },
            { name = "switchable", marker = "python_full_version < '3.13'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "later-entry", marker = "python_full_version >= '3.13'" },
            { name = "switchable", marker = "python_full_version < '3.13'" },
        ]

        [[package]]
        name = "shared"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/shared-2.0.0.tar.gz", hash = "sha256:c09e0026550169997ff12b4c435b88e8ef9aed1fbd7d24265daea1f686f46059", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared-2.0.0-py3-none-any.whl", hash = "sha256:50654d921e335114898f26df3c573dfe4d21fae95abdb27584dbbc624137a5d0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "start-one"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "start-two" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/start_one-1.0.0.tar.gz", hash = "sha256:22d25d454e961e26fef41b983e49015cab8b4c146c3ce9f16d93c0c87575f127", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/start_one-1.0.0-py3-none-any.whl", hash = "sha256:5f447eabf4147fa28087ad0d108371ea9b8b3845be40d386011c39fea68b4a38", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "start-three"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "later-root" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/start_three-1.0.0.tar.gz", hash = "sha256:586769f0408dcf60773e7f9ad8f6a3ede2aa94802a4dbe3be6b7244d7168ed1c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/start_three-1.0.0-py3-none-any.whl", hash = "sha256:b49a7c02dc2a267ebf9864cc6b1d2f83d3fc7266cf3fb176787709cec3ae7536", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "start-two"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "start-three" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/start_two-1.0.0.tar.gz", hash = "sha256:3f6155f68efda504903194ae10121704405c2d5d1e03ea36c181d49f4f8b73ac", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/start_two-1.0.0-py3-none-any.whl", hash = "sha256:81f078a796a83e7263d79b81b0b80f30e1d8e321c98e1a335725eba2ced5f0fd", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "switchable"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "shared" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/switchable-2.0.0.tar.gz", hash = "sha256:2e2ea479ce728997d329e9b97a99baf12da7795b7589055ed7727e0013abfa23", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/switchable-2.0.0-py3-none-any.whl", hash = "sha256:9fcc98c965309c0a9dbb24ac7a6e1bc64b1ce31ea7d73f36bca2b0339f8105fa", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// `shared` is a direct requirement in the earlier fork and a transitive
/// requirement in the delayed sibling. The `lowest-direct` mode must select
/// `shared==1` for the direct requirement and `shared==2` for the transitive
/// requirement. The `fewest` fork strategy must not coordinate these different
/// resolution policies.
///
///
/// ```text
/// coordinated-mode-lowest-direct
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires later-entry>=1.0.0 ; python_full_version >= '3.13'
/// │   │   └── satisfied by later-entry-1.0.0
/// │   └── requires shared>=1.0.0,<=2.0.0 ; python_full_version < '3.13'
/// │       ├── satisfied by shared-1.0.0
/// │       └── satisfied by shared-2.0.0
/// ├── broad-parent
/// │   └── broad-parent-1.0.0
/// │       └── requires shared>=1.0.0,<=2.0.0
/// │           ├── satisfied by shared-1.0.0
/// │           └── satisfied by shared-2.0.0
/// ├── delay-one
/// │   └── delay-one-1.0.0
/// │       └── requires delay-two>=1.0.0
/// │           └── satisfied by delay-two-1.0.0
/// ├── delay-two
/// │   └── delay-two-1.0.0
/// │       └── requires broad-parent>=1.0.0
/// │           └── satisfied by broad-parent-1.0.0
/// ├── later-entry
/// │   └── later-entry-1.0.0
/// │       └── requires delay-one>=1.0.0
/// │           └── satisfied by delay-one-1.0.0
/// └── shared
///     ├── shared-1.0.0
///     └── shared-2.0.0
/// ```
#[test]
fn coordinated_mode_lowest_direct() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/coordinated/coordinated-mode-lowest-direct.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''shared>=1.0.0,<=2.0.0 ; python_full_version < '3.13'''',
          '''later-entry>=1.0.0 ; python_full_version >= '3.13'''',
        ]
        requires-python = ">=3.12, <3.14"
        [tool.uv]
        resolution = "lowest-direct"
        fork-strategy = "fewest"
        environments = [
          '''python_full_version == '3.12.*'''',
          '''python_full_version == '3.13.*'''',
        ]
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    // The direct fork keeps `shared==1`, while the transitive fork selects `shared==2`.
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12, <3.14"
        resolution-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]
        supported-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]

        [options]
        resolution-mode = "lowest-direct"
        fork-strategy = "fewest"

        [[package]]
        name = "broad-parent"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "shared", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        sdist = { url = "http://[LOCALHOST]/files/broad_parent-1.0.0.tar.gz", hash = "sha256:e7eeb667f6b83b5afe2855b494e99a8795e2b38051011a0ada68871f6513072d", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/broad_parent-1.0.0-py3-none-any.whl", hash = "sha256:0f74a08085bb43f963113ab17e9c1e06078c42b78366687858710618f5578ce3", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-one"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-two" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/delay_one-1.0.0.tar.gz", hash = "sha256:403ead681ca0a78d58f3996de50d22766c4e4098b5e95103746fe562ebefed04", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_one-1.0.0-py3-none-any.whl", hash = "sha256:36700dfdae3859f49695f43ff5b559ce038dda022265b1c102ff848172d34f1d", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-two"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "broad-parent" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/delay_two-1.0.0.tar.gz", hash = "sha256:be5332d53d9ce6169c1c53b4523ceb8f211f654c0d8e7460e121a56ff9ddafb7", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_two-1.0.0-py3-none-any.whl", hash = "sha256:826a03f79a94b62a3741368175a0a747de8f71172f52eb695e3cd2241c16036f", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "later-entry"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-one" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/later_entry-1.0.0.tar.gz", hash = "sha256:6e257987a0e7dd1e168349ee03fa5af2c05e30bf7ae65c5aa0e301948d7b280c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/later_entry-1.0.0-py3-none-any.whl", hash = "sha256:1e09290e5b717943e91f8dd21ba9cad5af6ac4fda7d5c2039875df5764d884ae", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "later-entry", marker = "python_full_version >= '3.13'" },
            { name = "shared", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "python_full_version < '3.13'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "later-entry", marker = "python_full_version >= '3.13'", specifier = ">=1.0.0" },
            { name = "shared", marker = "python_full_version < '3.13'", specifier = ">=1.0.0,<=2.0.0" },
        ]

        [[package]]
        name = "shared"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "python_full_version < '3.13'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/shared-1.0.0.tar.gz", hash = "sha256:29242e0032cd4abe8ec185a5f0c198167191709b20778c7ff188dbd0037923f5", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared-1.0.0-py3-none-any.whl", hash = "sha256:9681756b32d3ae501a9e7129df974fe86476066cf3fcec82c82594bb91ecca07", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "shared"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "python_full_version >= '3.13'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/shared-2.0.0.tar.gz", hash = "sha256:c09e0026550169997ff12b4c435b88e8ef9aed1fbd7d24265daea1f686f46059", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared-2.0.0-py3-none-any.whl", hash = "sha256:50654d921e335114898f26df3c573dfe4d21fae95abdb27584dbbc624137a5d0", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// The `lowest` mode finishes the earlier fork with `switchable==1` and `shared==1`.
/// A delayed sibling then requires `shared==2`. The completed fork must backtrack
/// the parent to `switchable==2` so both environments can use `shared==2`, even
/// though that moves the earlier decisions upward in version order.
///
///
/// ```text
/// coordinated-mode-lowest-inverse
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires later-entry>=1.0.0 ; python_full_version >= '3.13'
/// │   │   └── satisfied by later-entry-1.0.0
/// │   └── requires switchable>=1.0.0 ; python_full_version < '3.13'
/// │       ├── satisfied by switchable-1.0.0
/// │       └── satisfied by switchable-2.0.0
/// ├── constrained
/// │   └── constrained-1.0.0
/// │       └── requires shared==2.0.0
/// │           └── satisfied by shared-2.0.0
/// ├── delay-one
/// │   └── delay-one-1.0.0
/// │       └── requires delay-two>=1.0.0
/// │           └── satisfied by delay-two-1.0.0
/// ├── delay-two
/// │   └── delay-two-1.0.0
/// │       └── requires constrained>=1.0.0
/// │           └── satisfied by constrained-1.0.0
/// ├── later-entry
/// │   └── later-entry-1.0.0
/// │       └── requires delay-one>=1.0.0
/// │           └── satisfied by delay-one-1.0.0
/// ├── shared
/// │   ├── shared-1.0.0
/// │   └── shared-2.0.0
/// └── switchable
///     ├── switchable-1.0.0
///     │   └── requires shared==1.0.0
///     │       └── satisfied by shared-1.0.0
///     └── switchable-2.0.0
///         └── requires shared==2.0.0
///             └── satisfied by shared-2.0.0
/// ```
#[test]
fn coordinated_mode_lowest_inverse() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/coordinated/coordinated-mode-lowest-inverse.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''switchable>=1.0.0 ; python_full_version < '3.13'''',
          '''later-entry>=1.0.0 ; python_full_version >= '3.13'''',
        ]
        requires-python = ">=3.12, <3.14"
        [tool.uv]
        resolution = "lowest"
        fork-strategy = "fewest"
        environments = [
          '''python_full_version == '3.12.*'''',
          '''python_full_version == '3.13.*'''',
        ]
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    // Both forks use `shared==2`, and the earlier fork selects `switchable==2`.
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12, <3.14"
        resolution-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]
        supported-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]

        [options]
        resolution-mode = "lowest"
        fork-strategy = "fewest"

        [[package]]
        name = "constrained"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "shared" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/constrained-1.0.0.tar.gz", hash = "sha256:05b2f346efcd08146ff8540a5ac30f8063e26bd44fc442307b58cad8281e6cab", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/constrained-1.0.0-py3-none-any.whl", hash = "sha256:f04a64c8f5a1b3eb168f28480f029bf1478d5f69b4c962d6f4f5ef0980464186", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-one"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-two" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/delay_one-1.0.0.tar.gz", hash = "sha256:403ead681ca0a78d58f3996de50d22766c4e4098b5e95103746fe562ebefed04", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_one-1.0.0-py3-none-any.whl", hash = "sha256:36700dfdae3859f49695f43ff5b559ce038dda022265b1c102ff848172d34f1d", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-two"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "constrained" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/delay_two-1.0.0.tar.gz", hash = "sha256:024bd7b5e6d14f285a06cce9dcd8ea410e2425abd320d91bda9558672b441662", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_two-1.0.0-py3-none-any.whl", hash = "sha256:1e561c24038081847af22722b2d55b482e81311782db56be0e75b13ab4a6967b", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "later-entry"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-one" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/later_entry-1.0.0.tar.gz", hash = "sha256:6e257987a0e7dd1e168349ee03fa5af2c05e30bf7ae65c5aa0e301948d7b280c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/later_entry-1.0.0-py3-none-any.whl", hash = "sha256:1e09290e5b717943e91f8dd21ba9cad5af6ac4fda7d5c2039875df5764d884ae", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "later-entry", marker = "python_full_version >= '3.13'" },
            { name = "switchable", marker = "python_full_version < '3.13'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "later-entry", marker = "python_full_version >= '3.13'", specifier = ">=1.0.0" },
            { name = "switchable", marker = "python_full_version < '3.13'", specifier = ">=1.0.0" },
        ]

        [[package]]
        name = "shared"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/shared-2.0.0.tar.gz", hash = "sha256:c09e0026550169997ff12b4c435b88e8ef9aed1fbd7d24265daea1f686f46059", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared-2.0.0-py3-none-any.whl", hash = "sha256:50654d921e335114898f26df3c573dfe4d21fae95abdb27584dbbc624137a5d0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "switchable"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "shared" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/switchable-2.0.0.tar.gz", hash = "sha256:2e2ea479ce728997d329e9b97a99baf12da7795b7589055ed7727e0013abfa23", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/switchable-2.0.0-py3-none-any.whl", hash = "sha256:9fcc98c965309c0a9dbb24ac7a6e1bc64b1ce31ea7d73f36bca2b0339f8105fa", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// A completed Python fork has a valid newer `parent`. Reusing a delayed sibling's
/// `shared` and `aux` versions selects an older `parent` and creates two platform
/// forks. The Windows child can resolve, but the other conflicts with the root's
/// `blocker==2` requirement. The incomplete replacement must not remove any of the
/// original Python region from the lock.
///
///
/// ```text
/// coordinated-nested-platform-unsatisfiable
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires blocker==2.0.0 ; python_full_version < '3.13'
/// │   │   └── satisfied by blocker-2.0.0
/// │   ├── requires delayed ; python_full_version >= '3.13'
/// │   │   └── satisfied by delayed-1.0.0
/// │   └── requires parent ; python_full_version < '3.13'
/// │       ├── satisfied by parent-1.0.0
/// │       └── satisfied by parent-2.0.0
/// ├── aux
/// │   ├── aux-1.0.0
/// │   └── aux-2.0.0
/// ├── blocker
/// │   ├── blocker-1.0.0
/// │   └── blocker-2.0.0
/// ├── branch
/// │   ├── branch-1.0.0
/// │   └── branch-2.0.0
/// │       └── requires blocker==1.0.0
/// │           └── satisfied by blocker-1.0.0
/// ├── constrained
/// │   └── constrained-1.0.0
/// │       ├── requires aux==1.0.0
/// │       │   └── satisfied by aux-1.0.0
/// │       └── requires shared==1.0.0
/// │           └── satisfied by shared-1.0.0
/// ├── delay-eight
/// │   └── delay-eight-1.0.0
/// │       └── requires constrained
/// │           └── satisfied by constrained-1.0.0
/// ├── delay-five
/// │   └── delay-five-1.0.0
/// │       └── requires delay-six
/// │           └── satisfied by delay-six-1.0.0
/// ├── delay-four
/// │   └── delay-four-1.0.0
/// │       └── requires delay-five
/// │           └── satisfied by delay-five-1.0.0
/// ├── delay-one
/// │   └── delay-one-1.0.0
/// │       └── requires delay-two
/// │           └── satisfied by delay-two-1.0.0
/// ├── delay-seven
/// │   └── delay-seven-1.0.0
/// │       └── requires delay-eight
/// │           └── satisfied by delay-eight-1.0.0
/// ├── delay-six
/// │   └── delay-six-1.0.0
/// │       └── requires delay-seven
/// │           └── satisfied by delay-seven-1.0.0
/// ├── delay-three
/// │   └── delay-three-1.0.0
/// │       └── requires delay-four
/// │           └── satisfied by delay-four-1.0.0
/// ├── delay-two
/// │   └── delay-two-1.0.0
/// │       └── requires delay-three
/// │           └── satisfied by delay-three-1.0.0
/// ├── delayed
/// │   └── delayed-1.0.0
/// │       └── requires delay-one
/// │           └── satisfied by delay-one-1.0.0
/// ├── parent
/// │   ├── parent-1.0.0
/// │   │   ├── requires aux==1.0.0
/// │   │   │   └── satisfied by aux-1.0.0
/// │   │   ├── requires branch==1.0.0 ; sys_platform == 'win32'
/// │   │   │   └── satisfied by branch-1.0.0
/// │   │   ├── requires branch==2.0.0 ; sys_platform != 'win32'
/// │   │   │   └── satisfied by branch-2.0.0
/// │   │   └── requires shared==1.0.0
/// │   │       └── satisfied by shared-1.0.0
/// │   └── parent-2.0.0
/// │       ├── requires aux==2.0.0
/// │       │   └── satisfied by aux-2.0.0
/// │       └── requires shared==2.0.0
/// │           └── satisfied by shared-2.0.0
/// └── shared
///     ├── shared-1.0.0
///     └── shared-2.0.0
/// ```
#[test]
fn coordinated_nested_platform_unsatisfiable() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server =
        PackseServer::new("fork/coordinated/coordinated-nested-platform-unsatisfiable.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''parent ; python_full_version < '3.13'''',
          '''blocker==2.0.0 ; python_full_version < '3.13'''',
          '''delayed ; python_full_version >= '3.13'''',
        ]
        requires-python = ">=3.12, <3.14"
        [tool.uv]
        fork-strategy = "fewest"
        environments = [
          '''python_full_version < '3.13'''',
          '''python_full_version >= '3.13'''',
        ]
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    // `parent==2`, `shared==2`, `aux==2`, and `blocker==2` cover every platform below
    // Python 3.13. The delayed sibling uses `shared==1` and `aux==1` at Python 3.13
    // and later. Neither `branch` version belongs in the final lock because one
    // replacement child fails.
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 17 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12, <3.14"
        resolution-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]
        supported-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]

        [options]
        fork-strategy = "fewest"

        [[package]]
        name = "aux"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "python_full_version >= '3.13'",
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/aux-1.0.0-py3-none-any.whl", hash = "sha256:76e9112337ba8f38a89501b8abdcf23ae73f487f366f7fe7c5e74a6b248f2fa4", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "aux"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "python_full_version < '3.13'",
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/aux-2.0.0-py3-none-any.whl", hash = "sha256:4eab674a6374bc42eb45ebe034518eef9d82efc986454b276d8ce42bc3247a1c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "blocker"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        wheels = [
            { url = "http://[LOCALHOST]/files/blocker-2.0.0-py3-none-any.whl", hash = "sha256:7bd797901a191398430389e811ad4b7a7f82d3fdd63b9137107ae32a3553c7df", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "constrained"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "aux", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
            { name = "shared", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/constrained-1.0.0-py3-none-any.whl", hash = "sha256:706bf893fd899f3c3acb07fd59b054b696d265171aea40613d75c10b4ad00011", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-eight"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "constrained" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_eight-1.0.0-py3-none-any.whl", hash = "sha256:f89061af255396de298e836c047c569d32f8fde1c65ac24c13b59b97decf8c10", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-five"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-six" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_five-1.0.0-py3-none-any.whl", hash = "sha256:67fc06e45f34f82ad35ac8d3c235ef309870276103673a01fb82ddb6af4ccf04", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-four"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-five" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_four-1.0.0-py3-none-any.whl", hash = "sha256:f67d6b3860d2c4b96b4be7fe5c1dc5e237e192f2cdeb8468f68aaf2d67339001", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-one"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-two" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_one-1.0.0-py3-none-any.whl", hash = "sha256:c1e8aca4b1866ffedbf054bd89c0d7d4fbc8647de899909cff7dadf9ff9d59ef", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-seven"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-eight" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_seven-1.0.0-py3-none-any.whl", hash = "sha256:76582914299bdc5b08fadd5acbadeba3049ca9e2fadcc2b0658bf3b72a13bed3", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-six"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-seven" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_six-1.0.0-py3-none-any.whl", hash = "sha256:ac6db54d2dfa64b7d55b1ab5e2232b2bc2235f9bac2cbd68147566d61253c17c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-three"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-four" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_three-1.0.0-py3-none-any.whl", hash = "sha256:26f1c369f51d6166a6aa01528946f16d91dd0e7ff260c6b9695a1c7dec262a6b", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-two"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-three" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_two-1.0.0-py3-none-any.whl", hash = "sha256:e9559538559209b102813ee3eb862d70a1853ff9913312d501ca6cc7dddf796d", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delayed"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-one" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delayed-1.0.0-py3-none-any.whl", hash = "sha256:cc3bd7eea5a6d2263f0ee72930379019167956dfc6b5941b33f133964aeb696c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "parent"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "aux", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
            { name = "shared", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/parent-2.0.0-py3-none-any.whl", hash = "sha256:65cacc6faed073f9ea0d2f0bb9c764f95ca69c45393dd0ec2deed81fa47e3a8b", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "blocker", marker = "python_full_version < '3.13'" },
            { name = "delayed", marker = "python_full_version >= '3.13'" },
            { name = "parent", marker = "python_full_version < '3.13'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "blocker", marker = "python_full_version < '3.13'", specifier = "==2.0.0" },
            { name = "delayed", marker = "python_full_version >= '3.13'" },
            { name = "parent", marker = "python_full_version < '3.13'" },
        ]

        [[package]]
        name = "shared"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "python_full_version >= '3.13'",
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/shared-1.0.0-py3-none-any.whl", hash = "sha256:9681756b32d3ae501a9e7129df974fe86476066cf3fcec82c82594bb91ecca07", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "shared"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "python_full_version < '3.13'",
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/shared-2.0.0-py3-none-any.whl", hash = "sha256:50654d921e335114898f26df3c573dfe4d21fae95abdb27584dbbc624137a5d0", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// A completed Python fork must change its `parent` version to reuse the `shared`
/// and `aux` versions required by a delayed sibling. The replacement introduces
/// two platform-specific versions of `branch`. Both new platform forks must resolve
/// before their combined result can replace the completed Python region.
///
///
/// ```text
/// coordinated-nested-platform
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires delayed ; python_full_version >= '3.13'
/// │   │   └── satisfied by delayed-1.0.0
/// │   └── requires parent ; python_full_version < '3.13'
/// │       ├── satisfied by parent-1.0.0
/// │       └── satisfied by parent-2.0.0
/// ├── aux
/// │   ├── aux-1.0.0
/// │   └── aux-2.0.0
/// ├── branch
/// │   ├── branch-1.0.0
/// │   └── branch-2.0.0
/// ├── constrained
/// │   └── constrained-1.0.0
/// │       ├── requires aux==1.0.0
/// │       │   └── satisfied by aux-1.0.0
/// │       └── requires shared==1.0.0
/// │           └── satisfied by shared-1.0.0
/// ├── delay-eight
/// │   └── delay-eight-1.0.0
/// │       └── requires constrained
/// │           └── satisfied by constrained-1.0.0
/// ├── delay-five
/// │   └── delay-five-1.0.0
/// │       └── requires delay-six
/// │           └── satisfied by delay-six-1.0.0
/// ├── delay-four
/// │   └── delay-four-1.0.0
/// │       └── requires delay-five
/// │           └── satisfied by delay-five-1.0.0
/// ├── delay-one
/// │   └── delay-one-1.0.0
/// │       └── requires delay-two
/// │           └── satisfied by delay-two-1.0.0
/// ├── delay-seven
/// │   └── delay-seven-1.0.0
/// │       └── requires delay-eight
/// │           └── satisfied by delay-eight-1.0.0
/// ├── delay-six
/// │   └── delay-six-1.0.0
/// │       └── requires delay-seven
/// │           └── satisfied by delay-seven-1.0.0
/// ├── delay-three
/// │   └── delay-three-1.0.0
/// │       └── requires delay-four
/// │           └── satisfied by delay-four-1.0.0
/// ├── delay-two
/// │   └── delay-two-1.0.0
/// │       └── requires delay-three
/// │           └── satisfied by delay-three-1.0.0
/// ├── delayed
/// │   └── delayed-1.0.0
/// │       └── requires delay-one
/// │           └── satisfied by delay-one-1.0.0
/// ├── parent
/// │   ├── parent-1.0.0
/// │   │   ├── requires aux==1.0.0
/// │   │   │   └── satisfied by aux-1.0.0
/// │   │   ├── requires branch==1.0.0 ; sys_platform == 'win32'
/// │   │   │   └── satisfied by branch-1.0.0
/// │   │   ├── requires branch==2.0.0 ; sys_platform != 'win32'
/// │   │   │   └── satisfied by branch-2.0.0
/// │   │   └── requires shared==1.0.0
/// │   │       └── satisfied by shared-1.0.0
/// │   └── parent-2.0.0
/// │       ├── requires aux==2.0.0
/// │       │   └── satisfied by aux-2.0.0
/// │       └── requires shared==2.0.0
/// │           └── satisfied by shared-2.0.0
/// └── shared
///     ├── shared-1.0.0
///     └── shared-2.0.0
/// ```
#[test]
fn coordinated_nested_platform() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/coordinated/coordinated-nested-platform.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''parent ; python_full_version < '3.13'''',
          '''delayed ; python_full_version >= '3.13'''',
        ]
        requires-python = ">=3.12, <3.14"
        [tool.uv]
        fork-strategy = "fewest"
        environments = [
          '''python_full_version < '3.13'''',
          '''python_full_version >= '3.13'''',
        ]
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    // The final lock has `parent==1`, `shared==1`, and `aux==1`. `branch==1` is needed
    // on Windows below Python 3.13, and `branch==2` is needed on other platforms below
    // Python 3.13. Removing two duplicate versions outweighs the new `branch` duplicate.
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 16 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12, <3.14"
        resolution-markers = [
            "python_full_version < '3.13' and sys_platform == 'win32'",
            "python_full_version < '3.13' and sys_platform != 'win32'",
            "python_full_version >= '3.13'",
        ]
        supported-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]

        [options]
        fork-strategy = "fewest"

        [[package]]
        name = "aux"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        wheels = [
            { url = "http://[LOCALHOST]/files/aux-1.0.0-py3-none-any.whl", hash = "sha256:76e9112337ba8f38a89501b8abdcf23ae73f487f366f7fe7c5e74a6b248f2fa4", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "branch"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "python_full_version < '3.13' and sys_platform == 'win32'",
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/branch-1.0.0-py3-none-any.whl", hash = "sha256:5eebeb6b9f6199175ec2467ebbce39fd236b7df54ab9d66166e7096858513220", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "branch"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "python_full_version < '3.13' and sys_platform != 'win32'",
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/branch-2.0.0-py3-none-any.whl", hash = "sha256:6d0e8da4a4868adb7804032f92e8f68d7521eef2ad9888e287a1db2bdfd9f92e", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "constrained"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "aux" },
            { name = "shared" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/constrained-1.0.0-py3-none-any.whl", hash = "sha256:706bf893fd899f3c3acb07fd59b054b696d265171aea40613d75c10b4ad00011", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-eight"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "constrained" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_eight-1.0.0-py3-none-any.whl", hash = "sha256:f89061af255396de298e836c047c569d32f8fde1c65ac24c13b59b97decf8c10", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-five"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-six" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_five-1.0.0-py3-none-any.whl", hash = "sha256:67fc06e45f34f82ad35ac8d3c235ef309870276103673a01fb82ddb6af4ccf04", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-four"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-five" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_four-1.0.0-py3-none-any.whl", hash = "sha256:f67d6b3860d2c4b96b4be7fe5c1dc5e237e192f2cdeb8468f68aaf2d67339001", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-one"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-two" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_one-1.0.0-py3-none-any.whl", hash = "sha256:c1e8aca4b1866ffedbf054bd89c0d7d4fbc8647de899909cff7dadf9ff9d59ef", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-seven"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-eight" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_seven-1.0.0-py3-none-any.whl", hash = "sha256:76582914299bdc5b08fadd5acbadeba3049ca9e2fadcc2b0658bf3b72a13bed3", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-six"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-seven" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_six-1.0.0-py3-none-any.whl", hash = "sha256:ac6db54d2dfa64b7d55b1ab5e2232b2bc2235f9bac2cbd68147566d61253c17c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-three"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-four" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_three-1.0.0-py3-none-any.whl", hash = "sha256:26f1c369f51d6166a6aa01528946f16d91dd0e7ff260c6b9695a1c7dec262a6b", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-two"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-three" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_two-1.0.0-py3-none-any.whl", hash = "sha256:e9559538559209b102813ee3eb862d70a1853ff9913312d501ca6cc7dddf796d", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delayed"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-one" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delayed-1.0.0-py3-none-any.whl", hash = "sha256:cc3bd7eea5a6d2263f0ee72930379019167956dfc6b5941b33f133964aeb696c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "parent"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "aux" },
            { name = "branch", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'win32'" },
            { name = "branch", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform != 'win32'" },
            { name = "shared" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/parent-1.0.0-py3-none-any.whl", hash = "sha256:7cc1603e0ae3f839a336e53fd35185ed9d91de960bf968fc745da016f91637c8", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "delayed", marker = "python_full_version >= '3.13'" },
            { name = "parent", marker = "python_full_version < '3.13'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "delayed", marker = "python_full_version >= '3.13'" },
            { name = "parent", marker = "python_full_version < '3.13'" },
        ]

        [[package]]
        name = "shared"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared-1.0.0-py3-none-any.whl", hash = "sha256:9681756b32d3ae501a9e7129df974fe86476066cf3fcec82c82594bb91ecca07", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// `parent[feature]` belongs to a completed platform fork. Reusing the delayed
/// sibling's `shared` and `aux` versions requires backtracking both `parent` and
/// its `feature` extra to an older release. That extra introduces disjoint Python
/// requirements on `branch`, so the replacement must retain the extra's dependency
/// edges across new Python forks while narrowing each child's supported Python range.
///
///
/// ```text
/// coordinated-nested-python-extra
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires delayed ; sys_platform != 'win32'
/// │   │   └── satisfied by delayed-1.0.0
/// │   └── requires parent[feature] ; sys_platform == 'win32'
/// │       ├── satisfied by parent-1.0.0
/// │       ├── satisfied by parent-1.0.0[feature]
/// │       ├── satisfied by parent-2.0.0
/// │       └── satisfied by parent-2.0.0[feature]
/// ├── aux
/// │   ├── aux-1.0.0
/// │   └── aux-2.0.0
/// ├── branch
/// │   ├── branch-1.0.0
/// │   └── branch-2.0.0
/// │       └── requires python>=3.13 (incompatible with environment)
/// ├── constrained
/// │   └── constrained-1.0.0
/// │       ├── requires aux==1.0.0
/// │       │   └── satisfied by aux-1.0.0
/// │       └── requires shared==1.0.0
/// │           └── satisfied by shared-1.0.0
/// ├── delay-eight
/// │   └── delay-eight-1.0.0
/// │       └── requires constrained
/// │           └── satisfied by constrained-1.0.0
/// ├── delay-five
/// │   └── delay-five-1.0.0
/// │       └── requires delay-six
/// │           └── satisfied by delay-six-1.0.0
/// ├── delay-four
/// │   └── delay-four-1.0.0
/// │       └── requires delay-five
/// │           └── satisfied by delay-five-1.0.0
/// ├── delay-one
/// │   └── delay-one-1.0.0
/// │       └── requires delay-two
/// │           └── satisfied by delay-two-1.0.0
/// ├── delay-seven
/// │   └── delay-seven-1.0.0
/// │       └── requires delay-eight
/// │           └── satisfied by delay-eight-1.0.0
/// ├── delay-six
/// │   └── delay-six-1.0.0
/// │       └── requires delay-seven
/// │           └── satisfied by delay-seven-1.0.0
/// ├── delay-three
/// │   └── delay-three-1.0.0
/// │       └── requires delay-four
/// │           └── satisfied by delay-four-1.0.0
/// ├── delay-two
/// │   └── delay-two-1.0.0
/// │       └── requires delay-three
/// │           └── satisfied by delay-three-1.0.0
/// ├── delayed
/// │   └── delayed-1.0.0
/// │       └── requires delay-one
/// │           └── satisfied by delay-one-1.0.0
/// ├── parent
/// │   ├── parent-1.0.0
/// │   │   ├── requires aux==1.0.0
/// │   │   │   └── satisfied by aux-1.0.0
/// │   │   └── requires shared==1.0.0
/// │   │       └── satisfied by shared-1.0.0
/// │   ├── parent-1.0.0[feature]
/// │   │   ├── requires branch==1.0.0 ; python_full_version < '3.13'
/// │   │   │   └── satisfied by branch-1.0.0
/// │   │   └── requires branch==2.0.0 ; python_full_version >= '3.13'
/// │   │       └── satisfied by branch-2.0.0
/// │   ├── parent-2.0.0
/// │   │   ├── requires aux==2.0.0
/// │   │   │   └── satisfied by aux-2.0.0
/// │   │   └── requires shared==2.0.0
/// │   │       └── satisfied by shared-2.0.0
/// │   └── parent-2.0.0[feature]
/// └── shared
///     ├── shared-1.0.0
///     └── shared-2.0.0
/// ```
#[test]
fn coordinated_nested_python_extra() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/coordinated/coordinated-nested-python-extra.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''parent[feature] ; sys_platform == 'win32'''',
          '''delayed ; sys_platform != 'win32'''',
        ]
        requires-python = ">=3.12, <3.14"
        [tool.uv]
        fork-strategy = "fewest"
        environments = [
          '''sys_platform == 'win32'''',
          '''sys_platform != 'win32'''',
        ]
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    // `parent[feature]==1` is selected only on Windows. `shared==1` and `aux==1` are
    // used on every platform. On Windows, `branch==1` covers Python below 3.13 and
    // `branch==2` covers Python 3.13 and later. The older `parent` introduces one
    // duplicate while removing the two duplicates of `shared` and `aux`.
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 16 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12, <3.14"
        resolution-markers = [
            "python_full_version < '3.13' and sys_platform == 'win32'",
            "python_full_version >= '3.13' and sys_platform == 'win32'",
            "sys_platform != 'win32'",
        ]
        supported-markers = [
            "sys_platform == 'win32'",
            "sys_platform != 'win32'",
        ]

        [options]
        fork-strategy = "fewest"

        [[package]]
        name = "aux"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        wheels = [
            { url = "http://[LOCALHOST]/files/aux-1.0.0-py3-none-any.whl", hash = "sha256:76e9112337ba8f38a89501b8abdcf23ae73f487f366f7fe7c5e74a6b248f2fa4", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "branch"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "python_full_version < '3.13' and sys_platform == 'win32'",
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/branch-1.0.0-py3-none-any.whl", hash = "sha256:5eebeb6b9f6199175ec2467ebbce39fd236b7df54ab9d66166e7096858513220", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "branch"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "python_full_version >= '3.13' and sys_platform == 'win32'",
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/branch-2.0.0-py3-none-any.whl", hash = "sha256:e7c116b7f14ec99e3d423c485eb8e5f4336853fb8f4907114a3bdb61ef7de935", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "constrained"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "aux" },
            { name = "shared" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/constrained-1.0.0-py3-none-any.whl", hash = "sha256:706bf893fd899f3c3acb07fd59b054b696d265171aea40613d75c10b4ad00011", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-eight"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "constrained" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_eight-1.0.0-py3-none-any.whl", hash = "sha256:f89061af255396de298e836c047c569d32f8fde1c65ac24c13b59b97decf8c10", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-five"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-six" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_five-1.0.0-py3-none-any.whl", hash = "sha256:67fc06e45f34f82ad35ac8d3c235ef309870276103673a01fb82ddb6af4ccf04", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-four"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-five" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_four-1.0.0-py3-none-any.whl", hash = "sha256:f67d6b3860d2c4b96b4be7fe5c1dc5e237e192f2cdeb8468f68aaf2d67339001", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-one"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-two" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_one-1.0.0-py3-none-any.whl", hash = "sha256:c1e8aca4b1866ffedbf054bd89c0d7d4fbc8647de899909cff7dadf9ff9d59ef", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-seven"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-eight" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_seven-1.0.0-py3-none-any.whl", hash = "sha256:76582914299bdc5b08fadd5acbadeba3049ca9e2fadcc2b0658bf3b72a13bed3", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-six"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-seven" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_six-1.0.0-py3-none-any.whl", hash = "sha256:ac6db54d2dfa64b7d55b1ab5e2232b2bc2235f9bac2cbd68147566d61253c17c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-three"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-four" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_three-1.0.0-py3-none-any.whl", hash = "sha256:26f1c369f51d6166a6aa01528946f16d91dd0e7ff260c6b9695a1c7dec262a6b", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-two"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-three" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_two-1.0.0-py3-none-any.whl", hash = "sha256:e9559538559209b102813ee3eb862d70a1853ff9913312d501ca6cc7dddf796d", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delayed"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-one" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/delayed-1.0.0-py3-none-any.whl", hash = "sha256:cc3bd7eea5a6d2263f0ee72930379019167956dfc6b5941b33f133964aeb696c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "parent"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "aux" },
            { name = "shared" },
        ]
        wheels = [
            { url = "http://[LOCALHOST]/files/parent-1.0.0-py3-none-any.whl", hash = "sha256:0e55165f293da760058a286f58b5485a1a21b0eac3df9825e59c3bab7bab9cd7", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [package.optional-dependencies]
        feature = [
            { name = "branch", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "python_full_version < '3.13'" },
            { name = "branch", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "python_full_version >= '3.13'" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "delayed", marker = "sys_platform != 'win32'" },
            { name = "parent", extra = ["feature"], marker = "sys_platform == 'win32'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "delayed", marker = "sys_platform != 'win32'" },
            { name = "parent", extras = ["feature"], marker = "sys_platform == 'win32'" },
        ]

        [[package]]
        name = "shared"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared-1.0.0-py3-none-any.whl", hash = "sha256:9681756b32d3ae501a9e7129df974fe86476066cf3fcec82c82594bb91ecca07", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// An earlier fork initially selects `switchable==2`, whose dependencies include
/// `shared==2` and an otherwise-unused `orphan`. A delayed sibling requires
/// `shared==1`. Backtracking the earlier parent to `switchable==1` removes `shared`
/// and `orphan` from that environment entirely, replacing them with `survivor`.
/// The coordinated version restriction is conditional on a package remaining
/// necessary; it must not introduce a synthetic root dependency on `shared==1`
/// or retain abandoned dependencies in the lockfile.
///
///
/// ```text
/// coordinated-package-disappears
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires later-entry ; python_full_version >= '3.13'
/// │   │   └── satisfied by later-entry-1.0.0
/// │   └── requires switchable ; python_full_version < '3.13'
/// │       ├── satisfied by switchable-1.0.0
/// │       └── satisfied by switchable-2.0.0
/// ├── delay-four
/// │   └── delay-four-1.0.0
/// │       └── requires shared==1.0.0
/// │           └── satisfied by shared-1.0.0
/// ├── delay-one
/// │   └── delay-one-1.0.0
/// │       └── requires delay-two
/// │           └── satisfied by delay-two-1.0.0
/// ├── delay-three
/// │   └── delay-three-1.0.0
/// │       └── requires delay-four
/// │           └── satisfied by delay-four-1.0.0
/// ├── delay-two
/// │   └── delay-two-1.0.0
/// │       └── requires delay-three
/// │           └── satisfied by delay-three-1.0.0
/// ├── later-entry
/// │   └── later-entry-1.0.0
/// │       └── requires delay-one
/// │           └── satisfied by delay-one-1.0.0
/// ├── orphan
/// │   └── orphan-1.0.0
/// ├── shared
/// │   ├── shared-1.0.0
/// │   └── shared-2.0.0
/// ├── survivor
/// │   └── survivor-1.0.0
/// └── switchable
///     ├── switchable-1.0.0
///     │   └── requires survivor==1.0.0
///     │       └── satisfied by survivor-1.0.0
///     └── switchable-2.0.0
///         ├── requires orphan==1.0.0
///         │   └── satisfied by orphan-1.0.0
///         └── requires shared==2.0.0
///             └── satisfied by shared-2.0.0
/// ```
#[test]
fn coordinated_package_disappears() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/coordinated/coordinated-package-disappears.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''switchable ; python_full_version < '3.13'''',
          '''later-entry ; python_full_version >= '3.13'''',
        ]
        requires-python = ">=3.12, <3.14"
        [tool.uv]
        fork-strategy = "fewest"
        environments = [
          '''python_full_version == '3.12.*'''',
          '''python_full_version == '3.13.*'''',
        ]
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 9 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12, <3.14"
        resolution-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]
        supported-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]

        [options]
        fork-strategy = "fewest"

        [[package]]
        name = "delay-four"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "shared" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/delay_four-1.0.0.tar.gz", hash = "sha256:e153efe023397a0b7b95196c883e38e1efa687a0545a74c19f49d71ce41d4684", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_four-1.0.0-py3-none-any.whl", hash = "sha256:0296b7d032be3dda11854567c4ef3b8f770a5e50ffae4de76fd97d9e9d5ddab0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-one"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-two" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/delay_one-1.0.0.tar.gz", hash = "sha256:24641f1b68d386ea2a2ef23ebc54305cb347d9a2745618057697c3ff4dbc5648", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_one-1.0.0-py3-none-any.whl", hash = "sha256:c1e8aca4b1866ffedbf054bd89c0d7d4fbc8647de899909cff7dadf9ff9d59ef", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-three"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-four" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/delay_three-1.0.0.tar.gz", hash = "sha256:66cbfc4abb20d334d5650fd5c98686a824e3e8ac1e9a80676c240f303243279b", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_three-1.0.0-py3-none-any.whl", hash = "sha256:26f1c369f51d6166a6aa01528946f16d91dd0e7ff260c6b9695a1c7dec262a6b", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-two"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-three" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/delay_two-1.0.0.tar.gz", hash = "sha256:5be60b7e5f0ceac15ea6128221b7dc5b47061c5551b80a7ea6549315cdefdd4f", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_two-1.0.0-py3-none-any.whl", hash = "sha256:e9559538559209b102813ee3eb862d70a1853ff9913312d501ca6cc7dddf796d", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "later-entry"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-one" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/later_entry-1.0.0.tar.gz", hash = "sha256:acce4eb5036ef90939082d27627aada5ec64781cf3e7ad62dcd5c8c4d0f27100", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/later_entry-1.0.0-py3-none-any.whl", hash = "sha256:b5b0da84fbb498341c35f39ab4218ad9644df46d6c4044b47e05a0a5a56b98f8", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "later-entry", marker = "python_full_version >= '3.13'" },
            { name = "switchable", marker = "python_full_version < '3.13'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "later-entry", marker = "python_full_version >= '3.13'" },
            { name = "switchable", marker = "python_full_version < '3.13'" },
        ]

        [[package]]
        name = "shared"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/shared-1.0.0.tar.gz", hash = "sha256:29242e0032cd4abe8ec185a5f0c198167191709b20778c7ff188dbd0037923f5", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared-1.0.0-py3-none-any.whl", hash = "sha256:9681756b32d3ae501a9e7129df974fe86476066cf3fcec82c82594bb91ecca07", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "survivor"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/survivor-1.0.0.tar.gz", hash = "sha256:acbc4e7cb12a681d138ec327409ad31b3b159b5ac5ab17d86fc580fe9a516518", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/survivor-1.0.0-py3-none-any.whl", hash = "sha256:5ab98f40fa608fa8ced6610cb7235dcb31c9090937075136be031a46c66e4bc5", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "switchable"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "survivor" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/switchable-1.0.0.tar.gz", hash = "sha256:9576efc4c064b93a3416383c6cc1985c92956157244c57f2ebd19dedd60a8f32", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/switchable-1.0.0-py3-none-any.whl", hash = "sha256:d309ab8102d958f22c4c83bb48ff43816f74ecb25f4a6c92d47c0b8987e85972", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// The earlier fork requires `shared==2.0rc1` and finishes before a delayed
/// sibling reaches its broader `shared` requirement. With prereleases globally
/// allowed, the sibling can reuse the selected prerelease even though stable
/// `shared==3` is also available.
///
///
/// ```text
/// coordinated-prerelease-eligible
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires later-entry ; python_full_version >= '3.13'
/// │   │   └── satisfied by later-entry-1.0.0
/// │   └── requires prerelease-parent ; python_full_version < '3.13'
/// │       └── satisfied by prerelease-parent-1.0.0
/// ├── broad-parent
/// │   └── broad-parent-1.0.0
/// │       └── requires shared>=1.0.0,<=3.0.0
/// │           ├── satisfied by shared-1.0.0
/// │           ├── satisfied by shared-2.0.0rc1
/// │           └── satisfied by shared-3.0.0
/// ├── delay-one
/// │   └── delay-one-1.0.0
/// │       └── requires delay-two
/// │           └── satisfied by delay-two-1.0.0
/// ├── delay-two
/// │   └── delay-two-1.0.0
/// │       └── requires broad-parent
/// │           └── satisfied by broad-parent-1.0.0
/// ├── later-entry
/// │   └── later-entry-1.0.0
/// │       └── requires delay-one
/// │           └── satisfied by delay-one-1.0.0
/// ├── prerelease-parent
/// │   └── prerelease-parent-1.0.0
/// │       └── requires shared==2.0.0rc1
/// │           └── satisfied by shared-2.0.0rc1
/// └── shared
///     ├── shared-1.0.0
///     ├── shared-2.0.0rc1
///     └── shared-3.0.0
/// ```
#[test]
fn coordinated_prerelease_eligible() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/coordinated/coordinated-prerelease-eligible.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''prerelease-parent ; python_full_version < '3.13'''',
          '''later-entry ; python_full_version >= '3.13'''',
        ]
        requires-python = ">=3.12, <3.14"
        [tool.uv]
        fork-strategy = "fewest"
        prerelease = "allow"
        environments = [
          '''python_full_version == '3.12.*'''',
          '''python_full_version == '3.13.*'''',
        ]
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    // Both forks use `shared==2.0rc1` because the sibling prerelease preference is eligible.
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12, <3.14"
        resolution-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]
        supported-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]

        [options]
        prerelease-mode = "allow"
        fork-strategy = "fewest"

        [[package]]
        name = "broad-parent"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "shared" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/broad_parent-1.0.0.tar.gz", hash = "sha256:ea3df34078180961e582e959e3feb9a31412785ff53401cc93cf059e891b23d4", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/broad_parent-1.0.0-py3-none-any.whl", hash = "sha256:2ce5f5e07476301dbe46177269aa079f255032a035ca79637c78355a2953453a", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-one"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-two" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/delay_one-1.0.0.tar.gz", hash = "sha256:24641f1b68d386ea2a2ef23ebc54305cb347d9a2745618057697c3ff4dbc5648", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_one-1.0.0-py3-none-any.whl", hash = "sha256:c1e8aca4b1866ffedbf054bd89c0d7d4fbc8647de899909cff7dadf9ff9d59ef", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-two"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "broad-parent" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/delay_two-1.0.0.tar.gz", hash = "sha256:79cc650b9649bb8b8b6c3c9734e88c4cdfe57c1da3547b1ae6e231cdb0f21035", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_two-1.0.0-py3-none-any.whl", hash = "sha256:6358aa460b7153880e4a15d9cc104c34fb0a2e2e868d25bb849191b3452e1db9", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "later-entry"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-one" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/later_entry-1.0.0.tar.gz", hash = "sha256:acce4eb5036ef90939082d27627aada5ec64781cf3e7ad62dcd5c8c4d0f27100", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/later_entry-1.0.0-py3-none-any.whl", hash = "sha256:b5b0da84fbb498341c35f39ab4218ad9644df46d6c4044b47e05a0a5a56b98f8", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "prerelease-parent"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "shared" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/prerelease_parent-1.0.0.tar.gz", hash = "sha256:178e058eff414839fbee62d2d2bd8a67bda83699fc3ece0d7dafcd1b301465c3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/prerelease_parent-1.0.0-py3-none-any.whl", hash = "sha256:addc83cfcf41f61b672efe4616f7286484b8a3643aa1202f67733ae3b2d98127", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "later-entry", marker = "python_full_version >= '3.13'" },
            { name = "prerelease-parent", marker = "python_full_version < '3.13'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "later-entry", marker = "python_full_version >= '3.13'" },
            { name = "prerelease-parent", marker = "python_full_version < '3.13'" },
        ]

        [[package]]
        name = "shared"
        version = "2.0.0rc1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/shared-2.0.0rc1.tar.gz", hash = "sha256:8a3bcf89f01d3d0e31c40605c0e4e4cf9b9249b6b7f4aeddd5c025de2157f72c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared-2.0.0rc1-py3-none-any.whl", hash = "sha256:2bd04770cfd6a4ea46d2e1ba4eb9bbac2c0e24fa4bd4742759c80cf1924c4e27", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// The earlier broad fork finishes with stable `shared==2` before a delayed sibling
/// requires `shared==2.5rc1`. Although the earlier version range contains that
/// prerelease, a hard cross-fork agreement must not force a prerelease into the
/// completed stable solution. Only stable versions are hard coordination
/// proposals.
///
///
/// ```text
/// coordinated-prerelease-hard-proposal-isolation
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires later-entry ; python_full_version >= '3.13'
/// │   │   └── satisfied by later-entry-1.0.0
/// │   └── requires stable-parent ; python_full_version < '3.13'
/// │       └── satisfied by stable-parent-1.0.0
/// ├── delay-one
/// │   └── delay-one-1.0.0
/// │       └── requires delay-two
/// │           └── satisfied by delay-two-1.0.0
/// ├── delay-two
/// │   └── delay-two-1.0.0
/// │       └── requires prerelease-parent
/// │           └── satisfied by prerelease-parent-1.0.0
/// ├── later-entry
/// │   └── later-entry-1.0.0
/// │       └── requires delay-one
/// │           └── satisfied by delay-one-1.0.0
/// ├── prerelease-parent
/// │   └── prerelease-parent-1.0.0
/// │       └── requires shared==2.5.0rc1
/// │           └── satisfied by shared-2.5.0rc1
/// ├── shared
/// │   ├── shared-1.0.0
/// │   ├── shared-2.0.0
/// │   └── shared-2.5.0rc1
/// └── stable-parent
///     └── stable-parent-1.0.0
///         └── requires shared>=1.0.0,<3.0.0
///             ├── satisfied by shared-1.0.0
///             ├── satisfied by shared-2.0.0
///             └── satisfied by shared-2.5.0rc1
/// ```
#[test]
fn coordinated_prerelease_hard_proposal_isolation() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server =
        PackseServer::new("fork/coordinated/coordinated-prerelease-hard-proposal-isolation.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''stable-parent ; python_full_version < '3.13'''',
          '''later-entry ; python_full_version >= '3.13'''',
        ]
        requires-python = ">=3.12, <3.14"
        [tool.uv]
        fork-strategy = "fewest"
        environments = [
          '''python_full_version == '3.12.*'''',
          '''python_full_version == '3.13.*'''',
        ]
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    // The completed broad fork retains stable `shared==2`, while the delayed fork uses `shared==2.5rc1`.
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12, <3.14"
        resolution-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]
        supported-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]

        [options]
        fork-strategy = "fewest"

        [[package]]
        name = "delay-one"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-two" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/delay_one-1.0.0.tar.gz", hash = "sha256:24641f1b68d386ea2a2ef23ebc54305cb347d9a2745618057697c3ff4dbc5648", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_one-1.0.0-py3-none-any.whl", hash = "sha256:c1e8aca4b1866ffedbf054bd89c0d7d4fbc8647de899909cff7dadf9ff9d59ef", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-two"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "prerelease-parent" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/delay_two-1.0.0.tar.gz", hash = "sha256:a6cefd4928344ec64301bcea849ed375f4668947eac0a525d8eef8c1f974b2c5", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_two-1.0.0-py3-none-any.whl", hash = "sha256:0e6e6ae48be25fba4b695cbbfc8a7513472a6ae17b244c2d04702e2334830f6c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "later-entry"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-one" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/later_entry-1.0.0.tar.gz", hash = "sha256:acce4eb5036ef90939082d27627aada5ec64781cf3e7ad62dcd5c8c4d0f27100", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/later_entry-1.0.0-py3-none-any.whl", hash = "sha256:b5b0da84fbb498341c35f39ab4218ad9644df46d6c4044b47e05a0a5a56b98f8", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "prerelease-parent"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "shared", version = "2.5.0rc1", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        sdist = { url = "http://[LOCALHOST]/files/prerelease_parent-1.0.0.tar.gz", hash = "sha256:12cd85d6d5948502c33d30c338df23cdb34156f8f6e45a834e1d425c29c93f99", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/prerelease_parent-1.0.0-py3-none-any.whl", hash = "sha256:91e352323c2e7844568bed5abe11884a9ca9212ea5f80fd3a776577d970b02af", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "later-entry", marker = "python_full_version >= '3.13'" },
            { name = "stable-parent", marker = "python_full_version < '3.13'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "later-entry", marker = "python_full_version >= '3.13'" },
            { name = "stable-parent", marker = "python_full_version < '3.13'" },
        ]

        [[package]]
        name = "shared"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "python_full_version < '3.13'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/shared-2.0.0.tar.gz", hash = "sha256:c09e0026550169997ff12b4c435b88e8ef9aed1fbd7d24265daea1f686f46059", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared-2.0.0-py3-none-any.whl", hash = "sha256:50654d921e335114898f26df3c573dfe4d21fae95abdb27584dbbc624137a5d0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "shared"
        version = "2.5.0rc1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "python_full_version >= '3.13'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/shared-2.5.0rc1.tar.gz", hash = "sha256:4051453fb51a05c15ada0451a1fa809c3e0410dd6a813a1d7af33ae2f88618c0", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared-2.5.0rc1-py3-none-any.whl", hash = "sha256:cf5380d67ac47d8cd30e26b41e09ebe02e6f0ce801f7a7097df48adb1a406070", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "stable-parent"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "shared", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        sdist = { url = "http://[LOCALHOST]/files/stable_parent-1.0.0.tar.gz", hash = "sha256:2aaceac14d902caa49ef96712d879cd276cdfc56d345547312697ea0d5edbbbd", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/stable_parent-1.0.0-py3-none-any.whl", hash = "sha256:de8003bb1a5667c032e5c6cdee4ed49c1c3f68696a2769f25bb68a69fb61f68d", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// The earlier fork requires `shared==2.0rc1` and finishes before a delayed
/// sibling reaches its broader `shared` requirement. The sibling has stable
/// `shared==3` available, so the resolver-sourced prerelease preference must not
/// override its preference for stable versions.
///
///
/// ```text
/// coordinated-prerelease-stable-isolation
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires later-entry ; python_full_version >= '3.13'
/// │   │   └── satisfied by later-entry-1.0.0
/// │   └── requires prerelease-parent ; python_full_version < '3.13'
/// │       └── satisfied by prerelease-parent-1.0.0
/// ├── broad-parent
/// │   └── broad-parent-1.0.0
/// │       └── requires shared>=1.0.0,<=3.0.0
/// │           ├── satisfied by shared-1.0.0
/// │           ├── satisfied by shared-2.0.0rc1
/// │           └── satisfied by shared-3.0.0
/// ├── delay-one
/// │   └── delay-one-1.0.0
/// │       └── requires delay-two
/// │           └── satisfied by delay-two-1.0.0
/// ├── delay-two
/// │   └── delay-two-1.0.0
/// │       └── requires broad-parent
/// │           └── satisfied by broad-parent-1.0.0
/// ├── later-entry
/// │   └── later-entry-1.0.0
/// │       └── requires delay-one
/// │           └── satisfied by delay-one-1.0.0
/// ├── prerelease-parent
/// │   └── prerelease-parent-1.0.0
/// │       └── requires shared==2.0.0rc1
/// │           └── satisfied by shared-2.0.0rc1
/// └── shared
///     ├── shared-1.0.0
///     ├── shared-2.0.0rc1
///     └── shared-3.0.0
/// ```
#[test]
fn coordinated_prerelease_stable_isolation() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/coordinated/coordinated-prerelease-stable-isolation.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''prerelease-parent ; python_full_version < '3.13'''',
          '''later-entry ; python_full_version >= '3.13'''',
        ]
        requires-python = ">=3.12, <3.14"
        [tool.uv]
        fork-strategy = "fewest"
        environments = [
          '''python_full_version == '3.12.*'''',
          '''python_full_version == '3.13.*'''',
        ]
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    // The earlier fork uses `shared==2.0rc1`; the broad sibling keeps stable `shared==3`.
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12, <3.14"
        resolution-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]
        supported-markers = [
            "python_full_version < '3.13'",
            "python_full_version >= '3.13'",
        ]

        [options]
        fork-strategy = "fewest"

        [[package]]
        name = "broad-parent"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "shared", version = "3.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        sdist = { url = "http://[LOCALHOST]/files/broad_parent-1.0.0.tar.gz", hash = "sha256:ea3df34078180961e582e959e3feb9a31412785ff53401cc93cf059e891b23d4", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/broad_parent-1.0.0-py3-none-any.whl", hash = "sha256:2ce5f5e07476301dbe46177269aa079f255032a035ca79637c78355a2953453a", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-one"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-two" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/delay_one-1.0.0.tar.gz", hash = "sha256:24641f1b68d386ea2a2ef23ebc54305cb347d9a2745618057697c3ff4dbc5648", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_one-1.0.0-py3-none-any.whl", hash = "sha256:c1e8aca4b1866ffedbf054bd89c0d7d4fbc8647de899909cff7dadf9ff9d59ef", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "delay-two"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "broad-parent" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/delay_two-1.0.0.tar.gz", hash = "sha256:79cc650b9649bb8b8b6c3c9734e88c4cdfe57c1da3547b1ae6e231cdb0f21035", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/delay_two-1.0.0-py3-none-any.whl", hash = "sha256:6358aa460b7153880e4a15d9cc104c34fb0a2e2e868d25bb849191b3452e1db9", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "later-entry"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "delay-one" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/later_entry-1.0.0.tar.gz", hash = "sha256:acce4eb5036ef90939082d27627aada5ec64781cf3e7ad62dcd5c8c4d0f27100", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/later_entry-1.0.0-py3-none-any.whl", hash = "sha256:b5b0da84fbb498341c35f39ab4218ad9644df46d6c4044b47e05a0a5a56b98f8", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "prerelease-parent"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "shared", version = "2.0.0rc1", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        sdist = { url = "http://[LOCALHOST]/files/prerelease_parent-1.0.0.tar.gz", hash = "sha256:178e058eff414839fbee62d2d2bd8a67bda83699fc3ece0d7dafcd1b301465c3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/prerelease_parent-1.0.0-py3-none-any.whl", hash = "sha256:addc83cfcf41f61b672efe4616f7286484b8a3643aa1202f67733ae3b2d98127", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "later-entry", marker = "python_full_version >= '3.13'" },
            { name = "prerelease-parent", marker = "python_full_version < '3.13'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "later-entry", marker = "python_full_version >= '3.13'" },
            { name = "prerelease-parent", marker = "python_full_version < '3.13'" },
        ]

        [[package]]
        name = "shared"
        version = "2.0.0rc1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "python_full_version < '3.13'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/shared-2.0.0rc1.tar.gz", hash = "sha256:8a3bcf89f01d3d0e31c40605c0e4e4cf9b9249b6b7f4aeddd5c025de2157f72c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared-2.0.0rc1-py3-none-any.whl", hash = "sha256:2bd04770cfd6a4ea46d2e1ba4eb9bbac2c0e24fa4bd4742759c80cf1924c4e27", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "shared"
        version = "3.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "python_full_version >= '3.13'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/shared-3.0.0.tar.gz", hash = "sha256:fdf45d886aa52e132d5ef8fa678aed76cc1d1f154a072cacb70af7a9752412b2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/shared-3.0.0-py3-none-any.whl", hash = "sha256:c6ae0879137e17eef27f067478241ee39169c34af4c5b78be26f61753dc4022d", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// This tests that sibling dependencies of a package that provokes a
/// fork are correctly filtered out of forks where they are otherwise
/// impossible.
///
/// In this case, a previous version of the universal resolver would
/// include both `b` and `c` in *both* of the forks produced by the
/// conflicting dependency specifications on `a`. This in turn led to
/// transitive dependency specifications on both `d==1.0.0` and `d==2.0.0`.
/// Since the universal resolver only forks based on local conditions, this
/// led to a failed resolution.
///
/// The correct thing to do here is to ensure that `b` is only part of the
/// `a==4.4.0` fork and `c` is only par of the `a==4.3.0` fork.
///
///
/// ```text
/// fork-filter-sibling-dependencies
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a==4.4.0 ; sys_platform == 'linux'
/// │   │   └── satisfied by a-4.4.0
/// │   ├── requires a==4.3.0 ; sys_platform == 'darwin'
/// │   │   └── satisfied by a-4.3.0
/// │   ├── requires b==1.0.0 ; sys_platform == 'linux'
/// │   │   └── satisfied by b-1.0.0
/// │   └── requires c==1.0.0 ; sys_platform == 'darwin'
/// │       └── satisfied by c-1.0.0
/// ├── a
/// │   ├── a-4.3.0
/// │   └── a-4.4.0
/// ├── b
/// │   └── b-1.0.0
/// │       └── requires d==1.0.0
/// │           └── satisfied by d-1.0.0
/// ├── c
/// │   └── c-1.0.0
/// │       └── requires d==2.0.0
/// │           └── satisfied by d-2.0.0
/// └── d
///     ├── d-1.0.0
///     └── d-2.0.0
/// ```
#[test]
fn fork_filter_sibling_dependencies() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/filter-sibling-dependencies.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a==4.4.0 ; sys_platform == 'linux'''',
          '''a==4.3.0 ; sys_platform == 'darwin'''',
          '''b==1.0.0 ; sys_platform == 'linux'''',
          '''c==1.0.0 ; sys_platform == 'darwin'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 7 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "sys_platform == 'linux'",
            "sys_platform == 'darwin'",
            "sys_platform != 'darwin' and sys_platform != 'linux'",
        ]

        [[package]]
        name = "a"
        version = "4.3.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'darwin'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-4.3.0.tar.gz", hash = "sha256:c827d6b38cef471d2cbb5c4a0ffd2b1b664fcaceb7468285291b09ae720cce85", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-4.3.0-py3-none-any.whl", hash = "sha256:801f46d2474bf22f2eed823a9a9343480a5ea089a45e949dbb1aad91a32cc14f", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "a"
        version = "4.4.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-4.4.0.tar.gz", hash = "sha256:44ef07c198ff128c43fea8095c6404169d67af81d7a2d5a9d14bf442d46141de", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-4.4.0-py3-none-any.whl", hash = "sha256:3df7c088229de8a0ee9765d9da6f543d5b40155fade904f0c95780fc843c8ed7", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "d", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        sdist = { url = "http://[LOCALHOST]/files/b-1.0.0.tar.gz", hash = "sha256:0c4fab34d536effcd612484e0340622ee38fc410901455c7f0a1099e1ac09bc3", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-1.0.0-py3-none-any.whl", hash = "sha256:ee44ff0b8963063959a61fdafa6d9742f5e03efec800a3661e4b081bb6343be1", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "c"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "d", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        sdist = { url = "http://[LOCALHOST]/files/c-1.0.0.tar.gz", hash = "sha256:9e5affe114cac181b24f22f6639d9a0dc3f1ec3e39752271f4ccdd23cac7957d", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/c-1.0.0-py3-none-any.whl", hash = "sha256:c243070ddb01de3029aebe2536aed4b43f324ebaef3a156304c9c4d7ae2f6a63", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "d"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/d-1.0.0.tar.gz", hash = "sha256:bbb9d05b6de19e47de8e49fcc69483e76cd868bd556c8173680756d53e6997d4", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/d-1.0.0-py3-none-any.whl", hash = "sha256:362166e5bd895367cc4ba5b7327949b7d417fe30cb3273a76b5db4a280dac05d", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "d"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'darwin'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/d-2.0.0.tar.gz", hash = "sha256:d5570875cb7c4bb8cba56f4e5aec850f5e6f21cb6ee0316b3a4f90be887edb75", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/d-2.0.0-py3-none-any.whl", hash = "sha256:3cf4357f0fdce5ed2a98b8befcef84e8b8150244737481ebc51d3d9ec000bc3b", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a", version = "4.3.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'darwin'" },
            { name = "a", version = "4.4.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'linux'" },
            { name = "b", marker = "sys_platform == 'linux'" },
            { name = "c", marker = "sys_platform == 'darwin'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a", marker = "sys_platform == 'darwin'", specifier = "==4.3.0" },
            { name = "a", marker = "sys_platform == 'linux'", specifier = "==4.4.0" },
            { name = "b", marker = "sys_platform == 'linux'", specifier = "==1.0.0" },
            { name = "c", marker = "sys_platform == 'darwin'", specifier = "==1.0.0" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// This test checks that we discard fork markers when using `--upgrade`.
///
///
/// ```text
/// fork-upgrade
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   └── requires foo
/// │       ├── satisfied by foo-1.0.0
/// │       └── satisfied by foo-2.0.0
/// ├── bar
/// │   ├── bar-1.0.0
/// │   └── bar-2.0.0
/// └── foo
///     ├── foo-1.0.0
///     │   ├── requires bar==1 ; sys_platform == 'linux'
///     │   │   └── satisfied by bar-1.0.0
///     │   └── requires bar==2 ; sys_platform != 'linux'
///     │       └── satisfied by bar-2.0.0
///     └── foo-2.0.0
///         └── requires bar==2
///             └── satisfied by bar-2.0.0
/// ```
#[test]
fn fork_upgrade() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/fork-upgrade.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''foo''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [[package]]
        name = "bar"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/bar-2.0.0.tar.gz", hash = "sha256:29e7bc76f76b7e939dcf1f8fe28b077c4631c11ecea673beb86d297dacda11eb", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/bar-2.0.0-py3-none-any.whl", hash = "sha256:563b1af3238a4ad819f2b95b74f940319a2ef30ed7991a2416fa98aa115da87d", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "foo"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "bar" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/foo-2.0.0.tar.gz", hash = "sha256:06645b1e3c66510cee0c5b852731f1144bbfa98700d36d77a3a1def134c32541", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/foo-2.0.0-py3-none-any.whl", hash = "sha256:36953b42725b8f3b6ebde327b8fab1d1e906ce0902c6272855851ea1964bc37a", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "foo" },
        ]

        [package.metadata]
        requires-dist = [{ name = "foo" }]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// The root cause the resolver to fork over `a`, but the markers on the variant
/// of `a` don't cover the entire marker space, they are missing Python 3.13.
/// Later, we have a dependency this very hole, which we still need to select,
/// instead of having two forks around but without Python 3.13 and omitting
/// `c` from the solution.
///
///
/// ```text
/// fork-incomplete-markers
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a==1 ; python_full_version < '3.13'
/// │   │   └── satisfied by a-1.0.0
/// │   ├── requires a==2 ; python_full_version >= '3.14'
/// │   │   └── satisfied by a-2.0.0
/// │   └── requires b
/// │       └── satisfied by b-1.0.0
/// ├── a
/// │   ├── a-1.0.0
/// │   └── a-2.0.0
/// ├── b
/// │   └── b-1.0.0
/// │       └── requires c ; python_full_version == '3.13.*'
/// │           └── satisfied by c-1.0.0
/// └── c
///     └── c-1.0.0
/// ```
#[test]
fn fork_incomplete_markers() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/incomplete-markers.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a==1 ; python_full_version < '3.13'''',
          '''a==2 ; python_full_version >= '3.14'''',
          '''b''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "python_full_version >= '3.14'",
            "python_full_version == '3.13.*'",
            "python_full_version < '3.13'",
        ]

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "python_full_version < '3.13'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:957f99ff1d65ce0d7883d50f4e67ed8d4b42e76d2c2b5e62384ff0ba538647b5", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:f936eedc194aa91ca01a4c6c9981136ca6c75ce6df47e3951b12522881dce809", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "a"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "python_full_version >= '3.14'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-2.0.0.tar.gz", hash = "sha256:9610291c2bd57390019f58ca72d0dd4584bb9e7073fa347633ed8bc7267fccfe", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-2.0.0-py3-none-any.whl", hash = "sha256:833374310e0a15880f3be9e6d082f527c9ac70129b2054d733da9b754315361f", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "c", marker = "python_full_version == '3.13.*'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/b-1.0.0.tar.gz", hash = "sha256:675547bdd1ec8b9552c086605f0ca400a2ef057934366281d0b357127e216384", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-1.0.0-py3-none-any.whl", hash = "sha256:b8c3f2688065abd235cc88b32446d4c807cfa3a1f2d676874bccc7e0f63137bd", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "c"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/c-1.0.0.tar.gz", hash = "sha256:699a07ff61aab66fcba4883a94c6d2b61afb7797fa956ae36f2efdf30d9dfbc7", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/c-1.0.0-py3-none-any.whl", hash = "sha256:78c0da7c5681d751d38b2e60c78d1e29d6125d91e68e5aeb22372fa66527ff95", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "python_full_version < '3.13'" },
            { name = "a", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "python_full_version >= '3.14'" },
            { name = "b" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a", marker = "python_full_version < '3.13'", specifier = "==1" },
            { name = "a", marker = "python_full_version >= '3.14'", specifier = "==2" },
            { name = "b" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// This is actually a non-forking test case that tests the tracking of marker
/// expressions in general. In this case, the dependency on `c` should have its
/// marker expressions automatically combined. In this case, it's `linux OR
/// darwin`, even though `linux OR darwin` doesn't actually appear verbatim as a
/// marker expression for any dependency on `c`.
///
///
/// ```text
/// fork-marker-accrue
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a==1.0.0 ; implementation_name == 'cpython'
/// │   │   └── satisfied by a-1.0.0
/// │   └── requires b==1.0.0 ; implementation_name == 'pypy'
/// │       └── satisfied by b-1.0.0
/// ├── a
/// │   └── a-1.0.0
/// │       └── requires c==1.0.0 ; sys_platform == 'linux'
/// │           └── satisfied by c-1.0.0
/// ├── b
/// │   └── b-1.0.0
/// │       └── requires c==1.0.0 ; sys_platform == 'darwin'
/// │           └── satisfied by c-1.0.0
/// └── c
///     └── c-1.0.0
/// ```
#[test]
fn fork_marker_accrue() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/marker-accrue.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a==1.0.0 ; implementation_name == 'cpython'''',
          '''b==1.0.0 ; implementation_name == 'pypy'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "c", marker = "sys_platform == 'linux'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:c6a69af4dc542ea244c21aae42b5639705975100871f1066c58be1245f013c95", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:8ed25f2453465e5f1e91e7b09e21b08234389dc5d6e3f7f27e89e801ba42d807", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "c", marker = "sys_platform == 'darwin'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/b-1.0.0.tar.gz", hash = "sha256:c855f831e0a345fc9b086051e2a8b02be6f9356bb131f240da96a793bb1d4d1c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-1.0.0-py3-none-any.whl", hash = "sha256:e21416492e59fd38a6e084b303f29fcde08805f201a4f9e4833c60cd18ebfc4f", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "c"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/c-1.0.0.tar.gz", hash = "sha256:699a07ff61aab66fcba4883a94c6d2b61afb7797fa956ae36f2efdf30d9dfbc7", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/c-1.0.0-py3-none-any.whl", hash = "sha256:78c0da7c5681d751d38b2e60c78d1e29d6125d91e68e5aeb22372fa66527ff95", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a", marker = "implementation_name == 'cpython'" },
            { name = "b", marker = "implementation_name == 'pypy'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a", marker = "implementation_name == 'cpython'", specifier = "==1.0.0" },
            { name = "b", marker = "implementation_name == 'pypy'", specifier = "==1.0.0" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// A basic test that ensures, at least in this one basic case, that forking in
/// universal resolution happens only when the corresponding marker expressions are
/// completely disjoint. Here, we provide two completely incompatible dependency
/// specifications with equivalent markers. Thus, they are trivially not disjoint,
/// and resolution should fail.
///
/// NOTE: This acts a regression test for the initial version of universal
/// resolution that would fork whenever a package was repeated in the list of
/// dependency specifications. So previously, this would produce a resolution with
/// both `1.0.0` and `2.0.0` of `a`. But of course, the correct behavior is to fail
/// resolving.
///
///
/// ```text
/// fork-marker-disjoint
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a>=2 ; sys_platform == 'linux'
/// │   │   └── satisfied by a-2.0.0
/// │   └── requires a<2 ; sys_platform == 'linux'
/// │       └── satisfied by a-1.0.0
/// └── a
///     ├── a-1.0.0
///     └── a-2.0.0
/// ```
#[test]
fn fork_marker_disjoint() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/marker-disjoint.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a>=2 ; sys_platform == 'linux'''',
          '''a<2 ; sys_platform == 'linux'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because your project depends on a{sys_platform == 'linux'}>=2 and a{sys_platform == 'linux'}<2, we can conclude that your project's requirements are unsatisfiable.
    "
    );

    Ok(())
}

/// This test builds on `fork-marker-inherit-combined`. Namely, we add
/// `or implementation_name == 'pypy'` to the dependency on `c`. While
/// `sys_platform == 'linux'` cannot be true because of the first fork,
/// the second fork which includes `b==1.0.0` happens precisely when
/// `implementation_name == 'pypy'`. So in this case, `c` should be
/// included.
///
///
/// ```text
/// fork-marker-inherit-combined-allowed
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a>=2 ; sys_platform == 'linux'
/// │   │   └── satisfied by a-2.0.0
/// │   └── requires a<2 ; sys_platform == 'darwin'
/// │       └── satisfied by a-1.0.0
/// ├── a
/// │   ├── a-1.0.0
/// │   │   ├── requires b>=2 ; implementation_name == 'cpython'
/// │   │   │   └── satisfied by b-2.0.0
/// │   │   └── requires b<2 ; implementation_name == 'pypy'
/// │   │       └── satisfied by b-1.0.0
/// │   └── a-2.0.0
/// ├── b
/// │   ├── b-1.0.0
/// │   │   └── requires c ; implementation_name == 'pypy' or sys_platform == 'linux'
/// │   │       └── satisfied by c-1.0.0
/// │   └── b-2.0.0
/// └── c
///     └── c-1.0.0
/// ```
#[test]
fn fork_marker_inherit_combined_allowed() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/marker-inherit-combined-allowed.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a>=2 ; sys_platform == 'linux'''',
          '''a<2 ; sys_platform == 'darwin'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "implementation_name == 'pypy' and sys_platform == 'darwin'",
            "implementation_name == 'cpython' and sys_platform == 'darwin'",
            "implementation_name != 'cpython' and implementation_name != 'pypy' and sys_platform == 'darwin'",
            "sys_platform == 'linux'",
            "sys_platform != 'darwin' and sys_platform != 'linux'",
        ]

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "implementation_name == 'pypy' and sys_platform == 'darwin'",
            "implementation_name == 'cpython' and sys_platform == 'darwin'",
            "implementation_name != 'cpython' and implementation_name != 'pypy' and sys_platform == 'darwin'",
        ]
        dependencies = [
            { name = "b", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "implementation_name == 'pypy'" },
            { name = "b", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "implementation_name == 'cpython'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:23d75e1acf1aaf735e83615f8baba2fa0d0e5f9b885706cfd017b9b72301cdab", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:ebe588eab684413e5969ec398e03f7386a8106c5c88a601dacb2781fe2d0c819", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "a"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-2.0.0.tar.gz", hash = "sha256:9610291c2bd57390019f58ca72d0dd4584bb9e7073fa347633ed8bc7267fccfe", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-2.0.0-py3-none-any.whl", hash = "sha256:833374310e0a15880f3be9e6d082f527c9ac70129b2054d733da9b754315361f", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "implementation_name == 'pypy' and sys_platform == 'darwin'",
        ]
        dependencies = [
            { name = "c" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/b-1.0.0.tar.gz", hash = "sha256:af6eb9a200314b36d0f49af106101b43445321bf148054ce024c92d79e93fa31", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-1.0.0-py3-none-any.whl", hash = "sha256:94d6ef21aaf5389c9ec11da5f313697b2f5a3b35f039735d2bcf0cfb7a6f88d1", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "implementation_name == 'cpython' and sys_platform == 'darwin'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/b-2.0.0.tar.gz", hash = "sha256:256a9af98c362451ef802d6462f06f8d4e26cc52543e8100cba63b584bae9a7c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-2.0.0-py3-none-any.whl", hash = "sha256:04cc57f7563029528b6d23283933b244b6f52ba1543fad54687c586d6e639fc4", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "c"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/c-1.0.0.tar.gz", hash = "sha256:699a07ff61aab66fcba4883a94c6d2b61afb7797fa956ae36f2efdf30d9dfbc7", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/c-1.0.0-py3-none-any.whl", hash = "sha256:78c0da7c5681d751d38b2e60c78d1e29d6125d91e68e5aeb22372fa66527ff95", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'darwin'" },
            { name = "a", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'linux'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a", marker = "sys_platform == 'darwin'", specifier = "<2" },
            { name = "a", marker = "sys_platform == 'linux'", specifier = ">=2" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// This test builds on `fork-marker-inherit-combined`. Namely, we add
/// `or implementation_name == 'cpython'` to the dependency on `c`.
/// While `sys_platform == 'linux'` cannot be true because of the first
/// fork, the second fork which includes `b==1.0.0` happens precisely
/// when `implementation_name == 'pypy'`, which is *also* disjoint with
/// `implementation_name == 'cpython'`. Therefore, `c` should not be
/// included here.
///
///
/// ```text
/// fork-marker-inherit-combined-disallowed
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a>=2 ; sys_platform == 'linux'
/// │   │   └── satisfied by a-2.0.0
/// │   └── requires a<2 ; sys_platform == 'darwin'
/// │       └── satisfied by a-1.0.0
/// ├── a
/// │   ├── a-1.0.0
/// │   │   ├── requires b>=2 ; implementation_name == 'cpython'
/// │   │   │   └── satisfied by b-2.0.0
/// │   │   └── requires b<2 ; implementation_name == 'pypy'
/// │   │       └── satisfied by b-1.0.0
/// │   └── a-2.0.0
/// ├── b
/// │   ├── b-1.0.0
/// │   │   └── requires c ; implementation_name == 'cpython' or sys_platform == 'linux'
/// │   │       └── satisfied by c-1.0.0
/// │   └── b-2.0.0
/// └── c
///     └── c-1.0.0
/// ```
#[test]
fn fork_marker_inherit_combined_disallowed() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/marker-inherit-combined-disallowed.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a>=2 ; sys_platform == 'linux'''',
          '''a<2 ; sys_platform == 'darwin'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "implementation_name == 'pypy' and sys_platform == 'darwin'",
            "implementation_name == 'cpython' and sys_platform == 'darwin'",
            "implementation_name != 'cpython' and implementation_name != 'pypy' and sys_platform == 'darwin'",
            "sys_platform == 'linux'",
            "sys_platform != 'darwin' and sys_platform != 'linux'",
        ]

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "implementation_name == 'pypy' and sys_platform == 'darwin'",
            "implementation_name == 'cpython' and sys_platform == 'darwin'",
            "implementation_name != 'cpython' and implementation_name != 'pypy' and sys_platform == 'darwin'",
        ]
        dependencies = [
            { name = "b", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "implementation_name == 'pypy'" },
            { name = "b", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "implementation_name == 'cpython'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:23d75e1acf1aaf735e83615f8baba2fa0d0e5f9b885706cfd017b9b72301cdab", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:ebe588eab684413e5969ec398e03f7386a8106c5c88a601dacb2781fe2d0c819", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "a"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-2.0.0.tar.gz", hash = "sha256:9610291c2bd57390019f58ca72d0dd4584bb9e7073fa347633ed8bc7267fccfe", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-2.0.0-py3-none-any.whl", hash = "sha256:833374310e0a15880f3be9e6d082f527c9ac70129b2054d733da9b754315361f", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "implementation_name == 'pypy' and sys_platform == 'darwin'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/b-1.0.0.tar.gz", hash = "sha256:c20e9062f4cf30f8e581130ae2e18959f0f294246eb3bdd9f25053bd72a74267", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-1.0.0-py3-none-any.whl", hash = "sha256:b9dea47846d57e4a52afe31d36ad8fc1e7c01505c768be8855222320a9028f3c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "implementation_name == 'cpython' and sys_platform == 'darwin'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/b-2.0.0.tar.gz", hash = "sha256:256a9af98c362451ef802d6462f06f8d4e26cc52543e8100cba63b584bae9a7c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-2.0.0-py3-none-any.whl", hash = "sha256:04cc57f7563029528b6d23283933b244b6f52ba1543fad54687c586d6e639fc4", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'darwin'" },
            { name = "a", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'linux'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a", marker = "sys_platform == 'darwin'", specifier = "<2" },
            { name = "a", marker = "sys_platform == 'linux'", specifier = ">=2" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// In this test, we check that marker expressions which provoke a fork
/// are carried through to subsequent forks. Here, the `a>=2` and `a<2`
/// dependency specifications create a fork, and then the `a<2` fork leads
/// to `a==1.0.0` with dependency specifications on `b>=2` and `b<2` that
/// provoke yet another fork. Finally, in the `b<2` fork, a dependency on
/// `c` is introduced whose marker expression is disjoint with the marker
/// expression that provoked the *first* fork. Therefore, `c` should be
/// entirely excluded from the resolution.
///
///
/// ```text
/// fork-marker-inherit-combined
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a>=2 ; sys_platform == 'linux'
/// │   │   └── satisfied by a-2.0.0
/// │   └── requires a<2 ; sys_platform == 'darwin'
/// │       └── satisfied by a-1.0.0
/// ├── a
/// │   ├── a-1.0.0
/// │   │   ├── requires b>=2 ; implementation_name == 'cpython'
/// │   │   │   └── satisfied by b-2.0.0
/// │   │   └── requires b<2 ; implementation_name == 'pypy'
/// │   │       └── satisfied by b-1.0.0
/// │   └── a-2.0.0
/// ├── b
/// │   ├── b-1.0.0
/// │   │   └── requires c ; sys_platform == 'linux'
/// │   │       └── satisfied by c-1.0.0
/// │   └── b-2.0.0
/// └── c
///     └── c-1.0.0
/// ```
#[test]
fn fork_marker_inherit_combined() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/marker-inherit-combined.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a>=2 ; sys_platform == 'linux'''',
          '''a<2 ; sys_platform == 'darwin'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "implementation_name == 'pypy' and sys_platform == 'darwin'",
            "implementation_name == 'cpython' and sys_platform == 'darwin'",
            "implementation_name != 'cpython' and implementation_name != 'pypy' and sys_platform == 'darwin'",
            "sys_platform == 'linux'",
            "sys_platform != 'darwin' and sys_platform != 'linux'",
        ]

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "implementation_name == 'pypy' and sys_platform == 'darwin'",
            "implementation_name == 'cpython' and sys_platform == 'darwin'",
            "implementation_name != 'cpython' and implementation_name != 'pypy' and sys_platform == 'darwin'",
        ]
        dependencies = [
            { name = "b", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "implementation_name == 'pypy'" },
            { name = "b", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "implementation_name == 'cpython'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:23d75e1acf1aaf735e83615f8baba2fa0d0e5f9b885706cfd017b9b72301cdab", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:ebe588eab684413e5969ec398e03f7386a8106c5c88a601dacb2781fe2d0c819", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "a"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-2.0.0.tar.gz", hash = "sha256:9610291c2bd57390019f58ca72d0dd4584bb9e7073fa347633ed8bc7267fccfe", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-2.0.0-py3-none-any.whl", hash = "sha256:833374310e0a15880f3be9e6d082f527c9ac70129b2054d733da9b754315361f", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "implementation_name == 'pypy' and sys_platform == 'darwin'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/b-1.0.0.tar.gz", hash = "sha256:de35bb11b581875ed4be3c930bcc4d98e7e9ac34d5fe678f74a27c2136b633f9", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-1.0.0-py3-none-any.whl", hash = "sha256:42ee7845ea1960c41a676a74c9add4f48915e78f031b0e0a7d3894a692b9b6dd", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "implementation_name == 'cpython' and sys_platform == 'darwin'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/b-2.0.0.tar.gz", hash = "sha256:256a9af98c362451ef802d6462f06f8d4e26cc52543e8100cba63b584bae9a7c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-2.0.0-py3-none-any.whl", hash = "sha256:04cc57f7563029528b6d23283933b244b6f52ba1543fad54687c586d6e639fc4", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'darwin'" },
            { name = "a", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'linux'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a", marker = "sys_platform == 'darwin'", specifier = "<2" },
            { name = "a", marker = "sys_platform == 'linux'", specifier = ">=2" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// This is like `fork-marker-inherit`, but where both `a>=2` and `a<2`
/// have a conditional dependency on `b`. For `a>=2`, the conditional
/// dependency on `b` has overlap with the `a>=2` marker expression, and
/// thus, `b` should be included *only* in the dependencies for `a==2.0.0`.
/// As with `fork-marker-inherit`, the `a<2` path should exclude `b==1.0.0`
/// since their marker expressions are disjoint.
///
///
/// ```text
/// fork-marker-inherit-isolated
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a>=2 ; sys_platform == 'linux'
/// │   │   └── satisfied by a-2.0.0
/// │   └── requires a<2 ; sys_platform == 'darwin'
/// │       └── satisfied by a-1.0.0
/// ├── a
/// │   ├── a-1.0.0
/// │   │   └── requires b ; sys_platform == 'linux'
/// │   │       └── satisfied by b-1.0.0
/// │   └── a-2.0.0
/// │       └── requires b ; sys_platform == 'linux'
/// │           └── satisfied by b-1.0.0
/// └── b
///     └── b-1.0.0
/// ```
#[test]
fn fork_marker_inherit_isolated() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/marker-inherit-isolated.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a>=2 ; sys_platform == 'linux'''',
          '''a<2 ; sys_platform == 'darwin'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "sys_platform == 'darwin'",
            "sys_platform == 'linux'",
            "sys_platform != 'darwin' and sys_platform != 'linux'",
        ]

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'darwin'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:bdb790e6d65140f316bfb33a6bc9ab03732245e8b5c6fd8efbcff7744530795d", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:0f04d2c483396a1af08a8baee4a39f08f4e4a1f9775c3b9bce5245852e864eba", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "a"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        dependencies = [
            { name = "b" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-2.0.0.tar.gz", hash = "sha256:90a1f56c11a242c7437e595d28b6903388568edcb2ab8111657cc36fb7297ece", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-2.0.0-py3-none-any.whl", hash = "sha256:f455166cf613308f098ecf7d0911ca68838ca203b05c62cdb54b3043ad27c2d0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/b-1.0.0.tar.gz", hash = "sha256:9b42692ac74b4da0eed1ef248b9be6bb0557c49507ba3b38f862b191a06d959c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-1.0.0-py3-none-any.whl", hash = "sha256:a4c65510001153cab97a29ff219ad86e0d4330653ca89d9d4c84187ccf14c621", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'darwin'" },
            { name = "a", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'linux'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a", marker = "sys_platform == 'darwin'", specifier = "<2" },
            { name = "a", marker = "sys_platform == 'linux'", specifier = ">=2" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// This is like `fork-marker-inherit`, but tests that the marker
/// expressions that provoke a fork are carried transitively through the
/// dependency graph. In this case, `a<2 -> b -> c -> d`, but where the
/// last dependency on `d` requires a marker expression that is disjoint
/// with the initial `a<2` dependency. Therefore, it ought to be completely
/// excluded from the resolution.
///
///
/// ```text
/// fork-marker-inherit-transitive
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a>=2 ; sys_platform == 'linux'
/// │   │   └── satisfied by a-2.0.0
/// │   └── requires a<2 ; sys_platform == 'darwin'
/// │       └── satisfied by a-1.0.0
/// ├── a
/// │   ├── a-1.0.0
/// │   │   └── requires b
/// │   │       └── satisfied by b-1.0.0
/// │   └── a-2.0.0
/// ├── b
/// │   └── b-1.0.0
/// │       └── requires c
/// │           └── satisfied by c-1.0.0
/// ├── c
/// │   └── c-1.0.0
/// │       └── requires d ; sys_platform == 'linux'
/// │           └── satisfied by d-1.0.0
/// └── d
///     └── d-1.0.0
/// ```
#[test]
fn fork_marker_inherit_transitive() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/marker-inherit-transitive.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a>=2 ; sys_platform == 'linux'''',
          '''a<2 ; sys_platform == 'darwin'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "sys_platform == 'darwin'",
            "sys_platform == 'linux'",
            "sys_platform != 'darwin' and sys_platform != 'linux'",
        ]

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'darwin'",
        ]
        dependencies = [
            { name = "b" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:75c52500ad189dbf1bd52b7db63c3a480b381039c554aace83b348c36c39aa25", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:a0f20c0172d171015f1637827325521d0feaa0b85c49058980f660080d96170c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "a"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-2.0.0.tar.gz", hash = "sha256:9610291c2bd57390019f58ca72d0dd4584bb9e7073fa347633ed8bc7267fccfe", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-2.0.0-py3-none-any.whl", hash = "sha256:833374310e0a15880f3be9e6d082f527c9ac70129b2054d733da9b754315361f", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "c" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/b-1.0.0.tar.gz", hash = "sha256:3b8dcd0978bd51a2c96c4580a742545bfcf43ff64c664721b68cc96a71c489d4", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-1.0.0-py3-none-any.whl", hash = "sha256:b1299c4860508b15790e950862174413d29090c6e08d069fe70297a6d4db5ee0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "c"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/c-1.0.0.tar.gz", hash = "sha256:97fd5ea7c4a6535adc8b5d3eccdaa599d33edd9e4eccd922d6684b71171c829a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/c-1.0.0-py3-none-any.whl", hash = "sha256:a154f7c821c8a9448936b30bcb743e04fb4a187be48bc502e8c465f83a839a0e", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'darwin'" },
            { name = "a", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'linux'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a", marker = "sys_platform == 'darwin'", specifier = "<2" },
            { name = "a", marker = "sys_platform == 'linux'", specifier = ">=2" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// This tests that markers which provoked a fork in the universal resolver
/// are used to ignore dependencies which cannot possibly be installed by a
/// resolution produced by that fork.
///
/// In this example, the `a<2` dependency is only active on Darwin
/// platforms. But the `a==1.0.0` distribution has a dependency on `b`
/// that is only active on Linux, where as `a==2.0.0` does not. Therefore,
/// when the fork provoked by the `a<2` dependency considers `b`, it should
/// ignore it because it isn't possible for `sys_platform == 'linux'` and
/// `sys_platform == 'darwin'` to be simultaneously true.
///
///
/// ```text
/// fork-marker-inherit
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a>=2 ; sys_platform == 'linux'
/// │   │   └── satisfied by a-2.0.0
/// │   └── requires a<2 ; sys_platform == 'darwin'
/// │       └── satisfied by a-1.0.0
/// ├── a
/// │   ├── a-1.0.0
/// │   │   └── requires b ; sys_platform == 'linux'
/// │   │       └── satisfied by b-1.0.0
/// │   └── a-2.0.0
/// └── b
///     └── b-1.0.0
/// ```
#[test]
fn fork_marker_inherit() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/marker-inherit.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a>=2 ; sys_platform == 'linux'''',
          '''a<2 ; sys_platform == 'darwin'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "sys_platform == 'darwin'",
            "sys_platform == 'linux'",
            "sys_platform != 'darwin' and sys_platform != 'linux'",
        ]

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'darwin'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:bdb790e6d65140f316bfb33a6bc9ab03732245e8b5c6fd8efbcff7744530795d", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:0f04d2c483396a1af08a8baee4a39f08f4e4a1f9775c3b9bce5245852e864eba", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "a"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-2.0.0.tar.gz", hash = "sha256:9610291c2bd57390019f58ca72d0dd4584bb9e7073fa347633ed8bc7267fccfe", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-2.0.0-py3-none-any.whl", hash = "sha256:833374310e0a15880f3be9e6d082f527c9ac70129b2054d733da9b754315361f", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'darwin'" },
            { name = "a", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'linux'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a", marker = "sys_platform == 'darwin'", specifier = "<2" },
            { name = "a", marker = "sys_platform == 'linux'", specifier = ">=2" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// This is like `fork-marker-inherit`, but it tests that dependency
/// filtering only occurs in the context of a fork.
///
/// For example, as in `fork-marker-inherit`, the `c` dependency of
/// `a<2` should be entirely excluded here since it is possible for
/// `sys_platform` to be simultaneously equivalent to Darwin and Linux.
/// However, the unconditional dependency on `b`, which in turn depends on
/// `c` for Linux only, should still incorporate `c` as the dependency is
/// not part of any fork.
///
///
/// ```text
/// fork-marker-limited-inherit
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a>=2 ; sys_platform == 'linux'
/// │   │   └── satisfied by a-2.0.0
/// │   ├── requires a<2 ; sys_platform == 'darwin'
/// │   │   └── satisfied by a-1.0.0
/// │   └── requires b
/// │       └── satisfied by b-1.0.0
/// ├── a
/// │   ├── a-1.0.0
/// │   │   └── requires c ; sys_platform == 'linux'
/// │   │       └── satisfied by c-1.0.0
/// │   └── a-2.0.0
/// ├── b
/// │   └── b-1.0.0
/// │       └── requires c ; sys_platform == 'linux'
/// │           └── satisfied by c-1.0.0
/// └── c
///     └── c-1.0.0
/// ```
#[test]
fn fork_marker_limited_inherit() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/marker-limited-inherit.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a>=2 ; sys_platform == 'linux'''',
          '''a<2 ; sys_platform == 'darwin'''',
          '''b''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "sys_platform == 'darwin'",
            "sys_platform == 'linux'",
            "sys_platform != 'darwin' and sys_platform != 'linux'",
        ]

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'darwin'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:4afce24fb7b6b495a2e3521c84d8703e9e1e31faf88f086a6516db3fbc87f7cc", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:83b170dfb388aa657648396e796df2890a54c7125464b7087714da4ee7aaab3b", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "a"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-2.0.0.tar.gz", hash = "sha256:9610291c2bd57390019f58ca72d0dd4584bb9e7073fa347633ed8bc7267fccfe", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-2.0.0-py3-none-any.whl", hash = "sha256:833374310e0a15880f3be9e6d082f527c9ac70129b2054d733da9b754315361f", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "c", marker = "sys_platform == 'linux'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/b-1.0.0.tar.gz", hash = "sha256:de35bb11b581875ed4be3c930bcc4d98e7e9ac34d5fe678f74a27c2136b633f9", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-1.0.0-py3-none-any.whl", hash = "sha256:42ee7845ea1960c41a676a74c9add4f48915e78f031b0e0a7d3894a692b9b6dd", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "c"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/c-1.0.0.tar.gz", hash = "sha256:699a07ff61aab66fcba4883a94c6d2b61afb7797fa956ae36f2efdf30d9dfbc7", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/c-1.0.0-py3-none-any.whl", hash = "sha256:78c0da7c5681d751d38b2e60c78d1e29d6125d91e68e5aeb22372fa66527ff95", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'darwin'" },
            { name = "a", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'linux'" },
            { name = "b" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a", marker = "sys_platform == 'darwin'", specifier = "<2" },
            { name = "a", marker = "sys_platform == 'linux'", specifier = ">=2" },
            { name = "b" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// This tests a case where the resolver forks because of non-overlapping marker
/// expressions on `b`. In the original universal resolver implementation, this
/// resulted in multiple versions of `a` being unconditionally included in the lock
/// file. So this acts as a regression test to ensure that only one version of `a`
/// is selected.
///
///
/// ```text
/// fork-marker-selection
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a
/// │   │   ├── satisfied by a-0.1.0
/// │   │   └── satisfied by a-0.2.0
/// │   ├── requires b>=2 ; sys_platform == 'linux'
/// │   │   └── satisfied by b-2.0.0
/// │   └── requires b<2 ; sys_platform == 'darwin'
/// │       └── satisfied by b-1.0.0
/// ├── a
/// │   ├── a-0.1.0
/// │   └── a-0.2.0
/// │       └── requires b>=2.0.0
/// │           └── satisfied by b-2.0.0
/// └── b
///     ├── b-1.0.0
///     └── b-2.0.0
/// ```
#[test]
fn fork_marker_selection() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/marker-selection.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a''',
          '''b>=2 ; sys_platform == 'linux'''',
          '''b<2 ; sys_platform == 'darwin'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "sys_platform == 'darwin'",
            "sys_platform == 'linux'",
            "sys_platform != 'darwin' and sys_platform != 'linux'",
        ]

        [[package]]
        name = "a"
        version = "0.1.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/a-0.1.0.tar.gz", hash = "sha256:758dc8fff4646aa2c7f2ba2f32bdad3004625ce13ea474f163fe60bcb4d1d7d2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-0.1.0-py3-none-any.whl", hash = "sha256:40f95c8868f537e1a289e86f8d75e208fa22d1f3b46c42834f526e89630f77c0", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'darwin'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/b-1.0.0.tar.gz", hash = "sha256:9b42692ac74b4da0eed1ef248b9be6bb0557c49507ba3b38f862b191a06d959c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-1.0.0-py3-none-any.whl", hash = "sha256:a4c65510001153cab97a29ff219ad86e0d4330653ca89d9d4c84187ccf14c621", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/b-2.0.0.tar.gz", hash = "sha256:256a9af98c362451ef802d6462f06f8d4e26cc52543e8100cba63b584bae9a7c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-2.0.0-py3-none-any.whl", hash = "sha256:04cc57f7563029528b6d23283933b244b6f52ba1543fad54687c586d6e639fc4", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a" },
            { name = "b", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'darwin'" },
            { name = "b", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'linux'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a" },
            { name = "b", marker = "sys_platform == 'darwin'", specifier = "<2" },
            { name = "b", marker = "sys_platform == 'linux'", specifier = ">=2" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

///
///
/// ```text
/// fork-marker-track
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a
/// │   │   ├── satisfied by a-1.3.1
/// │   │   ├── satisfied by a-2.0.0
/// │   │   ├── satisfied by a-3.1.0
/// │   │   └── satisfied by a-4.3.0
/// │   ├── requires b>=2.8 ; sys_platform == 'linux'
/// │   │   └── satisfied by b-2.8
/// │   └── requires b<2.8 ; sys_platform == 'darwin'
/// │       └── satisfied by b-2.7
/// ├── a
/// │   ├── a-1.3.1
/// │   │   └── requires c ; implementation_name == 'iron'
/// │   │       └── satisfied by c-1.10
/// │   ├── a-2.0.0
/// │   │   ├── requires b>=2.8
/// │   │   │   └── satisfied by b-2.8
/// │   │   └── requires c ; implementation_name == 'cpython'
/// │   │       └── satisfied by c-1.10
/// │   ├── a-3.1.0
/// │   │   ├── requires b>=2.8
/// │   │   │   └── satisfied by b-2.8
/// │   │   └── requires c ; implementation_name == 'pypy'
/// │   │       └── satisfied by c-1.10
/// │   └── a-4.3.0
/// │       └── requires b>=2.8
/// │           └── satisfied by b-2.8
/// ├── b
/// │   ├── b-2.7
/// │   └── b-2.8
/// └── c
///     └── c-1.10
/// ```
#[test]
fn fork_marker_track() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/marker-track.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a''',
          '''b>=2.8 ; sys_platform == 'linux'''',
          '''b<2.8 ; sys_platform == 'darwin'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "sys_platform == 'darwin'",
            "sys_platform == 'linux'",
            "sys_platform != 'darwin' and sys_platform != 'linux'",
        ]

        [[package]]
        name = "a"
        version = "1.3.1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "c", marker = "implementation_name == 'iron'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-1.3.1.tar.gz", hash = "sha256:b46ebdc4ecc8c6670e8da12889df8c1bd286cbc8eb94f69e4626670e17b760c8", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.3.1-py3-none-any.whl", hash = "sha256:7720c67a8765ed540f4bdac4f8653210787d1ae518eb20ac541344ea8002f81c", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "2.7"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'darwin'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/b-2.7.tar.gz", hash = "sha256:c3e58feccc8d0cb3b8654491f51b4d53bf75edb0c8c5f3fd039570609acc3957", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-2.7-py3-none-any.whl", hash = "sha256:973b02bdc3039f4fa7347e091da60de892a0a18b26effd20a1e17fdcf24a6dcd", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "2.8"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/b-2.8.tar.gz", hash = "sha256:1f6eb422782a5730a466c3a9e0c149653751c64f6c4cdfb44a860d8d4eda2d24", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-2.8-py3-none-any.whl", hash = "sha256:ce8467e32ba82112ebbfbdb23ffd9b537ce7502f09ab70cf65716a1a89eb1683", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "c"
        version = "1.10"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/c-1.10.tar.gz", hash = "sha256:776b6806df1500e84d6b312aaf8d036a9d0d2ed4eb05a5ba52d6420311afd024", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/c-1.10-py3-none-any.whl", hash = "sha256:c830c4360164be9ea0027abb265a7f4f7894d8ac454942d98c09464d21a99b96", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a" },
            { name = "b", version = "2.7", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'darwin'" },
            { name = "b", version = "2.8", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'linux'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a" },
            { name = "b", marker = "sys_platform == 'darwin'", specifier = "<2.8" },
            { name = "b", marker = "sys_platform == 'linux'", specifier = ">=2.8" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// This is the same setup as `non-local-fork-marker-transitive`, but the disjoint
/// dependency specifications on `c` use the same constraints and thus depend on
/// the same version of `c`. In this case, there is no conflict.
///
///
/// ```text
/// fork-non-fork-marker-transitive
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a==1.0.0
/// │   │   └── satisfied by a-1.0.0
/// │   └── requires b==1.0.0
/// │       └── satisfied by b-1.0.0
/// ├── a
/// │   └── a-1.0.0
/// │       └── requires c>=2.0.0 ; sys_platform == 'linux'
/// │           └── satisfied by c-2.0.0
/// ├── b
/// │   └── b-1.0.0
/// │       └── requires c>=2.0.0 ; sys_platform == 'darwin'
/// │           └── satisfied by c-2.0.0
/// └── c
///     ├── c-1.0.0
///     └── c-2.0.0
/// ```
#[test]
fn fork_non_fork_marker_transitive() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/non-fork-marker-transitive.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a==1.0.0''',
          '''b==1.0.0''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "c", marker = "sys_platform == 'linux'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:cd24667d7a4725e13a59e180b76b7e932a074cc7a8a20a18d353db979f4b6707", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:6de66136e60c4e5c1832fdd217af472ad50d9ebd177f4014b9b3f50904f80f5e", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "c", marker = "sys_platform == 'darwin'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/b-1.0.0.tar.gz", hash = "sha256:d6993f77c784de42e150f111902a2c88a867c555077dacaa6c3a1b71398784a4", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-1.0.0-py3-none-any.whl", hash = "sha256:8cb0c9eaa95e2cf767bf5e6caf4fecfa1b2b9fc09be53c5768372704574ac244", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "c"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/c-2.0.0.tar.gz", hash = "sha256:98b5a57ae857516af05cd6bc5c3f74d31a78cd6559594a51b00b45c4e3891905", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/c-2.0.0-py3-none-any.whl", hash = "sha256:4a585f74490e3c09faafdb7df1ebb51d5e41c67b82ef08b5b5fd2f4c251b4b23", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a" },
            { name = "b" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a", specifier = "==1.0.0" },
            { name = "b", specifier = "==1.0.0" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// This is like `non-local-fork-marker-transitive`, but the marker expressions are
/// placed on sibling dependency specifications. However, the actual dependency on
/// `c` is indirect, and thus, there's no fork detected by the universal resolver.
/// This in turn results in an unresolvable conflict on `c`.
///
///
/// ```text
/// fork-non-local-fork-marker-direct
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a==1.0.0 ; sys_platform == 'linux'
/// │   │   └── satisfied by a-1.0.0
/// │   └── requires b==1.0.0 ; sys_platform == 'darwin'
/// │       └── satisfied by b-1.0.0
/// ├── a
/// │   └── a-1.0.0
/// │       └── requires c<2.0.0
/// │           └── satisfied by c-1.0.0
/// ├── b
/// │   └── b-1.0.0
/// │       └── requires c>=2.0.0
/// │           └── satisfied by c-2.0.0
/// └── c
///     ├── c-1.0.0
///     └── c-2.0.0
/// ```
#[test]
fn fork_non_local_fork_marker_direct() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/non-local-fork-marker-direct.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a==1.0.0 ; sys_platform == 'linux'''',
          '''b==1.0.0 ; sys_platform == 'darwin'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because all versions of b depend on c>=2.0.0 and all versions of a depend on c<2.0.0, we can conclude that all versions of a and all versions of b are incompatible.
             And because your project depends on a{sys_platform == 'linux'}==1.0.0 and b{sys_platform == 'darwin'}==1.0.0, we can conclude that your project's requirements are unsatisfiable.
    "
    );

    Ok(())
}

/// This setup introduces dependencies on two distinct versions of `c`, where
/// each such dependency has a marker expression attached that would normally
/// make them disjoint. In a non-universal resolver, this is no problem. But in a
/// forking resolver that tries to create one universal resolution, this can lead
/// to two distinct versions of `c` in the resolution. This is in and of itself
/// not a problem, since that is an expected scenario for universal resolution.
/// The problem in this case is that because the dependency specifications for
/// `c` occur in two different points (i.e., they are not sibling dependency
/// specifications) in the dependency graph, the forking resolver does not "detect"
/// it, and thus never forks and thus this results in "no resolution."
///
///
/// ```text
/// fork-non-local-fork-marker-transitive
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a==1.0.0
/// │   │   └── satisfied by a-1.0.0
/// │   └── requires b==1.0.0
/// │       └── satisfied by b-1.0.0
/// ├── a
/// │   └── a-1.0.0
/// │       └── requires c<2.0.0 ; sys_platform == 'linux'
/// │           └── satisfied by c-1.0.0
/// ├── b
/// │   └── b-1.0.0
/// │       └── requires c>=2.0.0 ; sys_platform == 'darwin'
/// │           └── satisfied by c-2.0.0
/// └── c
///     ├── c-1.0.0
///     └── c-2.0.0
/// ```
#[test]
fn fork_non_local_fork_marker_transitive() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/non-local-fork-marker-transitive.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a==1.0.0''',
          '''b==1.0.0''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because all versions of b depend on c{sys_platform == 'darwin'}>=2.0.0 and all versions of a depend on c{sys_platform == 'linux'}<2.0.0, we can conclude that all versions of a and all versions of b are incompatible.
             And because your project depends on a==1.0.0 and b==1.0.0, we can conclude that your project's requirements are unsatisfiable.
    "
    );

    Ok(())
}

/// This scenario tests a very basic case of overlapping markers. Namely,
/// it emulates a common pattern in the ecosystem where marker expressions
/// are used to progressively increase the version constraints of a package
/// as the Python version increases.
///
/// In this case, there is actually a split occurring between
/// `python_version < '3.13'` and the other marker expressions, so this
/// isn't just a scenario with overlapping but non-disjoint markers.
///
/// In particular, this serves as a regression test. uv used to create a
/// lock file with a dependency on `a` with the following markers:
///
///     python_version < '3.13' or python_version >= '3.14'
///
/// But this implies that `a` won't be installed for Python 3.13, which is
/// clearly wrong.
///
/// The issue was that uv was intersecting *all* marker expressions. So
/// that `a>=1.1.0` and `a>=1.2.0` fork was getting `python_version >=
/// '3.13' and python_version >= '3.14'`, which, of course, simplifies
/// to `python_version >= '3.14'`. But this is wrong! It should be
/// `python_version >= '3.13' or python_version >= '3.14'`, which of course
/// simplifies to `python_version >= '3.13'`. And thus, the resulting forks
/// are not just disjoint but complete in this case.
///
/// Since there are no other constraints on `a`, this causes uv to select
/// `1.2.0` unconditionally. (The marker expressions get normalized out
/// entirely.)
///
///
/// ```text
/// fork-overlapping-markers-basic
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a>=1.0.0 ; python_full_version < '3.13'
/// │   │   ├── satisfied by a-1.0.0
/// │   │   ├── satisfied by a-1.1.0
/// │   │   └── satisfied by a-1.2.0
/// │   ├── requires a>=1.1.0 ; python_full_version >= '3.13'
/// │   │   ├── satisfied by a-1.1.0
/// │   │   └── satisfied by a-1.2.0
/// │   └── requires a>=1.2.0 ; python_full_version >= '3.14'
/// │       └── satisfied by a-1.2.0
/// └── a
///     ├── a-1.0.0
///     ├── a-1.1.0
///     └── a-1.2.0
/// ```
#[test]
fn fork_overlapping_markers_basic() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/overlapping-markers-basic.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a>=1.0.0 ; python_full_version < '3.13'''',
          '''a>=1.1.0 ; python_full_version >= '3.13'''',
          '''a>=1.2.0 ; python_full_version >= '3.14'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "python_full_version >= '3.14'",
            "python_full_version == '3.13.*'",
            "python_full_version < '3.13'",
        ]

        [[package]]
        name = "a"
        version = "1.2.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/a-1.2.0.tar.gz", hash = "sha256:2e50354becbab0cc152f51e0ded5bdf4d7487237d8c1c825a151286117287c62", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.2.0-py3-none-any.whl", hash = "sha256:ac8736ef11e0594522369998752b2780be46e71e034c07d700c7bd0f2cea9863", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a", marker = "python_full_version < '3.13'", specifier = ">=1.0.0" },
            { name = "a", marker = "python_full_version >= '3.13'", specifier = ">=1.1.0" },
            { name = "a", marker = "python_full_version >= '3.14'", specifier = ">=1.2.0" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// This test contains a bistable resolution scenario when not using ahead-of-time
/// splitting of resolution forks: We meet one of two fork points depending on the
/// preferences, creating a resolution whose preferences lead us the other fork
/// point.
///
/// In the first case, we are in cleaver 2 and fork on `sys_platform`, in the
/// second case, we are in foo 1 or bar 1 amd fork over `os_name`.
///
/// First case: We select cleaver 2, fork on `sys_platform`, we reject cleaver 2
/// (missing fork `os_name`), we select cleaver 1 and don't fork on `os_name` in
/// `fork-if-not-forked`, done.
/// Second case: We have preference cleaver 1, fork on `os_name` in
/// `fork-if-not-forked`, we reject cleaver 1, we select cleaver 2, we fork on
/// `sys_platform`, we accept cleaver 2 since we forked on `os_name`, done.
///
///
/// ```text
/// preferences-dependent-forking-bistable
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   └── requires cleaver
/// │       ├── satisfied by cleaver-1.0.0
/// │       └── satisfied by cleaver-2.0.0
/// ├── cleaver
/// │   ├── cleaver-1.0.0
/// │   │   ├── requires fork-if-not-forked!=2 ; sys_platform == 'linux'
/// │   │   │   ├── satisfied by fork-if-not-forked-1.0.0
/// │   │   │   └── satisfied by fork-if-not-forked-3.0.0
/// │   │   ├── requires fork-if-not-forked-proxy ; sys_platform != 'linux'
/// │   │   │   └── satisfied by fork-if-not-forked-proxy-1.0.0
/// │   │   ├── requires reject-cleaver1==1 ; sys_platform == 'linux'
/// │   │   │   └── satisfied by reject-cleaver1-1.0.0
/// │   │   └── requires reject-cleaver1-proxy
/// │   │       └── satisfied by reject-cleaver1-proxy-1.0.0
/// │   └── cleaver-2.0.0
/// │       ├── requires fork-sys-platform==1 ; sys_platform == 'linux'
/// │       │   └── satisfied by fork-sys-platform-1.0.0
/// │       ├── requires fork-sys-platform==2 ; sys_platform != 'linux'
/// │       │   └── satisfied by fork-sys-platform-2.0.0
/// │       ├── requires reject-cleaver2==1 ; os_name == 'posix'
/// │       │   └── satisfied by reject-cleaver2-1.0.0
/// │       └── requires reject-cleaver2-proxy
/// │           └── satisfied by reject-cleaver2-proxy-1.0.0
/// ├── fork-if-not-forked
/// │   ├── fork-if-not-forked-1.0.0
/// │   │   ├── requires fork-os-name==1 ; os_name == 'posix'
/// │   │   │   └── satisfied by fork-os-name-1.0.0
/// │   │   ├── requires fork-os-name==2 ; os_name != 'posix'
/// │   │   │   └── satisfied by fork-os-name-2.0.0
/// │   │   └── requires reject-cleaver1-proxy
/// │   │       └── satisfied by reject-cleaver1-proxy-1.0.0
/// │   ├── fork-if-not-forked-2.0.0
/// │   └── fork-if-not-forked-3.0.0
/// ├── fork-if-not-forked-proxy
/// │   └── fork-if-not-forked-proxy-1.0.0
/// │       └── requires fork-if-not-forked!=3
/// │           ├── satisfied by fork-if-not-forked-1.0.0
/// │           └── satisfied by fork-if-not-forked-2.0.0
/// ├── fork-os-name
/// │   ├── fork-os-name-1.0.0
/// │   └── fork-os-name-2.0.0
/// ├── fork-sys-platform
/// │   ├── fork-sys-platform-1.0.0
/// │   └── fork-sys-platform-2.0.0
/// ├── reject-cleaver1
/// │   ├── reject-cleaver1-1.0.0
/// │   └── reject-cleaver1-2.0.0
/// ├── reject-cleaver1-proxy
/// │   └── reject-cleaver1-proxy-1.0.0
/// │       └── requires reject-cleaver1==2 ; sys_platform != 'linux'
/// │           └── satisfied by reject-cleaver1-2.0.0
/// ├── reject-cleaver2
/// │   ├── reject-cleaver2-1.0.0
/// │   └── reject-cleaver2-2.0.0
/// └── reject-cleaver2-proxy
///     └── reject-cleaver2-proxy-1.0.0
///         └── requires reject-cleaver2==2 ; os_name != 'posix'
///             └── satisfied by reject-cleaver2-2.0.0
/// ```
#[test]
fn preferences_dependent_forking_bistable() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/preferences-dependent-forking-bistable.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''cleaver''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 8 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "sys_platform == 'linux'",
            "sys_platform != 'linux'",
        ]

        [[package]]
        name = "cleaver"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "fork-if-not-forked", version = "3.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'linux'" },
            { name = "fork-if-not-forked-proxy", marker = "sys_platform != 'linux'" },
            { name = "reject-cleaver1", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'linux'" },
            { name = "reject-cleaver1-proxy" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/cleaver-1.0.0.tar.gz", hash = "sha256:e0bb339ac91a41ac2ce20db4866abf934c1be9c1a0b1f83efd701f9cf0a3da3c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/cleaver-1.0.0-py3-none-any.whl", hash = "sha256:c9c97d652936b7293b36e54194d448350852ffa6fbfad051ba0671db46803d7b", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "fork-if-not-forked"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform != 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/fork_if_not_forked-2.0.0.tar.gz", hash = "sha256:53bb0ea79f0eb0fc38598b1d6bf4f8e15fc6d61342570db99e6756eb01823223", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/fork_if_not_forked-2.0.0-py3-none-any.whl", hash = "sha256:0604f383a6d7cf8fd69c252c85bdb34967743de531b2b0f4084378f00f5b4ccf", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "fork-if-not-forked"
        version = "3.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/fork_if_not_forked-3.0.0.tar.gz", hash = "sha256:15eca5a5864638d4b9a6343b844eb13fec827ad5cd6412df7399e80e60028075", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/fork_if_not_forked-3.0.0-py3-none-any.whl", hash = "sha256:dcfb9267c7ae0868f4f40e29574665b22e336e666c89a542a074355f7bac643e", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "fork-if-not-forked-proxy"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "fork-if-not-forked", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        sdist = { url = "http://[LOCALHOST]/files/fork_if_not_forked_proxy-1.0.0.tar.gz", hash = "sha256:cc3c846677ae440eb6f6460e7d9d67b68fdacb21950da4123c604d440f83bdc2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/fork_if_not_forked_proxy-1.0.0-py3-none-any.whl", hash = "sha256:704030c720c21eee6ad9453ccd4d44d983182a73c057ed89325201c5dcead7fe", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "cleaver" },
        ]

        [package.metadata]
        requires-dist = [{ name = "cleaver" }]

        [[package]]
        name = "reject-cleaver1"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/reject_cleaver1-1.0.0.tar.gz", hash = "sha256:d7897b77030f920c3cda8ff08cc1949d73c24cb479f4dfdaf406c4f9ae9b2f44", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/reject_cleaver1-1.0.0-py3-none-any.whl", hash = "sha256:b3f3e0d98b07e6ec99719106244c07fa85658f0be9aa20f28c957aa8efcbeef8", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "reject-cleaver1"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform != 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/reject_cleaver1-2.0.0.tar.gz", hash = "sha256:4d82244f63049cc6441cbb1e570469e2e997edf28f132e30b1bf38f7ca037581", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/reject_cleaver1-2.0.0-py3-none-any.whl", hash = "sha256:db12d456e29ce239bbfed102b8b613a088adde579403d11ae91b15a08011efd2", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "reject-cleaver1-proxy"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "reject-cleaver1", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform != 'linux'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/reject_cleaver1_proxy-1.0.0.tar.gz", hash = "sha256:3ee85cdcbf1fccefe03ff06787c7b38733e194c769deaf6686cc6190cd7a1cfe", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/reject_cleaver1_proxy-1.0.0-py3-none-any.whl", hash = "sha256:8032f56f5eaddb524e079a75ab95b25220f916ac692c377a6977b58e61a3508d", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// Like `preferences-dependent-forking`, but when we don't fork the resolution fails.
///
/// Consider a fresh run without preferences:
/// * We start with cleaver 2
/// * We fork
/// * We reject cleaver 2
/// * We find cleaver solution in fork 1 with foo 2 with bar 1
/// * We find cleaver solution in fork 2 with foo 1 with bar 2
/// * We write cleaver 1, foo 1, foo 2, bar 1 and bar 2 to the lockfile
///
/// In a subsequent run, we read the preference cleaver 1 from the lockfile (the preferences for foo and bar don't matter):
/// * We start with cleaver 1
/// * We're in universal mode, cleaver requires foo 1, bar 1
/// * foo 1 requires bar 2, conflict
///
/// Design sketch:
/// ```text
/// root -> clear, foo, bar
/// # Cause a fork, then forget that version.
/// cleaver 2 -> unrelated-dep==1; fork==1
/// cleaver 2 -> unrelated-dep==2; fork==2
/// cleaver 2 -> reject-cleaver-2
/// # Allow different versions when forking, but force foo 1, bar 1 in universal mode without forking.
/// cleaver 1 -> foo==1; fork==1
/// cleaver 1 -> bar==1; fork==2
/// # When we selected foo 1, bar 1 in universal mode for cleaver, this causes a conflict, otherwise we select bar 2.
/// foo 1 -> bar==2
/// ```
///
///
/// ```text
/// preferences-dependent-forking-conflicting
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires bar
/// │   │   ├── satisfied by bar-1.0.0
/// │   │   └── satisfied by bar-2.0.0
/// │   ├── requires cleaver
/// │   │   ├── satisfied by cleaver-1.0.0
/// │   │   └── satisfied by cleaver-2.0.0
/// │   └── requires foo
/// │       ├── satisfied by foo-1.0.0
/// │       └── satisfied by foo-2.0.0
/// ├── bar
/// │   ├── bar-1.0.0
/// │   └── bar-2.0.0
/// ├── cleaver
/// │   ├── cleaver-1.0.0
/// │   │   ├── requires bar==1 ; sys_platform != 'linux'
/// │   │   │   └── satisfied by bar-1.0.0
/// │   │   └── requires foo==1 ; sys_platform == 'linux'
/// │   │       └── satisfied by foo-1.0.0
/// │   └── cleaver-2.0.0
/// │       ├── requires reject-cleaver-2
/// │       │   └── satisfied by reject-cleaver-2-1.0.0
/// │       ├── requires unrelated-dep==1 ; sys_platform == 'linux'
/// │       │   └── satisfied by unrelated-dep-1.0.0
/// │       └── requires unrelated-dep==2 ; sys_platform != 'linux'
/// │           └── satisfied by unrelated-dep-2.0.0
/// ├── foo
/// │   ├── foo-1.0.0
/// │   │   └── requires bar==2
/// │   │       └── satisfied by bar-2.0.0
/// │   └── foo-2.0.0
/// ├── reject-cleaver-2
/// │   └── reject-cleaver-2-1.0.0
/// │       └── requires unrelated-dep==3
/// │           └── satisfied by unrelated-dep-3.0.0
/// └── unrelated-dep
///     ├── unrelated-dep-1.0.0
///     ├── unrelated-dep-2.0.0
///     └── unrelated-dep-3.0.0
/// ```
#[test]
fn preferences_dependent_forking_conflicting() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/preferences-dependent-forking-conflicting.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''cleaver''',
          '''foo''',
          '''bar''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    "
    );

    Ok(())
}

/// This test case is like "preferences-dependent-forking-bistable", but with three
/// states instead of two. The first two locks are in a different state, then we
/// enter the tristable state.
///
/// It's not polished, but it's useful to have something with a higher period
/// than 2 in our test suite.
///
///
/// ```text
/// preferences-dependent-forking-tristable
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires bar
/// │   │   ├── satisfied by bar-1.0.0
/// │   │   └── satisfied by bar-2.0.0
/// │   ├── requires cleaver
/// │   │   ├── satisfied by cleaver-1.0.0
/// │   │   └── satisfied by cleaver-2.0.0
/// │   └── requires foo
/// │       ├── satisfied by foo-1.0.0
/// │       └── satisfied by foo-2.0.0
/// ├── a
/// │   └── a-1.0.0
/// │       └── requires unrelated-dep3==1 ; os_name == 'posix'
/// │           └── satisfied by unrelated-dep3-1.0.0
/// ├── b
/// │   └── b-1.0.0
/// │       └── requires unrelated-dep3==2 ; os_name != 'posix'
/// │           └── satisfied by unrelated-dep3-2.0.0
/// ├── bar
/// │   ├── bar-1.0.0
/// │   │   ├── requires c!=3 ; sys_platform == 'linux'
/// │   │   │   ├── satisfied by c-1.0.0
/// │   │   │   └── satisfied by c-2.0.0
/// │   │   ├── requires d ; sys_platform != 'linux'
/// │   │   │   └── satisfied by d-1.0.0
/// │   │   └── requires reject-cleaver-1
/// │   │       └── satisfied by reject-cleaver-1-1.0.0
/// │   └── bar-2.0.0
/// ├── c
/// │   ├── c-1.0.0
/// │   │   ├── requires reject-cleaver-1
/// │   │   │   └── satisfied by reject-cleaver-1-1.0.0
/// │   │   ├── requires unrelated-dep2==1 ; os_name == 'posix'
/// │   │   │   └── satisfied by unrelated-dep2-1.0.0
/// │   │   └── requires unrelated-dep2==2 ; os_name != 'posix'
/// │   │       └── satisfied by unrelated-dep2-2.0.0
/// │   ├── c-2.0.0
/// │   └── c-3.0.0
/// ├── cleaver
/// │   ├── cleaver-1.0.0
/// │   │   ├── requires bar==1 ; sys_platform != 'linux'
/// │   │   │   └── satisfied by bar-1.0.0
/// │   │   └── requires foo==1 ; sys_platform == 'linux'
/// │   │       └── satisfied by foo-1.0.0
/// │   └── cleaver-2.0.0
/// │       ├── requires a
/// │       │   └── satisfied by a-1.0.0
/// │       ├── requires b
/// │       │   └── satisfied by b-1.0.0
/// │       ├── requires unrelated-dep==1 ; sys_platform == 'linux'
/// │       │   └── satisfied by unrelated-dep-1.0.0
/// │       └── requires unrelated-dep==2 ; sys_platform != 'linux'
/// │           └── satisfied by unrelated-dep-2.0.0
/// ├── d
/// │   └── d-1.0.0
/// │       └── requires c!=2
/// │           ├── satisfied by c-1.0.0
/// │           └── satisfied by c-3.0.0
/// ├── foo
/// │   ├── foo-1.0.0
/// │   │   ├── requires c!=3 ; sys_platform == 'linux'
/// │   │   │   ├── satisfied by c-1.0.0
/// │   │   │   └── satisfied by c-2.0.0
/// │   │   ├── requires c!=2 ; sys_platform != 'linux'
/// │   │   │   ├── satisfied by c-1.0.0
/// │   │   │   └── satisfied by c-3.0.0
/// │   │   └── requires reject-cleaver-1
/// │   │       └── satisfied by reject-cleaver-1-1.0.0
/// │   └── foo-2.0.0
/// ├── reject-cleaver-1
/// │   └── reject-cleaver-1-1.0.0
/// │       ├── requires unrelated-dep2==1 ; sys_platform == 'linux'
/// │       │   └── satisfied by unrelated-dep2-1.0.0
/// │       └── requires unrelated-dep2==2 ; sys_platform != 'linux'
/// │           └── satisfied by unrelated-dep2-2.0.0
/// ├── reject-cleaver-2
/// │   └── reject-cleaver-2-1.0.0
/// │       └── requires unrelated-dep3==3
/// │           └── satisfied by unrelated-dep3-3.0.0
/// ├── unrelated-dep
/// │   ├── unrelated-dep-1.0.0
/// │   ├── unrelated-dep-2.0.0
/// │   └── unrelated-dep-3.0.0
/// ├── unrelated-dep2
/// │   ├── unrelated-dep2-1.0.0
/// │   ├── unrelated-dep2-2.0.0
/// │   └── unrelated-dep2-3.0.0
/// └── unrelated-dep3
///     ├── unrelated-dep3-1.0.0
///     ├── unrelated-dep3-2.0.0
///     └── unrelated-dep3-3.0.0
/// ```
#[test]
fn preferences_dependent_forking_tristable() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/preferences-dependent-forking-tristable.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''cleaver''',
          '''foo''',
          '''bar''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 11 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "sys_platform == 'linux'",
            "sys_platform != 'linux'",
        ]

        [[package]]
        name = "bar"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform != 'linux'",
        ]
        dependencies = [
            { name = "d" },
            { name = "reject-cleaver-1" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/bar-1.0.0.tar.gz", hash = "sha256:4e9b7cce3ef47387b3b7a08bd81e5bb0ac0a19a62e3a0abf64119c18014f917a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/bar-1.0.0-py3-none-any.whl", hash = "sha256:3e2cba35e1d80892bb436521134f100c183b8f64677be0ec80ff6ee4c710aa43", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "bar"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/bar-2.0.0.tar.gz", hash = "sha256:29e7bc76f76b7e939dcf1f8fe28b077c4631c11ecea673beb86d297dacda11eb", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/bar-2.0.0-py3-none-any.whl", hash = "sha256:563b1af3238a4ad819f2b95b74f940319a2ef30ed7991a2416fa98aa115da87d", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "c"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/c-2.0.0.tar.gz", hash = "sha256:98b5a57ae857516af05cd6bc5c3f74d31a78cd6559594a51b00b45c4e3891905", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/c-2.0.0-py3-none-any.whl", hash = "sha256:4a585f74490e3c09faafdb7df1ebb51d5e41c67b82ef08b5b5fd2f4c251b4b23", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "c"
        version = "3.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform != 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/c-3.0.0.tar.gz", hash = "sha256:c01237d1b0816abee804906c0b118675aaa1aa1fdbb6e756c350ecc6ed500ebb", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/c-3.0.0-py3-none-any.whl", hash = "sha256:1e031928d6a855d65842904337ad7d984de1a64473b1166d548afbfe5a397341", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "cleaver"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "bar", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform != 'linux'" },
            { name = "foo", marker = "sys_platform == 'linux'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/cleaver-1.0.0.tar.gz", hash = "sha256:e48e43a500c95e61d1f1e18d830ec1df0ed3065842738493669f850a2c3da9ad", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/cleaver-1.0.0-py3-none-any.whl", hash = "sha256:f49d93330cfe3f7096636c506aa522a497eae44d08b07f6276caf784ec87f65b", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "d"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "c", version = "3.0.0", source = { registry = "http://[LOCALHOST]/simple/" } },
        ]
        sdist = { url = "http://[LOCALHOST]/files/d-1.0.0.tar.gz", hash = "sha256:41ae55064b4daf92964c6428bb14b433ef5ac82b96ddb2868460ac862b3e80c9", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/d-1.0.0-py3-none-any.whl", hash = "sha256:890292cd440f4d868e46d5e883bd2eee4ced094d39e44a7cd88f7d9f5049f5ef", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "foo"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "c", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'linux'" },
            { name = "c", version = "3.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform != 'linux'" },
            { name = "reject-cleaver-1" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/foo-1.0.0.tar.gz", hash = "sha256:693cd5b07b84a596dc7595d47d3bedebd7540b5455eedb4fa7ddf4196bbcb205", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/foo-1.0.0-py3-none-any.whl", hash = "sha256:a6a8594d7d818843d31479b216b325e9140b6c5b180a55427b03fd85bbd261ad", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "bar", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform != 'linux'" },
            { name = "bar", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'linux'" },
            { name = "cleaver" },
            { name = "foo" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "bar" },
            { name = "cleaver" },
            { name = "foo" },
        ]

        [[package]]
        name = "reject-cleaver-1"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "unrelated-dep2", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'linux'" },
            { name = "unrelated-dep2", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform != 'linux'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/reject_cleaver_1-1.0.0.tar.gz", hash = "sha256:c707cb6622bed86ca850cecb72a1e03e2dfef87b7f19018684f0530d5532cdb2", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/reject_cleaver_1-1.0.0-py3-none-any.whl", hash = "sha256:5b4d622444188c423a9476bb5c92358d7322f121d6e764c4b8d8bdc14e9f4017", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "unrelated-dep2"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/unrelated_dep2-1.0.0.tar.gz", hash = "sha256:6154dcfd6aa5dc62404702d8d66e908c7482a6107148323259be64e0268a1885", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/unrelated_dep2-1.0.0-py3-none-any.whl", hash = "sha256:a485b74955fe703c7c525c52ef72da556100f2ea4b49ca97e6fd6f183469fb43", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "unrelated-dep2"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform != 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/unrelated_dep2-2.0.0.tar.gz", hash = "sha256:b58f99b95e60c3b85f930cb1fae95ef3baba23e85eca0db721f82ad4e8c32cf6", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/unrelated_dep2-2.0.0-py3-none-any.whl", hash = "sha256:f666a4b92a10879856a6962ef582a2ab328edd174b8def895d859b10bfea49b6", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// This test contains a scenario where the solution depends on whether we fork, and whether we fork depends on the
/// preferences.
///
/// Consider a fresh run without preferences:
/// * We start with cleaver 2
/// * We fork
/// * We reject cleaver 2
/// * We find cleaver solution in fork 1 with foo 2 with bar 1
/// * We find cleaver solution in fork 2 with foo 1 with bar 2
/// * We write cleaver 1, foo 1, foo 2, bar 1 and bar 2 to the lockfile
///
/// In a subsequent run, we read the preference cleaver 1 from the lockfile (the preferences for foo and bar don't matter):
/// * We start with cleaver 1
/// * We're in universal mode, we resolve foo 1 and bar 1
/// * We write cleaver 1 and bar 1 to the lockfile
///
/// We call a resolution that's different on the second run to the first unstable.
///
/// Design sketch:
/// ```text
/// root -> clear, foo, bar
/// # Cause a fork, then forget that version.
/// cleaver 2 -> unrelated-dep==1; fork==1
/// cleaver 2 -> unrelated-dep==2; fork==2
/// cleaver 2 -> reject-cleaver-2
/// # Allow different versions when forking, but force foo 1, bar 1 in universal mode without forking.
/// cleaver 1 -> foo==1; fork==1
/// cleaver 1 -> bar==1; fork==2
/// ```
///
///
/// ```text
/// preferences-dependent-forking
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires bar
/// │   │   ├── satisfied by bar-1.0.0
/// │   │   └── satisfied by bar-2.0.0
/// │   ├── requires cleaver
/// │   │   ├── satisfied by cleaver-1.0.0
/// │   │   └── satisfied by cleaver-2.0.0
/// │   └── requires foo
/// │       ├── satisfied by foo-1.0.0
/// │       └── satisfied by foo-2.0.0
/// ├── bar
/// │   ├── bar-1.0.0
/// │   └── bar-2.0.0
/// ├── cleaver
/// │   ├── cleaver-1.0.0
/// │   │   ├── requires bar==1 ; sys_platform != 'linux'
/// │   │   │   └── satisfied by bar-1.0.0
/// │   │   └── requires foo==1 ; sys_platform == 'linux'
/// │   │       └── satisfied by foo-1.0.0
/// │   └── cleaver-2.0.0
/// │       ├── requires reject-cleaver-2
/// │       │   └── satisfied by reject-cleaver-2-1.0.0
/// │       ├── requires unrelated-dep==1 ; sys_platform == 'linux'
/// │       │   └── satisfied by unrelated-dep-1.0.0
/// │       └── requires unrelated-dep==2 ; sys_platform != 'linux'
/// │           └── satisfied by unrelated-dep-2.0.0
/// ├── foo
/// │   ├── foo-1.0.0
/// │   └── foo-2.0.0
/// ├── reject-cleaver-2
/// │   └── reject-cleaver-2-1.0.0
/// │       └── requires unrelated-dep==3
/// │           └── satisfied by unrelated-dep-3.0.0
/// └── unrelated-dep
///     ├── unrelated-dep-1.0.0
///     ├── unrelated-dep-2.0.0
///     └── unrelated-dep-3.0.0
/// ```
#[test]
fn preferences_dependent_forking() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/preferences-dependent-forking.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''cleaver''',
          '''foo''',
          '''bar''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "sys_platform == 'linux'",
            "sys_platform != 'linux'",
        ]

        [[package]]
        name = "bar"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform != 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/bar-1.0.0.tar.gz", hash = "sha256:bb9cb9098cc77ebe1f2085af0859f2332ab631348e58c687fa344aea81eb4043", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/bar-1.0.0-py3-none-any.whl", hash = "sha256:2fbf0e0a7dd4f48a8b1c2148f73ba0c314699d0bfa0a8ec9b9bcb8105882e9fc", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "bar"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/bar-2.0.0.tar.gz", hash = "sha256:29e7bc76f76b7e939dcf1f8fe28b077c4631c11ecea673beb86d297dacda11eb", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/bar-2.0.0-py3-none-any.whl", hash = "sha256:563b1af3238a4ad819f2b95b74f940319a2ef30ed7991a2416fa98aa115da87d", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "cleaver"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "bar", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform != 'linux'" },
            { name = "foo", marker = "sys_platform == 'linux'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/cleaver-1.0.0.tar.gz", hash = "sha256:e48e43a500c95e61d1f1e18d830ec1df0ed3065842738493669f850a2c3da9ad", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/cleaver-1.0.0-py3-none-any.whl", hash = "sha256:f49d93330cfe3f7096636c506aa522a497eae44d08b07f6276caf784ec87f65b", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "foo"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/foo-1.0.0.tar.gz", hash = "sha256:70bd56242b5a5c7f6c04694a8ed2aafdb036726de0fcca1dd1d2f1f467c71ee1", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/foo-1.0.0-py3-none-any.whl", hash = "sha256:df9a39d54a6d71872deb1537d7e331de0ede3b725d630a050fee3efd3fe5145b", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "bar", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform != 'linux'" },
            { name = "bar", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'linux'" },
            { name = "cleaver" },
            { name = "foo" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "bar" },
            { name = "cleaver" },
            { name = "foo" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// This scenario tries to check that the "remaining universe" handling in
/// the universal resolver is correct. Namely, whenever we create forks
/// from disjoint markers that don't union to the universe, we need to
/// create *another* fork corresponding to the difference between the
/// universe and the union of the forks.
///
/// But when we do this, that remaining universe fork needs to be created
/// like any other fork: it should start copying whatever set of forks
/// existed by the time we got to this point, intersecting the markers with
/// the markers describing the remaining universe and then filtering out
/// any dependencies that are disjoint with the resulting markers.
///
/// This test exercises that logic by ensuring that a package `z` in the
/// remaining universe is excluded based on the combination of markers
/// from a parent fork. That is, if the remaining universe fork does not
/// pick up the markers from the parent forks, then `z` would be included
/// because the remaining universe for _just_ the `b` dependencies of `a`
/// is `os_name != 'linux' and os_name != 'darwin'`, which is satisfied by
/// `z`'s marker of `sys_platform == 'windows'`. However, `a 1.0.0` is only
/// selected in the context of `a < 2 ; sys_platform == 'illumos'`, so `z`
/// should never appear in the resolution.
///
///
/// ```text
/// fork-remaining-universe-partitioning
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a>=2 ; sys_platform == 'windows'
/// │   │   └── satisfied by a-2.0.0
/// │   └── requires a<2 ; sys_platform == 'illumos'
/// │       └── satisfied by a-1.0.0
/// ├── a
/// │   ├── a-1.0.0
/// │   │   ├── requires b>=2 ; os_name == 'linux'
/// │   │   │   └── satisfied by b-2.0.0
/// │   │   ├── requires b<2 ; os_name == 'darwin'
/// │   │   │   └── satisfied by b-1.0.0
/// │   │   └── requires z ; sys_platform == 'windows'
/// │   │       └── satisfied by z-1.0.0
/// │   └── a-2.0.0
/// ├── b
/// │   ├── b-1.0.0
/// │   └── b-2.0.0
/// └── z
///     └── z-1.0.0
/// ```
#[test]
fn fork_remaining_universe_partitioning() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/remaining-universe-partitioning.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a>=2 ; sys_platform == 'windows'''',
          '''a<2 ; sys_platform == 'illumos'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "os_name == 'darwin' and sys_platform == 'illumos'",
            "os_name == 'linux' and sys_platform == 'illumos'",
            "os_name != 'darwin' and os_name != 'linux' and sys_platform == 'illumos'",
            "sys_platform == 'windows'",
            "sys_platform != 'illumos' and sys_platform != 'windows'",
        ]

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "os_name == 'darwin' and sys_platform == 'illumos'",
            "os_name == 'linux' and sys_platform == 'illumos'",
            "os_name != 'darwin' and os_name != 'linux' and sys_platform == 'illumos'",
        ]
        dependencies = [
            { name = "b", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "os_name == 'darwin'" },
            { name = "b", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "os_name == 'linux'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:217554d13af0a280cb3f5d95653c460bfdda5ea14c36a48e64fbaf39c7c6e16c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:de627f2f3dc58918c496b5b61036d5a8c19c769fa5c933889bd805366fa2de1d", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "a"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'windows'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-2.0.0.tar.gz", hash = "sha256:9610291c2bd57390019f58ca72d0dd4584bb9e7073fa347633ed8bc7267fccfe", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-2.0.0-py3-none-any.whl", hash = "sha256:833374310e0a15880f3be9e6d082f527c9ac70129b2054d733da9b754315361f", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "os_name == 'darwin' and sys_platform == 'illumos'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/b-1.0.0.tar.gz", hash = "sha256:9b42692ac74b4da0eed1ef248b9be6bb0557c49507ba3b38f862b191a06d959c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-1.0.0-py3-none-any.whl", hash = "sha256:a4c65510001153cab97a29ff219ad86e0d4330653ca89d9d4c84187ccf14c621", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "2.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "os_name == 'linux' and sys_platform == 'illumos'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/b-2.0.0.tar.gz", hash = "sha256:256a9af98c362451ef802d6462f06f8d4e26cc52543e8100cba63b584bae9a7c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-2.0.0-py3-none-any.whl", hash = "sha256:04cc57f7563029528b6d23283933b244b6f52ba1543fad54687c586d6e639fc4", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'illumos'" },
            { name = "a", version = "2.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'windows'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a", marker = "sys_platform == 'illumos'", specifier = "<2" },
            { name = "a", marker = "sys_platform == 'windows'", specifier = ">=2" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// This tests that a `Requires-Python` specifier will result in the
/// exclusion of dependency specifications that cannot possibly satisfy it.
///
/// In particular, this is tested via the `python_full_version` marker with
/// a pre-release version.
///
///
/// ```text
/// fork-requires-python-full-prerelease
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   └── requires a==1.0.0 ; python_full_version == '3.9'
/// │       └── satisfied by a-1.0.0
/// └── a
///     └── a-1.0.0
/// ```
#[test]
fn fork_requires_python_full_prerelease() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/requires-python-full-prerelease.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a==1.0.0 ; python_full_version == '3.9'''',
        ]
        requires-python = ">=3.10"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.10"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.metadata]
        requires-dist = [{ name = "a", marker = "python_full_version == '3.9'", specifier = "==1.0.0" }]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// This tests that a `Requires-Python` specifier will result in the
/// exclusion of dependency specifications that cannot possibly satisfy it.
///
/// In particular, this is tested via the `python_full_version` marker
/// instead of the more common `python_version` marker.
///
///
/// ```text
/// fork-requires-python-full
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   └── requires a==1.0.0 ; python_full_version == '3.9'
/// │       └── satisfied by a-1.0.0
/// └── a
///     └── a-1.0.0
/// ```
#[test]
fn fork_requires_python_full() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/requires-python-full.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a==1.0.0 ; python_full_version == '3.9'''',
        ]
        requires-python = ">=3.10"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.10"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.metadata]
        requires-dist = [{ name = "a", marker = "python_full_version == '3.9'", specifier = "==1.0.0" }]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// This tests that a `Requires-Python` specifier that includes a Python
/// patch version will not result in excluded a dependency specification
/// with a `python_version == '3.10'` marker.
///
/// This is a regression test for the universal resolver where it would
/// convert a `Requires-Python: >=3.10.1` specifier into a
/// `python_version >= '3.10.1'` marker expression, which would be
/// considered disjoint with `python_version == '3.10'`. Thus, the
/// dependency `a` below was erroneously excluded. It should be included.
///
///
/// ```text
/// fork-requires-python-patch-overlap
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   └── requires a==1.0.0 ; python_full_version == '3.10.*'
/// │       └── satisfied by a-1.0.0
/// └── a
///     └── a-1.0.0
///         └── requires python>=3.10
/// ```
#[test]
fn fork_requires_python_patch_overlap() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/requires-python-patch-overlap.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a==1.0.0 ; python_full_version == '3.10.*'''',
        ]
        requires-python = ">=3.10.1"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.10.1"

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:f49e4cc76cd214a2a67efbe254cd317fa72d09cc98cd3d06537083d987284267", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:277ee4f1bb98d9591ba3a3a28364d1f00a54384c08d8f32100afc01ae76491a5", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a", marker = "python_full_version < '3.11'" },
        ]

        [package.metadata]
        requires-dist = [{ name = "a", marker = "python_full_version == '3.10.*'", specifier = "==1.0.0" }]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// This tests that a `Requires-Python` specifier will result in the
/// exclusion of dependency specifications that cannot possibly satisfy it.
///
///
/// ```text
/// fork-requires-python
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   └── requires a==1.0.0 ; python_full_version == '3.9.*'
/// │       └── satisfied by a-1.0.0
/// └── a
///     └── a-1.0.0
/// ```
#[test]
fn fork_requires_python() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("fork/requires-python.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a==1.0.0 ; python_full_version == '3.9.*'''',
        ]
        requires-python = ">=3.10"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.10"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.metadata]
        requires-dist = [{ name = "a", marker = "python_full_version == '3.9.*'", specifier = "==1.0.0" }]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// Base and marker requirements retain the stable preference within the marker fork when the explicit requirement appears first.
///
/// ```text
/// prerelease-base-marker-stable-preference-explicit-first
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires c>=0.5a1 ; sys_platform == 'linux'
/// │   │   ├── satisfied by c-1.0.0
/// │   │   └── satisfied by c-2.0.0a1
/// │   └── requires c>=1.0
/// │       ├── satisfied by c-1.0.0
/// │       └── satisfied by c-2.0.0a1
/// └── c
///     ├── c-1.0.0
///     └── c-2.0.0a1
/// ```
#[test]
fn prerelease_base_marker_stable_preference_explicit_first() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new(
        "prereleases/prerelease-base-marker-stable-preference-explicit-first.toml",
    );

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''c>=0.5a1 ; sys_platform == 'linux'''',
          '''c>=1.0''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    // All platforms select the stable release of `c`, regardless of the explicit pre-release specifier or declaration order.
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "sys_platform == 'linux'",
            "sys_platform != 'linux'",
        ]

        [[package]]
        name = "c"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/c-1.0.0.tar.gz", hash = "sha256:699a07ff61aab66fcba4883a94c6d2b61afb7797fa956ae36f2efdf30d9dfbc7", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/c-1.0.0-py3-none-any.whl", hash = "sha256:78c0da7c5681d751d38b2e60c78d1e29d6125d91e68e5aeb22372fa66527ff95", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "c" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "c", specifier = ">=1.0" },
            { name = "c", marker = "sys_platform == 'linux'", specifier = ">=0.5a1" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// Base and marker requirements retain the stable preference within the marker fork when the plain requirement appears first.
///
/// ```text
/// prerelease-base-marker-stable-preference-plain-first
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires c>=1.0
/// │   │   ├── satisfied by c-1.0.0
/// │   │   └── satisfied by c-2.0.0a1
/// │   └── requires c>=0.5a1 ; sys_platform == 'linux'
/// │       ├── satisfied by c-1.0.0
/// │       └── satisfied by c-2.0.0a1
/// └── c
///     ├── c-1.0.0
///     └── c-2.0.0a1
/// ```
#[test]
fn prerelease_base_marker_stable_preference_plain_first() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server =
        PackseServer::new("prereleases/prerelease-base-marker-stable-preference-plain-first.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''c>=1.0''',
          '''c>=0.5a1 ; sys_platform == 'linux'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    // All platforms select the stable release of `c`, regardless of the explicit pre-release specifier or declaration order.
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "sys_platform == 'linux'",
            "sys_platform != 'linux'",
        ]

        [[package]]
        name = "c"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/c-1.0.0.tar.gz", hash = "sha256:699a07ff61aab66fcba4883a94c6d2b61afb7797fa956ae36f2efdf30d9dfbc7", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/c-1.0.0-py3-none-any.whl", hash = "sha256:78c0da7c5681d751d38b2e60c78d1e29d6125d91e68e5aeb22372fa66527ff95", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "c" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "c", specifier = ">=1.0" },
            { name = "c", marker = "sys_platform == 'linux'", specifier = ">=0.5a1" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// Requirements with the same marker retain the stable preference within their fork.
///
/// ```text
/// prerelease-marker-equivalent-stable-preference
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires c>=1.0 ; sys_platform == 'linux'
/// │   │   ├── satisfied by c-1.0.0
/// │   │   └── satisfied by c-2.0.0a1
/// │   └── requires c>=0.5a1 ; sys_platform == 'linux'
/// │       ├── satisfied by c-1.0.0
/// │       └── satisfied by c-2.0.0a1
/// └── c
///     ├── c-1.0.0
///     └── c-2.0.0a1
/// ```
#[test]
fn prerelease_marker_equivalent_stable_preference() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server =
        PackseServer::new("prereleases/prerelease-marker-equivalent-stable-preference.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''c>=1.0 ; sys_platform == 'linux'''',
          '''c>=0.5a1 ; sys_platform == 'linux'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    // The equivalent Linux requirements prefer `c==1.0.0` in the fork where they apply.
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [[package]]
        name = "c"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/c-1.0.0.tar.gz", hash = "sha256:699a07ff61aab66fcba4883a94c6d2b61afb7797fa956ae36f2efdf30d9dfbc7", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/c-1.0.0-py3-none-any.whl", hash = "sha256:78c0da7c5681d751d38b2e60c78d1e29d6125d91e68e5aeb22372fa66527ff95", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "c", marker = "sys_platform == 'linux'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "c", marker = "sys_platform == 'linux'", specifier = ">=0.5a1" },
            { name = "c", marker = "sys_platform == 'linux'", specifier = ">=1.0" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// A marker-scoped pre-release requirement from a rejected parent does not affect a plain alternate parent.
///
/// ```text
/// prerelease-marker-stable-preference-backtracks
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a
/// │   │   ├── satisfied by a-1.0.0
/// │   │   └── satisfied by a-2.0.0
/// │   └── requires d==1.0.0
/// │       └── satisfied by d-1.0.0
/// ├── a
/// │   ├── a-1.0.0
/// │   │   └── requires c>=1.0 ; sys_platform == 'linux'
/// │   │       ├── satisfied by c-1.0.0
/// │   │       └── satisfied by c-2.0.0a1
/// │   └── a-2.0.0
/// │       ├── requires c>=1.0 ; sys_platform == 'linux'
/// │       │   ├── satisfied by c-1.0.0
/// │       │   └── satisfied by c-2.0.0a1
/// │       ├── requires c>=0.5a1 ; sys_platform == 'linux'
/// │       │   ├── satisfied by c-1.0.0
/// │       │   └── satisfied by c-2.0.0a1
/// │       └── requires d==2.0.0
/// │           └── satisfied by d-2.0.0
/// ├── c
/// │   ├── c-1.0.0
/// │   └── c-2.0.0a1
/// └── d
///     ├── d-1.0.0
///     └── d-2.0.0
/// ```
#[test]
fn prerelease_marker_stable_preference_backtracks() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server =
        PackseServer::new("prereleases/prerelease-marker-stable-preference-backtracks.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a''',
          '''d==1.0.0''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    // The rejected parent version has a Linux pre-release requirement, but the selected alternate parent retains only its plain requirement and selects stable `c==1.0.0`.
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "c", marker = "sys_platform == 'linux'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:3af88a63bb82356a9dc643e557148dd6a50d20b0f74ee15828c7ba066be890cb", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:e5c939c5714630ae301b70dc2ba362c40f3ad6608cbc85035ee2c65be1f6ad70", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "c"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/c-1.0.0.tar.gz", hash = "sha256:699a07ff61aab66fcba4883a94c6d2b61afb7797fa956ae36f2efdf30d9dfbc7", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/c-1.0.0-py3-none-any.whl", hash = "sha256:78c0da7c5681d751d38b2e60c78d1e29d6125d91e68e5aeb22372fa66527ff95", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "d"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/d-1.0.0.tar.gz", hash = "sha256:bbb9d05b6de19e47de8e49fcc69483e76cd868bd556c8173680756d53e6997d4", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/d-1.0.0-py3-none-any.whl", hash = "sha256:362166e5bd895367cc4ba5b7327949b7d417fe30cb3273a76b5db4a280dac05d", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a" },
            { name = "d" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a" },
            { name = "d", specifier = "==1.0.0" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// A transitive pre-release requirement applies only to the universal-resolution fork in which its marker is active.
///
/// ```text
/// transitive-prerelease-forks
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   └── requires a
/// │       └── satisfied by a-1.0.0
/// ├── a
/// │   └── a-1.0.0
/// │       ├── requires c>=2.0.0b1 ; sys_platform == 'linux'
/// │       │   └── satisfied by c-2.0.0b1
/// │       └── requires c==1.0.0 ; sys_platform != 'linux'
/// │           └── satisfied by c-1.0.0
/// └── c
///     ├── c-1.0.0
///     └── c-2.0.0b1
/// ```
#[test]
fn transitive_prerelease_forks() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("prereleases/transitive-prerelease-forks.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    // Linux selects the required pre-release of `c`, while other platforms retain the stable release.
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "sys_platform != 'linux'",
            "sys_platform == 'linux'",
        ]

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "c", version = "1.0.0", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform != 'linux'" },
            { name = "c", version = "2.0.0b1", source = { registry = "http://[LOCALHOST]/simple/" }, marker = "sys_platform == 'linux'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:70f26f0ea30b0899e8c0e946a6b2c4d57dd4208a028da1a990cde29f4eed5d77", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:392d0dd8be953713399938d2dacc8a361d99eb376285adfa4f0f0e4d448ace2a", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "c"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform != 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/c-1.0.0.tar.gz", hash = "sha256:699a07ff61aab66fcba4883a94c6d2b61afb7797fa956ae36f2efdf30d9dfbc7", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/c-1.0.0-py3-none-any.whl", hash = "sha256:78c0da7c5681d751d38b2e60c78d1e29d6125d91e68e5aeb22372fa66527ff95", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "c"
        version = "2.0.0b1"
        source = { registry = "http://[LOCALHOST]/simple/" }
        resolution-markers = [
            "sys_platform == 'linux'",
        ]
        sdist = { url = "http://[LOCALHOST]/files/c-2.0.0b1.tar.gz", hash = "sha256:0d71a5a80e03d71f520e78b06ca94b2f90247332e39c7b90233591fc33797e36", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/c-2.0.0b1-py3-none-any.whl", hash = "sha256:4afb52babcf2d595eccb483f1b6a641ca25daa2015cd97c519eb9357cc9d69a7", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a" },
        ]

        [package.metadata]
        requires-dist = [{ name = "a" }]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// Check that we only include wheels that match the required Python version
///
/// ```text
/// requires-python-wheels
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   └── requires a==1.0.0
/// │       └── satisfied by a-1.0.0
/// └── a
///     └── a-1.0.0
///         └── requires python>=3.10
/// ```
#[test]
fn requires_python_wheels() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("tag_and_markers/requires-python-wheels.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a==1.0.0''',
        ]
        requires-python = ">=3.10"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.10"

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:f49e4cc76cd214a2a67efbe254cd317fa72d09cc98cd3d06537083d987284267", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-cp310-cp310-any.whl", hash = "sha256:34c6734e2427cc772605ac6710cf4e95c3556fd191b308b65c4d5f056fc95530", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/a-1.0.0-cp311-cp311-any.whl", hash = "sha256:a3aca28e2bd4f75c53a651a42fd54661d8aec43c1f9a4c703b3a301ad9deb5e2", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a" },
        ]

        [package.metadata]
        requires-dist = [{ name = "a", specifier = "==1.0.0" }]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// `c` is not reachable due to the markers, it should be excluded from the lockfile
///
/// ```text
/// unreachable-package
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   └── requires a==1.0.0 ; sys_platform == 'win32'
/// │       └── satisfied by a-1.0.0
/// ├── a
/// │   └── a-1.0.0
/// │       └── requires b==1.0.0 ; sys_platform == 'linux'
/// │           └── satisfied by b-1.0.0
/// └── b
///     └── b-1.0.0
/// ```
#[test]
fn unreachable_package() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("tag_and_markers/unreachable-package.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a==1.0.0 ; sys_platform == 'win32'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:9ea08e4e8cc22657585ae7aab665a536fd45dab0f612b3f7cf516aee045058ca", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:9c02f18ae50a1da3421fa31399fe1497172375a7a6c80e8d1485ed12c3e64ee3", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a", marker = "sys_platform == 'win32'" },
        ]

        [package.metadata]
        requires-dist = [{ name = "a", marker = "sys_platform == 'win32'", specifier = "==1.0.0" }]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// Check that we only include wheels that match the platform markers
///
/// ```text
/// unreachable-wheels
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a==1.0.0 ; sys_platform == 'win32'
/// │   │   └── satisfied by a-1.0.0
/// │   ├── requires b==1.0.0 ; sys_platform == 'linux'
/// │   │   └── satisfied by b-1.0.0
/// │   └── requires c==1.0.0 ; sys_platform == 'darwin'
/// │       └── satisfied by c-1.0.0
/// ├── a
/// │   └── a-1.0.0
/// ├── b
/// │   └── b-1.0.0
/// └── c
///     └── c-1.0.0
/// ```
#[test]
fn unreachable_wheels() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("tag_and_markers/unreachable-wheels.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a==1.0.0 ; sys_platform == 'win32'''',
          '''b==1.0.0 ; sys_platform == 'linux'''',
          '''c==1.0.0 ; sys_platform == 'darwin'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:957f99ff1d65ce0d7883d50f4e67ed8d4b42e76d2c2b5e62384ff0ba538647b5", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-cp312-cp312-win_amd64.whl", hash = "sha256:559a9c629536d99c1213064c303dcc6ee8dd2917824c0a0f6549f94d0be02abd", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/b-1.0.0.tar.gz", hash = "sha256:9b42692ac74b4da0eed1ef248b9be6bb0557c49507ba3b38f862b191a06d959c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-1.0.0-cp312-cp312-manylinux_2_17_x86_64.manylinux2014_x86_64.whl", hash = "sha256:4d0fc532c5d6e2f11ca522dfd3684b50b132dd1db78fba3ad510023f4d5830b7", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/b-1.0.0-cp312-cp312-musllinux_1_1_armv7l.whl", hash = "sha256:8f328487f947ee5119d348f2c73ab9119dd29349b840f6f26e0d351245d6084b", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "c"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/c-1.0.0.tar.gz", hash = "sha256:699a07ff61aab66fcba4883a94c6d2b61afb7797fa956ae36f2efdf30d9dfbc7", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/c-1.0.0-cp312-cp312-macosx_14_0_x86_64.whl", hash = "sha256:a25d2ba9ff1417f63313691a2f686f30b617a8696329023e44b7f7fc96573598", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a", marker = "sys_platform == 'win32'" },
            { name = "b", marker = "sys_platform == 'linux'" },
            { name = "c", marker = "sys_platform == 'darwin'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a", marker = "sys_platform == 'win32'", specifier = "==1.0.0" },
            { name = "b", marker = "sys_platform == 'linux'", specifier = "==1.0.0" },
            { name = "c", marker = "sys_platform == 'darwin'", specifier = "==1.0.0" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// Check the prioritization for virtual extra and marker packages
///
/// ```text
/// marker-variants-have-different-extras
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires psycopg[binary] ; platform_python_implementation != 'PyPy'
/// │   │   ├── satisfied by psycopg-1.0.0
/// │   │   └── satisfied by psycopg-1.0.0[binary]
/// │   └── requires psycopg ; platform_python_implementation == 'PyPy'
/// │       ├── satisfied by psycopg-1.0.0
/// │       └── satisfied by psycopg-1.0.0[binary]
/// ├── psycopg
/// │   ├── psycopg-1.0.0
/// │   │   └── requires tzdata ; sys_platform == 'win32'
/// │   │       └── satisfied by tzdata-1.0.0
/// │   └── psycopg-1.0.0[binary]
/// │       └── requires psycopg-binary ; implementation_name != 'pypy'
/// │           └── satisfied by psycopg-binary-1.0.0
/// ├── psycopg-binary
/// │   └── psycopg-binary-1.0.0
/// └── tzdata
///     └── tzdata-1.0.0
/// ```
#[test]
fn marker_variants_have_different_extras() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("tag_and_markers/virtual-package-extra-priorities.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''psycopg[binary] ; platform_python_implementation != 'PyPy'''',
          '''psycopg ; platform_python_implementation == 'PyPy'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        resolution-markers = [
            "platform_python_implementation != 'PyPy'",
            "platform_python_implementation == 'PyPy'",
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "psycopg" },
            { name = "psycopg", extra = ["binary"], marker = "platform_python_implementation != 'PyPy'" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "psycopg", marker = "platform_python_implementation == 'PyPy'" },
            { name = "psycopg", extras = ["binary"], marker = "platform_python_implementation != 'PyPy'" },
        ]

        [[package]]
        name = "psycopg"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "tzdata", marker = "sys_platform == 'win32'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/psycopg-1.0.0.tar.gz", hash = "sha256:33854afc2bc33353fa645560cc0082cf085dc2a0b94afa97ca499b442f3b1a4e", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/psycopg-1.0.0-py3-none-any.whl", hash = "sha256:e91c6b77d7c6b4262b81b606577e444d27fcdaa22d6603742ced03b39492eda3", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [package.optional-dependencies]
        binary = [
            { name = "psycopg-binary", marker = "implementation_name != 'pypy'" },
        ]

        [[package]]
        name = "psycopg-binary"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/psycopg_binary-1.0.0.tar.gz", hash = "sha256:bc28ec69da2e999fb6475f8f55098375766a57922031775a597998ddc825581a", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/psycopg_binary-1.0.0-py3-none-any.whl", hash = "sha256:35474aba5923a77d2aac9ac1e92bb5e9dc0468b3b2611e120b3947ae956a1482", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "tzdata"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/tzdata-1.0.0.tar.gz", hash = "sha256:32da4d714b0879cde291f6a55811e4ddbfdfd93bd17c0514f2f792f6cad38e38", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/tzdata-1.0.0-py3-none-any.whl", hash = "sha256:9dc909bec179ffb4d319a54ab73cb61e769c912be44743ff4411c5191432573f", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// Check the prioritization for virtual marker packages
///
/// ```text
/// virtual-package-extra-priorities
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   ├── requires a==1 ; python_full_version >= '3.8'
/// │   │   └── satisfied by a-1.0.0
/// │   └── requires b ; python_full_version >= '3.9'
/// │       ├── satisfied by b-1.0.0
/// │       └── satisfied by b-2.0.0
/// ├── a
/// │   ├── a-1.0.0
/// │   │   └── requires b==1 ; python_full_version >= '3.10'
/// │   │       └── satisfied by b-1.0.0
/// │   └── a-2.0.0
/// │       └── requires b==1 ; python_full_version >= '3.10'
/// │           └── satisfied by b-1.0.0
/// └── b
///     ├── b-1.0.0
///     └── b-2.0.0
/// ```
#[test]
fn virtual_package_extra_priorities() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("tag_and_markers/virtual-package-marker-priorities.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a==1 ; python_full_version >= '3.8'''',
          '''b ; python_full_version >= '3.9'''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "b" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:ceae689363b89a7feb87b681fc4f66b85ea8227eb50ae60134f69f6af12f8b3e", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:d3c47a26ec1e08259468942a50736244908a076baf0d5f931f62407814ed96e5", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        sdist = { url = "http://[LOCALHOST]/files/b-1.0.0.tar.gz", hash = "sha256:9b42692ac74b4da0eed1ef248b9be6bb0557c49507ba3b38f862b191a06d959c", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-1.0.0-py3-none-any.whl", hash = "sha256:a4c65510001153cab97a29ff219ad86e0d4330653ca89d9d4c84187ccf14c621", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a" },
            { name = "b" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a", marker = "python_full_version >= '3.8'", specifier = "==1" },
            { name = "b", marker = "python_full_version >= '3.9'" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// While both Linux and Windows are required and `win-only` has only a Windows wheel, `win-only` is also used only on Windows.
///
/// ```text
/// requires-python-subset
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   └── requires win-only ; sys_platform == 'win32'
/// │       └── satisfied by win-only-1.0.0
/// └── win-only
///     └── win-only-1.0.0
/// ```
#[test]
fn requires_python_subset() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("wheels/requires-python-subset.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''win-only ; sys_platform == 'win32'''',
        ]
        requires-python = ">=3.12"
        [tool.uv]
        required-environments = [
          '''sys_platform == 'linux'''',
          '''sys_platform == 'win32'''',
        ]
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"
        required-markers = [
            "sys_platform == 'linux'",
            "sys_platform == 'win32'",
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "win-only", marker = "sys_platform == 'win32'" },
        ]

        [package.metadata]
        requires-dist = [{ name = "win-only", marker = "sys_platform == 'win32'" }]

        [[package]]
        name = "win-only"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        wheels = [
            { url = "http://[LOCALHOST]/files/win_only-1.0.0-cp312-abi3-win_amd64.whl", hash = "sha256:b53b82ec335953e0c0d5dc75c59ce658fd9b6c04330126109a7ba104e0d107e3", upload-time = "2024-03-24T00:00:00Z" },
        ]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}

/// When a dependency is only required on a specific platform (like x86_64), omit wheels that target other platforms (like aarch64).
///
/// ```text
/// specific-architecture
/// ├── environment
/// │   └── python3.12
/// ├── root
/// │   └── requires a
/// │       └── satisfied by a-1.0.0
/// ├── a
/// │   └── a-1.0.0
/// │       ├── requires b ; platform_machine == 'x86_64'
/// │       │   └── satisfied by b-1.0.0
/// │       ├── requires c ; platform_machine == 'aarch64'
/// │       │   └── satisfied by c-1.0.0
/// │       └── requires d ; platform_machine == 'i686'
/// │           └── satisfied by d-1.0.0
/// ├── b
/// │   └── b-1.0.0
/// ├── c
/// │   └── c-1.0.0
/// └── d
///     └── d-1.0.0
/// ```
#[test]
fn specific_architecture() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("wheels/specific-architecture.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r###"
        [project]
        name = "project"
        version = "0.1.0"
        dependencies = [
          '''a''',
        ]
        requires-python = ">=3.12"
        "###,
    )?;

    let filters = context.filters();

    let mut cmd = context.lock();
    cmd.env_remove(EnvVars::UV_EXCLUDE_NEWER);
    cmd.arg("--index-url").arg(server.index_url());
    uv_snapshot!(filters, cmd, @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 5 packages in [TIME]
    "
    );

    let lock = context.read("uv.lock");
    insta::with_settings!({
        filters => filters,
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 3
        requires-python = ">=3.12"

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        dependencies = [
            { name = "b", marker = "platform_machine == 'x86_64'" },
            { name = "c", marker = "platform_machine == 'aarch64'" },
            { name = "d", marker = "platform_machine == 'i686'" },
        ]
        sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", hash = "sha256:d128ac9ff5b61e4db85dc86b943210cad23907d060f95a73914c7479acbbf9d1", upload-time = "2024-03-24T00:00:00Z" }
        wheels = [
            { url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", hash = "sha256:ccd718f14fb28f4e947c39f67a39897f08eaf09034d0641ffb6ce41e580dac1a", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "b"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        wheels = [
            { url = "http://[LOCALHOST]/files/b-1.0.0-cp313-cp313-freebsd_13_aarch64.whl", hash = "sha256:f631c1c1a0ffc6b2ce37901dca7c5e468d80f9a5107f3e3ec2dd231424e51447", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/b-1.0.0-cp313-cp313-freebsd_13_x86_64.whl", hash = "sha256:c0a23d73fe066699bb1ed79150f23a989692c419edde2dc07b6f07ad7df04bb6", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/b-1.0.0-cp313-cp313-macosx_10_9_x86_64.whl", hash = "sha256:afc62d392afde77dc8cb1a76ca661ea28622a990e3655d96b101301526790393", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/b-1.0.0-cp313-cp313-manylinux2010_x86_64.whl", hash = "sha256:baa416bab7cc0d502bc96bc57672f2fa83dd56b7dc37b701126e16fcc8ce115a", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "c"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        wheels = [
            { url = "http://[LOCALHOST]/files/c-1.0.0-cp313-cp313-freebsd_13_aarch64.whl", hash = "sha256:e34952726fbb61bd86f63a767fa1fcc10a8e27737ef515b597617ee7fcb808ea", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/c-1.0.0-cp313-cp313-freebsd_13_x86_64.whl", hash = "sha256:c73f855a843c0842e67e01683f9b8e97391ec4de6f4e59e3bc43b7b2218f2fb9", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/c-1.0.0-cp313-cp313-macosx_10_9_arm64.whl", hash = "sha256:27e36c0dab216a5e265e247d2fb7cab31448beb97cefc22cce301885210b54df", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/c-1.0.0-cp313-cp313-manylinux2010_aarch64.whl", hash = "sha256:4d267230c94b48fb96c8eabadfd39eeb5efcb5d712f699feba087b1bb7ee18b5", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "d"
        version = "1.0.0"
        source = { registry = "http://[LOCALHOST]/simple/" }
        wheels = [
            { url = "http://[LOCALHOST]/files/d-1.0.0-cp313-cp313-freebsd_13_aarch64.whl", hash = "sha256:faa00bef937a8020e48e398b106bf2085b327128af2e0a91a1596dfa07035b0a", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/d-1.0.0-cp313-cp313-freebsd_13_x86_64.whl", hash = "sha256:0535a2b64b540cf7ab4e124fa4673054113fd4d3957ae6b77b63833729e308c9", upload-time = "2024-03-24T00:00:00Z" },
            { url = "http://[LOCALHOST]/files/d-1.0.0-cp313-cp313-manylinux2010_i686.whl", hash = "sha256:66575b89a885afc891c6c84589a50b2f5112a0d1731c924df1fd376010e659e8", upload-time = "2024-03-24T00:00:00Z" },
        ]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }
        dependencies = [
            { name = "a" },
        ]

        [package.metadata]
        requires-dist = [{ name = "a" }]
        "#
        );
    });

    // Assert the idempotence of `uv lock` when resolving from the lockfile (`--locked`).
    context
        .lock()
        .arg("--locked")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();

    Ok(())
}
