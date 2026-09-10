use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use anyhow::Result;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use tar_codec::{ArchiveBuilder as _, EntryMetadata, TarEncoder};
use url::Url;
use walkdir::WalkDir;

use uv_static::EnvVars;
use uv_test::{TestContext, uv_snapshot};

const INERT_EXECUTABLE: &[u8] = b"#!/bin/sh\n: > \"$0.executed\"\nexit 97\n";

async fn archive(
    context: &TestContext,
    filename: &str,
    files: &[(&str, &[u8])],
) -> Result<(Url, String)> {
    let mut archive = TarEncoder::new(Vec::new()).builder();
    for (path, contents) in files {
        archive
            .add_file(path, *contents, EntryMetadata::default().executable(true))
            .await?;
    }
    let contents = archive.finish_into_inner().await?.into_inner();
    let file = context.temp_dir.child(filename);
    file.write_binary(&contents)?;
    let url = Url::from_file_path(file.path())
        .map_err(|()| anyhow::anyhow!("archive path must be absolute"))?;
    Ok((url, hex::encode(Sha256::digest(&contents))))
}

async fn catalog(context: &TestContext) -> Result<ChildPath> {
    let (pyodide_url, pyodide_sha) = archive(
        context,
        "pyodide.tar",
        &[("xbuildenv/pyodide-root/dist/python", INERT_EXECUTABLE)],
    )
    .await?;
    let (pypy_url, pypy_sha) = archive(
        context,
        "pypy.tar",
        &[
            ("pypy-root/bin/pypy3.11", INERT_EXECUTABLE),
            ("pypy-root/lib/pypy3.11/.keep", b""),
        ],
    )
    .await?;
    let pyodide = json!({
        "name": "cpython",
        "arch": { "family": "wasm32", "variant": null },
        "os": "emscripten",
        "libc": "musl",
        "major": 3,
        "minor": 13,
        "patch": 2,
        "prerelease": "",
        "url": pyodide_url,
        "sha256": pyodide_sha,
        "variant": null,
    });
    let mut older_pyodide = pyodide.clone();
    older_pyodide["minor"] = json!(12);
    older_pyodide["patch"] = json!(7);

    let downloads = context.temp_dir.child("downloads.json");
    downloads.write_str(&serde_json::to_string(&json!({
        "cpython-3.12.7-emscripten-wasm32-musl": older_pyodide,
        "cpython-3.13.2-emscripten-wasm32-musl": pyodide,
        "pypy-3.11.14-linux-x86_64-gnu": {
            "name": "pypy",
            "arch": { "family": "x86_64", "variant": null },
            "os": "linux",
            "libc": "gnu",
            "major": 3,
            "minor": 11,
            "patch": 14,
            "prerelease": "",
            "url": pypy_url,
            "sha256": pypy_sha,
            "variant": null,
        },
    }))?)?;
    Ok(downloads)
}

fn install(context: &TestContext, catalog: &ChildPath, path: &Path) -> Command {
    let mut command = context.python_install();
    command.env_clear();
    context.add_shared_env(&mut command, false);
    command
        .arg("--no-config")
        .arg("--offline")
        .arg("--no-bin")
        .arg("--no-registry")
        .arg("--python-downloads-json-url")
        .arg(catalog.path())
        .env(EnvVars::PATH, path)
        .env(EnvVars::UV_PYTHON_DOWNLOADS, "manual");
    command
}

fn assert_not_executed(context: &TestContext) -> Result<()> {
    for entry in WalkDir::new(context.root.path()) {
        let entry = entry?;
        assert!(
            !entry.file_name().to_string_lossy().ends_with(".executed"),
            "a fixture executable was invoked: {}",
            entry.path().display()
        );
    }
    Ok(())
}

#[tokio::test]
async fn python_install_pyodide_missing_node() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();
    let catalog = catalog(&context).await?;

    uv_snapshot!(context.filters(), install(&context, &catalog, context.bin_dir.path())
        .arg("cpython-3.13.2-emscripten-wasm32-musl"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.13.2 in [TIME]
     + pyodide-3.13.2-emscripten-wasm32-musl
    warning: Pyodide requires a `node` executable on `PATH`, but none was found
    ");

    // This currently reports another changed installation, rather than a true no-op.
    uv_snapshot!(context.filters(), install(&context, &catalog, context.bin_dir.path())
        .arg("pyodide@3.13"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.13.2 in [TIME]
     + pyodide-3.13.2-emscripten-wasm32-musl
    warning: Pyodide requires a `node` executable on `PATH`, but none was found
    ");

    assert_not_executed(&context)
}

#[tokio::test]
async fn python_install_pyodide_present_node() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();
    let catalog = catalog(&context).await?;
    let node = context.bin_dir.child("node");
    node.write_binary(INERT_EXECUTABLE)?;
    fs_err::set_permissions(node.path(), std::fs::Permissions::from_mode(0o755))?;

    uv_snapshot!(context.filters(), install(&context, &catalog, context.bin_dir.path())
        .arg("pyodide@3.13"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.13.2 in [TIME]
     + pyodide-3.13.2-emscripten-wasm32-musl
    ");

    assert_not_executed(&context)
}

#[tokio::test]
async fn python_install_pyodide_warning_is_specific() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();
    let catalog = catalog(&context).await?;

    uv_snapshot!(context.filters(), install(&context, &catalog, context.bin_dir.path())
        .arg("pypy-3.11.14-linux-x86_64-gnu"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed Python 3.11.14 in [TIME]
     + pypy-3.11.14-linux-x86_64-gnu
    ");

    uv_snapshot!(context.filters(), install(&context, &catalog, context.bin_dir.path())
        .arg("pypy-3.11.14-linux-x86_64-gnu"), @"
    exit_code: 0 (success)
    ----- stderr -----
    pypy-3.11.14-linux-x86_64-gnu is already installed
    ");

    assert_not_executed(&context)
}

#[tokio::test]
async fn python_install_pyodide_warns_once() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();
    let catalog = catalog(&context).await?;

    uv_snapshot!(context.filters(), install(&context, &catalog, context.bin_dir.path())
        .arg("pyodide@3.12")
        .arg("pyodide@3.13"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed 2 versions in [TIME]
     + pyodide-3.12.7-emscripten-wasm32-musl
     + pyodide-3.13.2-emscripten-wasm32-musl
    warning: Pyodide requires a `node` executable on `PATH`, but none was found
    ");

    assert_not_executed(&context)
}
