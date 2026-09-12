//! Execute finite Packse scenarios against a real uv binary.

use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::str::FromStr;

use anyhow::{Context, Result, bail, ensure};

use uv_configuration::TargetTriple;
use uv_pep440::Operator;
use uv_pep508::{MarkerEnvironment, MarkerEnvironmentBuilder, Requirement, VersionOrUrl};
use uv_python::PythonVersion;
use uv_static::EnvVars;

use crate::TestContext;

use super::PackseServer;
use super::evidence::{self, LockTrace};
use super::oracle::{ScenarioOracle, SearchResult, Selection};
use super::project::{ProjectSelection, ScenarioProject, project_name};
use super::scenario::{Scenario, ScenarioDocument};

/// A representative platform for fixed-environment resolver checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScenarioPlatform {
    Linux,
    Macos,
    Windows,
}

impl ScenarioPlatform {
    /// The explicit target triple passed to uv.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Linux => "x86_64-unknown-linux-gnu",
            Self::Macos => "aarch64-apple-darwin",
            Self::Windows => "x86_64-pc-windows-msvc",
        }
    }

    fn triple(self) -> TargetTriple {
        match self {
            Self::Linux => TargetTriple::X8664UnknownLinuxGnu,
            Self::Macos => TargetTriple::Aarch64AppleDarwin,
            Self::Windows => TargetTriple::X8664PcWindowsMsvc,
        }
    }
}

impl FromStr for ScenarioPlatform {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "linux" | "x86_64-unknown-linux-gnu" => Ok(Self::Linux),
            "macos" | "aarch64-apple-darwin" => Ok(Self::Macos),
            "windows" | "x86_64-pc-windows-msvc" => Ok(Self::Windows),
            _ => Err(format!(
                "unsupported scenario platform `{value}`; expected linux, macos, or windows"
            )),
        }
    }
}

impl fmt::Display for ScenarioPlatform {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A fully specified CPython environment, independent of the host running the check.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScenarioTarget {
    pub python: PythonVersion,
    pub platform: ScenarioPlatform,
}

impl ScenarioTarget {
    /// Construct a stable, deduplicated Cartesian product of Python versions and platforms.
    pub fn matrix(versions: &[PythonVersion], platforms: &[ScenarioPlatform]) -> Vec<Self> {
        let mut targets = Vec::new();
        for python in versions {
            for platform in platforms {
                let target = Self {
                    python: python.clone(),
                    platform: *platform,
                };
                if !targets.contains(&target) {
                    targets.push(target);
                }
            }
        }
        targets
    }

    /// Construct the marker values used by both the oracle and the uv command.
    pub fn markers(&self) -> Result<MarkerEnvironment> {
        let base = MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
            implementation_name: "cpython",
            implementation_version: "3.12.0",
            os_name: "posix",
            platform_machine: "x86_64",
            platform_python_implementation: "CPython",
            platform_release: "",
            platform_system: "Linux",
            platform_version: "",
            python_full_version: "3.12.0",
            python_version: "3.12",
            sys_platform: "linux",
        })?;
        Ok(self.platform.triple().markers(self.python.markers(base)))
    }
}

impl fmt::Display for ScenarioTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "CPython {} on {}", self.python, self.platform)
    }
}

/// A comparison between uv's result and the exhaustive oracle.
#[derive(Debug)]
pub struct CheckResult {
    /// The selection returned by uv, or `None` when both resolvers prove unsatisfiability.
    pub selection: Option<Selection>,
    /// The number of complete selections examined by the oracle.
    pub checked: usize,
}

/// A semantic contradiction between uv's fixed-environment output and the independent oracle.
///
/// Setup errors, unsupported inputs, exhausted search bounds, and subprocess failures without a
/// resolver conclusion are deliberately not assigned a kind. Reducers must not mistake those
/// failures for a preserved resolver counterexample.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScenarioFailureKind {
    FalseSatisfiable,
    FalseUnsatisfiable,
    InvalidPins,
    InvalidClosure,
}

