//! Search-history checks that complement the generated lockfile scenarios.

use anyhow::{Context, Result};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::formatdoc;

use uv_static::EnvVars;
use uv_test::packse::PackseServer;
use uv_test::packse::scenario::Scenario;

#[derive(Debug)]
struct Trial {
    package: String,
    source: String,
    target: String,
    splits: Vec<usize>,
    completed: Vec<String>,
}

#[derive(Debug)]
enum TrialOutcome {
    Accepted(String),
    Rejected(String),
    Abandoned(String),
}

impl TrialOutcome {
    fn score(&self) -> Option<(&'static str, &str)> {
        match self {
            Self::Accepted(reason) => Some(("accepted", reason)),
            Self::Rejected(reason) if reason.starts_with("duplicate count ") => {
                Some(("rejected", reason))
            }
            Self::Rejected(_) | Self::Abandoned(_) => None,
        }
    }
}

fn lock_trace(scenario: &str) -> Result<Vec<(Trial, TrialOutcome)>> {
    let scenario: Scenario = toml::from_str(scenario)?;
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::from_scenario(&scenario);
    let index_url = server.index_url();
    let requires_python = scenario
        .root
        .requires_python
        .as_ref()
        .context("trace scenario must declare its Python range")?;
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
                    .context("trace scenario must use nonempty environment markers")
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
        .env(
            EnvVars::RUST_LOG,
            "uv_resolver::resolver::coordination=debug",
        )
        .arg("--index-url")
        .arg(&index_url)
        .output()?;
    let stderr = String::from_utf8(output.stderr.clone())?;
    output.assert().success();

    let mut pending: Option<Trial> = None;
    let mut trials = Vec::new();
    for line in stderr.lines() {
        if let Some((_, attempt)) = line.split_once("Trying coordinated backtracking for ") {
            assert!(pending.is_none(), "unfinished trial: {pending:?}");
            let (package, source_target) = attempt
                .split_once(" from ")
                .context("trial must identify its source")?;
            let (source, target) = source_target
                .split_once(" in ")
                .context("trial must identify its target")?;
            assert_eq!(source, index_url);
            pending = Some(Trial {
                package: package.to_string(),
                source: source.to_string(),
                target: target.to_string(),
                splits: Vec::new(),
                completed: Vec::new(),
            });
        } else if let Some((_, target)) =
            line.split_once("Completed coordinated backtracking fork for ")
        {
            pending
                .as_mut()
                .context("completed child must belong to a trial")?
                .completed
                .push(target.to_string());
        } else if let Some((_, forks)) = line.split_once("Split coordinated backtracking into ") {
            let count = forks
                .strip_suffix(" forks")
                .context("split must identify its child count")?
                .parse()?;
            pending
                .as_mut()
                .context("split must belong to a trial")?
                .splits
                .push(count);
        } else {
            let outcome = if let Some((_, reason)) =
                line.split_once("Accepted coordinated backtracking")
            {
                Some(TrialOutcome::Accepted(
                    reason.trim_start_matches(':').trim().to_string(),
                ))
            } else if let Some((_, reason)) = line.split_once("Rejected coordinated backtracking") {
                Some(TrialOutcome::Rejected(
                    reason.trim_start_matches(':').trim().to_string(),
                ))
            } else if let Some((_, reason)) = line.split_once("Abandoned coordinated backtracking")
            {
                Some(TrialOutcome::Abandoned(
                    reason.trim_start_matches(':').trim().to_string(),
                ))
            } else {
                None
            };
            if let Some(outcome) = outcome {
                let trial = pending
                    .take()
                    .context("terminal outcome must belong to a trial")?;
                trials.push((trial, outcome));
            }
        }
    }
    assert!(pending.is_none(), "unfinished trial: {pending:?}");
    Ok(trials)
}

fn score_events(trials: &[(Trial, TrialOutcome)], target: &str) -> Vec<String> {
    let mut source = None;
    let mut events = Vec::new();
    for (trial, outcome) in trials {
        let Some((disposition, reason)) = outcome.score() else {
            continue;
        };
        assert_eq!(trial.target, target);
        if let Some(source) = source {
            assert_eq!(trial.source, source);
        } else {
            source = Some(trial.source.as_str());
        }
        events.push(format!("{}: {disposition}, {reason}", trial.package));
    }
    events
}

/// The final lock alone cannot distinguish replacing an agreement from never coordinating.
#[test]
fn coordinated_trace_replaces_withdrawn_agreement() -> Result<()> {
    let trials = lock_trace(include_str!(
        "../../../../test/scenarios/fork/coordinated/coordinated-agreement-replacement.toml"
    ))?;
    assert_eq!(
        score_events(&trials, "split `python_full_version == '3.12.*'`"),
        [
            "shared==1.0.0: accepted, duplicate count 1 -> 0",
            "shared==2.0.0: accepted, duplicate count 1 -> 0",
        ]
    );
    Ok(())
}

#[test]
fn coordinated_trace_accumulates_agreements() -> Result<()> {
    let trials = lock_trace(include_str!(
        "../../../../test/scenarios/fork/coordinated/coordinated-agreement-accumulation.toml"
    ))?;
    assert_eq!(
        score_events(&trials, "split `python_full_version == '3.12.*'`"),
        [
            "shared-a==1.0.0: accepted, duplicate count 1 -> 0",
            "shared-b==1.0.0: accepted, duplicate count 1 -> 0",
        ]
    );
    Ok(())
}

#[test]
fn coordinated_trace_rejects_equal_score() -> Result<()> {
    let trials = lock_trace(include_str!(
        "../../../../test/scenarios/fork/coordinated/coordinated-agreement-non-improvement.toml"
    ))?;
    assert_eq!(
        score_events(&trials, "split `python_full_version == '3.12.*'`"),
        [
            "shared-a==1.0.0: accepted, duplicate count 1 -> 0",
            "shared-b==1.0.0: rejected, duplicate count 1 -> 1",
        ]
    );
    Ok(())
}

/// A partial replacement cannot escape when a later child makes the trial unsatisfiable.
#[test]
fn coordinated_trace_discards_completed_nested_child() -> Result<()> {
    let trials = lock_trace(include_str!(
        "../../../../test/scenarios/fork/coordinated/coordinated-nested-platform-unsatisfiable.toml"
    ))?;
    let trial = trials
        .iter()
        .find_map(|(trial, outcome)| {
            (trial.splits == [2]
                && trial.completed.len() == 1
                && matches!(outcome, TrialOutcome::Abandoned(reason)
                    if reason.starts_with("after an unsatisfiable fork:")))
            .then_some(trial)
        })
        .with_context(|| {
            format!("expected a partially completed, unsatisfiable trial: {trials:#?}")
        })?;
    assert_eq!(trial.target, "split `python_full_version < '3.13'`");
    assert!(trial.completed[0].contains("sys_platform == 'win32'"));
    Ok(())
}
