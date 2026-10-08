use std::collections::BTreeMap;
use std::process::Command;

use anyhow::Result;
use assert_fs::prelude::*;
use indoc::formatdoc;

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
