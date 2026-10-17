//! Artifact selection when wheel and source distribution policies overlap.

use std::collections::BTreeMap;
use std::process::Command;

use anyhow::Result;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_pep508::Requirement;
use uv_test::archive::generate_source_archive;
use uv_test::packse::generate_wheel_with_files;
use uv_test::{TestContext, uv_snapshot};

struct ArtifactMarkers {
    source_built: ChildPath,
    wheel_installed: ChildPath,
}

impl ArtifactMarkers {
    fn assert_source_selected(&self) {
        self.source_built.assert(predicates::path::is_file());
        self.wheel_installed.assert(predicates::path::missing());
    }

    fn assert_wheel_selected(&self) {
        self.wheel_installed.assert("wheel\n");
        self.source_built.assert(predicates::path::missing());
    }
}

fn write_wheel(context: &TestContext, name: &str, requires: &[Requirement]) -> Result<ChildPath> {
    let package_name = name.parse::<uv_normalize::PackageName>()?;
    let marker = format!("{}/wheel-marker.txt", package_name.as_dist_info_name());
    let (filename, wheel) = generate_wheel_with_files(
        &package_name,
        &"1.0.0".parse()?,
        requires,
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[(&marker, "wheel\n")],
    );
    context.temp_dir.child(filename).write_binary(&wheel)?;
    Ok(ChildPath::new(context.site_packages()).child(marker))
}

fn write_sdist(context: &TestContext, name: &str, version: &str) -> Result<ChildPath> {
    let package_name = name.parse()?;
    let version = version.parse()?;
    let source_built = context.temp_dir.child(format!("{name}-built"));
    let source = generate_source_archive(&package_name, &version, "", Some(source_built.path()))?;
    context
        .temp_dir
        .child(format!(
            "{}-{version}.tar.gz",
            package_name.as_dist_info_name()
        ))
        .write_binary(&source)?;
    Ok(source_built)
}

/// Write both distribution types with markers identifying which artifact was selected.
fn wheel_and_sdist(context: &TestContext, name: &str) -> Result<ArtifactMarkers> {
    let wheel_installed = write_wheel(context, name, &[])?;
    let source_built = write_sdist(context, name, "1.0.0")?;
    Ok(ArtifactMarkers {
        source_built,
        wheel_installed,
    })
}

fn install(context: &TestContext) -> Command {
    let mut command = context.pip_install();
    command
        .arg("--no-index")
        .arg("--find-links")
        .arg(context.temp_dir.path());
    command
}

/// Serve the named local artifacts as a registry package.
async fn registry_index(context: &TestContext, name: &str, archives: &[&str]) -> MockServer {
    let server = MockServer::start().await;
    let mut files = Vec::with_capacity(archives.len());
    for &archive in archives {
        files.push(json!({
            "filename": archive,
            "url": format!("{}/{archive}", server.uri()),
            "hashes": {},
            "upload-time": "2024-01-01T00:00:00Z",
        }));
        Mock::given(method("GET"))
            .and(path(format!("/{archive}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(context.read_bytes(context.temp_dir.child(archive).path())),
            )
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path(format!("/simple/{name}/")))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            json!({ "meta": { "api-version": "1.0" }, "name": name, "files": files }).to_string(),
            "application/vnd.pypi.simple.v1+json",
        ))
        .mount(&server)
        .await;
    server
}

/// A package-specific source policy overrides the global wheel-only policy for a dependency.
#[test]
fn no_binary_package_overrides_only_binary_all() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    write_wheel(&context, "parent", &["example".parse()?])?;
    let markers = wheel_and_sdist(&context, "example")?;

    uv_snapshot!(context.filters(), install(&context)
        .arg("parent")
        .arg("--only-binary")
        .arg(":all:")
        .arg("--no-binary")
        .arg("example")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + example==1.0.0
     + parent==1.0.0
    ");

    markers.assert_source_selected();
    context.assert_installed("parent", "1.0.0");
    Ok(())
}

/// A package-specific wheel policy overrides the global source-only policy.
#[test]
fn only_binary_package_overrides_no_binary_all() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let markers = wheel_and_sdist(&context, "example")?;

    uv_snapshot!(context.filters(), install(&context)
        .arg("example")
        .arg("--no-binary")
        .arg(":all:")
        .arg("--only-binary")
        .arg("example")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + example==1.0.0
    ");

    markers.assert_wheel_selected();
    Ok(())
}

