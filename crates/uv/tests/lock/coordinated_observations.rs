//! End-to-end coverage for independently owned sibling observations.

use anyhow::{Context, Result};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::formatdoc;
use insta::assert_snapshot;

use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_static::EnvVars;
use uv_test::packse::PackseServer;
use uv_test::packse::scenario::Scenario;

/// Speculative selections do not prove that a live sibling published or withdrew a decision.
fn ordinary_selections(stderr: &str) -> Result<Vec<&str>> {
    let mut in_trial = false;
    let mut selections = Vec::new();
    for line in stderr.lines() {
        if line.contains("Trying coordinated backtracking for ") {
            assert!(!in_trial, "overlapping coordination trials: {stderr}");
            in_trial = true;
        } else if line.contains("Accepted coordinated backtracking")
            || line.contains("Rejected coordinated backtracking")
            || line.contains("Abandoned coordinated backtracking")
        {
            assert!(in_trial, "coordination outcome without a trial: {stderr}");
            in_trial = false;
        } else if !in_trial && let Some((_, selection)) = line.split_once("Selecting: ") {
            let (selection, _) = selection
                .split_once(" (")
                .context("registry selection must identify its distribution")?;
            selections.push(selection);
        }
    }
    assert!(!in_trial, "unfinished coordination trial: {stderr}");
    Ok(selections)
}

struct ObservationRun {
    stderr: String,
    selected: String,
}

fn observation_scenario() -> Result<Scenario> {
    toml::from_str(include_str!(
        "../../../../test/scenarios/fork/coordinated-observations-retained-version.toml"
    ))
    .context("failed to parse the observation scenario")
}

fn lock_observations(scenario: &Scenario) -> Result<ObservationRun> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::from_scenario(scenario);
    let requires_python = scenario
        .root
        .requires_python
        .as_ref()
        .context("observation scenario must declare its Python range")?;
    let dependencies = toml::Value::Array(
        scenario
            .root
            .requires
            .iter()
            .map(|requirement| toml::Value::String(requirement.to_string()))
            .collect(),
    );
    let environments = toml::Value::Array(
        scenario
            .resolver_options
            .environments
            .iter()
            .map(|environment| {
                environment
                    .contents()
                    .map(|marker| toml::Value::String(marker.to_string()))
                    .context("observation scenario must use nonempty environment markers")
            })
            .collect::<Result<_>>()?,
    );
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = "{requires_python}"
        dependencies = {dependencies}

        [tool.uv]
        fork-strategy = "fewest"
        environments = {environments}
    "#})?;

    let output = context
        .lock()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env(EnvVars::RUST_LOG, "uv_resolver::resolver=debug")
        .arg("--index-url")
        .arg(server.index_url())
        .output()?;
    let stderr = String::from_utf8(output.stderr.clone())?;
    output.assert().success();

    let locked = context.read("uv.lock");
    let output = context
        .export()
        .args(["--frozen", "--no-header", "--no-hashes", "--no-annotate"])
        .output()?;
    let requirements = String::from_utf8(output.stdout.clone())?;
    output.assert().success();
    let selected = requirements
        .lines()
        .filter(|line| {
            ["anchor==", "consumer==", "flaky==", "shared=="]
                .iter()
                .any(|prefix| line.starts_with(*prefix))
        })
        .collect::<Vec<_>>()
        .join("\n");

    context
        .lock()
        .args(["--locked", "--offline"])
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--index-url")
        .arg(server.index_url())
        .assert()
        .success();
    assert_eq!(locked, context.read("uv.lock"));
    Ok(ObservationRun { stderr, selected })
}

