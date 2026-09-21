//! Source-bound transport and closed-world admission for witnessed scenario locks.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::str::FromStr;

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};

use uv_cache::Cache;
use uv_normalize::{ExtraName, PackageName};
use uv_pep440::Version;
use uv_pep508::Requirement;
use uv_python::Interpreter;
use uv_resolver::no_solution_capture::{
    CaptureMetadata, CaptureOperation, CaptureScope, CaptureToken, ClosedWorldInventory,
    ClosedWorldNoSolution, NoSolutionEvidence, classify_closed_world_no_solution,
};
use uv_static::EnvVars;

use crate::{TEST_TIMESTAMP, TestContext};

use super::PackseServer;
use super::check::LockfileMode;
use super::evidence::ProcessIdentity;
use super::project::ScenarioProject;
use super::scenario::{Scenario, ScenarioDocument};
use super::server::ClosedWorldIndex;

#[cfg(test)]
mod tests;

// These bounds apply before the checker duplicates its TOML and served inputs. The resolver
// independently applies the checked capture's inventory, component, and candidate-join limits.
const MAX_INPUT_DEPTH: usize = 32;
const MAX_INPUT_ATOM_BYTES: usize = 16_384;
const MAX_INPUT_TEXT_BYTES: usize = 2 * 1024 * 1024;
const MAX_INPUT_WORK: usize = 1_000_000;
const MAX_INPUT_NAMES: usize = 4_096;
const MAX_INPUT_VERSIONS: usize = 65_536;
const MAX_INDEX_BYTES: usize = 64 * 1024 * 1024;
const CREDENTIAL_LOCK_FILE: &str = "credentials.toml.lock";

#[derive(Default)]
pub(super) struct InputBudget {
    work: usize,
    text: usize,
    distributions: usize,
    distribution_bytes: usize,
}

impl InputBudget {
    pub(super) fn work(&mut self, amount: usize) -> Result<()> {
        self.work = self.work.saturating_add(amount);
        ensure!(
            self.work <= MAX_INPUT_WORK,
            "structured input work limit exceeded"
        );
        Ok(())
    }

    pub(super) fn atom(&mut self, value: &str) -> Result<()> {
        ensure!(
            value.len() <= MAX_INPUT_ATOM_BYTES,
            "structured input atom limit exceeded"
        );
        self.text = self.text.saturating_add(value.len());
        ensure!(
            self.text <= MAX_INPUT_TEXT_BYTES,
            "structured input text limit exceeded"
        );
        self.work(1)
    }

    pub(super) fn distribution(&mut self, bytes: usize) -> Result<()> {
        self.distributions = self.distributions.saturating_add(1);
        self.distribution_bytes = self.distribution_bytes.saturating_add(bytes);
        ensure!(
            self.distributions <= MAX_INPUT_VERSIONS
                && bytes <= NoSolutionEvidence::MAX_JSON_BYTES
                && self.distribution_bytes <= MAX_INDEX_BYTES,
            "structured index byte or distribution limit exceeded"
        );
        self.work(1)
    }
}

/// Bound the retained raw document before another typed parse or witness walk, and reject aliases
/// that would otherwise collapse into one normalized map key during deserialization.
pub(super) fn validate_document(document: &ScenarioDocument) -> Result<()> {
    let mut budget = InputBudget::default();
    check_value(document.value(), 0, &mut budget)?;
    let Some(packages) = document
        .value()
        .get("packages")
        .and_then(toml::Value::as_table)
    else {
        return Ok(());
    };
    ensure!(
        packages.len() <= MAX_INPUT_NAMES,
        "structured input package limit exceeded"
    );
    let mut names = BTreeSet::new();
    let mut version_count = 0_usize;
    for (name, package) in packages {
        budget.work(1)?;
        ensure!(
            names.insert(PackageName::from_str(name)?),
            "duplicate normalized package name `{name}`"
        );
        let versions = package
            .get("versions")
            .and_then(toml::Value::as_table)
            .context("a scenario package has no version table")?;
        version_count = version_count.saturating_add(versions.len());
        ensure!(
            version_count <= MAX_INPUT_VERSIONS,
            "structured input version limit exceeded"
        );
        let mut normalized = BTreeSet::new();
        for (version, metadata) in versions {
            budget.work(1)?;
            ensure!(
                normalized.insert(Version::from_str(version)?),
                "duplicate normalized version `{name}=={version}`"
            );
            if let Some(extras) = metadata.get("extras").and_then(toml::Value::as_table) {
                let mut normalized = BTreeSet::new();
                for extra in extras.keys() {
                    budget.work(1)?;
                    ensure!(
                        normalized.insert(ExtraName::from_str(extra)?),
                        "duplicate normalized extra name `{extra}`"
                    );
                }
            }
        }
    }
    Ok(())
}