impl ScenarioFailureKind {
    /// Return the semantic failure carried by a checker error, if any.
    pub fn from_error(error: &anyhow::Error) -> Option<Self> {
        error.downcast_ref::<Self>().copied()
    }
}

impl fmt::Display for ScenarioFailureKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::FalseSatisfiable => "uv accepted an unsatisfiable graph",
            Self::FalseUnsatisfiable => "uv rejected a satisfiable graph",
            Self::InvalidPins => "uv returned invalid fixed-environment pins",
            Self::InvalidClosure => "uv returned an invalid dependency closure",
        })
    }
}

impl std::error::Error for ScenarioFailureKind {}

/// A demonstrated contradiction in a universal lockfile or one of its concrete exports.
///
/// A universal no-solution result without an unsatisfiable sampled environment remains
/// unclassified: successful samples do not prove that the entire marker universe is satisfiable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LockScenarioFailureKind {
    FalseSatisfiable,
    InvalidPins,
    InvalidClosure,
    ChangedLockfile,
    NonCanonicalLockfile,
}

impl LockScenarioFailureKind {
    /// Return a demonstrated lockfile failure, if one is carried by the error.
    pub fn from_error(error: &anyhow::Error) -> Option<Self> {
        error.downcast_ref::<Self>().copied()
    }
}

impl fmt::Display for LockScenarioFailureKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::FalseSatisfiable => "uv locked an unsatisfiable graph",
            Self::InvalidPins => "uv returned invalid lockfile export pins",
            Self::InvalidClosure => "uv returned an invalid lockfile dependency closure",
            Self::ChangedLockfile => "uv changed a lockfile during a read-only check",
            Self::NonCanonicalLockfile => "uv rejected its freshly written lockfile",
        })
    }
}

impl std::error::Error for LockScenarioFailureKind {}

/// The result of checking a universal lockfile and its concrete projections.
#[derive(Debug)]
pub enum LockCheckResult {
    /// A canonical, unchanged lockfile with a valid export in every requested environment.
    Satisfiable { projections: usize, checked: usize },
    /// A concrete environment in the project's supported range proves the lock unsatisfiable.
    Unsatisfiable {
        witness: ScenarioTarget,
        checked: usize,
    },
}

/// Run a fixed-environment `uv pip compile` and compare it with the finite-domain oracle.
///
/// The caller supplies a test context with a real uv binary and an available CPython interpreter.
/// All registry candidates come from a closed-world local server.
pub fn check_scenario(
    context: &TestContext,
    scenario: &Scenario,
    target: &ScenarioTarget,
    max_states: usize,
) -> Result<CheckResult> {
    check_scenario_inner(context, scenario, target, max_states, None)
}

/// Check a replayable fixture and save the command output and served distributions on failure.
///
/// The destination is only created when uv runs and its result fails the comparison. It must not
/// already exist, so evidence from a previous run cannot be replaced accidentally.
pub fn check_scenario_with_artifacts(
    context: &TestContext,
    document: &ScenarioDocument,
    target: &ScenarioTarget,
    max_states: usize,
    failure_dir: &Path,
) -> Result<CheckResult> {
    let scenario = document.scenario()?;
    check_scenario_inner(
        context,
        &scenario,
        target,
        max_states,
        Some((failure_dir, document)),
    )
}

