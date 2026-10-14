//! Artifact selection when wheel and source distribution policies overlap.

use std::collections::BTreeMap;
use std::process::Command;

use anyhow::Result;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;

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

/// Write both distribution types with markers identifying which artifact was selected.
fn wheel_and_sdist(context: &TestContext, name: &str) -> Result<ArtifactMarkers> {
    let wheel_installed = write_wheel(context, name, &[])?;
    let package_name = name.parse()?;
    let version = "1.0.0".parse()?;
    let source_built = context.temp_dir.child(format!("{name}-built"));
    let source = generate_source_archive(&package_name, &version, "", Some(source_built.path()))?;
    context
        .temp_dir
        .child(format!(
            "{}-{version}.tar.gz",
            package_name.as_dist_info_name()
        ))
        .write_binary(&source)?;
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