fn check_value(value: &toml::Value, depth: usize, budget: &mut InputBudget) -> Result<()> {
    ensure!(
        depth <= MAX_INPUT_DEPTH,
        "structured input nesting limit exceeded"
    );
    budget.work(1)?;
    match value {
        toml::Value::String(value) => budget.atom(value)?,
        toml::Value::Array(values) => {
            for value in values {
                check_value(value, depth + 1, budget)?;
            }
        }
        toml::Value::Table(table) => {
            for (key, value) in table {
                budget.atom(key)?;
                check_value(value, depth + 1, budget)?;
            }
        }
        toml::Value::Integer(_)
        | toml::Value::Float(_)
        | toml::Value::Boolean(_)
        | toml::Value::Datetime(_) => {}
    }
    Ok(())
}

fn build_inventory(
    project: &ScenarioProject<'_>,
    scenario: &Scenario,
    interpreter: &Interpreter,
) -> Result<ClosedWorldInventory> {
    let requires_python = scenario
        .root
        .requires_python
        .as_ref()
        .context("missing project Python policy")?;
    let mut inventory = ClosedWorldInventory::new(
        project.name(),
        requires_python,
        interpreter.python_version(),
    )?;
    for extra in scenario.root.optional_dependencies.keys() {
        inventory.add_project_extra(extra)?;
    }
    for group in project.group_names() {
        inventory.add_project_group(group)?;
    }
    for (name, package) in &scenario.packages {
        inventory.add_registry_package(name, package.versions.keys())?;
        for metadata in package.versions.values() {
            for extra in metadata.extras.keys() {
                inventory.add_registry_extra(name, extra)?;
            }
        }
    }
    for requirement in project
        .all_requirements()
        .chain(scenario.packages.values().flat_map(|package| {
            package.versions.values().flat_map(|metadata| {
                metadata
                    .requires
                    .iter()
                    .chain(metadata.extras.values().flatten())
            })
        }))
    {
        add_requirement(&mut inventory, requirement)?;
    }
    Ok(inventory)
}

fn add_requirement(inventory: &mut ClosedWorldInventory, requirement: &Requirement) -> Result<()> {
    inventory.add_registry_reference(&requirement.name)?;
    for extra in &requirement.extras {
        inventory.add_registry_extra(&requirement.name, extra)?;
    }
    Ok(())
}

pub(super) fn validate_scenario_policy(scenario: &Scenario) -> Result<()> {
    let options = &scenario.resolver_options;
    ensure!(
        options.python.is_none()
            && options.python_platform.is_none()
            && !options.prereleases
            && options.no_build.is_empty()
            && options.no_binary.is_empty()
            && options.environments.is_empty()
            && options.required_environments.is_empty(),
        "the scenario has an unsupported structured lock policy"
    );
    for package in scenario.packages.values() {
        for metadata in package.versions.values() {
            ensure!(
                metadata.wheel_tags.len() <= 1,
                "structured locks require one universal wheel per raw candidate"
            );
        }
    }
    Ok(())
}

