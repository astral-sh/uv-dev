//! Synchronization with cached source builds.

use anyhow::Result;
use assert_fs::prelude::*;
use indoc::formatdoc;
use predicates::prelude::predicate;

use uv_test::archive::generate_source_archive;
use uv_test::package_server::PackageServer;
use uv_test::uv_snapshot;

/// A cached source build cannot satisfy a requirement with a different distribution name.
#[tokio::test]
async fn mismatched_name_cached_wheel() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let backend_executed = context.temp_dir.child("backend-executed");
    let name = "iniconfig".parse()?;
    let archive =
        generate_source_archive(&name, &"2.0.0".parse()?, "", Some(backend_executed.path()))?;
    let server = PackageServer::new(&name).await;
    // The URL does not expose the package name; it must come from metadata.
    server.serve("source.tar.gz", &archive, None).await;
    let archive_url = server.file_url("source.tar.gz");
    let pyproject = context.temp_dir.child("pyproject.toml");
    pyproject.write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig @ {archive_url}"]
    "#})?;

    uv_snapshot!(context.filters(), context.sync().arg("--no-index"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0 (from http://[LOCALHOST]/source.tar.gz)
    ");

    // Keep the cache as the only source of metadata and detect any repeated backend invocation.
    fs_err::remove_file(backend_executed.path())?;
    fs_err::remove_file(context.temp_dir.child("uv.lock").path())?;
    fs_err::remove_dir_all(&context.venv)?;
    pyproject.write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["foo @ {archive_url}"]
    "#})?;

    uv_snapshot!(context.filters(), context.sync().args(["--offline", "--no-index"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Creating virtual environment at: .venv
    error: Failed to download and build `foo @ http://[LOCALHOST]/source.tar.gz`
      cause: Package metadata name `iniconfig` does not match given name `foo`
    ");
    backend_executed.assert(predicate::path::missing());
    Ok(())
}
