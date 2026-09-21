use std::process::ExitStatus;

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
#[cfg(windows)]
use std::os::windows::process::ExitStatusExt;

use super::*;
use crate::packse::evidence::{LockTrace, hash_executable};

const SCENARIO: &str = r#"
name = "structured-unit"
[root]
requires_python = ">=3.12,<3.15"
requires = ["a"]
[expected]
satisfiable = true
[packages.a.versions."1.0.0"]
requires = []
sdist = false
wheel = true
"#;

const SNAPSHOT_ERROR_CHILD: &str = "UV_TEST_STRUCTURED_SNAPSHOT_ERROR_CHILD";
const SNAPSHOT_ERROR_CAPTURE: &[u8] = b"{broken";
const SNAPSHOT_ERROR_PROJECT: &[u8] = b"[project]\nname = 'changed'\n";

fn document() -> Result<ScenarioDocument> {
    SCENARIO.parse()
}

/// A private command-policy unit fixture. Real resolver classification is exercised separately by
/// the conventional scenario integration tests; this fixture does not stand in for an uv solve.
fn lock_fixture(context: &TestContext, mode: LockfileMode) -> Result<StructuredLock> {
    let document = document()?;
    validate_document(&document)?;
    let scenario = document.scenario()?;
    let project = ScenarioProject::new(&scenario)?;
    let pyproject = project.pyproject()?;
    fs_err::write(context.temp_dir.join("pyproject.toml"), &pyproject)?;
    let server = PackseServer::from_scenario_without_build_dependencies(&scenario);
    let index = server.validate_closed_world(&scenario)?;
    let executable = fs_err::canonicalize(std::env::current_exe()?)?;
    let current_dir = fs_err::canonicalize(context.temp_dir.path())?;
    let directory = tempfile::Builder::new()
        .prefix("structured-unit-")
        .tempdir_in(context.root.path())?;
    let root = fs_err::canonicalize(directory.path())?;
    let home = root.join("home");
    let credentials = root.join("credentials");
    let temporary = root.join("tmp");
    let cache = root.join("cache");
    for path in [&home, &credentials, &temporary, &cache] {
        fs_err::create_dir(path)?;
    }
    fs_err::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(credentials.join(CREDENTIAL_LOCK_FILE))?;
    let netrc = root.join("netrc");
    fs_err::write(&netrc, b"")?;
    let environment = strict_environment(context, &home, &credentials, &netrc, &temporary)?;
    let mut inventory = ClosedWorldInventory::new(
        project.name(),
        &">=3.12,<3.15".parse()?,
        &"3.12.13".parse()?,
    )?;
    inventory.add_registry_package(&"a".parse()?, &["1.0.0".parse()?])?;
    Ok(StructuredLock {
        executable: executable.clone(),
        directory,
        current_dir,
        cache: cache.clone(),
        credentials,
        netrc,
        capture: root.join("capture.json"),
        nonce: "0123456789abcdef0123456789abcdef".to_owned(),
        metadata: match mode {
            LockfileMode::Standard => CaptureMetadata::Standard,
            LockfileMode::WithoutMetadata => CaptureMetadata::WithoutMetadata,
        },
        environment,
        lock_arguments: lock_arguments(&scenario, mode, &cache, server.index_url(), &executable)?,
        pyproject: pyproject.into_bytes(),
        inventory,
        index,
        project_before: None,
        prepared: None,
        run: None,
    })
}

fn context() -> Result<TestContext> {
    Ok(TestContext::new_with_versions_and_bin(
        &[],
        std::env::current_exe()?,
    ))
}

fn status(code: u8) -> ExitStatus {
    #[cfg(unix)]
    {
        ExitStatus::from_raw(i32::from(code) << 8)
    }
    #[cfg(windows)]
    {
        ExitStatus::from_raw(u32::from(code))
    }
}

fn output(code: u8, stdout: &[u8]) -> Output {
    Output {
        status: status(code),
        stdout: stdout.to_vec(),
        stderr: b"retained without parsing".to_vec(),
    }
}

fn record(lock: &mut StructuredLock, output: &Output) -> Result<()> {
    let digest = hash_executable(&lock.executable)?;
    lock.record_initial(
        ProcessIdentity {
            executable: lock.executable.clone(),
            producer_pid: std::process::id(),
            before_sha256: digest.clone(),
            after_sha256: Some(digest),
            after_error: None,
        },
        output,
    )
}