fn validate_project(project: &ScenarioProject<'_>, pyproject: &str) -> Result<()> {
    ensure!(
        pyproject.len() <= NoSolutionEvidence::MAX_JSON_BYTES,
        "generated project exceeds the structured byte bound"
    );
    ensure!(
        pyproject == project.pyproject()?,
        "generated project differs from the admitted scenario"
    );
    let value: toml::Value = toml::from_str(pyproject)?;
    let table = value
        .as_table()
        .context("generated project is not a TOML table")?;
    ensure!(
        table
            .keys()
            .all(|key| key == "project" || key == "dependency-groups"),
        "generated project contains unsupported configuration"
    );
    let package = table
        .get("project")
        .and_then(toml::Value::as_table)
        .context("missing generated project table")?;
    ensure!(
        package.keys().all(|key| matches!(
            key.as_str(),
            "name" | "version" | "requires-python" | "dependencies" | "optional-dependencies"
        )),
        "generated project has unsupported metadata"
    );
    ensure!(
        package.get("name").and_then(toml::Value::as_str) == Some(project.name().as_ref())
            && package.get("version").and_then(toml::Value::as_str) == Some("0.0.0"),
        "generated project identity changed"
    );
    Ok(())
}

fn validate_index_url(index: &str) -> Result<()> {
    let url = url::Url::parse(index)?;
    ensure!(
        url.scheme() == "http"
            && matches!(url.host_str(), Some("127.0.0.1" | "::1"))
            && url.port().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.path() == "/simple/"
            && url.query().is_none()
            && url.fragment().is_none(),
        "structured locks require one unnamed loopback Simple index"
    );
    Ok(())
}

fn path_text(path: &Path) -> Result<&str> {
    path.to_str()
        .context("structured command path is not UTF-8")
}

fn os_text(value: &OsStr) -> Result<&str> {
    value
        .to_str()
        .context("structured command value is not UTF-8")
}