fn backtrack_positions(selections: &[&str]) -> Result<(usize, usize, usize)> {
    let original = selections
        .iter()
        .position(|selection| selection.starts_with("flaky==2.0.0 "))
        .context("the fluctuating sibling must first choose flaky==2")?;
    let backtracked = selections
        .iter()
        .position(|selection| selection.starts_with("flaky==1.0.0 "))
        .context("ordinary backtracking must choose flaky==1")?;
    let consumer = selections
        .iter()
        .position(|selection| selection.starts_with("consumer==1.0.0 "))
        .context("the delayed consumer must be selected")?;
    assert!(
        original < backtracked && backtracked < consumer,
        "the consumer must follow the ordinary backtrack: {selections:#?}",
    );
    Ok((original, backtracked, consumer))
}

fn consumer_choice<'a>(selections: &[&'a str], consumer: usize) -> Result<&'a str> {
    selections[consumer + 1..]
        .iter()
        .copied()
        .find(|selection| selection.starts_with("shared=="))
        .context("the consumer must select its shared dependency")
}

/// Withdrawing one sibling's selected version cannot withdraw another sibling's identical
/// observation. The surviving version must remain available to later live preferences.
#[test]
fn coordinated_observations_survive_retraction() -> Result<()> {
    let run = lock_observations(&observation_scenario()?)?;
    let selections = ordinary_selections(&run.stderr)?;
    let (_, backtracked, consumer) = backtrack_positions(&selections)?;
    assert!(
        selections[..backtracked]
            .iter()
            .filter(|selection| selection.starts_with("shared==2.0.0 "))
            .count()
            >= 2,
        "both earlier siblings must select shared==2: {selections:#?}",
    );
    assert_eq!(
        consumer_choice(&selections, consumer)?,
        "shared==2.0.0 [preference]",
        "{}",
        run.stderr,
    );
    assert_snapshot!(run.selected, @"
    anchor==1.0.0 ; python_full_version < '3.13'
    consumer==1.0.0 ; python_full_version >= '3.14'
    flaky==1.0.0 ; python_full_version == '3.13.*'
    shared==1.0.0 ; python_full_version == '3.13.*'
    shared==2.0.0 ; python_full_version != '3.13.*'
    ");
    Ok(())
}

/// Once the last owner backtracks, its withdrawn version must no longer influence a later
/// sibling. Otherwise `highest` prefers the stale version over the remaining live observation.
#[test]
fn coordinated_observations_withdrawn_version_is_not_preferred() -> Result<()> {
    let mut scenario = observation_scenario()?;
    let anchor = scenario
        .packages
        .get_mut(&"anchor".parse::<PackageName>()?)
        .context("observation scenario must contain the anchor")?
        .versions
        .get_mut(&"1.0.0".parse::<Version>()?)
        .context("observation scenario must contain anchor==1")?;
    assert_eq!(anchor.requires.len(), 1);
    let requirement = anchor
        .requires
        .first_mut()
        .context("anchor must require shared")?;
    assert_eq!(requirement.to_string(), "shared==2.0.0");
    *requirement = "shared==1.0.0".parse()?;

    let run = lock_observations(&scenario)?;
    let selections = ordinary_selections(&run.stderr)?;
    let (original, backtracked, consumer) = backtrack_positions(&selections)?;
    assert!(
        selections[original + 1..backtracked]
            .iter()
            .any(|selection| selection.starts_with("shared==2.0.0 ")),
        "the fluctuating sibling must select shared==2 before backtracking: {selections:#?}",
    );
    assert!(
        selections[backtracked + 1..consumer]
            .iter()
            .any(|selection| selection.starts_with("shared==1.0.0 ")),
        "ordinary backtracking must replace shared==2 before the consumer: {selections:#?}",
    );
    assert_eq!(
        consumer_choice(&selections, consumer)?,
        "shared==1.0.0 [preference]",
        "{}",
        run.stderr,
    );
    assert_snapshot!(run.selected, @"
    anchor==1.0.0 ; python_full_version < '3.13'
    consumer==1.0.0 ; python_full_version >= '3.14'
    flaky==1.0.0 ; python_full_version == '3.13.*'
    shared==1.0.0
    ");
    Ok(())
}
