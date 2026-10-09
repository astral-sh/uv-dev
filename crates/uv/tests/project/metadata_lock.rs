use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
#[cfg(unix)]
use fs_err::os::unix::fs::symlink;
use indoc::{formatdoc, indoc};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use url::Url;
use uv_cache_key::cache_digest;
use uv_fs::{LockedFile, LockedFileMode};
use uv_static::EnvVars;
use uv_test::packse::PackseServer;
use uv_test::{TestContext, uv_snapshot};

async fn hold_metadata(path: &Path, kind: &str) -> Result<LockedFile> {
    let path = fs_err::canonicalize(path)?;
    Ok(LockedFile::acquire(
        std::env::temp_dir().join(format!("uv-{kind}-metadata-{}.lock", cache_digest(&path))),
        LockedFileMode::Exclusive,
        "metadata edit",
    )
    .await?)
}

async fn queued_command(
    mut command: Command,
) -> Result<(
    tokio::process::Child,
    tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
)> {
    command.env(EnvVars::RUST_LOG, "uv_fs=info");
    let mut child = tokio::process::Command::from(command)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stderr = child
        .stderr
        .take()
        .context("captured metadata command stderr")?;
    let mut reader = tokio::io::BufReader::new(stderr);
    let mut output = Vec::new();
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let start = output.len();
            if reader.read_until(b'\n', &mut output).await? == 0 {
                anyhow::bail!("command exited without waiting for its metadata resource");
            }
            let line = String::from_utf8_lossy(&output[start..]);
            if line.contains("Waiting to acquire exclusive lock") && line.contains("-metadata-") {
                return Ok::<_, anyhow::Error>(());
            }
        }
    })
    .await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(err)) => anyhow::bail!(
            "Failed while waiting for metadata admission: {err}\nCaptured stderr:\n{}",
            String::from_utf8_lossy(&output)
        ),
        Err(err) => anyhow::bail!(
            "Timed out waiting for metadata admission: {err}\nCaptured stderr:\n{}",
            String::from_utf8_lossy(&output)
        ),
    }
    let stderr = tokio::spawn(async move {
        reader.read_to_end(&mut output).await?;
        Ok(output)
    });
    Ok((child, stderr))
}

async fn finish_command(
    child: tokio::process::Child,
    stderr: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
) -> Result<()> {
    let mut output =
        tokio::time::timeout(Duration::from_secs(30), child.wait_with_output()).await??;
    output.stderr = stderr.await??;
    output.assert().success();
    Ok(())
}

/// Pause an actual requirements download after the command has entered its metadata admission.
async fn pending_requirements(listener: &TcpListener) -> Result<TcpStream> {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (stream, _) = listener.accept().await?;
        let mut reader = tokio::io::BufReader::new(stream);
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).await? == 0 {
                anyhow::bail!("requirements connection closed before its headers completed");
            }
            if line == "\r\n" {
                return Ok(reader.into_inner());
            }
        }
    })
    .await?
}

async fn release_requirements(mut stream: TcpStream, contents: &str) -> Result<()> {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{contents}",
        contents.len(),
    );
    tokio::time::timeout(
        Duration::from_secs(30),
        stream.write_all(response.as_bytes()),
    )
    .await??;
    Ok(())
}