pub(super) fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(super) fn hash_json(value: &impl Serialize) -> Result<String> {
    struct Writer {
        digest: Sha256,
        bytes: usize,
    }
    impl Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > NoSolutionEvidence::MAX_JSON_BYTES.saturating_sub(self.bytes) {
                return Err(io::Error::other(
                    "structured identity exceeds its byte bound",
                ));
            }
            self.bytes += bytes.len();
            self.digest.update(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Writer {
        digest: Sha256::new(),
        bytes: 0,
    };
    serde_json::to_writer(&mut writer, value)?;
    Ok(hex::encode(writer.digest.finalize()))
}

/// A private invocation assembled from the admitted project, index, and interpreter.
pub(super) struct StructuredLock {
    executable: PathBuf,
    directory: tempfile::TempDir,
    current_dir: PathBuf,
    cache: PathBuf,
    credentials: PathBuf,
    netrc: PathBuf,
    capture: PathBuf,
    nonce: String,
    metadata: CaptureMetadata,
    environment: BTreeMap<String, String>,
    lock_arguments: Vec<String>,
    pyproject: Vec<u8>,
    inventory: ClosedWorldInventory,
    index: ClosedWorldIndex,
    project_before: Option<FileObservation>,
    prepared: Option<PreparedCommand>,
    run: Option<StructuredRun>,
}

struct PreparedCommand {
    arguments: Vec<String>,
    environment: BTreeMap<String, String>,
}

struct StructuredRun {
    identity: ProcessIdentity,
    output: OutputIdentity,
    arguments: Vec<String>,
    environment: BTreeMap<String, String>,
    project_after: FileObservation,
    capture: FileObservation,
    policy_after: Result<(), String>,
}

#[derive(Serialize)]
struct OutputIdentity {
    exit_code: Option<i32>,
    success: bool,
    stdout_bytes: usize,
    stdout_sha256: String,
    stderr_bytes: usize,
    stderr_sha256: String,
}

impl OutputIdentity {
    fn new(output: &Output) -> Self {
        Self {
            exit_code: output.status.code(),
            success: output.status.success(),
            stdout_bytes: output.stdout.len(),
            stdout_sha256: hash_bytes(&output.stdout),
            stderr_bytes: output.stderr.len(),
            stderr_sha256: hash_bytes(&output.stderr),
        }
    }
}

impl StructuredLock {
    pub(super) fn new(
        context: &TestContext,
        scenario: &Scenario,
        server: &PackseServer,
        pyproject: &str,
        lockfile: LockfileMode,
        cache: &Path,
    ) -> Result<Self> {
        validate_scenario_policy(scenario)?;
        let project = ScenarioProject::new(scenario)?;
        validate_project(&project, pyproject)?;
        let index_url = server.index_url();
        validate_index_url(&index_url)?;
        let index = server.validate_closed_world(scenario)?;
        let executable = fs_err::canonicalize(&context.uv_bin)?;
        path_text(&executable)?;
        let current_dir = fs_err::canonicalize(context.temp_dir.path())?;
        let cache = fs_err::canonicalize(cache)?;
        let directory = tempfile::Builder::new()
            .prefix("scenario-structured-")
            .tempdir_in(context.root.path())?;
        let directory_path = fs_err::canonicalize(directory.path())?;
        let home = directory_path.join("home");
        let credentials = directory_path.join("credentials");
        let temporary = directory_path.join("tmp");
        for path in [&home, &credentials, &temporary] {
            fs_err::create_dir(path)?;
        }
        // Reading an absent credential store acquires this lock, even for an unauthenticated
        // index. Its empty regular file is the only credential-directory entry in this policy.
        fs_err::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(credentials.join(CREDENTIAL_LOCK_FILE))?;
        let netrc = directory_path.join("netrc");
        fs_err::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&netrc)?;
        let capture = directory_path.join("capture.json");
        let nonce = format!("{:032x}", fastrand::u128(..));
        let (_, selected_python) = context
            .python_versions
            .first()
            .context("structured locks require an available CPython interpreter")?;
        let selected_python = fs_err::canonicalize(selected_python)?;
        let python_cache = Cache::from_path(directory_path.join("python-query-cache"))
            .init_no_wait()?
            .context("the independent interpreter cache is busy")?;
        let interpreter = Interpreter::query(&selected_python, &python_cache)?;
        ensure!(
            interpreter.implementation_name() == "cpython",
            "structured locks require CPython"
        );
        let inventory = build_inventory(&project, scenario, &interpreter)?;
        let environment = strict_environment(context, &home, &credentials, &netrc, &temporary)?;
        let metadata = match lockfile {
            LockfileMode::Standard => CaptureMetadata::Standard,
            LockfileMode::WithoutMetadata => CaptureMetadata::WithoutMetadata,
        };
        let lock_arguments =
            lock_arguments(scenario, lockfile, &cache, index_url, &selected_python)?;
        Ok(Self {
            executable,
            directory,
            current_dir,
            cache,
            credentials,
            netrc,
            capture,
            nonce,
            metadata,
            environment,
            lock_arguments,
            pyproject: pyproject.as_bytes().to_vec(),
            inventory,
            index,
            project_before: None,
            prepared: None,
            run: None,
        })
    }

    fn base_command(&self, context: &TestContext) -> Command {
        let mut command = Command::new(&self.executable);
        // Apply ordinary test setup first, including `extra_env`, then replace the entire child
        // environment. This also removes dynamically named index credentials and unknown future
        // uv settings, rather than maintaining a partial denylist.
        context.add_shared_env(&mut command, false);
        command
            .env_clear()
            .envs(&self.environment)
            .current_dir(&self.current_dir);
        command
    }

    pub(super) fn command(&self, context: &TestContext, subcommand: &str) -> Command {
        let mut command = self.base_command(context);
        command
            .arg(subcommand)
            .arg("--cache-dir")
            .arg(&self.cache)
            .args(["--color", "never"]);
        command
    }

    pub(super) fn lock_command(&self, context: &TestContext) -> Command {
        let mut command = self.base_command(context);
        command.args(&self.lock_arguments);
        command
    }

    pub(super) fn prepare_initial(&mut self, command: &mut Command) -> Result<()> {
        ensure!(
            self.run.is_none() && self.project_before.is_none(),
            "structured evidence cannot run a second solve"
        );
        self.check_policy_files(true)?;
        ensure!(
            fs_err::symlink_metadata(&self.capture)
                .is_err_and(|error| error.kind() == ErrorKind::NotFound),
            "the capture destination already exists"
        );
        let project = FileObservation::read(&self.current_dir.join("pyproject.toml"));
        let valid_project = project
            .complete_contents()
            .is_some_and(|bytes| bytes == self.pyproject);
        self.project_before = Some(project);
        ensure!(
            valid_project,
            "the generated project changed before the lock command"
        );
        command
            .arg("--no-offline")
            .env(EnvVars::UV_INTERNAL__RESOLVER_CAPTURE, &self.capture)
            .env(EnvVars::UV_INTERNAL__RESOLVER_CAPTURE_REQUEST, &self.nonce);
        self.validate_command(command)?;
        self.prepared = Some(PreparedCommand {
            arguments: command_arguments(command)?,
            environment: command_environment(command)?,
        });
        Ok(())
    }

    fn expected_arguments(&self) -> Vec<String> {
        let mut arguments = self.lock_arguments.clone();
        arguments.push("--no-offline".to_owned());
        arguments
    }

    fn expected_environment(&self) -> Result<BTreeMap<String, String>> {
        let mut environment = self.environment.clone();
        environment.insert(
            EnvVars::UV_INTERNAL__RESOLVER_CAPTURE.to_owned(),
            path_text(&self.capture)?.to_owned(),
        );
        environment.insert(
            EnvVars::UV_INTERNAL__RESOLVER_CAPTURE_REQUEST.to_owned(),
            self.nonce.clone(),
        );
        Ok(environment)
    }

    fn validate_command(&self, command: &Command) -> Result<()> {
        ensure!(
            command.get_program() == self.executable.as_os_str()
                && command.get_current_dir() == Some(self.current_dir.as_path()),
            "the structured command identity changed"
        );
        ensure!(
            command_arguments(command)? == self.expected_arguments(),
            "the structured command has unsupported arguments"
        );
        ensure!(
            command_environment(command)? == self.expected_environment()?,
            "the structured command has unsupported environment settings"
        );
        Ok(())
    }

    pub(super) fn record_initial(
        &mut self,
        identity: ProcessIdentity,
        output: &Output,
    ) -> Result<()> {
        ensure!(
            self.project_before.is_some() && self.run.is_none(),
            "structured invocation was not prepared exactly once"
        );
        let prepared = self
            .prepared
            .take()
            .context("the actual structured command was not retained")?;
        self.run = Some(StructuredRun {
            identity,
            output: OutputIdentity::new(output),
            arguments: prepared.arguments,
            environment: prepared.environment,
            project_after: FileObservation::read(&self.current_dir.join("pyproject.toml")),
            capture: FileObservation::read(&self.capture),
            policy_after: self
                .check_policy_files(false)
                .map_err(|error| format!("{error:#}")),
        });
        Ok(())
    }

    pub(super) fn conclusion(&self) -> Result<Option<ClosedWorldNoSolution>> {
        let run = self
            .run
            .as_ref()
            .context("the structured lock was not recorded")?;
        run.identity.verify()?;
        ensure!(
            run.identity.executable == self.executable,
            "the captured executable identity changed"
        );
        ensure!(
            run.policy_after.is_ok(),
            "the strict command policy changed: {:?}",
            run.policy_after
        );
        ensure!(
            self.project_before
                .as_ref()
                .and_then(FileObservation::complete_contents)
                .is_some_and(|bytes| bytes == self.pyproject)
                && run
                    .project_after
                    .complete_contents()
                    .is_some_and(|bytes| bytes == self.pyproject),
            "the generated project changed during the lock command"
        );
        ensure!(
            run.output.stdout_bytes == 0,
            "the structured lock wrote unexpected stdout"
        );
        if run.output.success {
            ensure!(
                run.capture.is_missing(),
                "a successful lock unexpectedly published no-solution evidence"
            );
            return Ok(None);
        }
        ensure!(
            run.output.exit_code == Some(1),
            "the structured lock did not exit with status 1"
        );
        let bytes = run
            .capture
            .complete_contents()
            .context("the lock has no complete bounded capture file")?;
        let expected = CaptureToken::new(&self.nonce, run.identity.producer_pid)
            .context("invalid structured capture identity")?
            .for_lock(
                CaptureScope::Workspace,
                CaptureOperation::Write,
                self.metadata,
            );
        classify_closed_world_no_solution(bytes, &expected, &self.inventory)
            .map(Some)
            .context("the witnessed lock failure has no certified structured evidence")
    }

    fn check_policy_files(&self, fresh_cache: bool) -> Result<()> {
        ensure!(
            fs_err::symlink_metadata(&self.credentials)?
                .file_type()
                .is_dir(),
            "the strict credential store is not empty"
        );
        let mut entries = fs_err::read_dir(&self.credentials)?;
        let lock = entries.next().transpose()?;
        ensure!(
            entries.next().is_none()
                && lock.as_ref().is_some_and(|entry| {
                    entry.file_name() == CREDENTIAL_LOCK_FILE
                        && entry.file_type().is_ok_and(|kind| kind.is_file())
                }),
            "the strict credential store is not empty"
        );
        ensure!(
            fs_err::File::open(self.credentials.join(CREDENTIAL_LOCK_FILE))?.read(&mut [0])? == 0,
            "the strict credential store is not empty"
        );
        ensure!(
            fs_err::symlink_metadata(&self.netrc)?.file_type().is_file()
                && fs_err::read(&self.netrc)?.is_empty(),
            "the strict netrc file is not empty"
        );
        if fresh_cache {
            ensure!(
                fs_err::symlink_metadata(&self.cache)?.file_type().is_dir()
                    && fs_err::read_dir(&self.cache)?.next().is_none(),
                "the witnessed resolver cache is not fresh"
            );
        }
        ensure!(
            fs_err::symlink_metadata(self.current_dir.join("uv.toml"))
                .is_err_and(|error| error.kind() == ErrorKind::NotFound),
            "an unexpected uv.toml is present beside the generated project"
        );
        // Workspace discovery is distinct from uv configuration discovery. Do not allow an
        // ancestor pyproject to turn this generated project into a member of another workspace.
        for ancestor in self.current_dir.ancestors().skip(1) {
            match fs_err::symlink_metadata(ancestor.join("pyproject.toml")) {
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
                Ok(_) => bail!("an ancestor project is outside the structured lock model"),
            }
        }
        Ok(())
    }

    pub(super) fn write_artifacts(
        &self,
        directory: &Path,
        scenario_toml_sha256: &str,
        assignment_json_sha256: &str,
    ) -> Result<()> {
        let directory = directory.join("structured");
        fs_err::create_dir(&directory)?;
        if let Some(before) = &self.project_before {
            before.write_contents(&directory.join("pyproject.before.toml"))?;
        }
        if let Some(run) = &self.run {
            run.project_after
                .write_contents(&directory.join("pyproject.after.toml"))?;
            run.capture
                .write_contents(&directory.join("capture.json"))?;
        }
        fs_err::write(
            directory.join("invocation.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "protocol": "structured-v1",
                "request": self.nonce,
                "destination": self.capture,
                "expected": {"scope": "workspace", "operation": "write", "metadata": self.metadata},
                "project_sha256": hash_bytes(&self.pyproject),
                "scenario_toml_sha256": scenario_toml_sha256,
                "assignment_json_sha256": assignment_json_sha256,
                "index": self.index,
                "policy": {"index_strategy": "first-index", "torch_backend": null, "ignored_error_codes": [], "keyring_provider": "disabled", "no_build": true, "offline": false, "no_index": false, "environment_cleared": true, "fresh_cache": self.cache, "credentials": self.credentials, "netrc": self.netrc, "private_directory": self.directory.path()},
                "project_before": self.project_before.as_ref().map(|file| &file.metadata),
                "run": self.run.as_ref().map(|run| serde_json::json!({
                "identity": run.identity,
                "output": run.output,
                    "arguments": run.arguments,
                    "environment": run.environment,
                    "project_after": run.project_after.metadata,
                    "capture": run.capture.metadata,
                    "policy_after": run.policy_after,
                })),
            }))?,
        )?;
        Ok(())
    }
}

