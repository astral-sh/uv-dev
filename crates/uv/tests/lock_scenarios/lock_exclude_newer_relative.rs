use anyhow::Result;
use assert_fs::fixture::{FileWriteStr, PathChild};
use insta::assert_snapshot;
use uv_static::EnvVars;

use uv_test::{apply_filters, uv_snapshot};

/// Lock with a relative exclude-newer value.
///
/// Uses idna which has releases at:
/// - 3.6: 2023-11-25
/// - 3.7: 2024-04-11
#[test]
fn lock_exclude_newer_relative() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/exclude-newer-relative.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["dated-package"]
        "#,
    )?;

    // 3 weeks before 2024-05-01 is 2024-04-10, which is before dated-package 3.7 (released 2024-04-11).
    let current_timestamp = "2024-05-01T00:00:00Z";
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp)
        .arg("--exclude-newer")
        .arg("3 weeks"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    // Should resolve to dated-package 3.6 (released 2023-11-25, before cutoff of 2024-04-10)
    assert_snapshot!(apply_filters(lock.clone(), context.filters()), @r#"
    version = 1
    revision = 3
    requires-python = ">=3.12"

    [options]
    exclude-newer = "0001-01-01T00:00:00Z" # This has no effect and is included for backwards compatibility when using relative exclude-newer values.
    exclude-newer-span = "P3W"

    [[package]]
    name = "dated-package"
    version = "3.6"
    source = { registry = "http://[LOCALHOST]/simple/" }
    sdist = { url = "http://[LOCALHOST]/files/dated_package-3.6.tar.gz", hash = "sha256:6dbed36a4b6e818ecba383caa48a58b5aadf8aa70d1d5c68ce636b1af63fb2aa", upload-time = "2023-11-25T15:40:54.902Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/dated_package-3.6-py3-none-any.whl", hash = "sha256:b8a2bebad18dcbc2a3c6b8ac972ff58da9821607c65847fa1c100d09f45c56b9", upload-time = "2023-11-25T15:40:54.902Z" },
    ]

    [[package]]
    name = "project"
    version = "0.1.0"
    source = { virtual = "." }
    dependencies = [
        { name = "dated-package" },
    ]

    [package.metadata]
    requires-dist = [{ name = "dated-package" }]
    "#);

    // Changing the current time should not result in a new lockfile
    let later_timestamp = "2024-06-01T00:00:00Z";
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, later_timestamp)
        .arg("--exclude-newer")
        .arg("3 weeks")
        .arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    assert_eq!(context.read("uv.lock"), lock);

    // Changing the span to 2 weeks should cause a new resolution.
    // 2 weeks before 2024-05-01 is 2024-04-17, which is after dated-package 3.7 (released 2024-04-11).
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp)
        .arg("--exclude-newer")
        .arg("2 weeks")
        .arg("--upgrade"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolving despite existing lockfile due to change of exclude newer span from `P3W` to `P2W`
    Resolved 2 packages in [TIME]
    Updated dated-package v3.6 -> v3.7
    ");

    // Both `exclude-newer` values in the lockfile should be changed, and we should now have dated-package 3.7
    let lock = context.read("uv.lock");
    assert_snapshot!(apply_filters(lock.clone(), context.filters()), @r#"
    version = 1
    revision = 3
    requires-python = ">=3.12"

    [options]
    exclude-newer = "0001-01-01T00:00:00Z" # This has no effect and is included for backwards compatibility when using relative exclude-newer values.
    exclude-newer-span = "P2W"

    [[package]]
    name = "dated-package"
    version = "3.7"
    source = { registry = "http://[LOCALHOST]/simple/" }
    sdist = { url = "http://[LOCALHOST]/files/dated_package-3.7.tar.gz", hash = "sha256:8fa33530c052fc57d340e34dd007640fdcd8203447932e523ec3933218c7858c", upload-time = "2024-04-11T03:34:43.276Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/dated_package-3.7-py3-none-any.whl", hash = "sha256:f69499f64fa76dcba2ed9c05f9980cbad3405b1c5d43dfc6a9f0bce32dfb6497", upload-time = "2024-04-11T03:34:43.276Z" },
    ]

    [[package]]
    name = "project"
    version = "0.1.0"
    source = { virtual = "." }
    dependencies = [
        { name = "dated-package" },
    ]

    [package.metadata]
    requires-dist = [{ name = "dated-package" }]
    "#);

    // Similarly, using something like `--upgrade` should cause a new resolution
    let current_timestamp = "2024-06-01T00:00:00Z";
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp)
        .arg("--exclude-newer")
        .arg("2 weeks")
        .arg("--upgrade"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    // And the `exclude-newer` timestamp value in the lockfile should be changed
    let lock = context.read("uv.lock");
    assert_snapshot!(apply_filters(lock.clone(), context.filters()), @r#"
    version = 1
    revision = 3
    requires-python = ">=3.12"

    [options]
    exclude-newer = "0001-01-01T00:00:00Z" # This has no effect and is included for backwards compatibility when using relative exclude-newer values.
    exclude-newer-span = "P2W"

    [[package]]
    name = "dated-package"
    version = "3.7"
    source = { registry = "http://[LOCALHOST]/simple/" }
    sdist = { url = "http://[LOCALHOST]/files/dated_package-3.7.tar.gz", hash = "sha256:8fa33530c052fc57d340e34dd007640fdcd8203447932e523ec3933218c7858c", upload-time = "2024-04-11T03:34:43.276Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/dated_package-3.7-py3-none-any.whl", hash = "sha256:f69499f64fa76dcba2ed9c05f9980cbad3405b1c5d43dfc6a9f0bce32dfb6497", upload-time = "2024-04-11T03:34:43.276Z" },
    ]

    [[package]]
    name = "project"
    version = "0.1.0"
    source = { virtual = "." }
    dependencies = [
        { name = "dated-package" },
    ]

    [package.metadata]
    requires-dist = [{ name = "dated-package" }]
    "#);

    // Similarly, using something like `--refresh` should cause a new resolution
    let current_timestamp = "2024-07-01T00:00:00Z";
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp)
        .arg("--exclude-newer")
        .arg("2 weeks")
        .arg("--refresh"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    Ok(())
}