fn check_scenario_inner(
    context: &TestContext,
    scenario: &Scenario,
    target: &ScenarioTarget,
    max_states: usize,
    artifacts: Option<(&Path, &ScenarioDocument)>,
) -> Result<CheckResult> {
    let environment = target_environment(scenario, target)?;
    let oracle = ScenarioOracle::new(scenario, &environment)?;
    let expected = oracle.find_solution(max_states)?;
    let server = PackseServer::from_scenario_without_build_dependencies(scenario);
    let requirements = scenario
        .root
        .requires
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    fs_err::write(context.temp_dir.join("requirements.in"), &requirements)?;

    let mut command = context.pip_compile();
    command
        .arg("requirements.in")
        .arg("--no-config")
        .arg("--index-url")
        .arg(server.index_url())
        .arg("--python-version")
        .arg(target.python.to_string())
        .arg("--python-platform")
        .arg(target.platform.as_str())
        .arg("--no-build")
        .arg("--no-header")
        .arg("--no-annotate")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .env_remove(EnvVars::VIRTUAL_ENV);
    if let Some(resolution) = scenario.resolver_options.resolution {
        command.arg("--resolution").arg(resolution.to_string());
    }
    if scenario.resolver_options.prereleases {
        command.arg("--prerelease=allow");
    }
    let output = command.output().context("failed to run uv pip compile")?;
    let selection = match compare_output(&oracle, &environment, expected.solution.as_ref(), &output)
    {
        Ok(selection) => selection,
        Err(error) => {
            if let Some((directory, document)) = artifacts {
                if let Err(capture_error) = write_failure_artifacts(
                    directory,
                    document,
                    &requirements,
                    &command,
                    &output,
                    &server,
                ) {
                    return Err(error.context(format!(
                        "failed to save resolver evidence to `{}`: {capture_error:#}",
                        directory.display()
                    )));
                }
                return Err(error.context(format!(
                    "resolver evidence saved to `{}`",
                    directory.display()
                )));
            }
            return Err(error);
        }
    };
    Ok(CheckResult {
        selection,
        checked: expected.checked,
    })
}

fn write_failure_artifacts(
    directory: &Path,
    document: &ScenarioDocument,
    requirements: &str,
    command: &Command,
    output: &Output,
    server: &PackseServer,
) -> Result<()> {
    evidence::create_directory(directory)?;
    fs_err::write(directory.join("scenario.toml"), document.to_toml()?)?;
    fs_err::write(directory.join("requirements.in"), requirements)?;
    evidence::write_command(directory, command, output)?;
    server.write_distributions(&directory.join("index"))?;
    Ok(())
}

/// Check a universal project lock, its canonical round trip, and frozen requirements exports.
///
/// Successful locks are checked independently in every requested environment. A failed universal
/// resolution is only accepted when one of those environments supplies an unsatisfiable witness;
/// a few satisfiable samples cannot prove that the whole marker universe is satisfiable.
pub fn check_lock_scenario(
    context: &TestContext,
    scenario: &Scenario,
    targets: &[ScenarioTarget],
    max_states: usize,
) -> Result<LockCheckResult> {
    check_lock_scenario_inner(context, scenario, targets, max_states, None)
}

/// Check a universal lock and retain every command, resulting lockfile, and served distribution
/// when a comparison fails. The evidence directory must not already exist.
pub fn check_lock_scenario_with_artifacts(
    context: &TestContext,
    document: &ScenarioDocument,
    targets: &[ScenarioTarget],
    max_states: usize,
    failure_dir: &Path,
) -> Result<LockCheckResult> {
    let scenario = document.scenario()?;
    check_lock_scenario_inner(
        context,
        &scenario,
        targets,
        max_states,
        Some((failure_dir, document)),
    )
}

/// Check one universal project lock and independently validate explicit extra/group exports.
///
/// The lock must satisfy every optional dependency and dependency group together, regardless of
/// which exports are requested. Default groups are disabled for each export; `selections` names
/// the complete set of roots to include. Satisfiable results count each selection/environment pair
/// as one projection.
pub fn check_project_lock_scenario(
    context: &TestContext,
    scenario: &Scenario,
    targets: &[ScenarioTarget],
    selections: &[ProjectSelection],
    max_states: usize,
) -> Result<LockCheckResult> {
    check_project_lock_scenario_inner(context, scenario, targets, selections, max_states, None)
}

/// Check explicit project exports and retain the full command/lockfile trace on a discrepancy.
pub fn check_project_lock_scenario_with_artifacts(
    context: &TestContext,
    document: &ScenarioDocument,
    targets: &[ScenarioTarget],
    selections: &[ProjectSelection],
    max_states: usize,
    failure_dir: &Path,
) -> Result<LockCheckResult> {
    let scenario = document.scenario()?;
    check_project_lock_scenario_inner(
        context,
        &scenario,
        targets,
        selections,
        max_states,
        Some((failure_dir, document)),
    )
}