fn lock_arguments(
    scenario: &Scenario,
    lockfile: LockfileMode,
    cache: &Path,
    index_url: String,
    selected_python: &Path,
) -> Result<Vec<String>> {
    let mut arguments = vec![
        "lock".to_owned(),
        "--cache-dir".to_owned(),
        path_text(cache)?.to_owned(),
        "--color".to_owned(),
        "never".to_owned(),
        "--no-config".to_owned(),
        "--index-url".to_owned(),
        index_url,
        "--no-build".to_owned(),
    ];
    arguments.extend(scenario.resolver_options.selection_arguments());
    match lockfile {
        LockfileMode::Standard => {}
        LockfileMode::WithoutMetadata => arguments.extend([
            "--preview-features".to_owned(),
            "lock-without-metadata".to_owned(),
        ]),
    }
    arguments.extend([
        "--python".to_owned(),
        path_text(selected_python)?.to_owned(),
        "--no-python-downloads".to_owned(),
        "--index-strategy".to_owned(),
        "first-index".to_owned(),
        "--keyring-provider".to_owned(),
        "disabled".to_owned(),
    ]);
    Ok(arguments)
}

fn strict_environment(
    context: &TestContext,
    home: &Path,
    credentials: &Path,
    netrc: &Path,
    temporary: &Path,
) -> Result<BTreeMap<String, String>> {
    let mut environment = BTreeMap::new();
    if let Some(value) = std::env::var_os("SystemRoot").or_else(|| std::env::var_os("SYSTEMROOT")) {
        // Windows environment keys are case-insensitive, so retain this spelling only once.
        environment.insert("SystemRoot".to_owned(), os_text(&value)?.to_owned());
    }
    for name in [
        EnvVars::SYSTEMDRIVE,
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        EnvVars::RUST_MIN_STACK,
        EnvVars::UV_STACK_SIZE,
    ] {
        if let Some(value) = std::env::var_os(name) {
            environment.insert(name.to_owned(), os_text(&value)?.to_owned());
        }
    }
    let path = std::env::join_paths(std::iter::once(context.bin_dir.to_path_buf()).chain(
        std::env::split_paths(&std::env::var_os(EnvVars::PATH).unwrap_or_default()),
    ))?;
    environment.insert(EnvVars::PATH.to_owned(), os_text(&path)?.to_owned());
    for name in [
        EnvVars::HOME,
        EnvVars::APPDATA,
        EnvVars::USERPROFILE,
        EnvVars::XDG_CONFIG_HOME,
        EnvVars::XDG_CONFIG_DIRS,
        EnvVars::XDG_DATA_HOME,
    ] {
        environment.insert(name.to_owned(), path_text(home)?.to_owned());
    }
    for name in ["TMPDIR", "TMP", "TEMP"] {
        environment.insert(name.to_owned(), path_text(temporary)?.to_owned());
    }
    for (name, value) in [
        (EnvVars::UV_NO_WRAP, "1"),
        (EnvVars::UV_NO_SYSTEM_CONFIG, "1"),
        (EnvVars::COLUMNS, "100"),
        (EnvVars::UV_PYTHON_INSTALL_DIR, ""),
        (EnvVars::UV_PYTHON_DOWNLOADS, "never"),
        (EnvVars::UV_PYTHON_NO_REGISTRY, "1"),
        (EnvVars::UV_PYTHON_INSTALL_REGISTRY, "0"),
        (EnvVars::UV_TEST_NO_CLI_PROGRESS, "1"),
        (EnvVars::UV_TEST_CURRENT_TIMESTAMP, TEST_TIMESTAMP),
        (EnvVars::NO_PROXY, "127.0.0.1,localhost,::1"),
    ] {
        environment.insert(name.to_owned(), value.to_owned());
    }
    environment.insert(
        EnvVars::UV_PYTHON_SEARCH_PATH.to_owned(),
        os_text(&context.python_path())?.to_owned(),
    );
    environment.insert(
        EnvVars::UV_CREDENTIALS_DIR.to_owned(),
        path_text(credentials)?.to_owned(),
    );
    environment.insert(EnvVars::NETRC.to_owned(), path_text(netrc)?.to_owned());
    environment.insert(
        EnvVars::GIT_CEILING_DIRECTORIES.to_owned(),
        path_text(context.root.path())?.to_owned(),
    );
    if cfg!(unix) {
        environment.insert(EnvVars::SHELL.to_owned(), "bash".to_owned());
        environment.insert(EnvVars::LC_ALL.to_owned(), "C".to_owned());
    }
    Ok(environment)
}

