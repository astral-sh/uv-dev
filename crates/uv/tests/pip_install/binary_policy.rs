//! Artifact selection when wheel and source distribution policies overlap.

use std::collections::BTreeMap;
use std::process::Command;

use anyhow::Result;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;

use uv_pep508::Requirement;
use uv_test::archive::generate_source_archive;
use uv_test::packse::generate_wheel;
use uv_test::{TestContext, uv_snapshot};

fn write_wheel(context: &TestContext, name: &str, requires: &[Requirement]) -> Result<()> {
    let (filename, wheel) = generate_wheel(
        &name.parse()?,
        &"1.0.0".parse()?,
        requires,
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    context.temp_dir.child(filename).write_binary(&wheel)?;
    Ok(())
}

/// Write both distribution types and return a marker created only by the source build backend.
fn wheel_and_sdist(context: &TestContext, name: &str) -> Result<ChildPath> {
    write_wheel(context, name, &[])?;
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
    Ok(source_built)
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
    let source_built = wheel_and_sdist(&context, "example")?;

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

    source_built.assert(predicates::path::is_file());
    context.assert_installed("parent", "1.0.0");
    Ok(())
}
