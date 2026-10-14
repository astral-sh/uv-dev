//! Cache reuse and eviction for local distribution paths.

use std::process::Command;

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;

use uv_redacted::SafeUrl;
use uv_test::archive::generate_source_archive;
use uv_test::{TestContext, apply_filters};

fn context() -> TestContext {
    uv_test::test_context!("3.12")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units()
}

fn assert_stderr(context: &TestContext, command: &mut Command, expected: &str) {
    let output = command.assert().success().stdout("");
    let stderr = String::from_utf8_lossy(&output.get_output().stderr).into_owned();
    assert_eq!(apply_filters(stderr, context.filters()), expected);
}

enum CacheState {
    Cold,
    Warm,
}

fn assert_sync(context: &TestContext, archive: &ChildPath, cache: CacheState) {
    let filename = archive
        .file_name()
        .expect("archive filename")
        .to_string_lossy();
    let preparation = match cache {
        CacheState::Cold => "Prepared 1 package in [TIME]\n",
        CacheState::Warm => "",
    };
    let expected = format!(
        "Resolved 1 package in [TIME]\n{preparation}Installed 1 package in [TIME]\n + tqdm==1000.0.0 (from file://[TEMP_DIR]/{filename})\n"
    );
    assert_stderr(
        context,
        context
            .pip_sync()
            .args(["requirements.txt", "--strict", "--offline"]),
        &expected,
    );
}

fn cache_cycle(
    context: &TestContext,
    archive: &ChildPath,
    backend_marker: Option<&ChildPath>,
) -> Result<()> {
    let url = SafeUrl::from_file_path(archive.path()).expect("absolute archive path");
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&format!("tqdm @ {url}"))?;

    assert_sync(context, archive, CacheState::Cold);
    context.assert_command("import tqdm").success();
    if let Some(marker) = backend_marker {
        marker.assert(predicates::path::is_file());
        fs_err::remove_file(marker)?;
    }

    context.reset_venv();
    assert_sync(context, archive, CacheState::Warm);
    context.assert_command("import tqdm").success();
    if let Some(marker) = backend_marker {
        marker.assert(predicates::path::missing());
    }

    context.reset_venv();
    assert_stderr(
        context,
        context.clean().arg("tqdm"),
        "Removed [N] files ([SIZE])\n",
    );
    assert_sync(context, archive, CacheState::Cold);
    context.assert_command("import tqdm").success();
    if let Some(marker) = backend_marker {
        marker.assert(predicates::path::is_file());
    }

    Ok(())
}

#[test]
fn install_path_source_dist_cached() -> Result<()> {
    let context = context();
    let archive = context.temp_dir.child("tqdm-1000.0.0.tar.gz");
    let marker = context.temp_dir.child("backend-ran");
    archive.write_binary(&generate_source_archive(
        &"tqdm".parse()?,
        &"1000.0.0".parse()?,
        "",
        Some(marker.path()),
    )?)?;
    cache_cycle(&context, &archive, Some(&marker))
}

#[test]
fn install_path_built_dist_cached() -> Result<()> {
    let context = context();
    let filename = "tqdm-1000.0.0-py3-none-any.whl";
    let archive = context.temp_dir.child(filename);
    fs_err::copy(
        context.workspace_root.join("test/links").join(filename),
        &archive,
    )?;
    cache_cycle(&context, &archive, None)
}