struct LockProjection<'a> {
    target: &'a ScenarioTarget,
    environment: MarkerEnvironment,
    search: SearchResult,
}

fn check_lock_scenario_inner(
    context: &TestContext,
    scenario: &Scenario,
    targets: &[ScenarioTarget],
    max_states: usize,
    artifacts: Option<(&Path, &ScenarioDocument)>,
) -> Result<LockCheckResult> {
    ensure!(
        !targets.is_empty(),
        "at least one lock projection is required"
    );
    let Some(requires_python) = &scenario.root.requires_python else {
        bail!("lock scenarios require an explicit root Python range");
    };
    let mut searches = Vec::new();
    let mut checked = 0;
    for target in targets {
        let environment = target_environment(scenario, target)?;
        let oracle = ScenarioOracle::new(scenario, &environment)?;
        let search = oracle.find_solution(max_states)?;
        checked += search.checked;
        searches.push(LockProjection {
            target,
            environment,
            search,
        });
    }

    let server = PackseServer::from_scenario_without_build_dependencies(scenario);
    let root_name = project_name(scenario)?;
    let project = serde_json::json!({
        "project": {
            "name": root_name,
            "version": "0.0.0",
            "requires-python": requires_python.to_string(),
            "dependencies": scenario.root.requires.iter().map(ToString::to_string).collect::<Vec<_>>(),
        }
    });
    let mut run = LockRun::new(context, scenario, &server, toml::to_string(&project)?)?;
    let result = check_lock_scenario_run(&mut run, &searches, checked);
    run.finish(result, targets, None, artifacts)
}

