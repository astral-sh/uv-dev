use std::process::Command;

use anyhow::Result;
use assert_fs::prelude::*;
#[cfg(feature = "test-python")]
use predicates::prelude::predicate;
#[cfg(feature = "test-python")]
use sha2::{Digest, Sha256};

#[cfg(unix)]
use uv_fs::create_symlink;
#[cfg(unix)]
use uv_test::ReadOnlyDirectoryGuard;
#[cfg(feature = "test-python")]
use uv_test::archive::generate_source_archive;
#[cfg(feature = "test-python")]
use uv_test::package_server::PackageServer;
use uv_test::{get_bin, uv_snapshot};

/// A custom cache directory must configure commands and snapshot filters together.
#[test]
fn cache_dir_uses_configured_test_context_path() {
    let context = uv_test::test_context_with_versions!(&[]).with_cache_dir("project/cache");

    assert_eq!(
        context.cache_dir.path(),
        context.temp_dir.child("project").child("cache").path()
    );

    uv_snapshot!(context.filters(), context.command().arg("cache").arg("dir"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [CACHE_DIR]/
    ");
}

/// Invalid cache keys in a local dependency must fail instead of falling back to the default keys.
#[test]
fn cache_key_git_sequence() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let project = context.temp_dir.child("project");
    project.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"

        [tool.uv]
        cache-keys = [{ git = [true, false] }]
    "#})?;

    uv_snapshot!(context.filters(), context.pip_install().arg("./project").arg("--offline"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to build `project @ file://[TEMP_DIR]/project`
      cause: Failed to parse cache keys in `project/pyproject.toml`
      cause: TOML parse error at line 6, column 23
               |
             6 | cache-keys = [{ git = [true, false] }]
               |                       ^^^^^^^^^^^^^
             invalid type: sequence, expected a boolean or a Git cache key table
    ");

    Ok(())
}

/// When the active cache directory is inside an explicit build source, we should warn and continue
/// the build.
#[test]
#[cfg(feature = "test-python")]
fn build_warns_cache_inside_source() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_cache_dir("project/.uv-cache");
    let project = context.temp_dir.child("project");

    project.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = ["uv_build>=0.5.15,<10000"]
        build-backend = "uv_build"
        "#,
    )?;
    project.child("src/project/__init__.py").touch()?;

    uv_snapshot!(context.filters(), context.build().arg("--sdist").arg("project"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: The cache directory `project/.uv-cache` is inside the build source directory `project` and may be included in distributions
    Building source distribution...
    Successfully built project/dist/project-0.1.0.tar.gz
    ");

    project
        .child("dist/project-0.1.0.tar.gz")
        .assert(predicate::path::is_file());

    Ok(())
}

/// When the canonical cache directory is inside an explicit build source, we should warn even if
/// the configured cache path itself is outside the source.
#[test]
#[cfg(all(feature = "test-python", unix))]
fn build_warns_symlinked_cache_inside_source() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let project = context.temp_dir.child("project");

    project.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = ["uv_build>=0.5.15,<10000"]
        build-backend = "uv_build"
        "#,
    )?;
    project.child("src/project/__init__.py").touch()?;

    let cache_dir = project.child(".uv-cache");
    cache_dir.create_dir_all()?;
    let cache_link = context.temp_dir.child("cache-link");
    create_symlink(cache_dir.path(), cache_link.path())?;

    let mut command = Command::new(get_bin!());
    command
        .arg("build")
        .arg("--sdist")
        .arg("project")
        .arg("--cache-dir")
        .arg(cache_link.path());
    context.add_shared_env(&mut command, false);
    command.current_dir(context.temp_dir.path());

    uv_snapshot!(context.filters(), command, @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: The cache directory `cache-link` is inside the build source directory `project` and may be included in distributions
    Building source distribution...
    Successfully built project/dist/project-0.1.0.tar.gz
    ");

    project
        .child("dist/project-0.1.0.tar.gz")
        .assert(predicate::path::is_file());

    Ok(())
}

/// A cache in the workspace root is allowed when building a member that does not contain it.
#[test]
#[cfg(feature = "test-python")]
fn build_allows_cache_outside_selected_source() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_cache_dir("workspace/.uv-cache");
    let workspace = context.temp_dir.child("workspace");
    let member = workspace.child("member");

    workspace.child("pyproject.toml").write_str(
        r#"
        [tool.uv.workspace]
        members = ["member"]
        "#,
    )?;
    member.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "member"
        version = "0.1.0"
        requires-python = ">=3.12"

        [build-system]
        requires = ["uv_build>=0.5.15,<10000"]
        build-backend = "uv_build"
        "#,
    )?;
    member.child("src/member/__init__.py").touch()?;

    uv_snapshot!(context.filters(), context.build()
        .arg("--sdist")
        .arg("--package")
        .arg("member")
        .current_dir(&workspace), @"
    exit_code: 0 (success)
    ----- stderr -----
    Building source distribution...
    Successfully built dist/member-0.1.0.tar.gz
    ");

    workspace
        .child("dist/member-0.1.0.tar.gz")
        .assert(predicate::path::is_file());

    Ok(())
}

/// When the project directory defaults to a current directory inside the cache directory, we should
/// error before using the cache.
#[test]
fn cache_current_dir_inside_cache() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.command()
        .arg("cache")
        .arg("dir")
        .current_dir(context.cache_dir.path()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The project directory `.` is inside the cache directory `.`
    ");

    let child = context.cache_dir.child("child");
    child.create_dir_all()?;

    uv_snapshot!(context.filters(), context.command()
        .arg("cache")
        .arg("dir")
        .current_dir(child.path()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The project directory `.` is inside the cache directory `[CACHE_DIR]/`
    ");

    Ok(())
}

/// When the project directory is inside a symlinked cache directory, we should error before using
/// the cache.
#[test]
#[cfg(unix)]
fn cache_current_dir_inside_symlinked_cache() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    let cache_link = context.temp_dir.child("cache-link");
    create_symlink(context.cache_dir.path(), cache_link.path())?;

    let child = context.cache_dir.child("child");
    child.create_dir_all()?;

    let mut command = Command::new(get_bin!());
    command
        .arg("cache")
        .arg("dir")
        .arg("--cache-dir")
        .arg(cache_link.path());
    context.add_shared_env(&mut command, false);
    command.current_dir(child.path());

    uv_snapshot!(context.filters(), command, @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The project directory `.` is inside the cache directory `[CACHE_DIR]/`
    ");

    Ok(())
}

/// When a workspace is inside the cache directory, we should error before locking the workspace.
#[test]
fn cache_workspace_inside_cache() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    let workspace = context.cache_dir.child("workspace");
    workspace.child("pyproject.toml").write_str(
        r#"
        [tool.uv.workspace]
        members = ["member"]
        "#,
    )?;
    workspace
        .child("member")
        .child("pyproject.toml")
        .write_str(
            r#"
        [project]
        name = "member"
        version = "0.1.0"
        requires-python = ">=3.12"
        "#,
        )?;

    uv_snapshot!(context.filters(), context.lock()
        .arg("--project")
        .arg(workspace.path()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The project directory `[CACHE_DIR]/workspace` is inside the cache directory `[CACHE_DIR]/`
    ");

    Ok(())
}

/// When the cache directory is a non-canonical parent of the project directory, we should still
/// detect that the project is inside the cache.
#[test]
fn cache_project_inside_relative_parent_cache() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    let project = context.temp_dir.child("project");
    project.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        "#,
    )?;

    let mut command = Command::new(get_bin!());
    command.arg("lock").arg("--cache-dir").arg("..");
    context.add_shared_env(&mut command, false);
    command.current_dir(project.path());

    uv_snapshot!(context.filters(), command, @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The project directory `.` is inside the cache directory `[TEMP_DIR]/`
    ");

    Ok(())
}

/// When `--no-cache` is enabled, running from a project inside the configured cache directory
/// should not trip the persistent cache guard.
#[test]
#[cfg(feature = "test-python")]
fn cache_project_inside_cache_no_cache() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let project = context.cache_dir.child("project");
    project.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        "#,
    )?;

    uv_snapshot!(context.filters(), context.lock().arg("--no-cache").current_dir(&project), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Resolved 1 package in [TIME]
    ");

    Ok(())
}

/// When the cache directory cannot be created (e.g., due to permissions), we should show a
/// chained error message that indicates we failed to initialize the cache.
#[test]
#[cfg(unix)]
fn cache_init_failure() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_cache_dir("cache_parent/cache");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig"]
        "#,
    )?;

    // Create a read-only directory that will serve as the parent of the cache.
    // The guard sets it to read-only and restores original permissions on drop (including panic).
    let cache_parent = context.temp_dir.child("cache_parent");
    fs_err::create_dir(&cache_parent)?;
    let _guard = ReadOnlyDirectoryGuard::new(cache_parent.path())?;

    // Running a command should fail with a chained error about cache initialization
    uv_snapshot!(context.filters(), context.sync(), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to initialize cache at `cache_parent/cache`
      cause: failed to create directory `[CACHE_DIR]/`: Permission denied (os error 13)
    ");

    Ok(())
}

/// Index hashes must be checked before building an sdist or reading its metadata.
#[tokio::test]
#[cfg(feature = "test-python")]
async fn index_source_hashes() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_http_retries();
    let name = "ok".parse()?;
    let server = PackageServer::new(&name).await;
    let index_url = server.index_url();
    let filename = "ok-1.0.0.tar.gz";
    let marker = context.temp_dir.child("backend-marker");
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("ok==1.0.0")?;

    let archive = generate_source_archive(&name, &"1.0.0".parse()?, "", Some(marker.path()))?;
    let source_hash = hex::encode(Sha256::digest(&archive));
    let wrong_hash = "0".repeat(64);
    let context = context
        .with_filter((source_hash.clone(), "[SOURCE_HASH]"))
        .with_filter((wrong_hash.clone(), "[WRONG_HASH]"));
    server.serve(filename, &archive, Some(&wrong_hash)).await;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt").arg("--index-url").arg(&index_url), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download and build `ok==1.0.0`
      cause: Hash mismatch for `ok==1.0.0`

             Expected:
               sha256:[WRONG_HASH]

             Computed:
               sha256:[SOURCE_HASH]
    ");
    uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.txt").arg("--index-url").arg(&index_url).arg("--no-header").arg("--generate-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download and build `ok==1.0.0`
      cause: Hash mismatch for `ok==1.0.0`

             Expected:
               sha256:[WRONG_HASH]

             Computed:
               sha256:[SOURCE_HASH]
    ");
    marker.assert(predicate::path::missing());

    // The same source is usable once the index supplies its actual hash.
    server.serve(filename, &archive, Some(&source_hash)).await;
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt").arg("--index-url").arg(&index_url).arg("--refresh"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==1.0.0
    ");
    marker.assert(predicate::path::is_file());
    context.assert_installed("ok", "1.0.0");
    Ok(())
}
