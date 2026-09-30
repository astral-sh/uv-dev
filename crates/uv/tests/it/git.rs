use std::path::Path;
use std::process::Command;

use uv_cache_key::cache_digest;
use uv_fs::{LockedFile, LockedFileMode};
use uv_git::{GitHttpSettings, GitResolver, RepositoryReference};
use uv_git_types::{GitLfs, GitReference, GitUrl};
use uv_redacted::DisplaySafeUrl;

fn git(directory: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(directory)
        .args(args)
        .output()
        .expect("Git should be available");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn waiting_fetch_reuses_resolved_commit() {
    let temporary = tempfile::tempdir().unwrap();
    let repository = temporary.path().join("repository");
    fs_err::create_dir(&repository).unwrap();
    git(&repository, &["init", "--initial-branch=main"]);
    git(
        &repository,
        &[
            "-c",
            "user.name=uv test",
            "-c",
            "user.email=uv-test@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "Initial commit",
        ],
    );
    let url = GitUrl::from_fields(
        DisplaySafeUrl::from_file_path(&repository).unwrap(),
        GitReference::Branch("main".to_owned()),
        None,
        GitLfs::Disabled,
    )
    .unwrap();
    let cache = temporary.path().join("cache");
    let precise = GitResolver::default()
        .fetch(&url, GitHttpSettings::default(), cache.clone(), None)
        .await
        .unwrap()
        .git()
        .precise()
        .unwrap();

    let lock = LockedFile::acquire(
        cache.join("locks").join(cache_digest(url.repository())),
        LockedFileMode::Exclusive,
        url.repository(),
    )
    .await
    .unwrap();
    let resolver = GitResolver::default();
    let fetch = resolver.fetch(&url, GitHttpSettings::default(), cache, None);
    tokio::pin!(fetch);
    assert!(futures::poll!(&mut fetch).is_pending());

    // Another fetch resolves the branch while this fetch waits for its lock. Removing the remote
    // makes an unnecessary second network operation fail instead of silently fetching again.
    resolver.insert(RepositoryReference::from(&url), precise);
    fs_err::rename(&repository, temporary.path().join("unavailable")).unwrap();
    drop(lock);

    let fetched = fetch.await.unwrap();
    assert_eq!(fetched.git().precise(), Some(precise));
}