fn command_arguments(command: &Command) -> Result<Vec<String>> {
    command
        .get_args()
        .map(|argument| os_text(argument).map(str::to_owned))
        .collect()
}

fn command_environment(command: &Command) -> Result<BTreeMap<String, String>> {
    command
        .get_envs()
        .map(|(name, value)| {
            let value = value
                .context("the structured command contains an unexpected environment removal")?;
            Ok((os_text(name)?.to_owned(), os_text(value)?.to_owned()))
        })
        .collect()
}

#[derive(Debug)]
struct FileObservation {
    metadata: FileObservationMetadata,
    contents: Option<Vec<u8>>,
}

#[derive(Debug, Serialize)]
struct FileObservationMetadata {
    state: &'static str,
    reported_bytes: Option<u64>,
    retained_bytes: usize,
    sha256: Option<String>,
    complete: bool,
    error: Option<String>,
}

impl FileObservation {
    fn read(path: &Path) -> Self {
        let mut observation = Self {
            metadata: FileObservationMetadata {
                state: "invalid",
                reported_bytes: None,
                retained_bytes: 0,
                sha256: None,
                complete: false,
                error: None,
            },
            contents: None,
        };
        if let Err(error) = observation.read_inner(path) {
            observation.metadata.error = Some(format!("{error:#}"));
        }
        observation
    }

