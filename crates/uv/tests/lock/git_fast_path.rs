use std::fmt::Write;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use indoc::formatdoc;
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_static::EnvVars;

#[tokio::test]
async fn github_commit_lookup_shares_concurrent_successes() -> Result<()> {
    commit_lookups(false, false).await
}

#[tokio::test]
async fn github_commit_lookup_keeps_distinct_references() -> Result<()> {
    commit_lookups(true, false).await
}

#[tokio::test]
async fn github_commit_lookup_retries_failed_attempts() -> Result<()> {
    commit_lookups(false, true).await
}

async fn commit_lookups(distinct_references: bool, fail_first: bool) -> Result<()> {
    const REPOSITORY: &str = "https://github.com/uv-test/commit-lookups";
    let context = uv_test::test_context!("3.12");
    let repository = context.temp_dir.child("repository");
    let mut dependencies = String::new();
    let mut sources = String::new();
    for index in 0..8 {
        let name = format!("uv-commit-lookup-{index}");
        let branch = if distinct_references && index % 2 == 1 {
            "other"
        } else {
            "main"
        };
        repository
            .child(format!("package-{index}/pyproject.toml"))
            .write_str(&formatdoc! {r#"
                [project]
                name = "{name}"
                version = "1.0.0"
                requires-python = ">=3.8"
                dependencies = []
            "#})?;
        writeln!(dependencies, "    \"{name}\",")?;
        writeln!(
            sources,
            "{name} = {{ git = \"{REPOSITORY}\", branch = \"{branch}\", subdirectory = \"package-{index}\" }}"
        )?;
    }
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = [
        {dependencies}]

        [tool.uv.sources]
        {sources}
    "#})?;
    Command::new("git")
        .args(["init", "--initial-branch=main"])
        .arg(repository.path())
        .assert()
        .success();
    Command::new("git")
        .arg("-C")
        .arg(repository.path())
        .args(["add", "."])
        .assert()
        .success();
    Command::new("git")
        .arg("-C")
        .arg(repository.path())
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "Initial commit",
        ])
        .assert()
        .success();
    Command::new("git")
        .arg("-C")
        .arg(repository.path())
        .args(["branch", "other"])
        .assert()
        .success();
    let output = Command::new("git")
        .arg("-C")
        .arg(repository.path())
        .args(["rev-parse", "HEAD"])
        .output()?;
    assert!(output.status.success());
    let commit = String::from_utf8(output.stdout)?.trim().to_owned();
    let repository_url = Url::from_directory_path(repository.path())
        .map_err(|()| anyhow!("failed to convert repository path to file URL"))?;
    let repository_url = repository_url.as_str().trim_end_matches('/');

    let server = MockServer::start().await;
    if fail_first {
        Mock::given(method("GET"))
            .and(path("/uv-test/commit-lookups/commits/main"))
            .respond_with(ResponseTemplate::new(503).set_delay(Duration::from_millis(250)))
            .up_to_n_times(1)
            .with_priority(1)
            .expect(1)
            .mount(&server)
            .await;
    }
    for reference in if distinct_references {
        &["main", "other"][..]
    } else {
        &["main"][..]
    } {
        Mock::given(method("GET"))
            .and(path(format!("/uv-test/commit-lookups/commits/{reference}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(commit.clone())
                    .set_delay(Duration::from_millis(250)),
            )
            .expect(1)
            .mount(&server)
            .await;
    }
    context
        .lock()
        .arg("--no-index")
        .env("GIT_CONFIG_COUNT", "2")
        .env(
            "GIT_CONFIG_KEY_0",
            format!("url.{repository_url}.insteadOf"),
        )
        .env("GIT_CONFIG_VALUE_0", REPOSITORY)
        .env("GIT_CONFIG_KEY_1", "protocol.file.allow")
        .env("GIT_CONFIG_VALUE_1", "always")
        .env(EnvVars::UV_GITHUB_FAST_PATH_URL, server.uri())
        .env(EnvVars::UV_HTTP_RETRIES, "0")
        .env_remove(EnvVars::UV_NO_GITHUB_FAST_PATH)
        .assert()
        .success();
    server.verify().await;
    let lock: toml::Value =
        toml::from_str(&fs_err::read_to_string(context.temp_dir.child("uv.lock"))?)?;
    let packages = lock["package"]
        .as_array()
        .context("missing lock packages")?;
    assert_eq!(packages.len(), 9);
    for index in 0..8 {
        let name = format!("uv-commit-lookup-{index}");
        assert!(
            packages
                .iter()
                .any(|package| package["name"].as_str() == Some(name.as_str()))
        );
    }
    Ok(())
}