/// Test that exclude-newer changes in either direction work correctly:
/// - Getting OLDER (more restrictive): forces downgrade of invalid versions
/// - Getting NEWER (less restrictive): keeps existing versions stable (use --upgrade to get newer)
///
/// Uses idna which has releases at:
/// - 3.6: 2023-11-25
/// - 3.7: 2024-04-11
#[test]
fn lock_exclude_newer_older_vs_newer() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/exclude-newer-relative.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["dated-package"]
        "#,
    )?;

    // Start with a cutoff that allows dated-package 3.7 (released 2024-04-11)
    // 2 weeks before 2024-05-01 is 2024-04-17, which is AFTER dated-package 3.7 release
    let current_timestamp = "2024-05-01T00:00:00Z";
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp)
        .arg("--exclude-newer")
        .arg("2 weeks"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    assert!(
        lock.contains("version = \"3.7\""),
        "Expected dated-package 3.7 in lockfile"
    );

    // Now make exclude-newer OLDER (more restrictive): 3 weeks back from 2024-05-01 is 2024-04-10
    // This is BEFORE dated-package 3.7 release (2024-04-11), so 3.7 becomes INVALID and must be replaced
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp)
        .arg("--exclude-newer")
        .arg("3 weeks"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolving despite existing lockfile due to change of exclude newer span from `P2W` to `P3W`
    Resolved 2 packages in [TIME]
    Updated dated-package v3.7 -> v3.6
    ");

    let lock = context.read("uv.lock");
    assert!(
        lock.contains("version = \"3.6\""),
        "Expected dated-package 3.6 in lockfile after downgrade"
    );

    // Now make exclude-newer NEWER (less restrictive): back to 2 weeks (2024-04-17)
    // This allows dated-package 3.7 again, but existing version (3.6) is still valid so it stays
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp)
        .arg("--exclude-newer")
        .arg("2 weeks"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolving despite existing lockfile due to change of exclude newer span from `P3W` to `P2W`
    Resolved 2 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    assert!(
        lock.contains("version = \"3.6\""),
        "Expected dated-package 3.6 to stay stable without --upgrade"
    );

    // With --upgrade, should now get dated-package 3.7 since the constraint allows it
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp)
        .arg("--exclude-newer")
        .arg("2 weeks")
        .arg("--upgrade"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Updated dated-package v3.6 -> v3.7
    ");

    let lock = context.read("uv.lock");
    assert!(
        lock.contains("version = \"3.7\""),
        "Expected dated-package 3.7 after --upgrade"
    );

    Ok(())
}

