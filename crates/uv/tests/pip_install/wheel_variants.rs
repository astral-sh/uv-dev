use std::collections::BTreeMap;

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::indoc;
use predicates::prelude::predicate;
use serde_json::json;
use uv_static::EnvVars;
use uv_test::archive::generate_source_archive;
use uv_test::packse::generate_wheel_with_files;
use uv_test::uv_snapshot;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn variant_wheels_cannot_be_installed() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let links = context.temp_dir.child("links");
    links.create_dir_all()?;
    let wheel = links.child("ok-1.0.0-9-py3-none-any-null.whl");
    fs_err::copy(
        context
            .workspace_root
            .join("test/links/ok-1.0.0-py3-none-any.whl"),
        wheel.path(),
    )?;

    uv_snapshot!(
        context.filters(),
        context
            .pip_install()
            .arg("--no-index")
            .arg("--find-links")
            .arg(links.path())
            .arg("ok==1.0.0"),
        @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because ok==1.0.0 has only unsupported wheel variants and you require ok==1.0.0, we can conclude that your requirements are unsatisfiable.
    "
    );

    uv_snapshot!(
        context.filters(),
        context
            .pip_install()
            .arg("--no-index")
            .arg("--find-links")
            .arg(links.path())
            .arg("--preview-features")
            .arg("wheel-variants")
            .arg("ok==1.0.0"),
        @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because ok==1.0.0 has only unsupported wheel variants and you require ok==1.0.0, we can conclude that your requirements are unsatisfiable.
    "
    );

    uv_snapshot!(context.filters(), context.pip_install().arg(wheel.path()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to read `ok @ file://[TEMP_DIR]/links/ok-1.0.0-9-py3-none-any-null.whl`
      cause: Wheel variants are not supported yet: `ok-1.0.0-9-py3-none-any-null.whl`
    ");

    uv_snapshot!(
        context.filters(),
        context
            .pip_install()
            .arg("--preview-features")
            .arg("wheel-variants")
            .arg(wheel.path()),
        @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to read `ok @ file://[TEMP_DIR]/links/ok-1.0.0-9-py3-none-any-null.whl`
      cause: Wheel variants are not supported yet: `ok-1.0.0-9-py3-none-any-null.whl`
    "
    );
    Ok(())
}

#[test]
fn ordinary_wheels_are_selected_over_variants() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let links = context.temp_dir.child("links");
    links.create_dir_all()?;
    let (filename, wheel) = generate_wheel_with_files(
        &"ok".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[("ok/ordinary.py", "selected = True\n")],
    );
    links.child(filename).write_binary(&wheel)?;

    // Both variants contain valid wheel data, so rejecting an invalid archive cannot cause uv to
    // fall back to the ordinary wheel.
    fs_err::copy(
        context
            .workspace_root
            .join("test/links/ok-1.0.0-py3-none-any.whl"),
        links.child("ok-1.0.0-9-py3-none-any-cpu.whl"),
    )?;
    fs_err::copy(
        context
            .workspace_root
            .join("test/links/ok-2.0.0-py3-none-any.whl"),
        links.child("ok-2.0.0-py3-none-any-null.whl"),
    )?;

    uv_snapshot!(
        context.filters(),
        context
            .pip_install()
            .arg("--no-index")
            .arg("--find-links")
            .arg(links.path())
            .arg("--preview-features")
            .arg("wheel-variants")
            .arg("ok"),
        @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==1.0.0
    "
    );
    context
        .assert_command("from ok import ordinary; assert ordinary.selected")
        .success();

    Ok(())
}

/// With `--no-binary`, ordinary wheels can still provide metadata without running the sdist's
/// backend. A variant wheel must not displace them from metadata selection.
#[test]
fn no_binary_metadata_ignores_variant_wheels() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let links = context.temp_dir.child("links");
    links.create_dir_all()?;
    fs_err::copy(
        context
            .workspace_root
            .join("test/links/ok-1.0.0-py3-none-any.whl"),
        links.child("ok-1.0.0-py3-none-any.whl"),
    )?;
    links
        .child("ok-1.0.0-9-py3-none-any-cpu.whl")
        .write_str("invalid wheel")?;
    let backend_marker = context.temp_dir.child("backend-executed");
    let sdist = generate_source_archive(
        &"ok".parse()?,
        &"1.0.0".parse()?,
        "",
        Some(backend_marker.path()),
    )?;
    links.child("ok-1.0.0.tar.gz").write_binary(&sdist)?;
    context
        .temp_dir
        .child("requirements.in")
        .write_str("ok==1.0.0")?;

    uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in")
        .arg("--no-index")
        .arg("--find-links")
        .arg(links.path())
        .arg("--no-binary")
        .arg("ok"), @"
    exit_code: 0 (success)
    ----- stdout -----
    # This file was autogenerated by uv via the following command:
    #    uv pip compile --cache-dir [CACHE_DIR] requirements.in --no-index --no-binary ok
    ok==1.0.0
        # via -r requirements.in

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    backend_marker.assert(predicate::path::missing());
    Ok(())
}

