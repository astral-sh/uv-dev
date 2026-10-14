use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use fs_err::{read_to_string, remove_file, write};
#[cfg(feature = "test-universal")]
use indoc::formatdoc;
use indoc::indoc;

use uv_cache::CacheBucket;
use uv_redacted::SafeUrl;
use uv_test::{TestContext, uv_snapshot};

fn git_command(context: &TestContext, repository: &Path) -> Command {
    let mut command = context.external_command("git");
    command.arg("-C").arg(repository);
    command
}

/// Checkout readiness must not overwrite a repository's own `.ok` file.
#[test]
fn git_source_checkout_marker() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filter((r"@[0-9a-f]{40}", "@[COMMIT]"));
    let repository = context.temp_dir.child("repository");
    repository.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "example"
        version = "0.1.0"
    "#})?;
    repository.child(".ok").write_str("repository content\n")?;
    let git = || git_command(&context, repository.path());
    git().arg("init").assert().success();
    git()
        .args(["add", "--force", "pyproject.toml", ".ok"])
        .assert()
        .success();
    git()
        .args([
            "-c",
            "user.name=ferris",
            "-c",
            "user.email=ferris@example.com",
            "commit",
            "--no-gpg-sign",
            "-m",
            "Initial commit",
        ])
        .assert()
        .success();

    let repository_url = SafeUrl::from_directory_path(repository.path())
        .map_err(|()| anyhow!("failed to convert repository path to file URL"))?;
    let repository_url = repository_url.as_str().trim_end_matches('/');
    context
        .temp_dir
        .child("requirements.in")
        .write_str(&format!("example @ git+{repository_url}"))?;
    uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in").arg("--no-header").arg("--no-annotate"), @"
    exit_code: 0 (success)
    ----- stdout -----
    example @ git+file://[TEMP_DIR]/repository@[COMMIT]

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");

    let tracked_file = context
        .cache_files(CacheBucket::Git)?
        .into_iter()
        .find(|path| path.file_name().is_some_and(|name| name == ".ok"))
        .context("missing tracked .ok file")?;
    assert_eq!(read_to_string(&tracked_file)?, "repository content\n");
    let checkout = tracked_file
        .parent()
        .context("missing checkout directory")?;
    let marker = checkout.with_extension("ok");
    assert!(marker.is_file());

    let sentinel = checkout.join("sentinel");
    write(&sentinel, "")?;
    let mut compile = context.pip_compile();
    compile.arg("requirements.in").arg("--refresh");
    compile.assert().success();
    assert!(sentinel.exists());

    // An incomplete checkout must be recreated even when it contains a tracked `.ok` file.
    remove_file(&marker)?;
    compile.assert().success();
    assert!(!sentinel.exists());
    assert!(marker.is_file());
    assert_eq!(read_to_string(&tracked_file)?, "repository content\n");
    Ok(())
}

/// A pinned Git revision in `pylock.toml` is reused until an upgrade is requested.
/// See astral-sh/uv#18224.
#[cfg(feature = "test-universal")]
#[test]
fn pep_751_compile_git_preferences() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let repository = context.temp_dir.child("repository");
    repository.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "example"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;

    let git = || git_command(&context, repository.path());
    git().arg("init").assert().success();
    git()
        .args(["add", "--force", "pyproject.toml"])
        .assert()
        .success();
    let commit = |message: &str| -> Result<String> {
        git()
            .args([
                "-c",
                "user.name=ferris",
                "-c",
                "user.email=ferris@example.com",
                "commit",
                "--no-gpg-sign",
                "--allow-empty",
                "-m",
                message,
            ])
            .assert()
            .success();
        let output = git().args(["rev-parse", "HEAD"]).assert().success();
        Ok(String::from_utf8(output.get_output().stdout.clone())?
            .trim()
            .to_string())
    };
    let pinned = commit("Pinned revision")?;
    let latest = commit("Newer revision")?;
    assert_ne!(pinned, latest);

    let repository_url = SafeUrl::from_directory_path(repository.path())
        .map_err(|()| anyhow!("failed to convert repository path to file URL"))?;
    let repository_url = repository_url.as_str().trim_end_matches('/');
    context
        .temp_dir
        .child("requirements.in")
        .write_str(&format!("example @ git+{repository_url}"))?;
    let lockfile = |revision: &str| {
        formatdoc! {r#"
            lock-version = "1.0"
            created-by = "uv"
            requires-python = ">=3.12"

            [[packages]]
            name = "example"
            version = "0.1.0"
            vcs = {{ type = "git", url = "{repository_url}", commit-id = "{revision}" }}
        "#}
    };
    context
        .temp_dir
        .child("pylock.toml")
        .write_str(&lockfile(&pinned))?;

    let unchanged = context
        .pip_compile()
        .args(["requirements.in", "--universal", "-o", "pylock.toml"])
        .assert()
        .success();
    let unchanged_file = context.read("pylock.toml");
    unchanged.stdout(unchanged_file.clone());
    assert_eq!(
        toml::from_str::<toml::Value>(&unchanged_file)?,
        toml::from_str::<toml::Value>(&lockfile(&pinned))?
    );

    let upgraded = context
        .pip_compile()
        .args([
            "requirements.in",
            "--universal",
            "-o",
            "pylock.toml",
            "--upgrade",
        ])
        .assert()
        .success();
    let upgraded_file = context.read("pylock.toml");
    upgraded.stdout(upgraded_file.clone());
    assert_eq!(
        toml::from_str::<toml::Value>(&upgraded_file)?,
        toml::from_str::<toml::Value>(&lockfile(&latest))?
    );
    Ok(())
}