/// Each comma-separated package name applies the source policy to a dependency.
#[test]
fn no_binary_comma_separated() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    write_wheel(&context, "parent", &["alpha".parse()?, "beta".parse()?])?;
    let alpha = wheel_and_sdist(&context, "alpha")?;
    let beta = wheel_and_sdist(&context, "beta")?;

    uv_snapshot!(context.filters(), install(&context)
        .arg("parent")
        .arg("--no-binary=alpha,beta")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + alpha==1.0.0
     + beta==1.0.0
     + parent==1.0.0
    ");

    alpha.assert_source_selected();
    beta.assert_source_selected();
    Ok(())
}

/// Each comma-separated package name overrides the global source-only policy.
#[test]
fn only_binary_comma_separated() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    write_wheel(&context, "parent", &["alpha".parse()?, "beta".parse()?])?;
    let alpha = wheel_and_sdist(&context, "alpha")?;
    let beta = wheel_and_sdist(&context, "beta")?;

    uv_snapshot!(context.filters(), install(&context)
        .arg("parent")
        .arg("--no-binary=:all:")
        .arg("--only-binary=alpha,beta,parent")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + alpha==1.0.0
     + beta==1.0.0
     + parent==1.0.0
    ");

    alpha.assert_wheel_selected();
    beta.assert_wheel_selected();
    Ok(())
}

/// Simultaneous global source-only and wheel-only policies reject both artifacts.
// TODO(zanieb): We should have a better error message here.
#[test]
fn only_binary_all_and_no_binary_all() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let markers = wheel_and_sdist(&context, "anyio")?;

    uv_snapshot!(context.filters(), install(&context)
        .arg("anyio")
        .arg("--no-binary")
        .arg(":all:")
        .arg("--only-binary")
        .arg(":all:")
        .arg("--strict"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because anyio==1.0.0 has no usable wheels and only anyio==1.0.0 is available, we can conclude that all versions of anyio cannot be used.
             And because you require anyio, we can conclude that your requirements are unsatisfiable.

    hint: Wheels are required for `anyio` because building from source is disabled for all packages (i.e., with `--no-build`)
    ");

    markers.source_built.assert(predicates::path::missing());
    context.assert_not_installed("anyio");
    Ok(())
}

/// Cached registry artifacts remain subject to the active binary and build policies.
#[tokio::test]
async fn build_policy_cache() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let markers = wheel_and_sdist(&context, "idna")?;
    let server = registry_index(
        &context,
        "idna",
        &["idna-1.0.0-py3-none-any.whl", "idna-1.0.0.tar.gz"],
    )
    .await;
    let index_url = format!("{}/simple", server.uri());
    let install = || {
        let mut command = context.pip_install();
        command.arg("idna").arg("--index-url").arg(&index_url);
        command
    };

    // Download the published wheel, then reuse it from the cache.
    uv_snapshot!(context.filters(), install(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + idna==1.0.0
    ");
    markers.assert_wheel_selected();

    context.reset_venv();
    uv_snapshot!(context.filters(), install(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + idna==1.0.0
    ");
    markers.assert_wheel_selected();

    // A source-only policy must bypass the cached published wheel.
    context.reset_venv();
    uv_snapshot!(context.filters(), install().arg("--no-binary=:all:"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + idna==1.0.0
    ");
    markers.assert_source_selected();

    // The built wheel may be reused offline without executing the source backend again.
    fs_err::remove_file(markers.source_built.path())?;
    context.reset_venv();
    uv_snapshot!(context.filters(), install().arg("--no-binary=:all:").arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + idna==1.0.0
    ");
    context.assert_installed("idna", "1.0.0");
    markers.wheel_installed.assert(predicates::path::missing());
    markers.source_built.assert(predicates::path::missing());

    // With building disabled, either cached wheel can be reused without running the backend.
    context.reset_venv();
    uv_snapshot!(context.filters(), install().arg("--no-build").arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + idna==1.0.0
    ");
    context.assert_installed("idna", "1.0.0");
    markers.source_built.assert(predicates::path::missing());
    Ok(())
}

/// A requirements-file wheel-only policy rejects an available source distribution.
#[test]
fn only_binary_requirements_txt() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let source_built = write_sdist(&context, "django-allauth", "0.51.0")?;
    let requirements = context.temp_dir.child("requirements.txt");
    requirements.write_str("django_allauth==0.51.0\n--only-binary django_allauth\n")?;

    uv_snapshot!(context.filters(), install(&context)
        .arg("-r")
        .arg(requirements.path())
        .arg("--strict"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because django-allauth==0.51.0 has no usable wheels and you require django-allauth==0.51.0, we can conclude that your requirements are unsatisfiable.

    hint: Wheels are required for `django-allauth` because building from source is disabled for `django-allauth` (i.e., with `--no-build-package django-allauth`)
    ");

    source_built.assert(predicates::path::missing());
    context.assert_not_installed("django_allauth");
    Ok(())
}