/// A variant wheel cannot provide metadata for an sdist, even when it is the only wheel.
#[tokio::test]
async fn sdist_metadata_ignores_variant_wheels() -> Result<()> {
    let server = MockServer::start().await;
    let context = uv_test::test_context!("3.12").with_filter((server.uri(), "http://[LOCALHOST]"));
    let sdist = generate_source_archive(&"ok".parse()?, &"1.0.0".parse()?, "", None)?;
    let variant_filename = "ok-1.0.0-py3-none-any-cpu.whl";
    let variant_path = format!("/files/{variant_filename}");

    let simple_index = json!({
        "meta": { "api-version": "1.0" },
        "name": "ok",
        "files": [
            {
                "filename": "ok-1.0.0.tar.gz",
                "url": format!("{}/files/ok-1.0.0.tar.gz", server.uri()),
                "hashes": {},
            },
            {
                "filename": variant_filename,
                "url": format!("{}{variant_path}", server.uri()),
                "hashes": {},
            },
        ],
    });
    Mock::given(method("GET"))
        .and(path("/simple/ok/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            simple_index.to_string(),
            "application/vnd.pypi.simple.v1+json",
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/files/ok-1.0.0.tar.gz"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(sdist))
        .mount(&server)
        .await;
    Mock::given(path(variant_path))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--index-url")
        .arg(format!("{}/simple/", server.uri()))
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("ok==1.0.0"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==1.0.0
    ");
    Ok(())
}

#[test]
fn universal_lock_excludes_variant_wheels() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let links = context.temp_dir.child("links");
    links.create_dir_all()?;
    fs_err::copy(
        context
            .workspace_root
            .join("test/links/ok-1.0.0-py3-none-any.whl"),
        links.child("ok-1.0.0-py3-none-any.whl"),
    )?;
    links
        .child("ok-1.0.0-9-py3-none-any-cpu.whl")
        .write_str("invalid wheel")?;
    links
        .child("ok-2.0.0-py3-none-any-null.whl")
        .write_str("invalid wheel")?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "example"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["ok"]
    "#})?;

    uv_snapshot!(
        context.filters(),
        context
            .lock()
            .arg("--no-index")
            .arg("--find-links")
            .arg(links.path())
            .arg("--preview-features")
            .arg("wheel-variants"),
        @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    "
    );
    insta::with_settings!({filters => context.filters()}, {
        insta::assert_snapshot!(context.read("uv.lock"), @r#"
        version = 1
        revision = 5
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[package]]
        name = "example"
        version = "1.0.0"
        source = { virtual = "." }
        dependencies = [
            { name = "ok" },
        ]

        [package.metadata]
        requires-dist = [{ name = "ok" }]

        [[package]]
        name = "ok"
        version = "1.0.0"
        source = { registry = "[TEMP_DIR]/links" }
        wheels = [
            { path = "[TEMP_DIR]/links/ok-1.0.0-py3-none-any.whl" },
        ]
        "#);
    });

    // A lockfile containing a variant wheel must not make it installable.
    let lock = context.read("uv.lock").replace(
        "ok-1.0.0-py3-none-any.whl",
        "ok-1.0.0-9-py3-none-any-cpu.whl",
    );
    context.temp_dir.child("uv.lock").write_str(&lock)?;
    uv_snapshot!(context.filters(), context.sync()
        .arg("--frozen")
        .arg("--preview-features")
        .arg("wheel-variants"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Distribution `ok==1.0.0 @ registry+[TEMP_DIR]/links` can't be installed because it doesn't have a source distribution or wheel for the current platform
    ");
    Ok(())
}

async fn variant_only_index() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/simple/ok/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"<a href="../../files/ok-2.0.0-py3-none-any-null.whl">ok-2.0.0-py3-none-any-null.whl</a>"#,
            "text/html",
        ))
        .expect(1..)
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn registry_variant_wheels_are_not_selectable() {
    let server = variant_only_index().await;
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(
        context.filters(),
        context
            .pip_install()
            .arg("--index-url")
            .arg(format!("{}/simple/", server.uri()))
            .env_remove(EnvVars::UV_EXCLUDE_NEWER)
            .arg("--preview-features")
            .arg("wheel-variants")
            .arg("ok"),
        @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because ok==2.0.0 has only unsupported wheel variants and only ok==2.0.0 is available, we can conclude that all versions of ok cannot be used.
             And because you require ok, we can conclude that your requirements are unsatisfiable.
    "
    );
}

#[tokio::test]
async fn registry_variant_wheels_are_not_outdated_candidates() {
    let server = variant_only_index().await;
    let context = uv_test::test_context!("3.12");
    let installed_wheel = context
        .workspace_root
        .join("test/links/ok-1.0.0-py3-none-any.whl");
    context
        .pip_install()
        .arg(installed_wheel)
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_list()
        .arg("--outdated")
        .arg("--format")
        .arg("json")
        .arg("--index-url")
        .arg(format!("{}/simple/", server.uri()))
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--preview-features")
        .arg("wheel-variants"), @"
    exit_code: 0 (success)
    ----- stdout -----
    []
    ");
}