/// Membership publication drains a standalone writer that was admitted before the transition.
#[tokio::test]
async fn workspace_membership_waits_for_an_admitted_standalone_writer() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.11"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = []
    "#})?;
    let dep = context.temp_dir.child("dep");
    dep.create_dir_all()?;
    dep.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "dep"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = []
    "#})?;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let requirements = format!("http://{}/requirements.txt", listener.local_addr()?);
    let mut original = context.add();
    original
        .current_dir(dep.path())
        .args(["--frozen", "--python", "3.12", "-r"])
        .arg(&requirements);
    let original = tokio::process::Command::from(original)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let pending = pending_requirements(&listener).await?;

    let mut membership = context.add();
    membership.args(["./dep", "--workspace", "--frozen", "--python", "3.11"]);
    let (membership, stderr) = queued_command(membership).await?;
    assert!(
        !context
            .read("pyproject.toml")
            .contains("[tool.uv.workspace]")
    );
    release_requirements(pending, "alpha==1\n").await?;
    tokio::time::timeout(Duration::from_secs(30), original.wait_with_output())
        .await??
        .assert()
        .success();
    finish_command(membership, stderr).await?;

    uv_snapshot!(context.filters(), context.add()
        .args(["beta==1", "--package", "dep", "--frozen", "--python", "3.11"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.11.[X] interpreter at: [PYTHON-3.11]
    ");
    insta::assert_snapshot!(context.read("dep/pyproject.toml"), @r#"
    [project]
    name = "dep"
    version = "0.1.0"
    requires-python = ">=3.11"
    dependencies = [
        "alpha==1",
        "beta==1",
    ]
    "#);
    insta::assert_snapshot!(context.read("pyproject.toml"), @r#"
    [project]
    name = "parent"
    version = "0.1.0"
    requires-python = ">=3.11"
    dependencies = [
        "dep",
    ]

    [tool.uv.workspace]
    members = [
        "dep",
    ]

    [tool.uv.sources]
    dep = { workspace = true }
    "#);
    Ok(())
}

/// Candidate admission must precede the shared interpreter lock, including requirements files.
#[tokio::test]
async fn workspace_membership_does_not_hold_the_members_interpreter_before_admission() -> Result<()>
{
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;
    let dep = context.temp_dir.child("dep");
    dep.create_dir_all()?;
    dep.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "dep"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        package = false
    "#})?;
    local_wheel(&context, "alpha")?;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let requirements = format!("http://{}/requirements.txt", listener.local_addr()?);
    let mut membership = context.add();
    membership
        .args(["--workspace", "--python", "3.12", "--no-index", "-r"])
        .arg(&requirements)
        .arg("--find-links")
        .arg(context.temp_dir.child("wheels").path())
        .env(EnvVars::UV_PROJECT_ENVIRONMENT, context.venv.path());
    let membership = tokio::process::Command::from(membership)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let pending = pending_requirements(&listener).await?;

    // Both commands explicitly select the same environment; a misplaced lock becomes visible
    // as its real contention warning rather than leaving a test waiting for the default timeout.
    uv_snapshot!(context.filters(), context.add()
        .current_dir(dep.path())
        .args(["alpha==1", "--frozen", "--python", "3.12"])
        .env(EnvVars::UV_PROJECT_ENVIRONMENT, context.venv.path())
        .env(EnvVars::UV_LOCK_TIMEOUT, "1")
        .env(EnvVars::RUST_LOG, "uv_fs=info,uv_project_commands=warn"), @"exit_code: 0 (success)");
    let dep_url = Url::from_directory_path(dep.path())
        .map_err(|()| anyhow::anyhow!("dependency path must form a file URL"))?;
    release_requirements(pending, &format!("dep @ {dep_url}\n")).await?;
    tokio::time::timeout(Duration::from_secs(30), membership.wait_with_output())
        .await??
        .assert()
        .success();
    insta::assert_snapshot!(context.read("dep/pyproject.toml"), @r#"
    [project]
    name = "dep"
    version = "0.1.0"
    requires-python = ">=3.12"
    dependencies = [
        "alpha==1",
    ]

    [tool.uv]
    package = false
    "#);
    Ok(())
}

/// An outside member and its direct path share admission even through a workspace symlink.
#[tokio::test]
#[cfg(unix)]
async fn external_member_alias_and_standalone_writers_share_admission() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.11"]);
    let workspace = context.temp_dir.child("workspace");
    workspace.create_dir_all()?;
    let external = context.temp_dir.child("external");
    external.create_dir_all()?;
    external.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "dep"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = []
    "#})?;
    let alias = context.temp_dir.child("alias");
    symlink(external.path(), alias.path())?;
    workspace
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = []

        [tool.uv.workspace]
        members = [{alias:?}]
    "#, alias = alias.path()})?;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let requirements = format!("http://{}/requirements.txt", listener.local_addr()?);
    let mut direct = context.add();
    direct
        .current_dir(external.path())
        .args(["--frozen", "--python", "3.12", "-r"])
        .arg(&requirements);
    let direct = tokio::process::Command::from(direct)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let pending = pending_requirements(&listener).await?;

    let mut member = context.add();
    member.current_dir(workspace.path()).args([
        "beta==1",
        "--package",
        "dep",
        "--frozen",
        "--python",
        "3.11",
    ]);
    let (member, stderr) = queued_command(member).await?;
    release_requirements(pending, "alpha==1\n").await?;
    tokio::time::timeout(Duration::from_secs(30), direct.wait_with_output())
        .await??
        .assert()
        .success();
    finish_command(member, stderr).await?;
    insta::assert_snapshot!(context.read("external/pyproject.toml"), @r#"
    [project]
    name = "dep"
    version = "0.1.0"
    requires-python = ">=3.11"
    dependencies = [
        "alpha==1",
        "beta==1",
    ]
    "#);
    assert_eq!(
        context.read("alias/pyproject.toml"),
        context.read("external/pyproject.toml")
    );
    Ok(())
}

fn dependencies(context: &TestContext) -> Result<Vec<String>> {
    let contents = fs_err::read_to_string(context.temp_dir.join("pyproject.toml"))?;
    let project: toml::Value = toml::from_str(&contents)?;
    let mut dependencies = project["project"]["dependencies"]
        .as_array()
        .context("project dependencies")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .context("requirement string")
                .map(str::to_owned)
        })
        .collect::<Result<Vec<_>>>()?;
    dependencies.sort();
    Ok(dependencies)
}

#[tokio::test]
async fn queued_adds_reload_metadata_across_python_selections() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.11"]);
    let manifest = context.temp_dir.child("pyproject.toml");
    manifest.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = ["seed==1"]
    "#})?;
    let guard = hold_metadata(context.temp_dir.path(), "workspace").await?;
    let mut first = context.add();
    first.args(["alpha==1", "--frozen", "--python", "3.12"]);
    let (first, first_stderr) = queued_command(first).await?;
    let mut second = context.add();
    second.args(["beta==1", "--frozen", "--python", "3.11"]);
    let (second, second_stderr) = queued_command(second).await?;

    // The current owner publishes after both waiters have completed their initial discovery.
    manifest.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.11"
        dependencies = ["committed==1", "seed==1"]
    "#})?;
    drop(guard);
    finish_command(first, first_stderr).await?;
    finish_command(second, second_stderr).await?;
    insta::assert_debug_snapshot!(dependencies(&context)?, @r###"
    [
        "alpha==1",
        "beta==1",
        "committed==1",
        "seed==1",
    ]
    "###);
    assert!(!context.temp_dir.child("uv.lock").exists());
    Ok(())
}

#[tokio::test]
async fn queued_frozen_removes_keep_both_changes() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["alpha==1", "beta==1", "seed==1"]
    "#})?;
    let guard = hold_metadata(context.temp_dir.path(), "workspace").await?;
    let mut first = context.remove();
    first.args(["alpha", "--frozen"]);
    let (first, first_stderr) = queued_command(first).await?;
    let mut second = context.remove();
    second.args(["beta", "--frozen"]);
    let (second, second_stderr) = queued_command(second).await?;
    drop(guard);
    finish_command(first, first_stderr).await?;
    finish_command(second, second_stderr).await?;
    insta::assert_debug_snapshot!(dependencies(&context)?, @r###"
    [
        "seed==1",
    ]
    "###);
    assert!(!context.temp_dir.child("uv.lock").exists());
    Ok(())
}

fn local_wheel(context: &TestContext, name: &str) -> Result<()> {
    let directory = context.temp_dir.child("wheels");
    directory.create_dir_all()?;
    let (filename, contents) = uv_test::packse::generate_wheel(
        &name.parse()?,
        &"1".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    fs_err::write(directory.join(filename), contents)?;
    Ok(())
}

#[tokio::test]
async fn queued_script_add_and_remove_preserve_both_changes() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    local_wheel(&context, "alpha")?;
    local_wheel(&context, "seed")?;
    local_wheel(&context, "stale")?;
    let path = context.temp_dir.child("main.py");
    path.write_str(indoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = ["seed==1", "stale==1"]
        # ///
        print("script body")
    "#})?;
    let guard = hold_metadata(path.path(), "script").await?;
    let mut add = context.add();
    add.args([
        "--script",
        "main.py",
        "alpha==1",
        "--find-links",
        "wheels",
        "--no-index",
        "--offline",
    ]);
    let (add, add_stderr) = queued_command(add).await?;
    let mut remove = context.remove();
    remove.args(["--script", "main.py", "stale"]);
    let (remove, remove_stderr) = queued_command(remove).await?;
    drop(guard);
    finish_command(add, add_stderr).await?;
    finish_command(remove, remove_stderr).await?;
    let script = uv_scripts::Pep723Script::read(path.path())
        .await?
        .context("script metadata")?;
    let mut requirements = script
        .metadata
        .dependencies
        .context("script dependencies")?
        .into_iter()
        .map(|requirement| requirement.to_string())
        .collect::<Vec<_>>();
    requirements.sort();
    insta::assert_debug_snapshot!(requirements, @r###"
    [
        "alpha==1",
        "seed==1",
    ]
    "###);
    assert_eq!(script.postlude.trim(), "print(\"script body\")");
    assert!(!context.temp_dir.child("main.py.lock").exists());
    Ok(())
}

#[tokio::test]
async fn frozen_lock_reads_do_not_wait_for_metadata_writers() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;
    context.lock().arg("--offline").assert().success();
    let before = fs_err::read(context.temp_dir.join("uv.lock"))?;
    let _guard = hold_metadata(context.temp_dir.path(), "workspace").await?;
    let mut command = context.lock();
    command.args(["--frozen", "--offline"]);
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::process::Command::from(command)
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    output.assert().success();
    assert_eq!(fs_err::read(context.temp_dir.join("uv.lock"))?, before);
    Ok(())
}

#[tokio::test]
async fn queued_lock_uses_the_current_manifest() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let manifest = context.temp_dir.child("pyproject.toml");
    manifest.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;
    let guard = hold_metadata(context.temp_dir.path(), "workspace").await?;
    let mut command = context.lock();
    command.arg("--offline");
    let (command, stderr) = queued_command(command).await?;
    manifest.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.2.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;
    drop(guard);
    finish_command(command, stderr).await?;
    let lock: toml::Value =
        toml::from_str(&fs_err::read_to_string(context.temp_dir.join("uv.lock"))?)?;
    assert_eq!(lock["package"][0]["name"].as_str(), Some("project"));
    assert_eq!(lock["package"][0]["version"].as_str(), Some("0.2.0"));
    Ok(())
}

#[test]
fn initialization_reports_complete_workspace_deprecations_once() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "parent"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        dev-dependencies = []

        [tool.uv.workspace]
        members = ["child"]
    "#})?;
    let child = context.temp_dir.child("child");
    child.create_dir_all()?;
    child.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "child"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv]
        dev-dependencies = []
    "#})?;

    uv_snapshot!(context.filters(), context.init().args([
        "new", "--bare", "--vcs", "none", "--author-from", "none",
        "--no-pin-python", "--python", "3.12",
    ]), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: The `tool.uv.dev-dependencies` field (used in `child/pyproject.toml`, `pyproject.toml`) is deprecated and will be removed in a future release; use `dependency-groups.dev` instead
    Adding `new` as member of workspace `[TEMP_DIR]/`
    Initialized project `new` at `[TEMP_DIR]/new`
    ");
    Ok(())
}

#[tokio::test]
async fn queued_initializations_preserve_parent_workspace_members() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r"
        [tool.uv.workspace]
        members = []
    "})?;
    let guard = hold_metadata(context.temp_dir.path(), "workspace").await?;
    let mut first = context.init();
    first.args([
        "first",
        "--bare",
        "--vcs",
        "none",
        "--author-from",
        "none",
        "--no-pin-python",
        "--python",
        "3.12",
    ]);
    let (first, first_stderr) = queued_command(first).await?;
    let mut second = context.init();
    second.args([
        "second",
        "--bare",
        "--vcs",
        "none",
        "--author-from",
        "none",
        "--no-pin-python",
        "--python",
        "3.12",
    ]);
    let (second, second_stderr) = queued_command(second).await?;
    drop(guard);
    finish_command(first, first_stderr).await?;
    finish_command(second, second_stderr).await?;
    let project: toml::Value = toml::from_str(&fs_err::read_to_string(
        context.temp_dir.join("pyproject.toml"),
    )?)?;
    let mut members = project["tool"]["uv"]["workspace"]["members"]
        .as_array()
        .context("workspace members")?
        .iter()
        .map(|member| member.as_str().context("member path"))
        .collect::<Result<Vec<_>>>()?;
    members.sort_unstable();
    assert_eq!(members, ["first", "second"]);
    assert!(context.temp_dir.child("first/pyproject.toml").is_file());
    assert!(context.temp_dir.child("second/pyproject.toml").is_file());
    Ok(())
}

#[tokio::test]
async fn run_releases_metadata_before_starting_the_user_program() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;
    let mut command = context.run();
    command.args([
        "--offline",
        "python",
        "-c",
        "import sys; print('ready', flush=True); sys.stdin.read(1)",
    ]);
    let mut child = tokio::process::Command::from(command)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdout =
        tokio::io::BufReader::new(child.stdout.take().context("captured program stdout")?);
    let mut ready = String::new();
    tokio::time::timeout(Duration::from_secs(30), stdout.read_line(&mut ready)).await??;
    assert_eq!(ready.trim(), "ready");

    let mut edit = context.add();
    edit.args(["alpha==1", "--frozen"]);
    let update = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::process::Command::from(edit)
            .kill_on_drop(true)
            .output(),
    )
    .await;
    child
        .stdin
        .as_mut()
        .context("captured program stdin")?
        .write_all(b"S")
        .await?;
    child.wait_with_output().await?.assert().success();
    update??.assert().success();
    insta::assert_debug_snapshot!(dependencies(&context)?, @r###"
    [
        "alpha==1",
    ]
    "###);
    Ok(())
}