#[test]
fn raw_document_aliases_and_bounds_fail_closed() -> Result<()> {
    validate_document(&document()?)?;
    let duplicate_package = format!("{SCENARIO}\n[packages.A.versions.\"2.0.0\"]\nwheel = true\n");
    assert!(
        validate_document(&duplicate_package.parse()?)
            .expect_err("normalized package collision")
            .to_string()
            .contains("duplicate normalized package")
    );
    let duplicate_version = format!("{SCENARIO}\n[packages.a.versions.\"1.0\"]\nwheel = true\n");
    assert!(
        validate_document(&duplicate_version.parse()?)
            .expect_err("normalized version collision")
            .to_string()
            .contains("duplicate normalized version")
    );
    let duplicate_extra = format!(
        "{SCENARIO}\n[packages.a.versions.\"1.0.0\".extras]\nfeature_a = []\nfeature-a = []\n"
    );
    assert!(
        validate_document(&duplicate_extra.parse()?)
            .expect_err("normalized extra collision")
            .to_string()
            .contains("duplicate normalized extra")
    );

    let mut budget = InputBudget::default();
    assert!(budget.atom(&"x".repeat(MAX_INPUT_ATOM_BYTES + 1)).is_err());
    let mut budget = InputBudget::default();
    assert!(budget.work(MAX_INPUT_WORK + 1).is_err());
    let mut budget = InputBudget::default();
    assert!(
        budget
            .distribution(NoSolutionEvidence::MAX_JSON_BYTES + 1)
            .is_err()
    );
    let mut nested = toml::Value::Boolean(true);
    for _ in 0..=MAX_INPUT_DEPTH {
        nested = toml::Value::Array(vec![nested]);
    }
    assert!(check_value(&nested, 0, &mut InputBudget::default()).is_err());
    Ok(())
}

#[test]
fn only_the_generated_project_and_supported_index_policy_are_admitted() -> Result<()> {
    let scenario = document()?.scenario()?;
    let project = ScenarioProject::new(&scenario)?;
    validate_scenario_policy(&scenario)?;
    validate_project(&project, &project.pyproject()?)?;
    let mut changed: toml::Value = toml::from_str(&project.pyproject()?)?;
    changed["project"]["dependencies"] = toml::Value::Array(Vec::new());
    assert!(validate_project(&project, &toml::to_string(&changed)?).is_err());
    assert!(
        validate_project(
            &project,
            &format!(
                "{}\n[tool.uv]\nindex-strategy = 'unsafe-best-match'\n",
                project.pyproject()?
            )
        )
        .is_err()
    );
    for url in [
        "https://pypi.org/simple/",
        "http://user@127.0.0.1:1234/simple/",
        "http://127.0.0.1:1234/other/",
        "http://download.pytorch.org:1234/simple/",
        "http://127.0.0.1:1234/simple/?ignored-error-codes=401",
    ] {
        assert!(validate_index_url(url).is_err(), "{url}");
    }
    validate_index_url("http://127.0.0.1:1234/simple/")?;
    let exact = PackseServer::from_scenario_without_build_dependencies(&scenario);
    let identity = exact.validate_closed_world(&scenario)?;
    let value = serde_json::to_value(identity)?;
    assert_eq!(value["packages"], 1);
    assert_eq!(value["distributions"], 1);
    assert_eq!(
        value["manifest_sha256"]
            .as_str()
            .context("index digest")?
            .len(),
        64
    );
    assert!(
        PackseServer::from_scenario(&scenario)
            .validate_closed_world(&scenario)
            .is_err()
    );
    let mut unsupported = document()?.scenario()?;
    unsupported.resolver_options.prereleases = true;
    assert!(validate_scenario_policy(&unsupported).is_err());
    unsupported.resolver_options.prereleases = false;
    unsupported.resolver_options.environments = vec!["sys_platform == 'win32'".parse()?];
    validate_scenario_policy(&unsupported)?;
    let restricted = ScenarioProject::new(&unsupported)?;
    validate_project(&restricted, &restricted.pyproject()?)?;
    unsupported.resolver_options.required_environments = vec!["sys_platform == 'win32'".parse()?];
    assert!(validate_scenario_policy(&unsupported).is_err());
    Ok(())
}

