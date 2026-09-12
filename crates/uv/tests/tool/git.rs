use std::process::Command;

use anyhow::{Result, anyhow};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::indoc;
use url::Url;
use uv_test::TestContext;

/// Create a tagged tool repository with two entrypoints and a registry dependency.
pub(super) fn tool_repository(context: &TestContext) -> Result<String> {
    let repository = context.temp_dir.child("repository");
    repository.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "git-tool"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = ["extra-requirement"]

        [project.scripts]
        git-tool = "git_tool:main"
        git-tool-helper = "git_tool:main"

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
    "#})?;
    repository
        .child("src/git_tool/__init__.py")
        .write_str(indoc! {r#"
            def main():
                print("git-tool 1.0.0")
        "#})?;

    Command::new("git")
        .arg("init")
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
            "user.name=ferris",
            "-c",
            "user.email=ferris@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "Initial commit",
        ])
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .assert()
        .success();
    Command::new("git")
        .arg("-C")
        .arg(repository.path())
        .args(["tag", "1.0.0"])
        .assert()
        .success();

    let url = Url::from_directory_path(repository.path())
        .map_err(|()| anyhow!("failed to convert repository path to file URL"))?;
    Ok(format!("git+{}@1.0.0", url.as_str().trim_end_matches('/')))
}
