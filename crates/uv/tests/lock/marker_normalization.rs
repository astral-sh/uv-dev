//! Lockfile normalization and validation of impossible dependency markers.

use anyhow::Result;
use assert_fs::prelude::*;
use indoc::formatdoc;
use insta::assert_snapshot;

use uv_redacted::SafeUrl;
use uv_test::uv_snapshot;

/// Impossible dependency-group requirements remain false when the lock is validated.
///
/// False markers are serialized as `python_version < '0'`; reading them back as true would make
/// `uv lock --check` fail.
#[test]
fn normalize_false_marker_dependency_groups() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let local = context.temp_dir.child("local");
    local.create_dir_all()?;
    let local_url = SafeUrl::from_file_path(local.path())
        .map_err(|()| anyhow::anyhow!("local path is not a valid file URL"))?;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11"
        [dependency-groups]
        dev = [
            "pytest;python_full_version>'3.8' and python_full_version<'3.6'",
            "local @ {local_url} ; python_full_version>'3.8' and python_full_version<'3.6'",
        ]
    "#})?;

    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");

    uv_snapshot!(context.filters(), context.lock().arg("--check").arg("--offline").arg("--no-cache"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 5
        requires-python = ">=3.11"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.metadata]

        [package.metadata.requires-dev]
        dev = [
            { name = "local", marker = "python_version < '0'", directory = "[TEMP_DIR]/local" },
            { name = "pytest", marker = "python_version < '0'" },
        ]
        "#
        );
    });

    Ok(())
}

/// Impossible regular requirements remain false when the lock is validated.
#[test]
fn normalize_false_marker_requires_dist() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "debug"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = [
            "pytest; python_full_version>'3.8' and python_full_version<'3.6'"
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");

    uv_snapshot!(context.filters(), context.lock().arg("--check").arg("--offline").arg("--no-cache"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");

    let lock = context.read("uv.lock");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(
            lock, @r#"
        version = 1
        revision = 5
        requires-python = ">=3.11"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "debug"
        version = "0.1.0"
        source = { virtual = "." }

        [package.metadata]
        requires-dist = [{ name = "pytest", marker = "python_version < '0'" }]
        "#
        );
    });

    Ok(())
}

/// Check that multiple markers with known-incompatible platform values are all normalized to
/// `false`, independent of the order in which they are processed.
#[test]
fn lock_impossible_platform_markers() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.9"
        dependencies = [
            "certifi ; os_name == 'nt' and sys_platform == 'linux'",
            "idna ; os_name == 'posix' and sys_platform == 'win32'",
        ]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");

    uv_snapshot!(context.filters(), context.lock().arg("--check").arg("--offline").arg("--no-cache"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        assert_snapshot!(context.read("uv.lock"), @r#"
        version = 1
        revision = 5
        requires-python = ">=3.9"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "project"
        version = "0.1.0"
        source = { virtual = "." }

        [package.metadata]
        requires-dist = [
            { name = "certifi", marker = "python_version < '0'" },
            { name = "idna", marker = "python_version < '0'" },
        ]
        "#);
    });

    Ok(())
}
