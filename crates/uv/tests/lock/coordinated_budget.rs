//! End-to-end coverage for bounded coordination work.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use anyhow::{Context, Result};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_static::EnvVars;
use uv_test::packse::PackseServer;
use uv_test::packse::scenario::Scenario;

fn add_package(
    scenario: &mut String,
    name: &str,
    version: &str,
    requirements: impl IntoIterator<Item = String>,
) -> Result<()> {
    let requirements =
        toml::Value::Array(requirements.into_iter().map(toml::Value::String).collect());
    writeln!(
        scenario,
        "\n[packages.{name}.versions.\"{version}\"]\nsdist = false\nrequires = {requirements}",
    )?;
    Ok(())
}

fn add_chain(scenario: &mut String, prefix: &str, length: usize, last: &str) -> Result<()> {
    for index in 0..length {
        let next = if index + 1 == length {
            last.to_string()
        } else {
            format!("{prefix}-{:03}", index + 1)
        };
        add_package(scenario, &format!("{prefix}-{index:03}"), "1.0.0", [next])?;
    }
    Ok(())
}

/// Exhausting the proposal budget retains the completed cover and does not stop ordinary forks.
#[test]
fn lock_fewest_coordinated_backtracking_attempt_budget() -> Result<()> {
    const SHARED_PACKAGES: usize = 65;

    let context = uv_test::test_context!("3.12");
    let mut scenario = indoc! {r#"
        name = "coordinated-attempt-budget"

        [root]
        requires_python = ">=3.12,<3.15"
        requires = [
            "early ; python_version == '3.12'",
            "later-delay-000 ; python_version == '3.13'",
            "ordinary-delay-000 ; python_version == '3.14'",
        ]

        [expected]
        satisfiable = true
    "#}
    .to_string();
    add_package(
        &mut scenario,
        "early",
        "1.0.0",
        (0..SHARED_PACKAGES).map(|index| format!("shared-{index:03}>=1.0.0,<=2.0.0")),
    )?;
    add_package(
        &mut scenario,
        "later-pins",
        "1.0.0",
        (0..SHARED_PACKAGES).map(|index| format!("shared-{index:03}==1.0.0")),
    )?;
    for index in 0..SHARED_PACKAGES {
        let name = format!("shared-{index:03}");
        add_package(&mut scenario, &name, "1.0.0", [])?;
        add_package(&mut scenario, &name, "2.0.0", [])?;
    }
    // The early fork must finish before any older shared versions are observed. The ordinary
    // fork stays live until all 65 distinct coordination proposals have become available.
    add_chain(&mut scenario, "later-delay", 96, "later-pins")?;
    add_chain(&mut scenario, "ordinary-delay", 192, "ordinary-terminal")?;
    add_package(&mut scenario, "ordinary-terminal", "1.0.0", [])?;
    let scenario: Scenario = toml::from_str(&scenario)?;
    let server = PackseServer::from_scenario(&scenario);

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12,<3.15"
        dependencies = [
            "early ; python_version == '3.12'",
            "later-delay-000 ; python_version == '3.13'",
            "ordinary-delay-000 ; python_version == '3.14'",
        ]

        [tool.uv]
        fork-strategy = "fewest"
        environments = [
            "python_version == '3.12'",
            "python_version == '3.13'",
            "python_version == '3.14'",
        ]
    "#})?;

    let output = context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(
            EnvVars::RUST_LOG,
            "uv_resolver::resolver::coordination=debug",
        )
        .arg("--index-url")
        .arg(server.index_url())
        .output()?;
    let stderr = String::from_utf8(output.stderr.clone())?;
    output.assert().success();
    assert_eq!(
        stderr
            .matches("Trying coordinated backtracking for ")
            .count(),
        64,
        "{stderr}",
    );
    assert_eq!(
        stderr
            .matches("Accepted coordinated backtracking: ")
            .count(),
        64,
        "{stderr}",
    );

    let lock = context.read("uv.lock");
    let parsed: toml::Value = toml::from_str(&lock)?;
    let packages = parsed
        .get("package")
        .and_then(toml::Value::as_array)
        .context("lockfile must contain packages")?;
    let mut older = BTreeSet::new();
    let mut newer = BTreeSet::new();
    for package in packages {
        let Some(name) = package.get("name").and_then(toml::Value::as_str) else {
            continue;
        };
        if !name.starts_with("shared-") {
            continue;
        }
        match package.get("version").and_then(toml::Value::as_str) {
            Some("1.0.0") => {
                older.insert(name.to_string());
            }
            Some("2.0.0") => {
                newer.insert(name);
            }
            version => anyhow::bail!("unexpected version for {name}: {version:?}"),
        }
    }
    assert_eq!(
        older,
        (0..SHARED_PACKAGES)
            .map(|index| format!("shared-{index:03}"))
            .collect(),
    );
    assert_eq!(newer.len(), 1, "exactly one duplicate must remain");
    let duplicate = newer.first().context("one duplicate must remain")?;

    let output = context
        .export()
        .args(["--frozen", "--no-header", "--no-hashes", "--no-annotate"])
        .output()?;
    let requirements = String::from_utf8(output.stdout.clone())?;
    output.assert().success();
    let duplicate_prefix = format!("{duplicate}==");
    let selected = requirements
        .lines()
        .filter(|line| {
            line.starts_with(&duplicate_prefix) || line.starts_with("ordinary-terminal==")
        })
        .map(|line| line.replace(*duplicate, "[DUPLICATE]"))
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!(selected, @"
    ordinary-terminal==1.0.0 ; python_full_version >= '3.14'
    [DUPLICATE]==1.0.0 ; python_full_version == '3.13.*'
    [DUPLICATE]==2.0.0 ; python_full_version < '3.13'
    ");

    context
        .lock()
        .arg("--locked")
        .arg("--offline")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();
    assert_eq!(lock, context.read("uv.lock"));
    Ok(())
}