#[test]
fn structured_locks_bind_explicit_selection_policies() -> Result<()> {
    let scenario = format!(
        "{SCENARIO}\n[resolver_options]\nresolution = 'lowest'\nfork_strategy = 'fewest'\n"
    )
    .parse::<ScenarioDocument>()?
    .scenario()?;
    validate_scenario_policy(&scenario)?;
    let arguments = lock_arguments(
        &scenario,
        LockfileMode::Standard,
        Path::new("cache"),
        "http://127.0.0.1:1234/simple/".to_owned(),
        Path::new("python"),
    )?;
    for (option, value) in [("--resolution", "lowest"), ("--fork-strategy", "fewest")] {
        assert_eq!(
            arguments
                .windows(2)
                .filter(|pair| pair[0] == option && pair[1] == value)
                .count(),
            1
        );
    }
    Ok(())
}

#[test]
fn structured_locks_admit_only_the_generated_environment_policy() -> Result<()> {
    let scenario = format!(
        "{SCENARIO}\n[resolver_options]\nenvironments = [\"sys_platform == 'win32'\", \"sys_platform == 'linux'\"]\n"
    ).parse::<ScenarioDocument>()?.scenario()?;
    validate_scenario_policy(&scenario)?;
    let project = ScenarioProject::new(&scenario)?;
    let pyproject = project.pyproject()?;
    validate_project(&project, &pyproject)?;
    let mut changed: toml::Value = toml::from_str(&pyproject)?;
    changed["tool"]["uv"]["environments"]
        .as_array_mut()
        .context("environment list")?
        .reverse();
    assert!(validate_project(&project, &toml::to_string(&changed)?).is_err());
    let mut changed: toml::Value = toml::from_str(&pyproject)?;
    changed["tool"]["uv"]["index-strategy"] = "unsafe-best-match".into();
    assert!(validate_project(&project, &toml::to_string(&changed)?).is_err());
    let mut overlapping = scenario;
    overlapping
        .resolver_options
        .environments
        .push("python_version >= '3.12'".parse()?);
    assert!(validate_scenario_policy(&overlapping).is_err());
    Ok(())
}

#[test]
fn private_policy_files_are_checked_before_and_after_the_command() -> Result<()> {
    for (file, message) in [
        ("credentials", "the strict credential store is not empty"),
        ("netrc", "the strict netrc file is not empty"),
        ("cache", "the witnessed resolver cache is not fresh"),
        (
            "uv.toml",
            "an unexpected uv.toml is present beside the generated project",
        ),
        (
            "ancestor",
            "an ancestor project is outside the structured lock model",
        ),
    ] {
        let context = context()?;
        let mut lock = lock_fixture(&context, LockfileMode::Standard)?;
        let path = match file {
            "credentials" => lock.credentials.join("credentials.toml"),
            "netrc" => lock.netrc.clone(),
            "cache" => lock.cache.join("unexpected"),
            "uv.toml" => lock.current_dir.join("uv.toml"),
            "ancestor" => context.root.path().join("pyproject.toml"),
            _ => bail!("unknown policy fixture"),
        };
        fs_err::write(path, b"unexpected")?;
        let mut command = lock.lock_command(&context);
        assert_eq!(
            lock.prepare_initial(&mut command)
                .expect_err("the unsupported policy must stop before spawn")
                .to_string(),
            message
        );
        assert!(lock.run.is_none());
    }

    let context = context()?;
    let mut lock = lock_fixture(&context, LockfileMode::Standard)?;
    let mut command = lock.lock_command(&context);
    lock.prepare_initial(&mut command)?;
    fs_err::write(&lock.netrc, b"unexpected")?;
    record(&mut lock, &output(0, &[]))?;
    assert!(lock.conclusion().is_err());
    Ok(())
}