#[tokio::test]
async fn unrelated_workspaces_publish_independently() -> Result<()> {
    let owner = uv_test::test_context!("3.12");
    let _guard = hold_metadata(owner.temp_dir.path(), "workspace").await?;
    let other = uv_test::test_context!("3.12");
    other
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "other"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;
    let mut command = other.lock();
    command.arg("--offline");
    tokio::time::timeout(
        Duration::from_secs(30),
        tokio::process::Command::from(command)
            .kill_on_drop(true)
            .output(),
    )
    .await??
    .assert()
    .success();
    assert!(other.temp_dir.child("uv.lock").is_file());
    Ok(())
}

#[tokio::test]
async fn queued_package_edit_keeps_its_workspace_selection() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let root = context.temp_dir.child("pyproject.toml");
    root.write_str(indoc! {r#"
        [project]
        name = "root"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [tool.uv.workspace]
        members = ["member"]
    "#})?;
    let member = context.temp_dir.child("member/pyproject.toml");
    member.write_str(indoc! {r#"
        [project]
        name = "member"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;
    let original = fs_err::read(member.path())?;
    let guard = hold_metadata(context.temp_dir.path(), "workspace").await?;
    let mut command = context.add();
    command.args(["alpha==1", "--package", "member", "--frozen"]);
    let (command, stderr) = queued_command(command).await?;
    root.write_str(indoc! {r#"
        [project]
        name = "root"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;
    drop(guard);
    let mut output =
        tokio::time::timeout(Duration::from_secs(30), command.wait_with_output()).await??;
    output.stderr = stderr.await??;
    output.assert().failure().stderr(predicates::str::contains(
        "The workspace does not have a member member",
    ));
    assert_eq!(fs_err::read(member.path())?, original);
    Ok(())
}

#[tokio::test]
async fn queued_script_initialization_reloads_existing_contents() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let script = context.temp_dir.child("script.py");
    script.write_str("print('original')\n")?;
    let guard = hold_metadata(script.path(), "script").await?;
    let mut command = context.init();
    command.args(["--script", "script.py", "--python", "3.12"]);
    let (command, stderr) = queued_command(command).await?;
    script.write_str("print('committed while waiting')\n")?;
    drop(guard);
    finish_command(command, stderr).await?;
    insta::assert_snapshot!(fs_err::read_to_string(script.path())?, @r###"
    # /// script
    # requires-python = ">=3.12"
    # dependencies = []
    # ///

    print('committed while waiting')
    "###);
    Ok(())
}

#[tokio::test]
async fn queued_script_initialization_preserves_competing_creator() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let script = context.temp_dir.child("script.py");
    let destination = fs_err::canonicalize(context.temp_dir.path())?.join("script.py");
    let guard = LockedFile::acquire(
        std::env::temp_dir().join(format!(
            "uv-script-metadata-{}.lock",
            cache_digest(&destination)
        )),
        LockedFileMode::Exclusive,
        "script destination",
    )
    .await?;
    let mut command = context.init();
    command.args(["--script", "script.py", "--python", "3.12"]);
    let (command, stderr) = queued_command(command).await?;
    let committed = indoc! {r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = ["retained==1"]
        # ///
        print('created while waiting')
    "#};
    script.write_str(committed)?;
    drop(guard);
    let mut output =
        tokio::time::timeout(Duration::from_secs(30), command.wait_with_output()).await??;
    output.stderr = stderr.await??;
    output
        .assert()
        .failure()
        .stderr(predicates::str::contains("is already a PEP 723 script"));
    assert_eq!(fs_err::read_to_string(script.path())?, committed);
    Ok(())
}

#[tokio::test]
async fn queued_add_reads_updated_default_index() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let old_index = PackseServer::empty();
    let current_index = PackseServer::new("simple/single-package.toml");
    let manifest = context.temp_dir.child("pyproject.toml");
    manifest.write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [[tool.uv.index]]
        url = "{}"
        default = true
    "#, old_index.index_url()})?;
    let guard = hold_metadata(context.temp_dir.path(), "workspace").await?;
    let mut command = context.add();
    command.args(["a==1.0.0", "--no-sync"]);
    let (child, stderr) = queued_command(command).await?;

    manifest.write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [[tool.uv.index]]
        url = "{}"
        default = true
    "#, current_index.index_url()})?;
    drop(guard);
    finish_command(child, stderr).await?;
    insta::assert_debug_snapshot!(dependencies(&context)?, @r#"
    [
        "a==1.0.0",
    ]
    "#);
    let lock: toml::Value =
        toml::from_str(&fs_err::read_to_string(context.temp_dir.join("uv.lock"))?)?;
    assert_eq!(
        lock["package"][0]["source"]["registry"]
            .as_str()
            .map(|url| url.trim_end_matches('/')),
        Some(current_index.index_url().trim_end_matches('/'))
    );
    Ok(())
}

#[tokio::test]
async fn queued_add_keeps_cli_index_override() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let cli_index = PackseServer::new("simple/single-package.toml");
    let other_index = PackseServer::empty();
    let manifest = context.temp_dir.child("pyproject.toml");
    manifest.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;
    let guard = hold_metadata(context.temp_dir.path(), "workspace").await?;
    let mut command = context.add();
    command
        .args([
            "a==1.0.0",
            "--no-sync",
            "--default-index",
            &cli_index.index_url(),
        ])
        .env(EnvVars::UV_DEFAULT_INDEX, other_index.index_url());
    let (child, stderr) = queued_command(command).await?;

    manifest.write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [[tool.uv.index]]
        url = "{}"
        default = true
    "#, other_index.index_url()})?;
    drop(guard);
    finish_command(child, stderr).await?;
    insta::assert_debug_snapshot!(dependencies(&context)?, @r#"
    [
        "a==1.0.0",
    ]
    "#);
    Ok(())
}

#[tokio::test]
async fn queued_add_keeps_environment_index_override() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let environment_index = PackseServer::new("simple/single-package.toml");
    let filesystem_index = PackseServer::empty();
    let manifest = context.temp_dir.child("pyproject.toml");
    manifest.write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;
    let guard = hold_metadata(context.temp_dir.path(), "workspace").await?;
    let mut command = context.add();
    command
        .args(["a==1.0.0", "--no-sync"])
        .env(EnvVars::UV_DEFAULT_INDEX, environment_index.index_url());
    let (child, stderr) = queued_command(command).await?;

    manifest.write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []

        [[tool.uv.index]]
        url = "{}"
        default = true
    "#, filesystem_index.index_url()})?;
    drop(guard);
    finish_command(child, stderr).await?;
    insta::assert_debug_snapshot!(dependencies(&context)?, @r#"
    [
        "a==1.0.0",
    ]
    "#);
    Ok(())
}

#[tokio::test]
async fn queued_script_add_reads_updated_default_index() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let old_index = PackseServer::empty();
    let current_index = PackseServer::new("simple/single-package.toml");
    let script = context.temp_dir.child("main.py");
    script.write_str(&formatdoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = []
        # [[tool.uv.index]]
        # url = "{}"
        # default = true
        # ///
        print("script body")
    "#, old_index.index_url()})?;
    let guard = hold_metadata(script.path(), "script").await?;
    let mut command = context.add();
    command.args(["--script", "main.py", "a==1.0.0"]);
    let (child, stderr) = queued_command(command).await?;

    script.write_str(&formatdoc! {r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = []
        # [[tool.uv.index]]
        # url = "{}"
        # default = true
        # ///
        print("updated script body")
    "#, current_index.index_url()})?;
    drop(guard);
    finish_command(child, stderr).await?;
    let contents = fs_err::read_to_string(script.path())?;
    let index_filter = regex::escape(&current_index.index_url());
    insta::with_settings!({filters => [(index_filter.as_str(), "[INDEX]")]}, {
        insta::assert_snapshot!(contents, @r#"
        # /// script
        # requires-python = ">=3.12"
        # dependencies = [
        #     "a==1.0.0",
        # ]
        # [[tool.uv.index]]
        # url = "[INDEX]"
        # default = true
        # ///
        print("updated script body")
        "#);
    });
    Ok(())
}

#[tokio::test]
async fn sync_dry_run_does_not_wait_for_metadata_writers() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = []
    "#})?;
    context.lock().arg("--offline").assert().success();
    let before = fs_err::read(context.temp_dir.join("uv.lock"))?;
    let _guard = hold_metadata(context.temp_dir.path(), "workspace").await?;
    let mut command = context.sync();
    command
        .args(["--dry-run", "--offline"])
        .env(EnvVars::UV_LOCK_TIMEOUT, "1");
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::process::Command::from(command)
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    output.assert().success();
    assert_eq!(fs_err::read(context.temp_dir.join("uv.lock"))?, before);
    Ok(())
}