/// Lock with a relative exclude-newer-package value.
///
/// Uses idna which has releases at:
/// - 3.6: 2023-11-25
/// - 3.7: 2024-04-11
#[test]
fn lock_exclude_newer_package_relative() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/exclude-newer-relative.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["dated-package"]
        "#,
    )?;

    // 3 weeks before 2024-05-01 is 2024-04-10, which is before dated-package 3.7 (released 2024-04-11).
    let current_timestamp = "2024-05-01T00:00:00Z";
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp)
        .arg("--exclude-newer-package")
        .arg("dated-package=3 weeks"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    // Should resolve to dated-package 3.6 (released 2023-11-25, before cutoff of 2024-04-10)
    assert_snapshot!(apply_filters(lock.clone(), context.filters()), @r#"
    version = 1
    revision = 3
    requires-python = ">=3.12"

    [options]

    [options.exclude-newer-package]
    dated-package = { timestamp = "0001-01-01T00:00:00Z", span = "P3W" }

    [[package]]
    name = "dated-package"
    version = "3.6"
    source = { registry = "http://[LOCALHOST]/simple/" }
    sdist = { url = "http://[LOCALHOST]/files/dated_package-3.6.tar.gz", hash = "sha256:6dbed36a4b6e818ecba383caa48a58b5aadf8aa70d1d5c68ce636b1af63fb2aa", upload-time = "2023-11-25T15:40:54.902Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/dated_package-3.6-py3-none-any.whl", hash = "sha256:b8a2bebad18dcbc2a3c6b8ac972ff58da9821607c65847fa1c100d09f45c56b9", upload-time = "2023-11-25T15:40:54.902Z" },
    ]

    [[package]]
    name = "project"
    version = "0.1.0"
    source = { virtual = "." }
    dependencies = [
        { name = "dated-package" },
    ]

    [package.metadata]
    requires-dist = [{ name = "dated-package" }]
    "#);

    // Changing the current time should not result in a new lockfile
    let later_timestamp = "2024-06-01T00:00:00Z";
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, later_timestamp)
        .arg("--exclude-newer-package")
        .arg("dated-package=3 weeks")
        .arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    // Changing the span to 2 weeks should cause a new resolution.
    // 2 weeks before 2024-05-01 is 2024-04-17, which is after dated-package 3.7 (released 2024-04-11).
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp)
        .arg("--exclude-newer-package")
        .arg("dated-package=2 weeks")
        .arg("--upgrade"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolving despite existing lockfile due to change of exclude newer span from `P3W` to `P2W` for package `dated-package`
    Resolved 2 packages in [TIME]
    Updated dated-package v3.6 -> v3.7
    ");

    // Both `exclude-newer-package` values in the lockfile should be changed, and we should now have dated-package 3.7
    let lock = context.read("uv.lock");
    assert_snapshot!(apply_filters(lock.clone(), context.filters()), @r#"
    version = 1
    revision = 3
    requires-python = ">=3.12"

    [options]

    [options.exclude-newer-package]
    dated-package = { timestamp = "0001-01-01T00:00:00Z", span = "P2W" }

    [[package]]
    name = "dated-package"
    version = "3.7"
    source = { registry = "http://[LOCALHOST]/simple/" }
    sdist = { url = "http://[LOCALHOST]/files/dated_package-3.7.tar.gz", hash = "sha256:8fa33530c052fc57d340e34dd007640fdcd8203447932e523ec3933218c7858c", upload-time = "2024-04-11T03:34:43.276Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/dated_package-3.7-py3-none-any.whl", hash = "sha256:f69499f64fa76dcba2ed9c05f9980cbad3405b1c5d43dfc6a9f0bce32dfb6497", upload-time = "2024-04-11T03:34:43.276Z" },
    ]

    [[package]]
    name = "project"
    version = "0.1.0"
    source = { virtual = "." }
    dependencies = [
        { name = "dated-package" },
    ]

    [package.metadata]
    requires-dist = [{ name = "dated-package" }]
    "#);

    // Similarly, using something like `--upgrade` should cause a new resolution
    let current_timestamp = "2024-06-01T00:00:00Z";
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp)
        .arg("--exclude-newer-package")
        .arg("dated-package=2 weeks")
        .arg("--upgrade"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    // The `exclude-newer-package` span is unchanged; the timestamp is a placeholder
    let lock = context.read("uv.lock");
    assert_snapshot!(apply_filters(lock.clone(), context.filters()), @r#"
    version = 1
    revision = 3
    requires-python = ">=3.12"

    [options]

    [options.exclude-newer-package]
    dated-package = { timestamp = "0001-01-01T00:00:00Z", span = "P2W" }

    [[package]]
    name = "dated-package"
    version = "3.7"
    source = { registry = "http://[LOCALHOST]/simple/" }
    sdist = { url = "http://[LOCALHOST]/files/dated_package-3.7.tar.gz", hash = "sha256:8fa33530c052fc57d340e34dd007640fdcd8203447932e523ec3933218c7858c", upload-time = "2024-04-11T03:34:43.276Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/dated_package-3.7-py3-none-any.whl", hash = "sha256:f69499f64fa76dcba2ed9c05f9980cbad3405b1c5d43dfc6a9f0bce32dfb6497", upload-time = "2024-04-11T03:34:43.276Z" },
    ]

    [[package]]
    name = "project"
    version = "0.1.0"
    source = { virtual = "." }
    dependencies = [
        { name = "dated-package" },
    ]

    [package.metadata]
    requires-dist = [{ name = "dated-package" }]
    "#);

    Ok(())
}

/// Lock with a relative exclude-newer value from the `pyproject.toml`.
///
/// Uses idna which has releases at:
/// - 3.6: 2023-11-25
/// - 3.7: 2024-04-11
#[test]
fn lock_exclude_newer_relative_pyproject() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/exclude-newer-relative.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["dated-package"]

        [tool.uv]
        exclude-newer = "3 weeks"
        "#,
    )?;

    // 3 weeks before 2024-05-01 is 2024-04-10, which is before dated-package 3.7 (released 2024-04-11).
    let current_timestamp = "2024-05-01T00:00:00Z";
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    // Should resolve to dated-package 3.6 (released 2023-11-25, before cutoff of 2024-04-10)
    assert_snapshot!(apply_filters(lock.clone(), context.filters()), @r#"
    version = 1
    revision = 3
    requires-python = ">=3.12"

    [options]
    exclude-newer = "0001-01-01T00:00:00Z" # This has no effect and is included for backwards compatibility when using relative exclude-newer values.
    exclude-newer-span = "P3W"

    [[package]]
    name = "dated-package"
    version = "3.6"
    source = { registry = "http://[LOCALHOST]/simple/" }
    sdist = { url = "http://[LOCALHOST]/files/dated_package-3.6.tar.gz", hash = "sha256:6dbed36a4b6e818ecba383caa48a58b5aadf8aa70d1d5c68ce636b1af63fb2aa", upload-time = "2023-11-25T15:40:54.902Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/dated_package-3.6-py3-none-any.whl", hash = "sha256:b8a2bebad18dcbc2a3c6b8ac972ff58da9821607c65847fa1c100d09f45c56b9", upload-time = "2023-11-25T15:40:54.902Z" },
    ]

    [[package]]
    name = "project"
    version = "0.1.0"
    source = { virtual = "." }
    dependencies = [
        { name = "dated-package" },
    ]

    [package.metadata]
    requires-dist = [{ name = "dated-package" }]
    "#);

    Ok(())
}

