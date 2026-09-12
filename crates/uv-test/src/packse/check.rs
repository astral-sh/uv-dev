//! Execute finite Packse scenarios against a real uv binary.

use std::fmt;
use std::io::{ErrorKind, Read};
use std::path::Path;
use std::process::{Command, Output};
use std::str::FromStr;

use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};

use uv_configuration::TargetTriple;
use uv_normalize::PackageName;
use uv_pep440::Operator;
use uv_pep508::{MarkerEnvironment, MarkerEnvironmentBuilder, Requirement, VersionOrUrl};
use uv_python::PythonVersion;
use uv_static::EnvVars;

use crate::TestContext;

use super::PackseServer;
use super::oracle::{ScenarioOracle, Selection};
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
#[derive(Clone, Debug)]
pub struct ScenarioTarget {
    pub python: PythonVersion,
    pub platform: ScenarioPlatform,
}

impl ScenarioTarget {
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
    if let Some(parent) = directory
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs_err::create_dir_all(parent)?;
    }
    fs_err::create_dir(directory)?;
    fs_err::write(directory.join("scenario.toml"), document.to_toml()?)?;
    fs_err::write(directory.join("requirements.in"), requirements)?;
    fs_err::write(directory.join("stdout.txt"), &output.stdout)?;
    fs_err::write(directory.join("stderr.txt"), &output.stderr)?;

    let mut executable = fs_err::File::open(command.get_program())?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = executable.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    fs_err::write(
        directory.join("command.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "program": command.get_program().to_string_lossy(),
            "sha256": hex::encode(digest.finalize()),
            "args": command.get_args().map(|arg| arg.to_string_lossy()).collect::<Vec<_>>(),
            "status": output.status.code(),
        }))?,
    )?;
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
        searches.push((target, environment, search));
    }

    let server = PackseServer::from_scenario_without_build_dependencies(scenario);
    let mut root_name = PackageName::from_str("uv-scenario-root")?;
    while scenario.packages.contains_key(&root_name) {
        root_name = PackageName::from_str(&format!("{root_name}-root"))?;
    }
    let project = serde_json::json!({
        "project": {
            "name": root_name,
            "version": "0.0.0",
            "requires-python": requires_python.to_string(),
            "dependencies": scenario.root.requires.iter().map(ToString::to_string).collect::<Vec<_>>(),
        }
    });
    fs_err::write(
        context.temp_dir.join("pyproject.toml"),
        toml::to_string(&project)?,
    )?;
    let lock_path = context.temp_dir.join("uv.lock");
    if let Err(error) = fs_err::remove_file(&lock_path)
        && error.kind() != ErrorKind::NotFound
    {
        return Err(error.into());
    }

    let output = lock_command(context, scenario, &server)
        .output()
        .context("failed to run uv lock")?;
    if !output.status.success() {
        ensure_no_solution(&output, "uv lock")?;
        if let Some((target, _, _)) = searches
            .iter()
            .find(|(_, _, search)| search.solution.is_none())
        {
            return Ok(LockCheckResult::Unsatisfiable {
                witness: (*target).clone(),
                checked,
            });
        }
        bail!(
            "uv lock reported no solution, but the requested projections are satisfiable; \
             add marker environments to distinguish a resolver defect from an unsampled conflict:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let lock = fs_err::read(&lock_path)?;

    let output = lock_command(context, scenario, &server)
        .arg("--locked")
        .arg("--offline")
        .output()
        .context("failed to check the existing lockfile")?;
    ensure_success(&output, "uv lock --locked --offline")?;
    ensure!(
        fs_err::read(&lock_path)? == lock,
        "uv lock --locked changed the lockfile"
    );

    let output = lock_command(context, scenario, &server)
        .arg("--check")
        .arg("--refresh")
        .arg("--preview-features")
        .arg("lockfile-format-check")
        .output()
        .context("failed to check the canonical lockfile round trip")?;
    ensure_success(&output, "uv lock --check --refresh")?;
    ensure!(
        fs_err::read(&lock_path)? == lock,
        "uv lock --check changed the lockfile"
    );

    let output = context
        .export()
        .arg("--no-config")
        .arg("--frozen")
        .arg("--offline")
        .arg("--no-emit-project")
        .arg("--no-hashes")
        .arg("--no-header")
        .arg("--no-annotate")
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .output()
        .context("failed to export the frozen lockfile")?;
    ensure_success(&output, "uv export --frozen --offline")?;
    let requirements = std::str::from_utf8(&output.stdout)?;
    for (target, environment, search) in &searches {
        ensure!(
            search.solution.is_some(),
            "uv lock succeeded, but its {target} projection is unsatisfiable"
        );
        let selection = parse_pins(requirements, environment)
            .with_context(|| format!("invalid lock export for {target}"))?;
        ScenarioOracle::new(scenario, environment)?
            .validate(&selection)
            .with_context(|| format!("invalid lock dependency closure for {target}"))?;
    }
    ensure!(
        fs_err::read(&lock_path)? == lock,
        "frozen export changed the lockfile"
    );
    Ok(LockCheckResult::Satisfiable {
        projections: searches.len(),
        checked,
    })
}

fn target_environment(scenario: &Scenario, target: &ScenarioTarget) -> Result<MarkerEnvironment> {
    let environment = target.markers()?;
    ensure!(
        scenario
            .root
            .requires_python
            .as_ref()
            .is_none_or(|specifier| specifier.contains(environment.python_full_version())),
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
