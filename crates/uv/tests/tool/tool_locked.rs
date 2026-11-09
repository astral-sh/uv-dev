use std::env::consts::EXE_SUFFIX;
use std::process::Command;

use anyhow::Result;
#[cfg(feature = "test-git")]
use anyhow::anyhow;
#[cfg(feature = "test-git")]
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::formatdoc;
use insta::assert_snapshot;
use predicates::prelude::predicate;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[cfg(feature = "test-git")]
use uv_redacted::SafeUrl;
use uv_static::EnvVars;
use uv_test::package_server::PackageServer;
use uv_test::packse::WheelBuilder;
use uv_test::uv_snapshot;

const TOOL_ENTRY_POINTS: &str = "[console_scripts]\nlocked-tool = locked_tool.cli:main\n";
const TOOL_CLI: (&str, &str) = (
    "locked_tool/cli.py",
    "from importlib.metadata import version\ndef main(): print(version('idna'))\n",
);

fn lock_entry(
    name: &str,
    version: &str,
    index: &str,
    filename: &str,
    url: &str,
    hash: &str,
) -> String {
    formatdoc! {r#"
        [[packages]]
        name = "{name}"
        version = "{version}"
        index = "{index}"
        wheels = [
            {{ name = "{filename}", url = "{url}", hashes = {{ sha256 = "{hash}" }} }},
        ]
    "#}
}

const LOCK_HEADER: &str = "lock-version = \"1.0\"\ncreated-by = \"test\"\n";

async fn serve_wheel(
    server: &MockServer,
    filename: &str,
    bytes: Vec<u8>,
) -> (String, String, Value) {
    let url = format!("{}/files/{filename}", server.uri());
    let hash = hex::encode(Sha256::digest(&bytes));
    Mock::given(method("GET"))
        .and(path(format!("/files/{filename}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
        .mount(server)
        .await;
    let file = json!({
        "filename": filename,
        "url": url,
        "hashes": { "sha256": hash },
        "upload-time": "2024-01-01T00:00:00Z",
    });
    (url, hash, file)
}

async fn serve_index(server: &MockServer, package: &str, files: Vec<Value>) {
    Mock::given(method("GET"))
        .and(path(format!("/simple/{package}/")))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                json!({ "meta": { "api-version": "1.0" }, "name": package, "files": files })
                    .to_string(),
                "application/vnd.pypi.simple.v1+json",
            ),
        )
        .mount(server)
        .await;
}

/// A packaged pin selects an older compatible dependency and records the locked installation.
#[tokio::test]
async fn packaged_lock_install() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let wheels = context.temp_dir.child("wheels");
    let bin_dir = context.temp_dir.child("bin");
    let server = MockServer::start().await;
    let index = format!("{}/simple", server.uri());
    let (locked_filename, locked_wheel) =
        WheelBuilder::new(&"idna".parse()?, &"3.3".parse()?).build();
    let (locked_url, locked_hash, locked_file) =
        serve_wheel(&server, &locked_filename, locked_wheel).await;
    let (newer_filename, newer_wheel) =
        WheelBuilder::new(&"idna".parse()?, &"3.4".parse()?).build();
    let (_, _, newer_file) = serve_wheel(&server, &newer_filename, newer_wheel).await;
    serve_index(&server, "idna", vec![locked_file, newer_file]).await;

    let entry = lock_entry(
        "idna",
        "3.3",
        &index,
        &locked_filename,
        &locked_url,
        &locked_hash,
    );
    let packaged_lock = format!("{LOCK_HEADER}{entry}");
    let (tool_filename, tool_wheel) = WheelBuilder::new(&"locked-tool".parse()?, &"1.0.0".parse()?)
        .requires(&["idna>=3.3,<3.5".parse()?])
        .files(&[
            (
                "locked_tool-1.0.0.dist-info/entry_points.txt",
                TOOL_ENTRY_POINTS,
            ),
            TOOL_CLI,
            ("locked_tool-1.0.0.dist-info/pylock.toml", &packaged_lock),
        ])
        .build();
    wheels.child(tool_filename).write_binary(&tool_wheel)?;

    uv_snapshot!(context.filters(), context.tool_install()
        .args(["--locked", "--preview-features", "locked-tools", "--default-index", &index,
               "--find-links", "wheels", "locked-tool"])
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed [N] packages in [TIME]
     + idna==3.3
     + locked-tool==1.0.0
    Installed 1 executable: locked-tool
    ");

    insta::with_settings!({ filters => context.filters() }, {
        assert_snapshot!(context.read("tools/locked-tool/uv-receipt.toml"), @r#"
        [tool]
        locked = true
        requirements = [{ name = "locked-tool" }]
        entrypoints = [
            { name = "locked-tool", install-path = "[TEMP_DIR]/bin/locked-tool", from = "locked-tool" },
        ]

        [tool.options]
        index = [{ url = "http://[LOCALHOST]/simple", explicit = false, default = true, format = "simple", authenticate = "auto" }]
        find-links = ["file://[TEMP_DIR]/wheels"]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    Ok(())
}

/// Refine from wheel metadata within the requested Python range when flat index entries
/// omit Requires-Python.
#[test]
fn packaged_lock_refines_interpreter_from_wheel_metadata() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.14", "3.13"])
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let wheels = context.temp_dir.child("wheels");
    let bin_dir = context.temp_dir.child("bin");
    let packaged_lock = format!("{LOCK_HEADER}packages = []\n");
    let (tool_filename, tool_wheel) = WheelBuilder::new(&"locked-tool".parse()?, &"1.0.0".parse()?)
        .requires_python(Some(&">=3.13".parse()?))
        .files(&[
            (
                "locked_tool-1.0.0.dist-info/entry_points.txt",
                TOOL_ENTRY_POINTS,
            ),
            (
                "locked_tool/cli.py",
                "import sys\ndef main(): print(f'{sys.version_info.major}.{sys.version_info.minor}')\n",
            ),
            ("locked_tool-1.0.0.dist-info/pylock.toml", &packaged_lock),
        ])
        .build();
    wheels.child(tool_filename).write_binary(&tool_wheel)?;

    uv_snapshot!(context.filters(), context.tool_install()
        .args(["--locked", "--preview-features", "locked-tools", "--no-index",
               "--find-links", "wheels", "--python", ">=3.12,<3.14", "locked-tool"])
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed [N] packages in [TIME]
     + locked-tool==1.0.0
    Installed 1 executable: locked-tool
    ");

    uv_snapshot!(context.filters(), Command::new(bin_dir.join(format!("locked-tool{EXE_SUFFIX}"))), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.13
    ");

    Ok(())
}

#[test]
fn packaged_lock_required() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let wheels = context.temp_dir.child("wheels");
    let packaged_lock = format!("{LOCK_HEADER}packages = []\n");
    let (tool_filename, tool_wheel) = WheelBuilder::new(&"locked-tool".parse()?, &"1.0.0".parse()?)
        .requires(&["idna>=3.3,<3.5".parse()?])
        .files(&[
            (
                "locked_tool-1.0.0.dist-info/entry_points.txt",
                TOOL_ENTRY_POINTS,
            ),
            TOOL_CLI,
            ("locked_tool-1.0.0.dist-info/pylock.toml", &packaged_lock),
        ])
        .build();
    wheels.child(tool_filename).write_binary(&tool_wheel)?;
    let (tool_filename, tool_wheel) = WheelBuilder::new(&"locked-tool".parse()?, &"2.0.0".parse()?)
        .requires(&["idna>=3.3,<3.5".parse()?])
        .files(&[
            (
                "locked_tool-2.0.0.dist-info/entry_points.txt",
                TOOL_ENTRY_POINTS,
            ),
            TOOL_CLI,
        ])
        .build();
    wheels.child(tool_filename).write_binary(&tool_wheel)?;

    uv_snapshot!(context.filters(), context.tool_install()
        .args(["--locked", "--preview-features", "locked-tools", "--no-index",
               "--find-links", "wheels", "locked-tool"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `locked-tool==2.0.0` does not contain `pylock.toml` in its `.dist-info` directory; `--locked` requires a packaged lock
    ");

    Ok(())
}

/// A URL with the right name and digest still requires authorization from the configured index.
#[tokio::test]
async fn packaged_lock_rejects_unlisted_artifact() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let wheels = context.temp_dir.child("wheels");
    let server = PackageServer::new(&"idna".parse()?).await;
    let (filename, wheel) = WheelBuilder::new(&"idna".parse()?, &"3.3".parse()?).build();
    let hash = hex::encode(Sha256::digest(&wheel));
    server.serve(&filename, &wheel, Some(&hash)).await;

    let other_server = MockServer::start().await;
    let path_to_wheel = format!("/{filename}");
    let forged_url = format!("{}{path_to_wheel}", other_server.uri());
    Mock::given(path(path_to_wheel))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(wheel))
        .expect(0)
        .mount(&other_server)
        .await;

    let index = server.index_url();
    let entry = lock_entry("idna", "3.3", &index, &filename, &forged_url, &hash);
    let packaged_lock = format!("{LOCK_HEADER}{entry}");
    let (tool_filename, tool_wheel) = WheelBuilder::new(&"locked-tool".parse()?, &"1.0.0".parse()?)
        .requires(&["idna>=3.3,<3.5".parse()?])
        .files(&[
            (
                "locked_tool-1.0.0.dist-info/entry_points.txt",
                TOOL_ENTRY_POINTS,
            ),
            TOOL_CLI,
            ("locked_tool-1.0.0.dist-info/pylock.toml", &packaged_lock),
        ])
        .build();
    wheels.child(tool_filename).write_binary(&tool_wheel)?;

    uv_snapshot!(context.filters(), context.tool_install()
        .args(["--locked", "--preview-features", "locked-tools", "--default-index", &index,
               "--find-links", "wheels", "locked-tool"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The packaged lock for `locked-tool==1.0.0` contains unverified artifacts
      cause: URL for `idna==3.3` is not listed by http://[LOCALHOST]/simple: http://[LOCALHOST]/idna-3.3-py3-none-any.whl
    ");

    Ok(())
}

#[tokio::test]
async fn packaged_lock_checks_index_hash() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let wheels = context.temp_dir.child("wheels");
    let server = PackageServer::new(&"idna".parse()?).await;
    let (filename, wheel) = WheelBuilder::new(&"idna".parse()?, &"3.3".parse()?).build();
    let hash = hex::encode(Sha256::digest(&wheel));
    server.serve(&filename, &wheel, Some(&"0".repeat(64))).await;
    let index = server.index_url();
    let entry = lock_entry(
        "idna",
        "3.3",
        &index,
        &filename,
        &server.file_url(&filename),
        &hash,
    );
    let packaged_lock = format!("{LOCK_HEADER}{entry}");
    let (tool_filename, tool_wheel) = WheelBuilder::new(&"locked-tool".parse()?, &"1.0.0".parse()?)
        .requires(&["idna>=3.3,<3.5".parse()?])
        .files(&[
            (
                "locked_tool-1.0.0.dist-info/entry_points.txt",
                TOOL_ENTRY_POINTS,
            ),
            TOOL_CLI,
            ("locked_tool-1.0.0.dist-info/pylock.toml", &packaged_lock),
        ])
        .build();
    wheels.child(tool_filename).write_binary(&tool_wheel)?;
    let context = context.with_filter((hash, "[LOCK_HASH]"));

    uv_snapshot!(context.filters(), context.tool_install()
        .args(["--locked", "--preview-features", "locked-tools", "--default-index", &index,
               "--find-links", "wheels", "locked-tool"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Hash mismatch for `idna==3.3`

    Expected:
      sha256:0000000000000000000000000000000000000000000000000000000000000000
      sha256:[LOCK_HASH]

    Computed:
      sha256:[LOCK_HASH]
    ");

    Ok(())
}

#[tokio::test]
async fn packaged_lock_rejects_incompatible_constraint() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let wheels = context.temp_dir.child("wheels");
    let server = PackageServer::new(&"idna".parse()?).await;
    let (filename, wheel) = WheelBuilder::new(&"idna".parse()?, &"3.3".parse()?).build();
    let hash = hex::encode(Sha256::digest(&wheel));
    server.serve(&filename, &wheel, Some(&hash)).await;
    let index = server.index_url();
    let entry = lock_entry(
        "idna",
        "3.3",
        &index,
        &filename,
        &server.file_url(&filename),
        &hash,
    );
    let packaged_lock = format!("{LOCK_HEADER}{entry}");
    let (tool_filename, tool_wheel) = WheelBuilder::new(&"locked-tool".parse()?, &"1.0.0".parse()?)
        .requires(&["idna>=3.3,<3.5".parse()?])
        .files(&[
            (
                "locked_tool-1.0.0.dist-info/entry_points.txt",
                TOOL_ENTRY_POINTS,
            ),
            TOOL_CLI,
            ("locked_tool-1.0.0.dist-info/pylock.toml", &packaged_lock),
        ])
        .build();
    wheels.child(tool_filename).write_binary(&tool_wheel)?;
    context
        .temp_dir
        .child("constraints.txt")
        .write_str("idna>=3.4\n")?;

    uv_snapshot!(context.filters(), context.tool_install()
        .args(["--locked", "--preview-features", "locked-tools", "--default-index", &index,
               "--find-links", "wheels", "--constraint", "constraints.txt", "locked-tool"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The packaged lock selects `idna==3.3`, which is incompatible with constraint `idna>=3.4`
    ");

    Ok(())
}

#[tokio::test]
async fn packaged_lock_overrides_and_excludes() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let wheels = context.temp_dir.child("wheels");
    let bin_dir = context.temp_dir.child("bin");
    let server = MockServer::start().await;
    let index = format!("{}/simple", server.uri());
    let (override_name, override_wheel) = WheelBuilder::new(&"idna".parse()?, &"3.5".parse()?)
        .requires(&["beta>=1".parse()?, "gamma>=1".parse()?])
        .build();
    let (_, override_hash, override_file) =
        serve_wheel(&server, &override_name, override_wheel).await;
    serve_index(&server, "idna", vec![override_file]).await;
    let (beta_name, beta_wheel) = WheelBuilder::new(&"beta".parse()?, &"1.0".parse()?).build();
    let (beta_url, beta_hash, beta_file) = serve_wheel(&server, &beta_name, beta_wheel).await;
    serve_index(&server, "beta", vec![beta_file]).await;

    let idna_entry = lock_entry(
        "idna",
        "3.3",
        &index,
        "idna-3.3-py3-none-any.whl",
        &format!("{}/unavailable/idna-3.3-py3-none-any.whl", server.uri()),
        &"0".repeat(64),
    );
    let beta_entry = lock_entry("beta", "1.0", &index, &beta_name, &beta_url, &beta_hash);
    let packaged_lock = format!("{LOCK_HEADER}{idna_entry}{beta_entry}");
    let (tool_filename, tool_wheel) = WheelBuilder::new(&"locked-tool".parse()?, &"1.0.0".parse()?)
        .requires(&["idna>=3.3,<3.5".parse()?])
        .files(&[
            (
                "locked_tool-1.0.0.dist-info/entry_points.txt",
                TOOL_ENTRY_POINTS,
            ),
            TOOL_CLI,
            ("locked_tool-1.0.0.dist-info/pylock.toml", &packaged_lock),
        ])
        .build();
    wheels.child(tool_filename).write_binary(&tool_wheel)?;
    context
        .temp_dir
        .child("overrides.txt")
        .write_str(&format!("idna==3.5 --hash=sha256:{override_hash}\n"))?;
    context
        .temp_dir
        .child("constraints.txt")
        .write_str(&format!("beta @ {beta_url}#sha256={beta_hash}\n"))?;
    context
        .temp_dir
        .child("excludes.txt")
        .write_str("gamma\n")?;

    uv_snapshot!(context.filters(), context.tool_install()
        .args(["--locked", "--preview-features", "locked-tools", "--default-index", &index,
               "--find-links", "wheels", "--override", "overrides.txt",
               "--constraints", "constraints.txt", "--exclude", "excludes.txt", "locked-tool"])
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed [N] packages in [TIME]
     + beta==1.0
     + idna==3.5
     + locked-tool==1.0.0
    Installed 1 executable: locked-tool
    ");

    Ok(())
}

/// A dependency missing after an override is an ordinary resolution failure.
#[tokio::test]
async fn packaged_lock_reports_unsatisfiable_override() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let wheels = context.temp_dir.child("wheels");
    let server = PackageServer::new(&"idna".parse()?).await;
    let (filename, wheel) = WheelBuilder::new(&"idna".parse()?, &"3.3".parse()?).build();
    let hash = hex::encode(Sha256::digest(&wheel));
    server.serve(&filename, &wheel, Some(&hash)).await;
    let index = server.index_url();
    let entry = lock_entry(
        "idna",
        "3.3",
        &index,
        &filename,
        &server.file_url(&filename),
        &hash,
    );
    let packaged_lock = format!("{LOCK_HEADER}{entry}");
    let (tool_filename, tool_wheel) = WheelBuilder::new(&"locked-tool".parse()?, &"1.0.0".parse()?)
        .requires(&["idna>=3.3,<3.5".parse()?])
        .files(&[
            (
                "locked_tool-1.0.0.dist-info/entry_points.txt",
                TOOL_ENTRY_POINTS,
            ),
            TOOL_CLI,
            ("locked_tool-1.0.0.dist-info/pylock.toml", &packaged_lock),
        ])
        .build();
    wheels.child(tool_filename).write_binary(&tool_wheel)?;
    context
        .temp_dir
        .child("overrides.txt")
        .write_str("idna==3.5\n")?;

    uv_snapshot!(context.filters(), context.tool_install()
        .args(["--locked", "--preview-features", "locked-tools", "--default-index", &index,
               "--find-links", "wheels", "--override", "overrides.txt", "locked-tool"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because there is no version of idna==3.5 and you require idna==3.5, we can conclude that your requirements are unsatisfiable.
    ");

    Ok(())
}

/// A user override can select a Git wheel without supplying an archive hash.
#[test]
#[cfg(feature = "test-git")]
fn packaged_lock_accepts_unhashed_git_wheel_override() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filter((r"@[0-9a-f]{40}", "@[COMMIT]"))
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let wheels = context.temp_dir.child("wheels");
    let repository = context.temp_dir.child("repository");
    let (filename, wheel) = WheelBuilder::new(&"idna".parse()?, &"3.5".parse()?).build();
    repository.child(&filename).write_binary(&wheel)?;

    Command::new("git")
        .current_dir(repository.path())
        .arg("init")
        .assert()
        .success();
    Command::new("git")
        .current_dir(repository.path())
        .args(["add", &filename])
        .assert()
        .success();
    Command::new("git")
        .current_dir(repository.path())
        .args([
            "-c",
            "user.name=Example",
            "-c",
            "user.email=example@example.com",
            "commit",
            "-m",
            "Add wheel",
        ])
        .assert()
        .success();
    let output = Command::new("git")
        .current_dir(repository.path())
        .args(["rev-parse", "HEAD"])
        .output()?;
    assert!(output.status.success());
    let commit = String::from_utf8(output.stdout)?;
    let commit = commit.trim();
    let repository_url = SafeUrl::from_directory_path(repository.path())
        .map_err(|()| anyhow!("failed to convert repository path to a file URL"))?;
    context.temp_dir.child("overrides.txt").write_str(&format!(
        "idna @ git+{repository_url}@{commit}#path={filename}\n",
    ))?;
    let packaged_lock = format!("{LOCK_HEADER}packages = []\n");
    let (tool_filename, tool_wheel) = WheelBuilder::new(&"locked-tool".parse()?, &"1.0.0".parse()?)
        .requires(&["idna>=3.3,<3.5".parse()?])
        .files(&[
            (
                "locked_tool-1.0.0.dist-info/entry_points.txt",
                TOOL_ENTRY_POINTS,
            ),
            TOOL_CLI,
            ("locked_tool-1.0.0.dist-info/pylock.toml", &packaged_lock),
        ])
        .build();
    wheels.child(tool_filename).write_binary(&tool_wheel)?;

    let bin_dir = context.temp_dir.child("bin");
    let system_path = std::env::var_os(EnvVars::PATH).unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(bin_dir.to_path_buf()).chain(std::env::split_paths(&system_path)),
    )?;
    uv_snapshot!(context.filters(), context.tool_install()
        .args(["--locked", "--preview-features", "locked-tools", "--no-index",
               "--find-links", "wheels", "--override", "overrides.txt", "locked-tool"])
        .env(EnvVars::PATH, path), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + idna==3.5 (from git+file://[TEMP_DIR]/repository/@[COMMIT]#path=idna-3.5-py3-none-any.whl)
     + locked-tool==1.0.0
    Installed 1 executable: locked-tool
    ");

    Ok(())
}

/// A replacement wheel with an invalid console script fails during synchronization.
#[test]
fn packaged_lock_removes_launchers_after_failed_replacement() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let wheels = context.temp_dir.child("wheels");
    let bin_dir = context.temp_dir.child("bin");
    let executable = bin_dir.child(format!("locked-tool{}", std::env::consts::EXE_SUFFIX));

    let (filename, wheel) = WheelBuilder::new(&"locked-tool".parse()?, &"1.0.0".parse()?)
        .entry_points(&["locked-tool".to_string()])
        .build();
    wheels.child(filename).write_binary(&wheel)?;

    let packaged_lock = format!("{LOCK_HEADER}packages = []\n");
    let (filename, wheel) = WheelBuilder::new(&"locked-tool".parse()?, &"2.0.0".parse()?)
        .files(&[
            ("locked_tool-2.0.0.dist-info/pylock.toml", &packaged_lock),
            (
                "locked_tool-2.0.0.dist-info/entry_points.txt",
                "[console_scripts]\nlocked-tool = invalid\n",
            ),
        ])
        .build();
    wheels.child(filename).write_binary(&wheel)?;

    uv_snapshot!(context.filters(), context.tool_install()
        .args(["--no-preview", "--no-index", "--find-links", "wheels", "locked-tool==1.0.0"])
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + locked-tool==1.0.0
    Installed 1 executable: locked-tool
    ");
    executable.assert(predicate::path::exists());

    uv_snapshot!(context.filters(), context.tool_install()
        .args(["--force", "--locked", "--preview-features", "locked-tools",
               "--no-index", "--find-links", "wheels", "locked-tool==2.0.0"])
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to install: locked_tool-2.0.0-py3-none-any.whl (locked-tool==2.0.0)
      cause: The wheel is invalid: invalid console script: 'invalid'
    ");
    context
        .temp_dir
        .child("tools/locked-tool")
        .assert(predicate::path::missing());
    assert_eq!(
        fs_err::symlink_metadata(executable.path())
            .err()
            .map(|err| err.kind()),
        Some(std::io::ErrorKind::NotFound),
    );

    Ok(())
}

#[test]
fn packaged_lock_reports_malformed_artifact_filename() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_tool_dirs();
    let wheels = context.temp_dir.child("wheels");
    let hash = "0".repeat(64);
    let lock = formatdoc! {r#"
        {LOCK_HEADER}
        [[packages]]
        name = "idna"
        version = "3.3"
        wheels = [
            {{ url = "https://example.invalid/invalid.whl", hashes = {{ sha256 = "{hash}" }} }},
        ]
    "#};
    let (tool_filename, tool_wheel) = WheelBuilder::new(&"locked-tool".parse()?, &"1.0.0".parse()?)
        .requires(&["idna>=3.3,<3.5".parse()?])
        .files(&[
            (
                "locked_tool-1.0.0.dist-info/entry_points.txt",
                TOOL_ENTRY_POINTS,
            ),
            TOOL_CLI,
            ("locked_tool-1.0.0.dist-info/pylock.toml", &lock),
        ])
        .build();
    wheels.child(tool_filename).write_binary(&tool_wheel)?;

    uv_snapshot!(context.filters(), context.tool_install()
        .args(["--locked", "--preview-features", "locked-tools", "--no-index",
               "--find-links", "wheels", "locked-tool"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The packaged lock for `locked-tool==1.0.0` contains unverified artifacts
      cause: The wheel filename `invalid.whl` is invalid: Must have a version
    ");

    Ok(())
}