/// Lock with a relative exclude-newer-package value from the `pyproject.toml`.
///
/// Uses idna which has releases at:
/// - 3.6: 2023-11-25
/// - 3.7: 2024-04-11
#[test]
fn lock_exclude_newer_package_relative_pyproject() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/exclude-newer-relative.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["dated-package"]

        [tool.uv]
        exclude-newer-package = { dated-package = "3 weeks" }
        "#,
    )?;

    // 3 weeks before 2024-05-01 is 2024-04-10, which is before dated-package 3.7 (released 2024-04-11).
    let current_timestamp = "2024-05-01T00:00:00Z";
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    // Should resolve to dated-package 3.6 (released 2023-11-25, before cutoff of 2024-04-10)
    assert_snapshot!(apply_filters(lock.clone(), context.filters()), @r#"
    version = 1
    revision = 3
    requires-python = ">=3.12"

    [options]

    [options.exclude-newer-package]
    dated-package = { timestamp = "0001-01-01T00:00:00Z", span = "P3W" }

    [[package]]
    name = "dated-package"
    version = "3.6"
    source = { registry = "http://[LOCALHOST]/simple/" }
    sdist = { url = "http://[LOCALHOST]/files/dated_package-3.6.tar.gz", hash = "sha256:6dbed36a4b6e818ecba383caa48a58b5aadf8aa70d1d5c68ce636b1af63fb2aa", upload-time = "2023-11-25T15:40:54.902Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/dated_package-3.6-py3-none-any.whl", hash = "sha256:b8a2bebad18dcbc2a3c6b8ac972ff58da9821607c65847fa1c100d09f45c56b9", upload-time = "2023-11-25T15:40:54.902Z" },
    ]

    [[package]]
    name = "project"
    version = "0.1.0"
    source = { virtual = "." }
    dependencies = [
        { name = "dated-package" },
    ]

    [package.metadata]
    requires-dist = [{ name = "dated-package" }]
    "#);

    Ok(())
}