fn check_lock_scenario_run(
    run: &mut LockRun<'_>,
    searches: &[LockProjection<'_>],
    checked: usize,
) -> Result<LockCheckResult> {
    let output = run.resolve()?;
    if let Some(witness) = check_lock_resolution(&output, searches)? {
        return Ok(LockCheckResult::Unsatisfiable { witness, checked });
    }
    let lock = run.check_round_trip()?;
    let output = run.run_command("export", run.export_command())?;
    ensure_success(&output, "uv export --frozen --offline")?;
    let requirements =
        std::str::from_utf8(&output.stdout).context(LockScenarioFailureKind::InvalidPins)?;
    for projection in searches {
        let target = projection.target;
        let selection = parse_pins(requirements, &projection.environment)
            .with_context(|| format!("invalid lock export for {target}"))
            .context(LockScenarioFailureKind::InvalidPins)?;
        ScenarioOracle::new(run.scenario, &projection.environment)?
            .validate(&selection)
            .with_context(|| format!("invalid lock dependency closure for {target}: {selection:?}"))
            .context(LockScenarioFailureKind::InvalidClosure)?;
    }
    run.ensure_unchanged(&lock, "frozen export")?;
    Ok(LockCheckResult::Satisfiable {
        projections: searches.len(),
        checked,
    })
}

fn check_project_lock_scenario_inner(
    context: &TestContext,
    scenario: &Scenario,
    targets: &[ScenarioTarget],
    selections: &[ProjectSelection],
    max_states: usize,
    artifacts: Option<(&Path, &ScenarioDocument)>,
) -> Result<LockCheckResult> {
    ensure!(
        !targets.is_empty(),
        "at least one lock projection is required"
    );
    ensure!(
        !selections.is_empty(),
        "at least one project selection is required"
    );
    let project = ScenarioProject::new(scenario)?;
    for selection in selections {
        project.requirements(selection)?;
    }
    let all = project.all_selection();
    let mut searches = Vec::new();
    let mut checked = 0;
    for target in targets {
        let environment = target_environment(scenario, target)?;
        let search = project
            .oracle(&environment, &all)?
            .find_solution(max_states)?;
        checked += search.checked;
        searches.push(LockProjection {
            target,
            environment,
            search,
        });
    }

    let server = PackseServer::from_scenario_without_build_dependencies(scenario);
    let mut run = LockRun::new(context, scenario, &server, project.pyproject()?)?;
    let result =
        check_project_lock_scenario_run(&mut run, &project, &searches, selections, checked);
    run.finish(result, targets, Some(selections), artifacts)
}

fn check_project_lock_scenario_run(
    run: &mut LockRun<'_>,
    project: &ScenarioProject<'_>,
    searches: &[LockProjection<'_>],
    selections: &[ProjectSelection],
    checked: usize,
) -> Result<LockCheckResult> {
    let output = run.resolve()?;
    if let Some(witness) = check_lock_resolution(&output, searches)? {
        return Ok(LockCheckResult::Unsatisfiable { witness, checked });
    }
    let lock = run.check_round_trip()?;
    for selection in selections {
        let output = run.run_command("project-export", run.project_export_command(selection))?;
        ensure_success(&output, "uv export --frozen --offline")
            .with_context(|| format!("failed to export {selection}"))?;
        let requirements =
            std::str::from_utf8(&output.stdout).context(LockScenarioFailureKind::InvalidPins)?;
        for projection in searches {
            let target = projection.target;
            let pins = parse_pins(requirements, &projection.environment)
                .with_context(|| format!("invalid {selection} export for {target}"))
                .context(LockScenarioFailureKind::InvalidPins)?;
            project
                .oracle(&projection.environment, selection)?
                .validate(&pins)
                .with_context(|| {
                    format!("invalid {selection} dependency closure for {target}: {pins:?}")
                })
                .context(LockScenarioFailureKind::InvalidClosure)?;
        }
        run.ensure_unchanged(&lock, "frozen project export")?;
    }
    Ok(LockCheckResult::Satisfiable {
        projections: searches
            .len()
            .checked_mul(selections.len())
            .context("the number of project projections overflows usize")?,
        checked,
    })
}

/// Check a universal conclusion against sampled lock-wide root sets.
fn check_lock_resolution(
    output: &Output,
    searches: &[LockProjection<'_>],
) -> Result<Option<ScenarioTarget>> {
    let unsatisfiable = searches
        .iter()
        .find(|projection| projection.search.solution.is_none());
    if output.status.success() {
        if let Some(projection) = unsatisfiable {
            return Err(anyhow::anyhow!(
                "uv lock succeeded, but its {} projection is unsatisfiable",
                projection.target
            )
            .context(LockScenarioFailureKind::FalseSatisfiable));
        }
        return Ok(None);
    }
    ensure_no_solution(output, "uv lock")?;
    if let Some(projection) = unsatisfiable {
        return Ok(Some(projection.target.clone()));
    }
    bail!(
        "uv lock reported no solution, but the requested projections are satisfiable; \
         add marker environments to distinguish a resolver defect from an unsampled conflict:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

struct LockRun<'a> {
    context: &'a TestContext,
    scenario: &'a Scenario,
    server: &'a PackseServer,
    pyproject: String,
    lock_path: PathBuf,
    trace: LockTrace,
    canonical_diff: Option<String>,
}

impl<'a> LockRun<'a> {
    fn new(
        context: &'a TestContext,
        scenario: &'a Scenario,
        server: &'a PackseServer,
        pyproject: String,
    ) -> Result<Self> {
        fs_err::write(context.temp_dir.join("pyproject.toml"), &pyproject)?;
        let lock_path = context.temp_dir.join("uv.lock");
        if let Err(error) = fs_err::remove_file(&lock_path)
            && error.kind() != ErrorKind::NotFound
        {
            return Err(error.into());
        }
        Ok(Self {
            context,
            scenario,
            server,
            pyproject,
            lock_path,
            trace: LockTrace::default(),
            canonical_diff: None,
        })
    }

    fn run_command(&mut self, label: &'static str, command: Command) -> Result<Output> {
        self.trace.run(label, command, &self.lock_path)
    }

    fn resolve(&mut self) -> Result<Output> {
        self.run_command(
            "lock",
            lock_command(self.context, self.scenario, self.server),
        )
    }

    fn check_round_trip(&mut self) -> Result<Vec<u8>> {
        let lock = fs_err::read(&self.lock_path)?;
        let mut command = lock_command(self.context, self.scenario, self.server);
        command.arg("--locked").arg("--offline");
        let output = self.run_command("locked-offline", command)?;
        ensure_success(&output, "uv lock --locked --offline")?;
        self.ensure_unchanged(&lock, "uv lock --locked")?;

        let output = self.run_command("canonical-check", self.canonical_check_command())?;
        ensure_canonical_lock(&output)?;
        self.ensure_unchanged(&lock, "uv lock --check")?;
        Ok(lock)
    }

    fn canonical_check_command(&self) -> Command {
        let mut command = lock_command(self.context, self.scenario, self.server);
        command
            .arg("--check")
            .arg("--refresh")
            .arg("--preview-features")
            .arg("lockfile-format-check");
        command
    }

    /// Retain the lockfile uv would write after rejecting the freshly written one.
    fn capture_canonical_refresh(&mut self) -> Result<()> {
        let original = fs_err::read_to_string(&self.lock_path)?;
        let mut command = lock_command(self.context, self.scenario, self.server);
        command
            .arg("--refresh")
            .arg("--preview-features")
            .arg("lockfile-format-check");
        let output = self.run_command("canonical-refresh", command)?;
        ensure_success(&output, "uv lock --refresh")?;
        let refreshed = fs_err::read_to_string(&self.lock_path)?;
        self.canonical_diff = Some(crate::diff_snapshot(&original, &refreshed, 4));
        let output = self.run_command(
            "canonical-check-after-refresh",
            self.canonical_check_command(),
        )?;
        ensure_success(
            &output,
            "uv lock --check --refresh after updating the lockfile",
        )
    }

    fn export_command(&self) -> Command {
        let mut command = self.context.export();
        command
            .arg("--no-config")
            .arg("--frozen")
            .arg("--offline")
            .arg("--no-emit-project")
            .arg("--no-hashes")
            .arg("--no-header")
            .arg("--no-annotate")
            .env_remove(EnvVars::UV_EXCLUDE_NEWER);
        command
    }

    fn project_export_command(&self, selection: &ProjectSelection) -> Command {
        let mut command = self.export_command();
        command.arg("--no-default-groups");
        if selection.include_project {
            for extra in &selection.extras {
                command.arg("--extra").arg(extra.to_string());
            }
            for group in &selection.groups {
                command.arg("--group").arg(group.to_string());
            }
        } else {
            for group in &selection.groups {
                command.arg("--only-group").arg(group.to_string());
            }
        }
        command
    }

    fn ensure_unchanged(&self, expected: &[u8], command: &str) -> Result<()> {
        if fs_err::read(&self.lock_path)? != expected {
            return Err(anyhow::anyhow!("{command} changed the lockfile")
                .context(LockScenarioFailureKind::ChangedLockfile));
        }
        Ok(())
    }

    fn finish<T>(
        &mut self,
        result: Result<T>,
        targets: &[ScenarioTarget],
        selections: Option<&[ProjectSelection]>,
        artifacts: Option<(&Path, &ScenarioDocument)>,
    ) -> Result<T> {
        let mut error = match result {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        if let Some((directory, document)) = artifacts
            && !self.trace.is_empty()
        {
            if LockScenarioFailureKind::from_error(&error)
                == Some(LockScenarioFailureKind::NonCanonicalLockfile)
                && let Err(capture_error) = self.capture_canonical_refresh()
            {
                error = error.context(format!(
                    "failed to capture the refreshed lockfile: {capture_error:#}"
                ));
            }
            if let Err(capture_error) =
                self.write_artifacts(directory, document, targets, selections, &error)
            {
                return Err(error.context(format!(
                    "failed to save lock evidence to `{}`: {capture_error:#}",
                    directory.display()
                )));
            }
            return Err(error.context(format!("lock evidence saved to `{}`", directory.display())));
        }
        Err(error)
    }

    fn write_artifacts(
        &self,
        directory: &Path,
        document: &ScenarioDocument,
        targets: &[ScenarioTarget],
        selections: Option<&[ProjectSelection]>,
        error: &anyhow::Error,
    ) -> Result<()> {
        evidence::create_directory(directory)?;
        fs_err::write(directory.join("scenario.toml"), document.to_toml()?)?;
        fs_err::write(directory.join("pyproject.toml"), &self.pyproject)?;
        fs_err::write(
            directory.join("failure.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "kind": LockScenarioFailureKind::from_error(error).map(|kind| kind.to_string()),
                "error": format!("{error:#}"),
                "targets": targets.iter().map(|target| serde_json::json!({
                    "python": target.python.to_string(),
                    "platform": target.platform.as_str(),
                })).collect::<Vec<_>>(),
                "project_selections": selections,
            }))?,
        )?;
        self.trace.write(directory)?;
        if let Some(diff) = &self.canonical_diff {
            fs_err::write(directory.join("initial-to-refreshed.diff"), diff)?;
        }
        self.server.write_distributions(&directory.join("index"))?;
        Ok(())
    }
}

fn target_environment(scenario: &Scenario, target: &ScenarioTarget) -> Result<MarkerEnvironment> {
    let environment = target.markers()?;
    ensure!(
        scenario
            .root
            .requires_python
            .as_ref()
            .is_none_or(|specifier| {
                specifier.contains(&environment.python_full_version().only_release())
            }),
        "Python {} is outside the scenario root's supported range",
        target.python
    );
    Ok(environment)
}

fn lock_command(context: &TestContext, scenario: &Scenario, server: &PackseServer) -> Command {
    let mut command = context.lock();
    command
        .arg("--no-config")
        .arg("--index-url")
        .arg(server.index_url())
        .arg("--no-build")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER);
    if let Some(resolution) = scenario.resolver_options.resolution {
        command.arg("--resolution").arg(resolution.to_string());
    }
    if scenario.resolver_options.prereleases {
        command.arg("--prerelease=allow");
    }
    command
}

fn ensure_success(output: &Output, command: &str) -> Result<()> {
    ensure!(
        output.status.success(),
        "{command} failed ({}):\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn ensure_canonical_lock(output: &Output) -> Result<()> {
    let result = ensure_success(output, "uv lock --check --refresh");
    if is_canonical_lock_mismatch(
        output.status.code(),
        &String::from_utf8_lossy(&output.stderr),
    ) {
        result.context(LockScenarioFailureKind::NonCanonicalLockfile)
    } else {
        result
    }
}

fn is_canonical_lock_mismatch(status: Option<i32>, stderr: &str) -> bool {
    status == Some(1)
        && stderr
            .contains("The lockfile at `uv.lock` needs to be updated, but `--check` was provided.")
}

fn ensure_no_solution(output: &Output, command: &str) -> Result<()> {
    let stderr = String::from_utf8_lossy(&output.stderr);
    ensure!(
        output.status.code() == Some(1) && stderr.contains("No solution found"),
        "{command} failed without a resolver conclusion ({}):\n{stderr}",
        output.status
    );
    Ok(())
}

fn compare_output(
    oracle: &ScenarioOracle<'_>,
    environment: &MarkerEnvironment,
    expected: Option<&Selection>,
    output: &Output,
) -> Result<Option<Selection>> {
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success() {
        let requirements =
            std::str::from_utf8(&output.stdout).context(ScenarioFailureKind::InvalidPins)?;
        let selection =
            parse_pins(requirements, environment).context(ScenarioFailureKind::InvalidPins)?;
        if expected.is_none() {
            return Err(
                anyhow::anyhow!("uv selected {selection:?}:\n{requirements}")
                    .context(ScenarioFailureKind::FalseSatisfiable),
            );
        }
        oracle
            .validate(&selection)
            .with_context(|| format!("uv selected {selection:?}:\n{requirements}"))
            .context(ScenarioFailureKind::InvalidClosure)?;
        Ok(Some(selection))
    } else {
        ensure_no_solution(output, "uv pip compile")?;
        if let Some(solution) = expected {
            return Err(anyhow::anyhow!("the oracle found {solution:?}:\n{stderr}")
                .context(ScenarioFailureKind::FalseUnsatisfiable));
        }
        Ok(None)
    }
}

/// Read exact registry pins, projecting any environment markers onto a concrete target.
///
/// Two different versions active in the same environment are rejected instead of allowing the
/// last line to hide an overlapping universal-resolution fork.
pub fn parse_pins(contents: &str, environment: &MarkerEnvironment) -> Result<Selection> {
    let mut selection = Selection::new();
    for line in contents.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let requirement: Requirement = Requirement::from_str(line)
            .with_context(|| format!("invalid requirement in resolver output: {line}"))?;
        if !requirement.evaluate_markers(environment, &[]) {
            continue;
        }
        let Some(VersionOrUrl::VersionSpecifier(specifiers)) = &requirement.version_or_url else {
            bail!("resolver output is not an exact registry pin: {line}");
        };
        let [specifier] = &**specifiers else {
            bail!("resolver output is not an exact registry pin: {line}");
        };
        ensure!(
            *specifier.operator() == Operator::Equal,
            "resolver output is not an exact registry pin: {line}"
        );
        let version = specifier.version();
        if let Some(previous) = selection.insert(requirement.name.clone(), version.clone()) {
            ensure!(
                previous == *version,
                "overlapping pins for {}: {previous} and {version}",
                requirement.name
            );
        }
    }
    Ok(selection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_only_fresh_lock_rejections() {
        let mismatch =
            "error: The lockfile at `uv.lock` needs to be updated, but `--check` was provided.";
        assert!(is_canonical_lock_mismatch(Some(1), mismatch));
        assert!(!is_canonical_lock_mismatch(Some(0), mismatch));
        assert!(!is_canonical_lock_mismatch(Some(2), mismatch));
        assert!(!is_canonical_lock_mismatch(
            Some(1),
            "error: Failed to download a wheel"
        ));
    }

    #[test]
    fn target_matrices_are_stable_and_deduplicated() {
        let versions = ["3.12", "3.13", "3.12"]
            .map(|version| PythonVersion::from_str(version).expect("valid Python version"));
        let targets = ScenarioTarget::matrix(
            &versions,
            &[
                ScenarioPlatform::Linux,
                ScenarioPlatform::Windows,
                ScenarioPlatform::Linux,
            ],
        );
        insta::assert_snapshot!(
            targets.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"),
            @"
        CPython 3.12 on x86_64-unknown-linux-gnu
        CPython 3.12 on x86_64-pc-windows-msvc
        CPython 3.13 on x86_64-unknown-linux-gnu
        CPython 3.13 on x86_64-pc-windows-msvc
        "
        );
        assert!(ScenarioTarget::matrix(&[], &[ScenarioPlatform::Linux]).is_empty());
        assert!(ScenarioTarget::matrix(&versions, &[]).is_empty());
    }

    #[test]
    fn rejects_overlapping_pins() -> Result<()> {
        let target = ScenarioTarget {
            python: PythonVersion::from_str("3.12").expect("valid Python version"),
            platform: ScenarioPlatform::Linux,
        };
        let environment = target.markers()?;
        let error = parse_pins(
            "a==1; python_version >= '3.11'\na==2; sys_platform == 'linux'",
            &environment,
        )
        .expect_err("the active pins overlap");
        insta::assert_snapshot!(error, @"overlapping pins for a: 1 and 2");
        let selection = parse_pins(
            "a==1; sys_platform == 'win32'\na==2; sys_platform == 'linux'",
            &environment,
        )?;
        insta::assert_snapshot!(
            selection.iter().map(|(name, version)| format!("{name}=={version}")).collect::<Vec<_>>().join("\n"),
            @"a==2"
        );
        Ok(())
    }
}
