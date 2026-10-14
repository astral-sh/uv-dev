//! Synchronization from older lockfile formats.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};
use uv_test::packse::PackseServer;
use uv_test::uv_snapshot;

/// Legacy underscore-spelled timestamps must be read for both source distributions and wheels.
#[test]
fn upload_time_alias() -> Result<()> {
    let server = PackseServer::new("simple/single-package.toml");
    let hashes: BTreeMap<_, _> = server.files().collect();
    let sdist = "a-1.0.0.tar.gz";
    let wheel = "a-1.0.0-py3-none-any.whl";
    let sdist_hash = *hashes.get(sdist).context("missing generated sdist")?;
    let wheel_hash = *hashes.get(wheel).context("missing generated wheel")?;
    let context = uv_test::test_context!("3.12")
        .with_filter((sdist_hash, "[SDIST_HASH]"))
        .with_filter((wheel_hash, "[WHEEL_HASH]"));
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["a==1.0.0"]
    "#})?;
    context.temp_dir.child("uv.lock").write_str(&formatdoc! {r#"
        version = 1
        revision = 2
        requires-python = ">=3.12"

        [[package]]
        name = "a"
        version = "1.0.0"
        source = {{ registry = "{index_url}" }}
        sdist = {{ url = "{sdist_url}", hash = "sha256:{sdist_hash}", upload_time = "2023-01-01T00:00:00Z" }}
        wheels = [{{ url = "{wheel_url}", hash = "sha256:{wheel_hash}", upload_time = "2023-01-02T00:00:00Z" }}]

        [[package]]
        name = "project"
        version = "0.1.0"
        source = {{ virtual = "." }}
        dependencies = [{{ name = "a" }}]

        [package.metadata]
        requires-dist = [{{ name = "a", specifier = "==1.0.0" }}]
    "#,
        index_url = server.index_url(),
        sdist_url = server.file_url(sdist),
        wheel_url = server.file_url(wheel),
    })?;

    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + a==1.0.0
    ");

    uv_snapshot!(context.filters(), context.sync().arg("--frozen"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");

    // Synchronization alone would also succeed if unknown timestamp fields were discarded.
    uv_snapshot!(context.filters(), context.export().args(["--frozen", "--offline", "--format", "pylock.toml", "--no-header"]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    lock-version = "1.0"
    created-by = "uv"
    requires-python = ">=3.12"

    [[packages]]
    name = "a"
    version = "1.0.0"
    index = "http://[LOCALHOST]/simple/"
    sdist = { url = "http://[LOCALHOST]/files/a-1.0.0.tar.gz", upload-time = 2023-01-01T00:00:00Z, hashes = { sha256 = "[SDIST_HASH]" } }
    wheels = [{ url = "http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl", upload-time = 2023-01-02T00:00:00Z, hashes = { sha256 = "[WHEEL_HASH]" } }]
    "#);

    Ok(())
}