/// Lock with both global and per-package relative exclude-newer values.
///
/// Uses idna which has releases at:
/// - 3.6: 2023-11-25
/// - 3.7: 2024-04-11
///
/// And typing-extensions which has releases at:
/// - 4.10.0: 2024-02-25
/// - 4.11.0: 2024-04-05
#[test]
fn lock_exclude_newer_relative_global_and_package() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/exclude-newer-relative.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["dated-package", "dated-support"]
        "#,
    )?;

    // Use a fixed timestamp so the test is reproducible.
    // Current time: 2024-05-01
    // Global: 3 weeks back = 2024-04-10 (before dated-package 3.7 released 2024-04-11) → dated-package 3.6
    // Per-package: 2 weeks back = 2024-04-17 (after dated-support 4.11.0 released 2024-04-05) → dated-support 4.11.0
    let current_timestamp = "2024-05-01T00:00:00Z";

    // Lock with both global exclude-newer and package-specific override using relative durations
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp)
        .arg("--exclude-newer")
        .arg("3 weeks")
        .arg("--exclude-newer-package")
        .arg("dated-support=2 weeks"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    // dated-package 3.6 (global cutoff 2024-04-10 is before 3.7 release on 2024-04-11)
    // dated-support 4.11.0 (per-package cutoff 2024-04-17 is after 4.11.0 release on 2024-04-05)
    assert_snapshot!(apply_filters(lock.clone(), context.filters()), @r#"
    version = 1
    revision = 3
    requires-python = ">=3.12"

    [options]
    exclude-newer = "0001-01-01T00:00:00Z" # This has no effect and is included for backwards compatibility when using relative exclude-newer values.
    exclude-newer-span = "P3W"

    [options.exclude-newer-package]
    dated-support = { timestamp = "0001-01-01T00:00:00Z", span = "P2W" }

    [[package]]
    name = "dated-package"
    version = "3.6"
    source = { registry = "http://[LOCALHOST]/simple/" }
    sdist = { url = "http://[LOCALHOST]/files/dated_package-3.6.tar.gz", hash = "sha256:6dbed36a4b6e818ecba383caa48a58b5aadf8aa70d1d5c68ce636b1af63fb2aa", upload-time = "2023-11-25T15:40:54.902Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/dated_package-3.6-py3-none-any.whl", hash = "sha256:b8a2bebad18dcbc2a3c6b8ac972ff58da9821607c65847fa1c100d09f45c56b9", upload-time = "2023-11-25T15:40:54.902Z" },
    ]

    [[package]]
    name = "dated-support"
    version = "4.11.0"
    source = { registry = "http://[LOCALHOST]/simple/" }
    sdist = { url = "http://[LOCALHOST]/files/dated_support-4.11.0.tar.gz", hash = "sha256:4c2f6bbad5b330fe45be40bb45143fefa8702fdacb01d95434f3f8b0df300b1f", upload-time = "2024-04-05T12:35:47.093Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/dated_support-4.11.0-py3-none-any.whl", hash = "sha256:cc2beaa723dad903f6fd3d7ee61c83fa69dcdfe2b50e68cde46fecc624e042e9", upload-time = "2024-04-05T12:35:47.093Z" },
    ]

    [[package]]
    name = "project"
    version = "0.1.0"
    source = { virtual = "." }
    dependencies = [
        { name = "dated-package" },
        { name = "dated-support" },
    ]

    [package.metadata]
    requires-dist = [
        { name = "dated-package" },
        { name = "dated-support" },
    ]
    "#);

    // Changing the current time should not invalidate the lockfile
    let later_timestamp = "2024-07-01T00:00:00Z";
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, later_timestamp)
        .arg("--exclude-newer")
        .arg("3 weeks")
        .arg("--exclude-newer-package")
        .arg("dated-support=2 weeks")
        .arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");

    // Changing the global span to 2 weeks should cause a new resolution.
    // 2 weeks before 2024-05-01 is 2024-04-17 (after dated-package 3.7 released 2024-04-11) → dated-package 3.7
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp)
        .arg("--exclude-newer")
        .arg("2 weeks")
        .arg("--exclude-newer-package")
        .arg("dated-support=2 weeks")
        .arg("--upgrade"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolving despite existing lockfile due to change of exclude newer span from `P3W` to `P2W`
    Resolved 3 packages in [TIME]
    Updated dated-package v3.6 -> v3.7
    ");

    // Changing the package-specific span should also invalidate the lockfile
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp)
        .arg("--exclude-newer")
        .arg("2 weeks")
        .arg("--exclude-newer-package")
        .arg("dated-support=3 days"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolving despite existing lockfile due to change of exclude newer span from `P2W` to `P3D` for package `dated-support`
    Resolved 3 packages in [TIME]
    ");

    // Use an absolute global value and relative package value
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp)
        .arg("--exclude-newer")
        .arg("2024-05-20T00:00:00Z")
        .arg("--exclude-newer-package")
        .arg("dated-support=2 weeks"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolving despite existing lockfile due to removal of exclude newer span
    Resolved 3 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    // dated-package 3.7 (absolute cutoff 2024-05-20 is after 3.7 release on 2024-04-11)
    // dated-support 4.11.0 (relative cutoff 2024-04-17)
    assert_snapshot!(apply_filters(lock.clone(), context.filters()), @r#"
    version = 1
    revision = 3
    requires-python = ">=3.12"

    [options]
    exclude-newer = "2024-05-20T00:00:00Z"

    [options.exclude-newer-package]
    dated-support = { timestamp = "0001-01-01T00:00:00Z", span = "P2W" }

    [[package]]
    name = "dated-package"
    version = "3.7"
    source = { registry = "http://[LOCALHOST]/simple/" }
    sdist = { url = "http://[LOCALHOST]/files/dated_package-3.7.tar.gz", hash = "sha256:8fa33530c052fc57d340e34dd007640fdcd8203447932e523ec3933218c7858c", upload-time = "2024-04-11T03:34:43.276Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/dated_package-3.7-py3-none-any.whl", hash = "sha256:f69499f64fa76dcba2ed9c05f9980cbad3405b1c5d43dfc6a9f0bce32dfb6497", upload-time = "2024-04-11T03:34:43.276Z" },
    ]

    [[package]]
    name = "dated-support"
    version = "4.11.0"
    source = { registry = "http://[LOCALHOST]/simple/" }
    sdist = { url = "http://[LOCALHOST]/files/dated_support-4.11.0.tar.gz", hash = "sha256:4c2f6bbad5b330fe45be40bb45143fefa8702fdacb01d95434f3f8b0df300b1f", upload-time = "2024-04-05T12:35:47.093Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/dated_support-4.11.0-py3-none-any.whl", hash = "sha256:cc2beaa723dad903f6fd3d7ee61c83fa69dcdfe2b50e68cde46fecc624e042e9", upload-time = "2024-04-05T12:35:47.093Z" },
    ]

    [[package]]
    name = "project"
    version = "0.1.0"
    source = { virtual = "." }
    dependencies = [
        { name = "dated-package" },
        { name = "dated-support" },
    ]

    [package.metadata]
    requires-dist = [
        { name = "dated-package" },
        { name = "dated-support" },
    ]
    "#);

    // Use a relative global value and absolute package value
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp)
        .arg("--exclude-newer")
        .arg("3 weeks")
        .arg("--exclude-newer-package")
        .arg("dated-support=2024-04-01T00:00:00Z"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolving despite existing lockfile due to addition of exclude newer span `P3W`
    Resolved 3 packages in [TIME]
    Updated dated-package v3.7 -> v3.6
    Updated dated-support v4.11.0 -> v4.10.0
    ");

    let lock = context.read("uv.lock");
    // dated-package 3.6 (relative cutoff 2024-04-10 is before 3.7 release on 2024-04-11)
    // dated-support 4.10.0 (absolute cutoff 2024-04-01 is before 4.11.0 release on 2024-04-05)
    assert_snapshot!(apply_filters(lock.clone(), context.filters()), @r#"
    version = 1
    revision = 3
    requires-python = ">=3.12"

    [options]
    exclude-newer = "0001-01-01T00:00:00Z" # This has no effect and is included for backwards compatibility when using relative exclude-newer values.
    exclude-newer-span = "P3W"

    [options.exclude-newer-package]
    dated-support = "2024-04-01T00:00:00Z"

    [[package]]
    name = "dated-package"
    version = "3.6"
    source = { registry = "http://[LOCALHOST]/simple/" }
    sdist = { url = "http://[LOCALHOST]/files/dated_package-3.6.tar.gz", hash = "sha256:6dbed36a4b6e818ecba383caa48a58b5aadf8aa70d1d5c68ce636b1af63fb2aa", upload-time = "2023-11-25T15:40:54.902Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/dated_package-3.6-py3-none-any.whl", hash = "sha256:b8a2bebad18dcbc2a3c6b8ac972ff58da9821607c65847fa1c100d09f45c56b9", upload-time = "2023-11-25T15:40:54.902Z" },
    ]

    [[package]]
    name = "dated-support"
    version = "4.10.0"
    source = { registry = "http://[LOCALHOST]/simple/" }
    sdist = { url = "http://[LOCALHOST]/files/dated_support-4.10.0.tar.gz", hash = "sha256:88c4ab66a5231995bc654e353e7deea36fd2ee7cbb859a710772702aff98b29d", upload-time = "2024-02-25T22:12:49.693Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/dated_support-4.10.0-py3-none-any.whl", hash = "sha256:a8272748b917f3104743ee49210d10658212277762c090d1c0fc8b1acaca68e2", upload-time = "2024-02-25T22:12:49.693Z" },
    ]

    [[package]]
    name = "project"
    version = "0.1.0"
    source = { virtual = "." }
    dependencies = [
        { name = "dated-package" },
        { name = "dated-support" },
    ]

    [package.metadata]
    requires-dist = [
        { name = "dated-package" },
        { name = "dated-support" },
    ]
    "#);

    Ok(())
}

