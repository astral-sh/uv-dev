use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};

use anyhow::{Result, anyhow};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};
use url::Url;

use uv_cache::{Cache, CacheBucket};
use uv_cache_key::{RepositoryUrl, cache_digest};
use uv_git::supports_relative_worktrees;
use uv_git_types::GitOid;
#[cfg(any(unix, all(feature = "test-git-lfs", feature = "test-pypi")))]
use uv_static::EnvVars;
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

fn checkouts(root: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = fs_err::read_dir(root)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.retain(|path| path.is_dir());
    paths.sort();
    Ok(paths)
}

#[test]
fn worktrees_share_and_reuse_checkouts() -> Result<()> {
    if !supports_relative_worktrees() {
        return Ok(());
    }
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
    repository.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "dependency"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;
    repository.child("module.py").write_str("VALUE = 1\n")?;
    git(repository.path(), &["add", "."])?;
    git(repository.path(), &["commit", "-m", "Initial version"])?;
    let revision = git(repository.path(), &["rev-parse", "HEAD"])?;
    let short_revision = revision.parse::<GitOid>()?.as_short_str().to_owned();
    let url = Url::from_directory_path(repository.path())
        .map_err(|()| anyhow!("invalid repository path"))?;
    let ident = cache_digest(&RepositoryUrl::parse(url.as_str())?);
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
    uv_snapshot!(context.filters(), context.lock()
        .args(["--preview-features", "git-worktrees", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    let database = context.cache_dir.child("git-v1/worktrees/db").child(&ident);
    let checkout_root = context
        .cache_dir
        .child("git-v1/worktrees/checkouts")
        .child(&ident);
    let first_checkout = checkout_root.child(&short_revision);
    let first_revision = revision;

    repository.child("module.py").write_str("VALUE = 2\n")?;
    git(repository.path(), &["add", "."])?;
    git(repository.path(), &["commit", "-m", "Update module"])?;
    let revision = git(repository.path(), &["rev-parse", "HEAD"])?;
    let short_revision = revision.parse::<GitOid>()?.as_short_str().to_owned();
    fs_err::remove_file(context.temp_dir.child("uv.lock"))?;
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
    uv_snapshot!(context.filters(), context.lock()
        .args(["--preview-features", "git-worktrees", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    let second_checkout = checkout_root.child(short_revision);
    let paths = checkouts(checkout_root.path())?;
    assert_eq!(paths.len(), 2);
    assert!(paths.contains(&first_checkout.to_path_buf()));
    assert!(paths.contains(&second_checkout.to_path_buf()));
    assert!(!fs_err::read_to_string(database.child(".git/config"))?.contains("promisor = true"));

    assert_eq!(
        fs_err::canonicalize(git(
            first_checkout.path(),
            &["rev-parse", "--path-format=absolute", "--git-common-dir"]
        )?)?,
        fs_err::canonicalize(database.child(".git"))?,
    );
    assert_eq!(
        fs_err::canonicalize(git(
            second_checkout.path(),
            &["rev-parse", "--path-format=absolute", "--git-common-dir"]
        )?)?,
        fs_err::canonicalize(database.child(".git"))?,
    );

    // Changing Git's abbreviation length must not change an existing checkout's path.
    git(database.path(), &["config", "core.abbrev", "12"])?;
    fs_err::remove_file(context.temp_dir.child("uv.lock"))?;
    fs_err::remove_dir_all(
        Cache::from_path(context.cache_dir.path()).bucket(CacheBucket::SourceDistributions),
    )?;
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["dependency @ git+{url}@{first_revision}"]
    "#})?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--preview-features", "git-worktrees", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    assert_eq!(checkouts(checkout_root.path())?, paths);
    assert_eq!(
        git(first_checkout.path(), &["rev-parse", "HEAD"])?,
        first_revision
    );
    Ok(())
}

#[test]
fn partial_worktree_recovers_after_cache_move() -> Result<()> {
    if !supports_relative_worktrees() {
        return Ok(());
    }
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
    repository.child("module.py").write_str("VALUE = 1\n")?;
    git(repository.path(), &["add", "."])?;
    git(repository.path(), &["commit", "-m", "Initial version"])?;
    let revision = git(repository.path(), &["rev-parse", "HEAD"])?;
    let short_revision = revision.parse::<GitOid>()?.as_short_str().to_owned();
    let url = Url::from_directory_path(repository.path())
        .map_err(|()| anyhow!("invalid repository path"))?;
    let ident = cache_digest(&RepositoryUrl::parse(url.as_str())?);
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
    uv_snapshot!(context.filters(), context.lock()
        .args(["--preview-features", "git-worktrees,git-partial-fetches", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    let database = context
        .cache_dir
        .child("git-v1/partial/worktrees/db")
        .child(&ident);
    assert!(fs_err::read_to_string(database.child(".git/config"))?.contains("promisor = true"));
    fs_err::rename(
        repository.path(),
        context.temp_dir.child("unavailable-repository").path(),
    )?;
    let moved = context.root.child("moved-cache");
    fs_err::rename(context.cache_dir.path(), moved.path())?;
    let context = context.with_cache_dir(moved.path());
    let checkout = context
        .cache_dir
        .child("git-v1/partial/worktrees/checkouts")
        .child(&ident)
        .child(short_revision);

    // The moved checkout must open before uv has a chance to repair its metadata.
    assert_eq!(git(checkout.path(), &["rev-parse", "HEAD"])?, revision);
    let link = fs_err::read_to_string(checkout.child(".git"))?;
    let target = link
        .strip_prefix("gitdir: ")
        .ok_or_else(|| anyhow!("invalid Git link"))?;
    assert!(!Path::new(target.trim()).is_absolute());
    fs_err::remove_file(checkout.path().with_extension("ok"))?;
    fs_err::remove_file(checkout.child("module.py"))?;
    fs_err::remove_file(context.temp_dir.child("uv.lock"))?;
    fs_err::remove_dir_all(
        Cache::from_path(context.cache_dir.path()).bucket(CacheBucket::SourceDistributions),
    )?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--preview-features", "git-worktrees,git-partial-fetches", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    assert_eq!(
        fs_err::read_to_string(checkout.child("module.py"))?.trim(),
        "VALUE = 1"
    );
    assert_eq!(git(checkout.path(), &["rev-parse", "HEAD"])?, revision);
    Ok(())
}

/// An unresolved short commit must not replace a database with existing worktrees.
#[test]
fn missing_revision_preserves_database() -> Result<()> {
    if !supports_relative_worktrees() {
        return Ok(());
    }
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
    repository.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "dependency"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;
    git(repository.path(), &["add", "."])?;
    git(repository.path(), &["commit", "-m", "Initial version"])?;
    let revision = git(repository.path(), &["rev-parse", "HEAD"])?;
    let short_revision = revision.parse::<GitOid>()?.as_short_str().to_owned();
    let url = Url::from_directory_path(repository.path())
        .map_err(|()| anyhow!("invalid repository path"))?;
    let ident = cache_digest(&RepositoryUrl::parse(url.as_str())?);
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
    uv_snapshot!(context.filters(), context.lock()
        .args(["--preview-features", "git-worktrees", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    let checkout = context
        .cache_dir
        .child("git-v1/worktrees/checkouts")
        .child(&ident)
        .child(short_revision);
    fs_err::remove_file(context.temp_dir.child("uv.lock"))?;
    fs_err::remove_dir_all(
        Cache::from_path(context.cache_dir.path()).bucket(CacheBucket::SourceDistributions),
    )?;
    let missing_revision = "deadbeef";
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(&formatdoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["dependency @ git+{url}@{missing_revision}"]
    "#})?;
    let context = context
        .with_filter((r"`[^`]*/git(?:[.]exe)? ", "`git "))
        .with_filter((r"\(exit code: 128\)", "(exit status: 128)"));
    uv_snapshot!(context.filters(), context.lock()
        .args(["--preview-features", "git-worktrees", "--offline"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download and build `dependency @ git+file://[TEMP_DIR]/repository/@deadbeef`
      cause: Git operation failed
      cause: failed to find branch, tag, or commit `deadbeef`
      cause: process didn't exit successfully: `git rev-parse 'deadbeef^0'` (exit status: 128)
             --- stdout
             deadbeef^0

             --- stderr
             fatal: ambiguous argument 'deadbeef^0': unknown revision or path not in the working tree.
             Use '--' to separate paths from revisions, like this:
             'git <command> [<revision>...] -- [<file>...]'
    ");
    assert_eq!(git(checkout.path(), &["rev-parse", "HEAD"])?, revision);
    Ok(())
}

/// Failure to open an existing database must leave its worktree registrations intact.
#[test]
fn database_open_error_preserves_worktree() -> Result<()> {
    if !supports_relative_worktrees() {
        return Ok(());
    }
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
    repository.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "dependency"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;
    git(repository.path(), &["add", "."])?;
    git(repository.path(), &["commit", "-m", "Initial version"])?;
    let revision = git(repository.path(), &["rev-parse", "HEAD"])?;
    let short_revision = revision.parse::<GitOid>()?.as_short_str().to_owned();
    let url = Url::from_directory_path(repository.path())
        .map_err(|()| anyhow!("invalid repository path"))?;
    let ident = cache_digest(&RepositoryUrl::parse(url.as_str())?);
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
    uv_snapshot!(context.filters(), context.lock()
        .args(["--preview-features", "git-worktrees", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    let database = context.cache_dir.child("git-v1/worktrees/db").child(&ident);
    let checkout = context
        .cache_dir
        .child("git-v1/worktrees/checkouts")
        .child(&ident)
        .child(short_revision);
    let registration = git(checkout.path(), &["rev-parse", "--absolute-git-dir"])?;
    let config = database.child(".git/config");
    let original_config = fs_err::read_to_string(&config)?;
    config.write_str("[invalid\n")?;
    fs_err::remove_file(context.temp_dir.child("uv.lock"))?;
    fs_err::remove_dir_all(
        Cache::from_path(context.cache_dir.path()).bucket(CacheBucket::SourceDistributions),
    )?;
    let context = context
        .with_filtered_path(database.path(), "DATABASE")
        .with_filter((revision.as_str(), "[COMMIT]"))
        .with_filter((r"`[^`]*/git(?:[.]exe)? ", "`git "))
        .with_filter((r"\(exit code: 128\)", "(exit status: 128)"));
    uv_snapshot!(context.filters(), context.lock()
        .args(["--preview-features", "git-worktrees", "--offline"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download and build `dependency @ git+file://[TEMP_DIR]/repository/@[COMMIT]`
      cause: Git operation failed
      cause: process didn't exit successfully: `git rev-parse` (exit status: 128)
             --- stderr
             fatal: bad config line 1 in file .git/config
    ");
    assert_eq!(fs_err::read_to_string(&config)?, "[invalid\n");
    assert!(Path::new(&registration).is_dir());
    config.write_str(&original_config)?;
    assert_eq!(git(checkout.path(), &["rev-parse", "HEAD"])?, revision);
    Ok(())
}

#[test]
fn submodules_use_clone_checkout() -> Result<()> {
    if !supports_relative_worktrees() {
        return Ok(());
    }
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
    repository.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "dependency"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;
    repository.child(".gitmodules").write_str("")?;
    git(repository.path(), &["add", "."])?;
    git(repository.path(), &["commit", "-m", "Initial version"])?;
    let revision = git(repository.path(), &["rev-parse", "HEAD"])?;
    let short_revision = revision.parse::<GitOid>()?.as_short_str().to_owned();
    let url = Url::from_directory_path(repository.path())
        .map_err(|()| anyhow!("invalid repository path"))?;
    let ident = cache_digest(&RepositoryUrl::parse(url.as_str())?);
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
    uv_snapshot!(context.filters(), context.lock()
        .args(["--preview-features", "git-worktrees", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    let checkout = context
        .cache_dir
        .child("git-v1/worktrees/checkouts")
        .child(&ident)
        .child(short_revision);
    assert!(checkout.child(".git").is_dir());
    assert_eq!(git(checkout.path(), &["rev-parse", "HEAD"])?, revision);
    assert!(!context.cache_dir.child("git-v1/checkouts").exists());
    Ok(())
}

#[test]
fn disabling_worktrees_uses_legacy_cache() -> Result<()> {
    if !supports_relative_worktrees() {
        return Ok(());
    }
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
    repository.child("pyproject.toml").write_str(indoc! {r#"
        [project]
        name = "dependency"
        version = "0.1.0"
        requires-python = ">=3.12"
    "#})?;
    git(repository.path(), &["add", "."])?;
    git(repository.path(), &["commit", "-m", "Initial version"])?;
    let revision = git(repository.path(), &["rev-parse", "HEAD"])?;
    let short_revision = revision.parse::<GitOid>()?.as_short_str().to_owned();
    let url = Url::from_directory_path(repository.path())
        .map_err(|()| anyhow!("invalid repository path"))?;
    let ident = cache_digest(&RepositoryUrl::parse(url.as_str())?);
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
    uv_snapshot!(context.filters(), context.lock()
        .args(["--preview-features", "git-worktrees", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    let worktree = context
        .cache_dir
        .child("git-v1/worktrees/checkouts")
        .child(&ident)
        .child(&short_revision);
    assert!(worktree.child(".git").is_file());
    fs_err::remove_file(context.temp_dir.child("uv.lock"))?;
    fs_err::remove_dir_all(
        Cache::from_path(context.cache_dir.path()).bucket(CacheBucket::SourceDistributions),
    )?;
    uv_snapshot!(context.filters(), context.lock().args(["--no-preview", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    let checkouts = checkouts(
        context
            .cache_dir
            .child("git-v1/checkouts")
            .child(&ident)
            .path(),
    )?;
    assert_eq!(checkouts.len(), 1);
    let checkout = checkouts
        .first()
        .ok_or_else(|| anyhow!("missing checkout"))?;
    assert!(checkout.join(".git").is_dir());
    assert_eq!(git(checkout, &["rev-parse", "HEAD"])?, revision);
    assert_eq!(git(worktree.path(), &["rev-parse", "HEAD"])?, revision);
    Ok(())
}

#[test]
#[cfg(unix)]
fn older_git_falls_back_to_partial_clones() -> Result<()> {
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
    git(repository.path(), &["add", "."])?;
    git(repository.path(), &["commit", "-m", "Initial version"])?;
    let revision = git(repository.path(), &["rev-parse", "HEAD"])?;
    let url = Url::from_directory_path(repository.path())
        .map_err(|()| anyhow!("invalid repository path"))?;
    let ident = cache_digest(&RepositoryUrl::parse(url.as_str())?);
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
    let shim_dir = context.temp_dir.child("old-git");
    shim_dir.create_dir_all()?;
    let shim = shim_dir.child("git");
    shim.write_str(indoc! {r#"
        #!/bin/sh
        if [ "$1" = --version ]; then
            printf '%s\n' 'git version 2.47.0'
            exit 0
        fi
        PATH="$UV_TEST_GIT_ORIGINAL_PATH" exec git "$@"
    "#})?;
    fs_err::set_permissions(shim.path(), Permissions::from_mode(0o755))?;
    let original_path =
        std::env::var_os(EnvVars::PATH).ok_or_else(|| anyhow!("PATH is not set"))?;
    let path = std::env::join_paths(
        std::iter::once(shim_dir.path().to_path_buf()).chain(std::env::split_paths(&original_path)),
    )?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--preview-features", "git-worktrees,git-partial-fetches", "--offline"])
        .env(EnvVars::PATH, path)
        .env("UV_TEST_GIT_ORIGINAL_PATH", original_path), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    ");
    let checkouts = checkouts(
        context
            .cache_dir
            .child("git-v1/partial/checkouts")
            .child(&ident)
            .path(),
    )?;
    assert_eq!(checkouts.len(), 1);
    let checkout = checkouts
        .first()
        .ok_or_else(|| anyhow!("missing checkout"))?;
    let database = context.cache_dir.child("git-v1/partial/db").child(&ident);
    assert!(checkout.join(".git").is_dir());
    assert!(fs_err::read_to_string(database.child(".git/config"))?.contains("promisor = true"));
    assert!(!context.cache_dir.child("git-v1/partial/worktrees").exists());
    assert_eq!(git(checkout, &["rev-parse", "HEAD"])?, revision);
    Ok(())
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
    let root = if supports_relative_worktrees() {
        context
            .cache_dir
            .child("git-v1/partial/worktrees/checkouts")
    } else {
        context.cache_dir.child("git-v1/partial/checkouts")
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
    fs_err::remove_file(context.temp_dir.child("uv.lock"))?;
    fs_err::remove_dir_all(
        Cache::from_path(context.cache_dir.path()).bucket(CacheBucket::SourceDistributions),
    )?;
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
