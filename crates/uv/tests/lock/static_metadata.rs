use std::collections::BTreeMap;
use std::process::Command;

use anyhow::Result;
#[cfg(feature = "test-git")]
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};

use uv_test::packse::generate_wheel;
use uv_test::{TestContext, uv_snapshot};

fn wheelhouse(context: &TestContext) -> Result<()> {
    let links = context.temp_dir.child("links");
    links.create_dir_all()?;
    for name in ["parent", "a", "b"] {
        let (filename, bytes) = generate_wheel(
            &name.parse()?,
            &"1.0.0".parse()?,
            &[],
            &BTreeMap::new(),
            None,
            "py3-none-any",
            &[],
        );
        links.child(filename).write_binary(&bytes)?;
    }
    Ok(())
}

fn configure(
    context: &TestContext,
    resolution_inputs: bool,
    versioned: bool,
    dependencies: &[&str],
) -> Result<()> {
    let preview = if resolution_inputs {
        "preview-features = [\"resolution-inputs\"]"
    } else {
        ""
    };
    let mut project = formatdoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        {preview}
    "#};
    for dependency in dependencies {
        let version = if versioned { "version = '1.0.0'" } else { "" };
        project.push_str(&formatdoc! {r#"

            [[tool.uv.dependency-metadata]]
            name = "parent"
            {version}
            requires-dist = ["{dependency}"]
        "#});
    }
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&project)?;
    Ok(())
}

fn lock(context: &TestContext) -> Command {
    let mut command = context.lock();
    command.args(["--offline", "--no-index", "--find-links", "links"]);
    command
}

fn package_names(context: &TestContext) -> Result<Vec<String>> {
    let lock: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    Ok(lock["package"]
        .as_array()
        .expect("lockfile packages")
        .iter()
        .map(|package| package["name"].as_str().expect("package name").to_string())
        .collect())
}

#[test]
fn equivalent_static_metadata_payloads_keep_the_lock() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    wheelhouse(&context)?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = []

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["b", "a>=0", "a>=1"]
        requires-python = ">=3.9,>=3.10"
        provides-extra = ["gpu", "cpu", "gpu"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    assert_eq!(package_names(&context)?, ["a", "b", "parent", "project"]);
    let original = context.read("uv.lock");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = []

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a>=1", "b"]
        requires-python = ">=3.10"
        provides-extra = ["cpu", "gpu"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"])
        .arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    assert_eq!(context.read("uv.lock"), original);

    // A fresh resolve compares payload semantics without rewriting equivalent inputs.
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--find-links", "links", "--refresh", "--locked"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--find-links", "links", "--refresh"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    assert_eq!(context.read("uv.lock"), original);

    let parsed: toml::Value = toml::from_str(&original)?;
    let payload = &parsed["manifest"]["dependency-metadata"][0];
    assert_eq!(
        payload["requires-dist"]
            .as_array()
            .expect("requirements")
            .len(),
        3
    );
    assert_eq!(
        payload["provides-extras"].as_array().expect("extras").len(),
        3
    );
    Ok(())
}

#[test]
fn equivalent_static_metadata_payloads_keep_the_lock_normalized() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    wheelhouse(&context)?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = ["lockfile-normalization"]

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["b", "a>=0", "a>=1"]
        requires-python = ">=3.9,>=3.10"
        provides-extra = ["gpu", "cpu", "gpu"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    assert_eq!(package_names(&context)?, ["a", "b", "parent", "project"]);
    let original = context.read("uv.lock");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = ["lockfile-normalization"]

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a>=1", "b"]
        requires-python = ">=3.10"
        provides-extra = ["cpu", "gpu"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"])
        .arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    assert_eq!(context.read("uv.lock"), original);

    // A fresh resolve compares payload semantics without rewriting equivalent inputs.
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--find-links", "links", "--refresh", "--locked"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--find-links", "links", "--refresh"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    assert_eq!(context.read("uv.lock"), original);

    let parsed: toml::Value = toml::from_str(&original)?;
    let payload = &parsed["manifest"]["dependency-metadata"][0];
    assert_eq!(
        payload["requires-dist"]
            .as_array()
            .expect("requirements")
            .len(),
        2
    );
    assert_eq!(
        payload["provides-extras"].as_array().expect("extras").len(),
        2
    );
    Ok(())
}

#[test]
fn equivalent_static_metadata_payloads_keep_the_lock_without_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    wheelhouse(&context)?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = ["lock-without-metadata"]

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["b", "a>=0", "a>=1"]
        requires-python = ">=3.9,>=3.10"
        provides-extra = ["gpu", "cpu", "gpu"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    assert_eq!(package_names(&context)?, ["a", "b", "parent", "project"]);
    let original = context.read("uv.lock");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = ["lock-without-metadata"]

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a>=1", "b"]
        requires-python = ">=3.10"
        provides-extra = ["cpu", "gpu"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"])
        .arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    assert_eq!(context.read("uv.lock"), original);

    // A fresh resolve compares payload semantics without rewriting equivalent inputs.
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--find-links", "links", "--refresh", "--locked"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--find-links", "links", "--refresh"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    assert_eq!(context.read("uv.lock"), original);

    let parsed: toml::Value = toml::from_str(&original)?;
    let payload = &parsed["manifest"]["dependency-metadata"][0];
    assert_eq!(
        payload["requires-dist"]
            .as_array()
            .expect("requirements")
            .len(),
        3
    );
    assert_eq!(
        payload["provides-extras"].as_array().expect("extras").len(),
        3
    );
    Ok(())
}

#[test]
fn equivalent_static_metadata_payloads_keep_the_lock_normalized_without_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    wheelhouse(&context)?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = ["lock-without-metadata", "lockfile-normalization"]

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["b", "a>=0", "a>=1"]
        requires-python = ">=3.9,>=3.10"
        provides-extra = ["gpu", "cpu", "gpu"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    assert_eq!(package_names(&context)?, ["a", "b", "parent", "project"]);
    let original = context.read("uv.lock");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = ["lock-without-metadata", "lockfile-normalization"]

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a>=1", "b"]
        requires-python = ">=3.10"
        provides-extra = ["cpu", "gpu"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"])
        .arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    assert_eq!(context.read("uv.lock"), original);

    // A fresh resolve compares payload semantics without rewriting equivalent inputs.
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--find-links", "links", "--refresh", "--locked"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--find-links", "links", "--refresh"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    assert_eq!(context.read("uv.lock"), original);

    let parsed: toml::Value = toml::from_str(&original)?;
    let payload = &parsed["manifest"]["dependency-metadata"][0];
    assert_eq!(
        payload["requires-dist"]
            .as_array()
            .expect("requirements")
            .len(),
        2
    );
    assert_eq!(
        payload["provides-extras"].as_array().expect("extras").len(),
        2
    );
    Ok(())
}

#[test]
fn equivalent_static_path_wheel_metadata_keeps_the_lock() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    wheelhouse(&context)?;
    let pyproject = context.temp_dir.child("pyproject.toml");
    pyproject.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv.sources]
        parent = { path = "links/parent-1.0.0-py3-none-any.whl" }

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["b", "a>=0", "a>=1"]
        requires-python = ">=3.9,>=3.10"
        provides-extra = ["gpu", "cpu", "gpu"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    let original = context.read("uv.lock");
    insta::with_settings!({ filters => context.filters() }, {
        insta::assert_snapshot!(original, @r#"
        version = 1
        revision = 5
        requires-python = ">=3.12"

        [options]
        exclude-newer = "2024-03-25T00:00:00Z"

        [[manifest.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["b", "a>=0", "a>=1"]
        requires-python = ">=3.9, >=3.10"
        provides-extras = ["gpu", "cpu", "gpu"]

        [[package]]
        name = "a"
        version = "1.0.0"
        source = { registry = "links" }
        wheels = [
            { path = "a-1.0.0-py3-none-any.whl" },
        ]

        [[package]]
        name = "b"
        version = "1.0.0"
        source = { registry = "links" }
        wheels = [
            { path = "b-1.0.0-py3-none-any.whl" },
        ]

        [[package]]
        name = "parent"
        version = "1.0.0"
        source = { path = "links/parent-1.0.0-py3-none-any.whl" }
        dependencies = [
            { name = "a" },
            { name = "b" },
        ]
        wheels = [
            { filename = "parent-1.0.0-py3-none-any.whl", hash = "sha256:a4aea21704ea482b671a53998378d2365acd69b5c497aa3bc9c5d4bd97cd8683" },
        ]

        [package.metadata]
        requires-dist = [
            { name = "a", specifier = ">=0" },
            { name = "a", specifier = ">=1" },
            { name = "b" },
        ]
        provides-extras = ["gpu", "cpu", "gpu"]

        [[package]]
        name = "project"
        version = "1.0.0"
        source = { virtual = "." }
        dependencies = [
            { name = "parent" },
        ]

        [package.metadata]
        requires-dist = [{ name = "parent", path = "links/parent-1.0.0-py3-none-any.whl" }]
        "#);
    });

    pyproject.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv.sources]
        parent = { path = "links/parent-1.0.0-py3-none-any.whl" }

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a>=1", "b"]
        requires-python = ">=3.10"
        provides-extra = ["cpu", "gpu"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]).arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock().args([
        "--no-index", "--find-links", "links", "--refresh", "--locked",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock().args([
        "--no-index", "--find-links", "links", "--refresh",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    ");
    assert_eq!(original, context.read("uv.lock"));
    Ok(())
}

#[test]
fn static_metadata_payload_prerelease_policy_requires_new_lock() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    wheelhouse(&context)?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = []

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a>=1rc1,>=1"]
        requires-python = ">=3.10"
        provides-extra = []
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    let original = context.read("uv.lock");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = []

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a>=1"]
        requires-python = ">=3.10"
        provides-extra = []
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"])
        .arg("--locked"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(context.read("uv.lock"), original);
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"])
        .arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    Ok(())
}

#[test]
fn static_metadata_payload_exact_pin_policy_requires_new_lock() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    wheelhouse(&context)?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = []

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a==1"]
        requires-python = ">=3.10"
        provides-extra = []
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    let original = context.read("uv.lock");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = []

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a==1,>=0"]
        requires-python = ">=3.10"
        provides-extra = []
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"])
        .arg("--locked"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(context.read("uv.lock"), original);
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"])
        .arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    Ok(())
}

#[test]
fn static_metadata_payload_python_wildcard_precision_requires_new_lock() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    wheelhouse(&context)?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = []

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a"]
        requires-python = ">=3.9,!=3.10.*"
        provides-extra = []
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    let original = context.read("uv.lock");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = []

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a"]
        requires-python = ">=3.9,!=3.10.0.*"
        provides-extra = []
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"])
        .arg("--locked"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(context.read("uv.lock"), original);
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"])
        .arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    Ok(())
}

#[test]
fn static_metadata_payload_extra_removal_requires_new_lock() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    wheelhouse(&context)?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = []

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a"]
        requires-python = ">=3.10"
        provides-extra = ["gpu"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    let original = context.read("uv.lock");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = []

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a"]
        requires-python = ">=3.10"
        provides-extra = []
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"])
        .arg("--locked"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(context.read("uv.lock"), original);
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"])
        .arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    Ok(())
}

#[test]
fn static_metadata_payload_prerelease_policy_requires_new_lock_without_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    wheelhouse(&context)?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = ["lock-without-metadata"]

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a>=1rc1,>=1"]
        requires-python = ">=3.10"
        provides-extra = []
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    let original = context.read("uv.lock");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = ["lock-without-metadata"]

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a>=1"]
        requires-python = ">=3.10"
        provides-extra = []
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"])
        .arg("--locked"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(context.read("uv.lock"), original);
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"])
        .arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    Ok(())
}

#[test]
fn static_metadata_payload_exact_pin_policy_requires_new_lock_without_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    wheelhouse(&context)?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = ["lock-without-metadata"]

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a==1"]
        requires-python = ">=3.10"
        provides-extra = []
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    let original = context.read("uv.lock");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = ["lock-without-metadata"]

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a==1,>=0"]
        requires-python = ">=3.10"
        provides-extra = []
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"])
        .arg("--locked"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(context.read("uv.lock"), original);
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"])
        .arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    Ok(())
}

#[test]
fn static_metadata_payload_python_wildcard_precision_requires_new_lock_without_metadata()
-> Result<()> {
    let context = uv_test::test_context!("3.12");
    wheelhouse(&context)?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = ["lock-without-metadata"]

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a"]
        requires-python = ">=3.9,!=3.10.*"
        provides-extra = []
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    let original = context.read("uv.lock");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = ["lock-without-metadata"]

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a"]
        requires-python = ">=3.9,!=3.10.0.*"
        provides-extra = []
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"])
        .arg("--locked"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(context.read("uv.lock"), original);
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"])
        .arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    Ok(())
}

#[test]
fn static_metadata_payload_extra_removal_requires_new_lock_without_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    wheelhouse(&context)?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = ["lock-without-metadata"]

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a"]
        requires-python = ">=3.10"
        provides-extra = ["gpu"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    let original = context.read("uv.lock");

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["parent==1.0.0"]

        [tool.uv]
        preview-features = ["lock-without-metadata"]

        [[tool.uv.dependency-metadata]]
        name = "parent"
        version = "1.0.0"
        requires-dist = ["a"]
        requires-python = ">=3.10"
        provides-extra = []
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"])
        .arg("--locked"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(context.read("uv.lock"), original);
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    uv_snapshot!(context.filters(), context.lock()
        .args(["--offline", "--no-index", "--find-links", "links"])
        .arg("--locked"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    Ok(())
}

#[test]
fn repeated_static_metadata_follows_declaration_order() -> Result<()> {
    insta::allow_duplicates! {
        for resolution_inputs in [false, true] {
            for versioned in [false, true] {
                let context = uv_test::test_context!("3.12");
                wheelhouse(&context)?;
                configure(&context, resolution_inputs, versioned, &["a", "b"])?;
                uv_snapshot!(context.filters(), lock(&context), @"
                exit_code: 0 (success)
                ----- stderr -----
                Resolved 3 packages in [TIME]
                ");
                assert_eq!(package_names(&context)?, ["a", "parent", "project"]);
                let original = context.read("uv.lock");

                configure(&context, resolution_inputs, versioned, &["b", "a"])?;
                uv_snapshot!(context.filters(), lock(&context).arg("--locked"), @"
                exit_code: 1 (failure)
                ----- stderr -----
                Resolved 3 packages in [TIME]
                error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

                hint: To update the lockfile, run `uv lock`.
                ");
                assert_eq!(context.read("uv.lock"), original);
                uv_snapshot!(context.filters(), lock(&context), @"
                exit_code: 0 (success)
                ----- stderr -----
                Resolved 3 packages in [TIME]
                Removed a v1.0.0
                Added b v1.0.0
                ");
                assert_eq!(package_names(&context)?, ["b", "parent", "project"]);
                let updated = context.read("uv.lock");
                let parsed: toml::Value = toml::from_str(&updated)?;
                assert_eq!(parsed["manifest"]["dependency-metadata-ordered"].as_bool(), Some(true));
                assert_eq!(parsed["manifest"]["dependency-metadata"].as_array().expect("metadata").len(), 2);
                uv_snapshot!(context.filters(), lock(&context).arg("--locked"), @"
                exit_code: 0 (success)
                ----- stderr -----
                Resolved 3 packages in [TIME]
                ");
                uv_snapshot!(context.filters(), lock(&context), @"
                exit_code: 0 (success)
                ----- stderr -----
                Resolved 3 packages in [TIME]
                ");
                assert_eq!(context.read("uv.lock"), updated);
            }
        }
        Ok(())
    }
}

#[test]
fn legacy_static_metadata_order_requires_resolution() -> Result<()> {
    insta::allow_duplicates! {
        for resolution_inputs in [false, true] {
            let context = uv_test::test_context!("3.12");
            wheelhouse(&context)?;
            configure(&context, resolution_inputs, true, &["b", "a"])?;
            uv_snapshot!(context.filters(), lock(&context), @"
            exit_code: 0 (success)
            ----- stderr -----
            Resolved 3 packages in [TIME]
            ");
            assert_eq!(package_names(&context)?, ["b", "parent", "project"]);

            // A sorted manifest cannot reveal which declaration produced the recorded graph.
            let mut legacy: toml::Value = toml::from_str(&context.read("uv.lock"))?;
            let manifest = legacy["manifest"].as_table_mut().expect("manifest");
            manifest.remove("dependency-metadata-ordered");
            manifest.get_mut("dependency-metadata").expect("metadata").as_array_mut().expect("metadata entries")
                .sort_by_key(|entry| entry["requires-dist"].to_string());
            let legacy = toml::to_string(&legacy)?;
            context.temp_dir.child("uv.lock").write_str(&legacy)?;
            configure(&context, resolution_inputs, true, &["a", "b"])?;
            uv_snapshot!(context.filters(), lock(&context).arg("--locked"), @"
            exit_code: 1 (failure)
            ----- stderr -----
            Resolved 3 packages in [TIME]
            error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

            hint: To update the lockfile, run `uv lock`.
            ");
            assert_eq!(context.read("uv.lock"), legacy);
            uv_snapshot!(context.filters(), lock(&context), @"
            exit_code: 0 (success)
            ----- stderr -----
            Resolved 3 packages in [TIME]
            Added a v1.0.0
            Removed b v1.0.0
            ");
            assert_eq!(package_names(&context)?, ["a", "parent", "project"]);
            uv_snapshot!(context.filters(), lock(&context).arg("--locked"), @"
            exit_code: 0 (success)
            ----- stderr -----
            Resolved 3 packages in [TIME]
            ");
        }
        Ok(())
    }
}

#[test]
fn identical_static_metadata_entries_retain_cardinality() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    wheelhouse(&context)?;
    configure(&context, true, true, &["a", "a"])?;
    uv_snapshot!(context.filters(), lock(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    let original = context.read("uv.lock");
    configure(&context, true, true, &["a"])?;
    uv_snapshot!(context.filters(), lock(&context).arg("--locked"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(context.read("uv.lock"), original);
    let parsed: toml::Value = toml::from_str(&original)?;
    assert_eq!(
        parsed["manifest"]["dependency-metadata"]
            .as_array()
            .expect("metadata")
            .len(),
        2
    );
    uv_snapshot!(context.filters(), lock(&context), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    ");
    let unique: toml::Value = toml::from_str(&context.read("uv.lock"))?;
    assert!(
        unique["manifest"]
            .get("dependency-metadata-ordered")
            .is_none()
    );
    Ok(())
}

#[test]
fn static_metadata_distinct_keys_and_name_retention() -> Result<()> {
    insta::allow_duplicates! {
        for resolution_inputs in [false, true] {
            let context = uv_test::test_context!("3.12");
            wheelhouse(&context)?;
            configure(&context, resolution_inputs, true, &["a"])?;
            let project = context.read("pyproject.toml");
            context.temp_dir.child("pyproject.toml").write_str(&formatdoc! {r#"
                {project}

                [[tool.uv.dependency-metadata]]
                name = "parent"
                requires-dist = ["b"]

                [[tool.uv.dependency-metadata]]
                name = "unused"
                version = "1.0.0"
                requires-dist = ["a"]

                [[tool.uv.dependency-metadata]]
                name = "unused"
                version = "1.0.0"
                requires-dist = ["b"]
            "#})?;
            uv_snapshot!(context.filters(), lock(&context), @"
            exit_code: 0 (success)
            ----- stderr -----
            Resolved 3 packages in [TIME]
            ");
            assert_eq!(package_names(&context)?, ["a", "parent", "project"]);
            let original = context.read("uv.lock");
            let parsed: toml::Value = toml::from_str(&original)?;
            let manifest = &parsed["manifest"];
            let entries = manifest["dependency-metadata"].as_array().expect("metadata");
            assert_eq!(entries.len(), if resolution_inputs { 2 } else { 4 });
            assert_eq!(manifest.get("dependency-metadata-ordered").is_some(), !resolution_inputs);

            // Exact matches outrank fallback entries regardless of their relative declaration order.
            let mut project: toml::Value = toml::from_str(&context.read("pyproject.toml"))?;
            project["tool"]["uv"]["dependency-metadata"].as_array_mut().expect("metadata").swap(0, 1);
            context.temp_dir.child("pyproject.toml").write_str(&toml::to_string(&project)?)?;
            uv_snapshot!(context.filters(), lock(&context).arg("--locked"), @"
            exit_code: 0 (success)
            ----- stderr -----
            Resolved 3 packages in [TIME]
            ");
            uv_snapshot!(context.filters(), lock(&context), @"
            exit_code: 0 (success)
            ----- stderr -----
            Resolved 3 packages in [TIME]
            ");
            assert_eq!(context.read("uv.lock"), original);
        }
        Ok(())
    }
}

#[cfg(feature = "test-git")]
#[test]
fn legacy_static_metadata_singleton_can_hide_git_duplicates() -> Result<()> {
    insta::allow_duplicates! {
        for resolution_inputs in [false, true] {
            let context = uv_test::test_context!("3.12");
            wheelhouse(&context)?;
            let repository = context.temp_dir.child("repository");
            repository.child("pyproject.toml").write_str(&formatdoc! {r#"
                [project]
                name = "parent"
                version = "1.0.0"
                requires-python = ">=3.12"
                dependencies = ["b"]
            "#})?;
            repository.child("PKG-INFO").write_str("Metadata-Version: 2.2\nName: parent\nVersion: 1.0.0\nRequires-Python: >=3.12\nRequires-Dist: b\n")?;
            let hooks = context.temp_dir.child("empty-hooks");
            hooks.create_dir_all()?;
            let config = context.temp_dir.child("empty-gitconfig");
            config.touch()?;
            for arguments in [vec!["init"], vec!["add", "."], vec!["commit", "-m", "fixture"]] {
                context.external_command("git")
                    .current_dir(&repository)
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .env("GIT_CONFIG_GLOBAL", config.path())
                    .env_remove("GIT_DIR")
                    .env_remove("GIT_WORK_TREE")
                    .args(["-c", "user.name=Example", "-c", "user.email=example@example.invalid", "-c", "commit.gpgsign=false", "-c"])
                    .arg(format!("core.hooksPath={}", hooks.path().display()))
                    .args(arguments)
                    .assert()
                    .success();
            }
            let repository_url = url::Url::from_directory_path(repository.path())
                .map_err(|()| anyhow::anyhow!("repository path is not absolute"))?;
            let sources = formatdoc! {r#"

                [tool.uv.sources]
                parent = {{ git = "{repository_url}" }}
            "#};
            let configure_git = |dependencies: &[&str]| -> Result<()> {
                configure(&context, resolution_inputs, true, dependencies)?;
                let project = context.read("pyproject.toml");
                context.temp_dir.child("pyproject.toml").write_str(&format!("{project}{sources}"))?;
                Ok(())
            };
            let git_lock = || {
                let mut command = context.lock();
                command.args(["--no-index", "--find-links", "links"]);
                command
            };

            configure_git(&["a", "a"])?;
            git_lock().assert().success();
            assert_eq!(package_names(&context)?, ["b", "parent", "project"]);
            let mut legacy: toml::Value = toml::from_str(&context.read("uv.lock"))?;
            let manifest = legacy["manifest"].as_table_mut().expect("manifest");
            manifest.remove("dependency-metadata-ordered");
            manifest.get_mut("dependency-metadata").expect("metadata").as_array_mut().expect("entries").dedup();
            let legacy = toml::to_string(&legacy)?;
            context.temp_dir.child("uv.lock").write_str(&legacy)?;

            configure_git(&["a"])?;
            uv_snapshot!(context.filters(), git_lock().arg("--locked"), @"
            exit_code: 1 (failure)
            ----- stderr -----
            Resolved 3 packages in [TIME]
            error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

            hint: To update the lockfile, run `uv lock`.
            ");
            assert_eq!(context.read("uv.lock"), legacy);
            uv_snapshot!(context.filters(), git_lock(), @"
            exit_code: 0 (success)
            ----- stderr -----
            Resolved 3 packages in [TIME]
            Added a v1.0.0
            Removed b v1.0.0
            ");
            assert_eq!(package_names(&context)?, ["a", "parent", "project"]);
            let updated = context.read("uv.lock");
            let parsed: toml::Value = toml::from_str(&updated)?;
            assert_eq!(parsed["manifest"]["dependency-metadata-ordered"].as_bool(), Some(true));
            uv_snapshot!(context.filters(), git_lock().arg("--locked"), @"
            exit_code: 0 (success)
            ----- stderr -----
            Resolved 3 packages in [TIME]
            ");
            assert_eq!(context.read("uv.lock"), updated);
        }
        Ok(())
    }
}