/// Lock with various relative exclude newer value formats.
#[test]
fn lock_exclude_newer_relative_values() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/exclude-newer-relative.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["late-package"]
        "#,
    )?;

    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--exclude-newer")
        .arg("1 day"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--exclude-newer")
        .arg("30days"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolving despite existing lockfile due to change of exclude newer span from `P1D` to `P30D`
    Resolved 2 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--exclude-newer")
        .arg("P1D"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolving despite existing lockfile due to change of exclude newer span from `P30D` to `P1D`
    Resolved 2 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--exclude-newer")
        .arg("1 week"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolving despite existing lockfile due to change of exclude newer span from `P1D` to `P1W`
    Resolved 2 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--exclude-newer")
        .arg("1 week ago"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolving despite existing lockfile due to change of exclude newer span from `P1W` to `-P1W`
    Resolved 2 packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--exclude-newer")
        .arg("3 months"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: invalid value '3 months' for '--exclude-newer <EXCLUDE_NEWER>': Duration `3 months` uses 'months' which is not allowed; use days instead, e.g., `90 days`.

    For more information, try '--help'.
    ");

    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--exclude-newer")
        .arg("2 months ago"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: invalid value '2 months ago' for '--exclude-newer <EXCLUDE_NEWER>': Duration `2 months ago` uses 'months' which is not allowed; use days instead, e.g., `60 days`.

    For more information, try '--help'.
    ");

    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--exclude-newer")
        .arg("1 year"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: invalid value '1 year' for '--exclude-newer <EXCLUDE_NEWER>': Duration `1 year` uses unit 'years' which is not allowed; use days instead, e.g., `365 days`.

    For more information, try '--help'.
    ");

    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--exclude-newer")
        .arg("1 year ago"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: invalid value '1 year ago' for '--exclude-newer <EXCLUDE_NEWER>': Duration `1 year ago` uses unit 'years' which is not allowed; use days instead, e.g., `365 days`.

    For more information, try '--help'.
    ");

    uv_snapshot!(context.filters(), context
        .lock()
        .arg("--exclude-newer")
        .arg("invalid span"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: invalid value 'invalid span' for '--exclude-newer <EXCLUDE_NEWER>': `invalid span` could not be parsed as a valid exclude-newer value (expected a date like `2024-01-01`, a timestamp like `2024-01-01T00:00:00Z`, or a duration like `3 days` or `P3D`)

    For more information, try '--help'.
    ");

    uv_snapshot!(context.filters(), context
        .lock()
        .arg("--exclude-newer")
        .arg("P4Z"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: invalid value 'P4Z' for '--exclude-newer <EXCLUDE_NEWER>': `P4Z` could not be parsed as an ISO 8601 duration: expected to find date unit designator suffix (`Y`, `M`, `W` or `D`), but found `Z` instead

    For more information, try '--help'.
    ");

    uv_snapshot!(context.filters(), context
        .lock()
        .arg("--exclude-newer")
        .arg("2006-12-02T02:07:43Z"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolving despite existing lockfile due to removal of exclude newer span
    error: No solution found when resolving dependencies
      cause: Because there are no versions of late-package and late-package==2.0.0 was published after the exclude newer time, we can conclude that all versions of late-package cannot be used.
             And because your project depends on late-package, we can conclude that your project's requirements are unsatisfiable.
    ");

    uv_snapshot!(context.filters(), context
        .lock()
        .arg("--exclude-newer")
        .arg("12/02/2006"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: invalid value '12/02/2006' for '--exclude-newer <EXCLUDE_NEWER>': `12/02/2006` could not be parsed as a valid exclude-newer value (expected a date like `2024-01-01`, a timestamp like `2024-01-01T00:00:00Z`, or a duration like `3 days` or `P3D`)

    For more information, try '--help'.
    ");

    uv_snapshot!(context.filters(), context
        .lock()
        .arg("--exclude-newer")
        .arg("2 weak"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: invalid value '2 weak' for '--exclude-newer <EXCLUDE_NEWER>': `2 weak` could not be parsed as a duration: failed to parse input in the "friendly" duration format: parsed value 'P2W', but unparsed input "eak" remains (expected no unparsed input)

    For more information, try '--help'.
    "#);

    uv_snapshot!(context.filters(), context
        .lock()
        .arg("--exclude-newer")
        .arg("30"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: invalid value '30' for '--exclude-newer <EXCLUDE_NEWER>': `30` could not be parsed as a valid exclude-newer value (expected a date like `2024-01-01`, a timestamp like `2024-01-01T00:00:00Z`, or a duration like `3 days` or `P3D`)

    For more information, try '--help'.
    ");

    uv_snapshot!(context.filters(), context
        .lock()
        .arg("--exclude-newer")
        .arg("1000000 years"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: invalid value '1000000 years' for '--exclude-newer <EXCLUDE_NEWER>': `1000000 years` could not be parsed as a duration: failed to parse input in the "friendly" duration format: failed to set value for year unit on span: parameter 'years' is not in the required range of -19998..=19998

    For more information, try '--help'.
    "#);

    Ok(())
}

#[test]
fn lock_exclude_newer_relative_no_timestamp_in_lockfile() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/exclude-newer-relative.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["dated-package"]

        [tool.uv]
        exclude-newer = "3 weeks"
        "#,
    )?;

    let current_timestamp = "2024-05-01T00:00:00Z";
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    assert_snapshot!(apply_filters(lock.clone(), context.filters()), @r#"
    version = 1
    revision = 3
    requires-python = ">=3.12"

    [options]
    exclude-newer = "0001-01-01T00:00:00Z" # This has no effect and is included for backwards compatibility when using relative exclude-newer values.
    exclude-newer-span = "P3W"

    [[package]]
    name = "dated-package"
    version = "3.6"
    source = { registry = "http://[LOCALHOST]/simple/" }
    sdist = { url = "http://[LOCALHOST]/files/dated_package-3.6.tar.gz", hash = "sha256:6dbed36a4b6e818ecba383caa48a58b5aadf8aa70d1d5c68ce636b1af63fb2aa", upload-time = "2023-11-25T15:40:54.902Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/dated_package-3.6-py3-none-any.whl", hash = "sha256:b8a2bebad18dcbc2a3c6b8ac972ff58da9821607c65847fa1c100d09f45c56b9", upload-time = "2023-11-25T15:40:54.902Z" },
    ]

    [[package]]
    name = "project"
    version = "0.1.0"
    source = { virtual = "." }
    dependencies = [
        { name = "dated-package" },
    ]

    [package.metadata]
    requires-dist = [{ name = "dated-package" }]
    "#);

    // Manually remove the exclude-newer timestamp from the lockfile, leaving the span.
    let lock = lock.replace("exclude-newer = \"0001-01-01T00:00:00Z\" # This has no effect and is included for backwards compatibility when using relative exclude-newer values.\n", "");
    context.temp_dir.child("uv.lock").write_str(&lock)?;

    // The lockfile now has no exclude-newer timestamp, but the span is still present.
    // Since the span matches the `pyproject.toml` configuration, the lockfile is still
    // treated as valid — the missing timestamp alone does not trigger re-resolution.
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    // The lockfile retains the span but the timestamp is not restored or updated.
    let lock = context.read("uv.lock");
    assert_snapshot!(apply_filters(lock.clone(), context.filters()), @r#"
    version = 1
    revision = 3
    requires-python = ">=3.12"

    [options]
    exclude-newer-span = "P3W"

    [[package]]
    name = "dated-package"
    version = "3.6"
    source = { registry = "http://[LOCALHOST]/simple/" }
    sdist = { url = "http://[LOCALHOST]/files/dated_package-3.6.tar.gz", hash = "sha256:6dbed36a4b6e818ecba383caa48a58b5aadf8aa70d1d5c68ce636b1af63fb2aa", upload-time = "2023-11-25T15:40:54.902Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/dated_package-3.6-py3-none-any.whl", hash = "sha256:b8a2bebad18dcbc2a3c6b8ac972ff58da9821607c65847fa1c100d09f45c56b9", upload-time = "2023-11-25T15:40:54.902Z" },
    ]

    [[package]]
    name = "project"
    version = "0.1.0"
    source = { virtual = "." }
    dependencies = [
        { name = "dated-package" },
    ]

    [package.metadata]
    requires-dist = [{ name = "dated-package" }]
    "#);

    Ok(())
}

#[test]
fn lock_exclude_newer_package_relative_no_timestamp_in_lockfile() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/exclude-newer-relative.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["dated-package"]

        [tool.uv]
        exclude-newer-package = { dated-package = "3 weeks" }
        "#,
    )?;

    let current_timestamp = "2024-05-01T00:00:00Z";
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    let lock = context.read("uv.lock");
    assert_snapshot!(apply_filters(lock.clone(), context.filters()), @r#"
    version = 1
    revision = 3
    requires-python = ">=3.12"

    [options]

    [options.exclude-newer-package]
    dated-package = { timestamp = "0001-01-01T00:00:00Z", span = "P3W" }

    [[package]]
    name = "dated-package"
    version = "3.6"
    source = { registry = "http://[LOCALHOST]/simple/" }
    sdist = { url = "http://[LOCALHOST]/files/dated_package-3.6.tar.gz", hash = "sha256:6dbed36a4b6e818ecba383caa48a58b5aadf8aa70d1d5c68ce636b1af63fb2aa", upload-time = "2023-11-25T15:40:54.902Z" }
    wheels = [
        { url = "http://[LOCALHOST]/files/dated_package-3.6-py3-none-any.whl", hash = "sha256:b8a2bebad18dcbc2a3c6b8ac972ff58da9821607c65847fa1c100d09f45c56b9", upload-time = "2023-11-25T15:40:54.902Z" },
    ]

    [[package]]
    name = "project"
    version = "0.1.0"
    source = { virtual = "." }
    dependencies = [
        { name = "dated-package" },
    ]

    [package.metadata]
    requires-dist = [{ name = "dated-package" }]
    "#);

    // Manually remove the per-package exclude-newer timestamp from the lockfile, leaving the span.
    let lock = lock.replace(
        "dated-package = { timestamp = \"0001-01-01T00:00:00Z\", span = \"P3W\" }",
        "dated-package = { span = \"P3W\" }",
    );
    context.temp_dir.child("uv.lock").write_str(&lock)?;

    // Unlike the global case, a per-package entry with only a span (no timestamp) fails to
    // deserialize.
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse `uv.lock`
      cause: TOML parse error at line 5, column 1
               |
             5 | [options]
               | ^^^^^^^^^
             data did not match any variant of untagged enum Helper
    ");

    Ok(())
}

/// Lock with various relative exclude newer value formats in a `pyproject.toml`.
#[test]
fn lock_exclude_newer_relative_values_pyproject() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/exclude-newer-relative.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["late-package"]

        [tool.uv]
        exclude-newer = "invalid span"
        "#,
    )?;

    uv_snapshot!(context.filters(), context
        .lock(), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Failed to parse `pyproject.toml` during settings discovery:
      TOML parse error at line 9, column 25
        |
      9 |         exclude-newer = "invalid span"
        |                         ^^^^^^^^^^^^^^
      `invalid span` could not be parsed as a valid exclude-newer value (expected a date like `2024-01-01`, a timestamp like `2024-01-01T00:00:00Z`, or a duration like `3 days` or `P3D`)

    Resolved 2 packages in [TIME]
    "#);

    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["late-package"]

        [tool.uv]
        exclude-newer = "2 foos"
        "#,
    )?;

    uv_snapshot!(context.filters(), context
        .lock(), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Failed to parse `pyproject.toml` during settings discovery:
      TOML parse error at line 9, column 25
        |
      9 |         exclude-newer = "2 foos"
        |                         ^^^^^^^^
      `2 foos` could not be parsed as a duration: failed to parse input in the "friendly" duration format: expected to find unit designator suffix (e.g., `years` or `secs`) after parsing integer

    Resolved 2 packages in [TIME]
    "#);

    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["late-package"]

        [tool.uv]
        exclude-newer = "P4Z"
        "#,
    )?;

    uv_snapshot!(context.filters(), context
        .lock(), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Failed to parse `pyproject.toml` during settings discovery:
      TOML parse error at line 9, column 25
        |
      9 |         exclude-newer = "P4Z"
        |                         ^^^^^
      `P4Z` could not be parsed as an ISO 8601 duration: expected to find date unit designator suffix (`Y`, `M`, `W` or `D`), but found `Z` instead

    Resolved 2 packages in [TIME]
    "#);

    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["late-package"]

        [tool.uv]
        exclude-newer = "10"
        "#,
    )?;

    uv_snapshot!(context.filters(), context
        .lock(), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Failed to parse `pyproject.toml` during settings discovery:
      TOML parse error at line 9, column 25
        |
      9 |         exclude-newer = "10"
        |                         ^^^^
      `10` could not be parsed as a valid exclude-newer value (expected a date like `2024-01-01`, a timestamp like `2024-01-01T00:00:00Z`, or a duration like `3 days` or `P3D`)

    Resolved 2 packages in [TIME]
    "#);

    Ok(())
}

/// When a relative span is configured for `exclude-newer-package`, the lockfile
/// should use a fixed no-op sentinel for the stored timestamp.
#[test]
fn lock_exclude_newer_package_relative_noop_timestamp() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/exclude-newer-relative.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["dated-package"]
        "#,
    )?;

    let current_timestamp = "2024-05-01T00:00:00Z";
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, current_timestamp)
        .arg("--exclude-newer-package")
        .arg("dated-package=3 weeks"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    // The lockfile stores the no-op sentinel timestamp alongside the span.
    let lock = context.read("uv.lock");
    assert!(
        lock.contains(r#"dated-package = { timestamp = "0001-01-01T00:00:00Z", span = "P3W" }"#),
        "expected no-op sentinel in lockfile, got:\n{lock}"
    );

    // Locking again at a later time should yield an identical lockfile, even
    // without `--locked`, because the span is unchanged and the stored
    // timestamp is a fixed sentinel.
    let later_timestamp = "2024-06-01T00:00:00Z";
    uv_snapshot!(context.filters(), context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::UV_TEST_CURRENT_TIMESTAMP, later_timestamp)
        .arg("--exclude-newer-package")
        .arg("dated-package=3 weeks"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");

    assert_eq!(context.read("uv.lock"), lock);

    Ok(())
}
