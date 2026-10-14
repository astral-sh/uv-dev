//! Installing distributions from Git repositories.

use anyhow::{Result, anyhow};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::indoc;
use url::Url;

use uv_static::EnvVars;
use uv_test::uv_snapshot;

/// Repository-specific Git variables from the caller must not affect uv's Git subprocesses.
#[test]
fn install_git_with_git_envs_set() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filter((r"@[0-9a-f]{40}", "@[COMMIT]"));
    let repository = context.temp_dir.child("repository");
    repository.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "ok"
        version = "1.0.0"

        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    repository.child("backend.py").write_str(indoc! {r#"
        from pathlib import Path
        from shutil import copyfile

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            wheel = Path(__file__).with_name("ok-1.0.0-py3-none-any.whl")
            copyfile(wheel, Path(wheel_directory) / wheel.name)
            return wheel.name
    "#})?;
    fs_err::copy(
        context
            .workspace_root
            .join("test/links/ok-1.0.0-py3-none-any.whl"),
        repository.child("ok-1.0.0-py3-none-any.whl"),
    )?;
    let git = || {
        let mut command = context.external_command("git");
        command.arg("-C").arg(repository.path());
        command
    };
    git().arg("init").assert().success();
    git()
        .args([
            "add",
            "--force",
            "pyproject.toml",
            "backend.py",
            "ok-1.0.0-py3-none-any.whl",
        ])
        .assert()
        .success();
    git()
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--no-gpg-sign",
            "-m",
            "Initial commit",
        ])
        .assert()
        .success();

    let repository_url = Url::from_directory_path(repository.path())
        .map_err(|()| anyhow!("failed to convert repository path to a file URL"))?;
    let repository_url = repository_url.as_str().trim_end_matches('/');
    let inherited_path = context.temp_dir.child("inherited-git-path");
    uv_snapshot!(context.filters(), context.pip_install()
        .arg(format!("ok @ git+{repository_url}"))
        .arg("--offline")
        .env(EnvVars::GIT_DIR, inherited_path.path())
        .env(EnvVars::GIT_COMMON_DIR, inherited_path.path())
        .env(EnvVars::GIT_WORK_TREE, inherited_path.path())
        .env(EnvVars::GIT_INDEX_FILE, inherited_path.path())
        .env(EnvVars::GIT_OBJECT_DIRECTORY, inherited_path.path())
        .env(EnvVars::GIT_ALTERNATE_OBJECT_DIRECTORIES, inherited_path.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==1.0.0 (from git+file://[TEMP_DIR]/repository@[COMMIT])
    ");
    inherited_path.assert(predicates::path::missing());
    context.assert_installed("ok", "1.0.0");
    Ok(())
}