#[test]
fn only_the_empty_credential_lock_is_allowed_before_and_after_the_command() -> Result<()> {
    const MESSAGE: &str = "the strict credential store is not empty";
    for after in [false, true] {
        for mutation in [
            "missing",
            "nonempty",
            "directory",
            "credential",
            "sibling",
            #[cfg(unix)]
            "symlink",
        ] {
            let context = context()?;
            let mut lock = lock_fixture(&context, LockfileMode::Standard)?;
            let mut command = lock.lock_command(&context);
            if after {
                lock.prepare_initial(&mut command)?;
            }
            let path = lock.credentials.join(CREDENTIAL_LOCK_FILE);
            match mutation {
                "missing" => fs_err::remove_file(&path)?,
                "nonempty" => fs_err::write(&path, b"unexpected")?,
                "directory" => {
                    fs_err::remove_file(&path)?;
                    fs_err::create_dir(&path)?;
                }
                "credential" => {
                    fs_err::write(lock.credentials.join("credentials.toml"), b"")?;
                }
                "sibling" => fs_err::write(lock.credentials.join("unexpected"), b"")?,
                #[cfg(unix)]
                "symlink" => {
                    fs_err::remove_file(&path)?;
                    fs_err::os::unix::fs::symlink(&lock.netrc, &path)?;
                }
                _ => bail!("unknown credential fixture"),
            }
            if after {
                record(&mut lock, &output(0, &[]))?;
                assert_eq!(
                    lock.run.as_ref().context("retained command")?.policy_after,
                    Err(MESSAGE.to_owned()),
                    "{mutation}"
                );
                assert!(lock.conclusion().is_err(), "{mutation}");
            } else {
                assert_eq!(
                    lock.prepare_initial(&mut command)
                        .expect_err("invalid credentials must stop before spawn")
                        .to_string(),
                    MESSAGE,
                    "{mutation}"
                );
                assert!(lock.run.is_none());
            }
        }
    }
    Ok(())
}

#[test]
fn strict_command_scrubs_dynamic_credentials_and_rejects_later_injection() -> Result<()> {
    let context = context()?
        .with_env("UV_INDEX_PRIVATE_USERNAME", "unexpected")
        .with_env("UV_INDEX_PRIVATE_PASSWORD", "unexpected")
        .with_env(EnvVars::UV_TORCH_BACKEND, "cu129")
        .with_env(EnvVars::UV_INDEX_STRATEGY, "unsafe-best-match")
        .with_env(EnvVars::UV_CONFIG_FILE, "unexpected.toml")
        .with_env(EnvVars::UV_TEST_AVAILABLE_VERSION_CUTOFF, TEST_TIMESTAMP)
        .with_env(EnvVars::UV_INTERNAL__SHOW_DERIVATION_TREE, "1")
        .with_env(EnvVars::UV_INTERNAL__RESOLVER_CAPTURE, "inherited.json")
        .with_env(
            EnvVars::UV_INTERNAL__RESOLVER_CAPTURE_REQUEST,
            "ffffffffffffffffffffffffffffffff",
        )
        .with_env(EnvVars::HTTP_PROXY, "http://127.0.0.1:9/");
    let mut lock = lock_fixture(&context, LockfileMode::WithoutMetadata)?;
    let mut command = lock.lock_command(&context);
    lock.prepare_initial(&mut command)?;
    let environment = command_environment(&command)?;
    for name in [
        "UV_INDEX_PRIVATE_USERNAME",
        "UV_INDEX_PRIVATE_PASSWORD",
        EnvVars::UV_TORCH_BACKEND,
        EnvVars::UV_INDEX_STRATEGY,
        EnvVars::UV_CONFIG_FILE,
        EnvVars::UV_TEST_AVAILABLE_VERSION_CUTOFF,
        EnvVars::UV_INTERNAL__SHOW_DERIVATION_TREE,
        EnvVars::HTTP_PROXY,
    ] {
        assert!(!environment.contains_key(name), "{name}");
    }
    assert_eq!(
        environment[EnvVars::UV_INTERNAL__RESOLVER_CAPTURE_REQUEST],
        lock.nonce
    );
    assert!(
        command_arguments(&command)?
            .windows(2)
            .any(|args| args == ["--index-strategy", "first-index"])
    );
    assert!(
        command_arguments(&command)?
            .windows(2)
            .any(|args| args == ["--preview-features", "lock-without-metadata"])
    );
    command.arg("--index-strategy").arg("unsafe-best-match");
    assert!(lock.validate_command(&command).is_err());
    for (name, value) in [
        ("UV_INDEX_PRIVATE_PASSWORD", "unexpected"),
        (EnvVars::UV_TEST_AVAILABLE_VERSION_CUTOFF, TEST_TIMESTAMP),
    ] {
        let mut changed = lock.lock_command(&context);
        changed
            .arg("--no-offline")
            .env(EnvVars::UV_INTERNAL__RESOLVER_CAPTURE, &lock.capture)
            .env(EnvVars::UV_INTERNAL__RESOLVER_CAPTURE_REQUEST, &lock.nonce)
            .env(name, value);
        assert!(lock.validate_command(&changed).is_err());
    }
    for command in [
        lock.lock_command(&context),
        lock.command(&context, "export"),
    ] {
        let environment = command_environment(&command)?;
        assert!(!environment.contains_key(EnvVars::UV_INTERNAL__RESOLVER_CAPTURE));
        assert!(!environment.contains_key(EnvVars::UV_INTERNAL__RESOLVER_CAPTURE_REQUEST));
    }
    assert!(
        lock.prepare_initial(&mut lock.lock_command(&context))
            .is_err()
    );
    Ok(())
}

