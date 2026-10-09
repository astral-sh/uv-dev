use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Result, anyhow};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};
use url::Url;

use uv_cache_key::{RepositoryUrl, cache_digest};
#[cfg(all(feature = "test-git-lfs", feature = "test-pypi"))]
use uv_static::EnvVars;
use uv_test::TestContext;
#[cfg(all(feature = "test-git-lfs", feature = "test-pypi"))]
use uv_test::uv_snapshot;

fn git(repository: &Path, arguments: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .output()?
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    Ok(String::from_utf8(output)?.trim().to_owned())
}

fn clear_source_metadata(context: &TestContext) -> Result<()> {
    fs_err::remove_file(context.temp_dir.child("uv.lock"))?;
    for entry in fs_err::read_dir(context.cache_dir.path())? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with("sdists-v") {
            fs_err::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
}

fn set_revision(context: &TestContext, url: &Url, revision: &str) -> Result<()> {
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["dependency @ git+{url}@{revision}"]
    "#})?;
    Ok(())
}

fn supports_relative_worktrees(repository: &Path) -> Result<bool> {
    let version = git(repository, &["--version"])?;
    let version = version
        .strip_prefix("git version ")
        .ok_or_else(|| anyhow!("unexpected Git version: {version}"))?;
    let mut components = version.split('.');
    let major = components
        .next()
        .ok_or_else(|| anyhow!("missing Git major version"))?
        .parse::<u32>()?;
    let minor = components
        .next()
        .ok_or_else(|| anyhow!("missing Git minor version"))?
        .parse::<u32>()?;
    Ok((major, minor) >= (2, 48))
}

fn checkouts(root: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = fs_err::read_dir(root)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.retain(|path| path.is_dir());
    paths.sort();
    Ok(paths)
}

fn run(partial_fetches: bool) -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let repository = context.temp_dir.child("repository");
    repository.create_dir_all()?;
    git(repository.path(), &["init", "--template="])?;
    git(repository.path(), &["config", "user.name", "Alice"])?;
    git(
        repository.path(),
        &["config", "user.email", "alice@example.com"],
    )?;
    git(repository.path(), &["config", "commit.gpgsign", "false"])?;
    git(
        repository.path(),
        &["config", "uploadpack.allowFilter", "true"],
    )?;
    repository.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "dependency"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;
    let url = Url::from_directory_path(repository.path())
        .map_err(|()| anyhow!("invalid repository path"))?;
    let ident = cache_digest(&RepositoryUrl::parse(url.as_str())?);
    let features = if partial_fetches {
        "git-worktrees,git-partial-fetches"
    } else {
        "git-worktrees"
    };
    let supported = supports_relative_worktrees(repository.path())?;
    let root = if supported {
        context.cache_dir.child("git-v1/worktrees")
    } else {
        context.cache_dir.child("git-v1")
    };
    let database = root.child("db").child(&ident);
    let checkout_root = root.child("checkouts").child(&ident);

    let mut revisions = Vec::new();
    for contents in ["VALUE = 1\n", "VALUE = 2\n"] {
        repository.child("module.py").write_str(contents)?;
        git(repository.path(), &["add", "."])?;
        git(repository.path(), &["commit", "-m", "Update module"])?;
        let revision = git(repository.path(), &["rev-parse", "HEAD"])?;
        set_revision(&context, &url, &revision)?;
        context
            .lock()
            .arg("--preview-features")
            .arg(features)
            .arg("--offline")
            .assert()
            .success();
        revisions.push(revision);
    }
    let paths = checkouts(checkout_root.path())?;
    assert_eq!(paths.len(), 2);
    let config = fs_err::read_to_string(database.child(".git/config"))?;
    assert_eq!(config.contains("promisor = true"), partial_fetches);
    if !supported {
        assert!(paths.iter().all(|path| path.join(".git").is_dir()));
        return Ok(());
    }

    // A missing revision must not destroy the database shared by existing worktrees.
    set_revision(&context, &url, "deadbeef")?;
    context
        .lock()
        .arg("--preview-features")
        .arg(features)
        .arg("--offline")
        .assert()
        .failure();
    assert_eq!(
        git(&paths[0], &["rev-parse", "--is-inside-work-tree"])?,
        "true"
    );

    // Git's automatic abbreviation length can change as the shared database grows.
    // Changing it must not create another checkout for an already cached revision.
    git(database.path(), &["config", "core.abbrev", "12"])?;
    clear_source_metadata(&context)?;
    set_revision(&context, &url, &revisions[0])?;
    context
        .lock()
        .arg("--preview-features")
        .arg(features)
        .arg("--offline")
        .assert()
        .success();
    assert_eq!(checkouts(checkout_root.path())?, paths);

    for path in &paths {
        let link = fs_err::read_to_string(path.join(".git"))?;
        let target = link
            .strip_prefix("gitdir: ")
            .ok_or_else(|| anyhow!("invalid Git link"))?;
        assert!(!Path::new(target.trim()).is_absolute());
        let common = git(
            path,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?;
        assert_eq!(
            fs_err::canonicalize(common)?,
            fs_err::canonicalize(database.child(".git"))?
        );
    }

    // Every object needed to restore either revision is in the shared database.
    clear_source_metadata(&context)?;
    let hidden = context.temp_dir.child("unavailable-repository");
    fs_err::rename(repository.path(), hidden.path())?;
    let moved = context.root.child("moved-cache");
    fs_err::rename(context.cache_dir.path(), moved.path())?;
    let context = context.with_cache_dir(moved.path());
    let checkout_root = context
        .cache_dir
        .child("git-v1/worktrees/checkouts")
        .child(&ident);
    for revision in &revisions {
        set_revision(&context, &url, revision)?;
        context
            .lock()
            .arg("--preview-features")
            .arg(features)
            .arg("--offline")
            .assert()
            .success();
        let checkout = checkouts(checkout_root.path())?
            .into_iter()
            .find(|path| git(path, &["rev-parse", "HEAD"]).is_ok_and(|head| head == *revision))
            .ok_or_else(|| anyhow!("missing checkout"))?;
        fs_err::remove_file(checkout.with_extension("ok"))?;
        fs_err::remove_file(checkout.join("module.py"))?;
        clear_source_metadata(&context)?;
        context
            .lock()
            .arg("--preview-features")
            .arg(features)
            .arg("--offline")
            .assert()
            .success();
        assert!(checkout.join("module.py").is_file());
        clear_source_metadata(&context)?;
    }

    // Submodule metadata selects an ordinary clone in the worktree cache.
    fs_err::rename(hidden.path(), repository.path())?;
    repository.child(".gitmodules").write_str("")?;
    git(repository.path(), &["add", ".gitmodules"])?;
    git(
        repository.path(),
        &["commit", "-m", "Add submodule metadata"],
    )?;
    let revision = git(repository.path(), &["rev-parse", "HEAD"])?;
    set_revision(&context, &url, &revision)?;
    context
        .lock()
        .arg("--preview-features")
        .arg(features)
        .arg("--offline")
        .assert()
        .success();
    let checkout = checkouts(checkout_root.path())?
        .into_iter()
        .find(|path| git(path, &["rev-parse", "HEAD"]).is_ok_and(|head| head == revision))
        .ok_or_else(|| anyhow!("missing submodule checkout"))?;
    assert!(checkout.join(".git").is_dir());

    // Disabling preview uses a separate database without worktree extensions.
    clear_source_metadata(&context)?;
    context
        .lock()
        .arg("--no-preview")
        .arg("--offline")
        .assert()
        .success();
    let normal = context.cache_dir.child("git-v1/checkouts").child(&ident);
    assert!(
        checkouts(normal.path())?
            .iter()
            .all(|path| path.join(".git").is_dir())
    );
    Ok(())
}

#[test]
fn full_fetches() -> Result<()> {
    run(false)
}

#[test]
fn partial_fetches() -> Result<()> {
    run(true)
}

#[test]
#[cfg(all(feature = "test-git-lfs", feature = "test-pypi"))]
fn lfs() -> Result<()> {
    let context = uv_test::test_context!("3.13")
        .with_git_lfs_config()
        .with_env(
            EnvVars::UV_PREVIEW_FEATURES,
            "git-partial-fetches,git-worktrees",
        );
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.13"
        dependencies = []
    "#})?;
    uv_snapshot!(context.filters(), context.add()
        .arg("test-lfs-repo @ git+https://github.com/astral-sh/test-lfs-repo")
        .arg("--rev").arg("261c828b8e05251f3a3e4f6b47b149d691c7efbb")
        .arg("--lfs"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + test-lfs-repo==0.1.0 (from git+https://github.com/astral-sh/test-lfs-repo@261c828b8e05251f3a3e4f6b47b149d691c7efbb#lfs=true)
    ");
    let root = if supports_relative_worktrees(context.temp_dir.path())? {
        context.cache_dir.child("git-v1/worktrees/checkouts")
    } else {
        context.cache_dir.child("git-v1/checkouts")
    };
    let repository =
        RepositoryUrl::parse("https://github.com/astral-sh/test-lfs-repo")?.with_lfs(Some(true));
    let checkout = checkouts(root.child(cache_digest(&repository)).path())?
        .into_iter()
        .find(|path| {
            git(path, &["rev-parse", "HEAD"])
                .is_ok_and(|head| head == "261c828b8e05251f3a3e4f6b47b149d691c7efbb")
        })
        .ok_or_else(|| anyhow!("missing LFS checkout"))?;
    git(&checkout, &["lfs", "fsck", "--objects", "HEAD"])?;

    // Reconstruct the worktree using the LFS objects in the shared database.
    clear_source_metadata(&context)?;
    fs_err::remove_dir_all(&checkout)?;
    uv_snapshot!(context.filters(), context.sync()
        .arg("--offline")
        .arg("--reinstall"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ test-lfs-repo==0.1.0 (from git+https://github.com/astral-sh/test-lfs-repo@261c828b8e05251f3a3e4f6b47b149d691c7efbb#lfs=true)
    ");
    git(&checkout, &["lfs", "fsck", "--objects", "HEAD"])?;
    uv_snapshot!(context.filters(), context.python_command()
        .arg("-c")
        .arg("import test_lfs_repo.lfs_module"), @"
    exit_code: 0 (success)
    ");
    Ok(())
}