    fn read_inner(&mut self, path: &Path) -> Result<()> {
        let initial = match fs_err::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.metadata.state = "missing";
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        ensure!(
            initial.file_type().is_file(),
            "the evidence path is not a regular non-symlink file"
        );
        self.metadata.reported_bytes = Some(initial.len());
        let file = fs_err::File::open(path)?;
        ensure!(
            file.metadata()?.is_file(),
            "the opened evidence is not a regular file"
        );
        let mut bytes = Vec::with_capacity(
            usize::try_from(initial.len())
                .unwrap_or(usize::MAX)
                .min(NoSolutionEvidence::MAX_JSON_BYTES + 1),
        );
        let read = file
            .take((NoSolutionEvidence::MAX_JSON_BYTES + 1) as u64)
            .read_to_end(&mut bytes);
        let within_bound = bytes.len() <= NoSolutionEvidence::MAX_JSON_BYTES;
        bytes.truncate(NoSolutionEvidence::MAX_JSON_BYTES);
        self.metadata.state = if within_bound { "retained" } else { "prefix" };
        self.metadata.retained_bytes = bytes.len();
        self.metadata.sha256 = Some(hash_bytes(&bytes));
        self.contents = Some(bytes);
        read?;
        ensure!(
            within_bound,
            "the evidence exceeds the reader byte bound; only a prefix is retained"
        );
        let after = fs_err::symlink_metadata(path)?;
        ensure!(
            after.file_type().is_file()
                && after.len() == initial.len()
                && initial.len() == self.metadata.retained_bytes as u64,
            "the evidence changed during its bounded read"
        );
        self.metadata.complete = true;
        Ok(())
    }

    fn is_missing(&self) -> bool {
        self.metadata.state == "missing" && self.metadata.error.is_none()
    }

    fn complete_contents(&self) -> Option<&[u8]> {
        self.metadata
            .complete
            .then_some(self.contents.as_deref())
            .flatten()
    }

    fn write_contents(&self, path: &Path) -> Result<()> {
        if let Some(bytes) = &self.contents {
            fs_err::write(path, bytes)?;
        }
        Ok(())
    }
}