#[test]
fn lockfile_snapshot_error_child() -> Result<()> {
    if std::env::var_os(SNAPSHOT_ERROR_CHILD).as_deref() != Some(OsStr::new("1")) {
        return Ok(());
    }
    let capture = std::env::var_os(EnvVars::UV_INTERNAL__RESOLVER_CAPTURE)
        .context("the child capture path was provided")?;
    let netrc = std::env::var_os(EnvVars::NETRC).context("the child netrc path was provided")?;
    fs_err::write(PathBuf::from(capture), SNAPSHOT_ERROR_CAPTURE)?;
    fs_err::write("pyproject.toml", SNAPSHOT_ERROR_PROJECT)?;
    fs_err::write(PathBuf::from(netrc), b"unexpected")?;
    fs_err::create_dir("uv.lock")?;
    Ok(())
}

#[test]
fn lockfile_snapshot_errors_retain_completed_child_observations() -> Result<()> {
    let context = context()?;
    let mut lock = lock_fixture(&context, LockfileMode::Standard)?;
    let private_directory = lock.directory.path().to_owned();
    let lock_path = lock.current_dir.join("uv.lock");
    lock.project_before = Some(FileObservation::read(
        &lock.current_dir.join("pyproject.toml"),
    ));

    // This real child exercises evidence retention, not resolver classification. Its command is
    // deliberately outside the admitted uv invocation and its capture is malformed.
    let mut command = lock.base_command(&context);
    command
        .args([
            "--exact",
            "packse::structured::tests::lockfile_snapshot_error_child",
            "--nocapture",
        ])
        .env(SNAPSHOT_ERROR_CHILD, "1")
        .env(EnvVars::UV_INTERNAL__RESOLVER_CAPTURE, &lock.capture)
        .env(EnvVars::UV_INTERNAL__RESOLVER_CAPTURE_REQUEST, &lock.nonce);
    assert!(lock.validate_command(&command).is_err());
    lock.prepared = Some(PreparedCommand {
        arguments: command_arguments(&command)?,
        environment: command_environment(&command)?,
    });

    let mut trace = LockTrace::default();
    let error = trace
        .run_bound("lock", command, &lock_path, |identity, output| {
            lock.record_initial(identity, output)
        })
        .expect_err("reading the child-created lockfile directory must fail");
    assert!(error.downcast_ref::<io::Error>().is_some());
    assert!(lock_path.is_dir());
    let run = lock.run.as_ref().context("the child was recorded")?;
    assert!(run.output.success);
    assert_eq!(run.output.exit_code, Some(0));
    assert_ne!(run.identity.producer_pid, std::process::id());
    run.identity.verify()?;
    assert_eq!(
        run.capture.complete_contents(),
        Some(SNAPSHOT_ERROR_CAPTURE)
    );
    assert_eq!(
        run.project_after.complete_contents(),
        Some(SNAPSHOT_ERROR_PROJECT)
    );
    assert_eq!(
        run.policy_after.as_ref().expect_err("changed netrc"),
        "the strict netrc file is not empty"
    );
    let producer_pid = run.identity.producer_pid;
    assert!(lock.conclusion().is_err());

    let evidence = context.root.path().join("snapshot-error-evidence");
    fs_err::create_dir(&evidence)?;
    trace.write(&evidence)?;
    lock.write_artifacts(&evidence, "scenario-digest", "assignment-digest")?;
    drop(lock);
    assert!(!private_directory.exists());
    assert_eq!(
        fs_err::read(evidence.join("structured/capture.json"))?,
        SNAPSHOT_ERROR_CAPTURE
    );
    assert_eq!(
        fs_err::read(evidence.join("structured/pyproject.after.toml"))?,
        SNAPSHOT_ERROR_PROJECT
    );
    let invocation: serde_json::Value =
        serde_json::from_slice(&fs_err::read(evidence.join("structured/invocation.json"))?)?;
    let command_dir = evidence.join("commands/01-lock");
    let command: serde_json::Value =
        serde_json::from_slice(&fs_err::read(command_dir.join("command.json"))?)?;
    assert_eq!(command["status"], 0);
    assert_eq!(command["process_identity"], invocation["run"]["identity"]);
    assert_eq!(invocation["run"]["identity"]["producer_pid"], producer_pid);
    assert_eq!(invocation["run"]["capture"]["complete"], true);
    assert_eq!(
        invocation["run"]["output"]["stdout_sha256"],
        hash_bytes(&fs_err::read(command_dir.join("stdout.txt"))?)
    );
    assert_eq!(
        invocation["run"]["output"]["stderr_sha256"],
        hash_bytes(&fs_err::read(command_dir.join("stderr.txt"))?)
    );
    assert_eq!(
        invocation["run"]["policy_after"]["Err"],
        "the strict netrc file is not empty"
    );
    Ok(())
}

