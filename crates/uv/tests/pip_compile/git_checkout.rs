use anyhow::{Context, Result, anyhow};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use fs_err::{read_to_string, remove_file, write};
use indoc::indoc;

use uv_cache::CacheBucket;
use uv_redacted::SafeUrl;
use uv_test::uv_snapshot;

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
    let git = || {
        let mut command = context.external_command("git");
        command.arg("-C").arg(repository.path());
        command
    };
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