#[test]
fn bounded_file_read_retains_rejected_bytes_without_certifying_them() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("capture.json");
    assert!(FileObservation::read(&path).is_missing());
    fs_err::write(&path, b"not JSON")?;
    let file = FileObservation::read(&path);
    assert_eq!(file.complete_contents(), Some(b"not JSON".as_slice()));
    assert_eq!(file.metadata.sha256, Some(hash_bytes(b"not JSON")));
    fs_err::write(&path, vec![b'x'; NoSolutionEvidence::MAX_JSON_BYTES + 1])?;
    let file = FileObservation::read(&path);
    assert!(file.complete_contents().is_none());
    assert_eq!(file.metadata.state, "prefix");
    assert_eq!(
        file.metadata.retained_bytes,
        NoSolutionEvidence::MAX_JSON_BYTES
    );
    assert!(file.metadata.error.is_some());
    assert!(
        FileObservation::read(directory.path())
            .complete_contents()
            .is_none()
    );
    #[cfg(unix)]
    {
        let link = directory.path().join("link.json");
        fs_err::os::unix::fs::symlink(&path, &link)?;
        let file = FileObservation::read(&link);
        assert!(file.contents.is_none());
        assert!(!file.is_missing());
    }
    Ok(())
}

#[test]
fn terminal_capture_contract_never_uses_stderr_or_falls_back() -> Result<()> {
    let context = context()?;
    for (output, accepted) in [
        (output(0, &[]), true),
        (output(1, &[]), false),
        (output(2, &[]), false),
        (output(0, b"unexpected"), false),
    ] {
        let mut lock = lock_fixture(&context, LockfileMode::Standard)?;
        let mut command = lock.lock_command(&context);
        lock.prepare_initial(&mut command)?;
        record(&mut lock, &output)?;
        assert_eq!(lock.conclusion().is_ok(), accepted);
    }

    let mut lock = lock_fixture(&context, LockfileMode::Standard)?;
    let mut command = lock.lock_command(&context);
    lock.prepare_initial(&mut command)?;
    fs_err::write(&lock.capture, b"{broken")?;
    record(&mut lock, &output(1, &[]))?;
    assert!(lock.conclusion().is_err());
    let evidence = lock.directory.path().join("failure");
    fs_err::create_dir(&evidence)?;
    lock.write_artifacts(&evidence, "scenario-digest", "assignment-digest")?;
    assert_eq!(
        fs_err::read(evidence.join("structured/capture.json"))?,
        b"{broken"
    );
    let invocation: serde_json::Value =
        serde_json::from_slice(&fs_err::read(evidence.join("structured/invocation.json"))?)?;
    assert_eq!(invocation["run"]["capture"]["complete"], true);
    assert_eq!(
        invocation["run"]["identity"]["producer_pid"],
        std::process::id()
    );
    assert_eq!(invocation["run"]["output"]["exit_code"], 1);
    assert_eq!(
        invocation["run"]["output"]["stderr_sha256"],
        hash_bytes(b"retained without parsing")
    );

    let mut lock = lock_fixture(&context, LockfileMode::Standard)?;
    let mut command = lock.lock_command(&context);
    lock.prepare_initial(&mut command)?;
    fs_err::write(&lock.capture, b"{broken")?;
    record(&mut lock, &output(0, &[]))?;
    assert!(lock.conclusion().is_err());
    Ok(())
}
