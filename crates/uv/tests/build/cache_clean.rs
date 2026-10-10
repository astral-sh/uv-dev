#[cfg(target_os = "macos")]
use std::fs::Permissions;
#[cfg(target_os = "macos")]
use std::os::unix::fs::PermissionsExt;

use anyhow::{Context, Result};
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};
use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[cfg(target_os = "linux")]
use std::process::Command;

use uv_cache::{Cache, CacheBucket, CleanReporter};
#[cfg(unix)]
use uv_fs::link::{LinkMode, LinkOptions, link_dir};
use uv_static::EnvVars;

#[cfg(unix)]
use uv_test::assert_path_missing;
use uv_test::uv_snapshot;

/// `cache clean` should remove all packages.
#[test]
fn clean_all() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("typing-extensions\niniconfig")?;

    // Install a requirement, to populate the cache.
    context
        .pip_sync()
        .arg("requirements.txt")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.clean().arg("--verbose"), @"
    exit_code: 0 (success)
    ----- stderr -----
    DEBUG Searching for user configuration in: [UV_USER_CONFIG_DIR]/uv.toml
    DEBUG uv [VERSION] ([COMMIT] DATE)
    Clearing cache at: [CACHE_DIR]/
    Removed [N] files ([SIZE])
    ");

    Ok(())
}

/// Cache cleanup should count hardlinked storage only when its final link is removed.
#[cfg(unix)]
#[test]
fn clean_all_hardlinked_file() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_counts();

    // Remove unrelated cache entries so the retained hardlink is the only cached data.
    context.clean().assert().success();
    context.cache_dir.create_dir_all()?;

    // Keep the retained hardlink beside the cache so both entries share a filesystem.
    let retained = context.cache_dir.path().with_file_name("retained.bin");
    fs_err::write(&retained, vec![42; 1024 * 1024])?;
    fs_err::OpenOptions::new()
        .write(true)
        .open(&retained)?
        .sync_all()?;

    let cached = context.cache_dir.child("hardlinked.bin");
    fs_err::hard_link(&retained, &cached)?;

    // Counting the externally retained hardlink would incorrectly report 1.0MiB.
    uv_snapshot!(context.filters(), context.clean(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Clearing cache at: [CACHE_DIR]/
    Removed [N] files (0B)
    ");

    context.cache_dir.create_dir_all()?;
    fs_err::hard_link(&retained, &cached)?;

    uv_snapshot!(context.filters(), context.clean().arg("--preview-features").arg("cache-physical-space"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Clearing cache at: [CACHE_DIR]/
    Removed [N] files (0B)
    ");

    assert!(retained.is_file());

    context.cache_dir.create_dir_all()?;
    cached.write_binary(&vec![42; 1024 * 1024])?;
    fs_err::OpenOptions::new()
        .write(true)
        .open(cached.path())?
        .sync_all()?;
    fs_err::hard_link(&cached, context.cache_dir.child("second-hardlink.bin"))?;

    // Counting each hardlink separately would incorrectly report 2.0MiB.
    uv_snapshot!(context.filters(), context.clean().arg("--preview-features").arg("cache-physical-space"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Clearing cache at: [CACHE_DIR]/
    Removed [N] files (1.0MiB)
    ");

    Ok(())
}

/// `cache clean` should fall back to logical space on unsupported filesystems.
#[cfg(unix)]
#[test]
fn clean_all_physical_space_unsupported_fs() -> Result<()> {
    let Some(context) = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_cache_on_alt_fs()?
    else {
        return Ok(());
    };

    context
        .cache_dir
        .child("cached.bin")
        .write_binary(&vec![42; 1024 * 1024])?;

    uv_snapshot!(context.filters(), context.clean().arg("--preview-features").arg("cache-physical-space"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Clearing cache at: [ALT_FS]/[CACHE_DIR]/
    Removed [N] files (1.0MiB)
    ");

    Ok(())
}

/// `cache clean` should report physical space for copy-on-write clones in preview mode.
#[cfg(unix)]
#[test]
fn clean_all_cloned_file() -> Result<()> {
    let Some(context) = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_cache_on_cow_fs()?
    else {
        return Ok(());
    };
    let retained = context.cache_dir.path().with_file_name("retained");
    fs_err::create_dir_all(&retained)?;
    let original = retained.join("original.bin");
    fs_err::write(&original, vec![42; 1024 * 1024])?;

    // Remove unrelated cache entries so the cloned file is the only allocated data being cleaned.
    context.clean().assert().success();
    context.cache_dir.create_dir_all()?;

    let cached = context.cache_dir.child("cloned");
    let link_mode = link_dir(&retained, &cached, &LinkOptions::new(LinkMode::Clone))?;
    assert_eq!(
        link_mode,
        LinkMode::Clone,
        "the configured copy-on-write filesystem did not clone the cached file"
    );

    uv_snapshot!(context.filters(), context.clean().arg("--preview"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Clearing cache at: [COW_FS]/[CACHE_DIR]/
    Removed [N] files (0B)
    ");

    assert!(original.is_file());

    Ok(())
}

/// Clones shared only within the cache should be counted once when their final reference is removed.
#[cfg(unix)]
#[test]
fn clean_all_cached_clones() -> Result<()> {
    let Some(context) = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_cache_on_cow_fs()?
    else {
        return Ok(());
    };
    let original = context.cache_dir.child("original");
    original.create_dir_all()?;
    original
        .child("original.bin")
        .write_binary(&vec![42; 1024 * 1024])?;

    let cloned = context.cache_dir.child("cloned");
    let link_mode = link_dir(&original, &cloned, &LinkOptions::new(LinkMode::Clone))?;
    assert_eq!(
        link_mode,
        LinkMode::Clone,
        "the configured copy-on-write filesystem did not clone the cached file"
    );

    uv_snapshot!(context.filters(), context.clean().arg("--preview-features").arg("cache-physical-space"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Clearing cache at: [COW_FS]/[CACHE_DIR]/
    Removed [N] files (1.0MiB)
    ");

    Ok(())
}

/// Unknown compressed extents should not discard measurements for unrelated cache entries.
#[cfg(target_os = "linux")]
#[test]
fn clean_all_compressed_file() -> Result<()> {
    let Some(context) = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_cache_on_cow_fs()?
    else {
        return Ok(());
    };
    let measured = context.cache_dir.child("measured.bin");
    measured.write_binary(&vec![42; 1024 * 1024])?;
    fs_err::OpenOptions::new()
        .write(true)
        .open(measured.path())?
        .sync_all()?;

    let compressed = context.cache_dir.child("compressed.bin");
    fs_err::File::create(compressed.path())?;
    Command::new("btrfs")
        .args(["property", "set"])
        .arg(compressed.path())
        .args(["compression", "zstd"])
        .assert()
        .success();
    compressed.write_binary(&vec![42; 1024 * 1024])?;
    fs_err::OpenOptions::new()
        .write(true)
        .open(compressed.path())?
        .sync_all()?;

    uv_snapshot!(context.filters(), context.clean().arg("--preview-features").arg("cache-physical-space"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Clearing cache at: [COW_FS]/[CACHE_DIR]/
    Removed [N] files (at least 1.0MiB)
    ");

    Ok(())
}

/// `cache clear` should behave as an alias of `cache clean`.
#[test]
fn clear_all_alias() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("typing-extensions\niniconfig")?;

    // Install a requirement, to populate the cache.
    context
        .pip_sync()
        .arg("requirements.txt")
        .assert()
        .success();

    let mut command = context.command();
    command.arg("cache").arg("clear").arg("--verbose");

    uv_snapshot!(context.filters(), command, @"
    exit_code: 0 (success)
    ----- stderr -----
    DEBUG Searching for user configuration in: [UV_USER_CONFIG_DIR]/uv.toml
    DEBUG uv [VERSION] ([COMMIT] DATE)
    Clearing cache at: [CACHE_DIR]/
    Removed [N] files ([SIZE])
    ");

    Ok(())
}

#[tokio::test]
async fn clean_force() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_counts()
        .with_filtered_sizes_and_units();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("typing-extensions\niniconfig")?;

    // Install a requirement, to populate the cache.
    context
        .pip_sync()
        .arg("requirements.txt")
        .assert()
        .success();

    // When unlocked, `--force` should still take a lock
    uv_snapshot!(context.filters(), context.clean().arg("--verbose").arg("--force"), @"
    exit_code: 0 (success)
    ----- stderr -----
    DEBUG Searching for user configuration in: [UV_USER_CONFIG_DIR]/uv.toml
    DEBUG uv [VERSION] ([COMMIT] DATE)
    Clearing cache at: [CACHE_DIR]/
    Removed [N] files ([SIZE])
    ");

    // Install a requirement, to re-populate the cache.
    context
        .pip_sync()
        .arg("requirements.txt")
        .assert()
        .success();

    // When locked, `--force` should proceed without blocking
    let _cache = uv_cache::Cache::from_path(context.cache_dir.path())
        .with_exclusive_lock()
        .await;
    uv_snapshot!(context.filters(), context.clean().arg("--verbose").arg("--force"), @"
    exit_code: 0 (success)
    ----- stderr -----
    DEBUG Searching for user configuration in: [UV_USER_CONFIG_DIR]/uv.toml
    DEBUG uv [VERSION] ([COMMIT] DATE)
    DEBUG Lock is busy for `[CACHE_DIR]/`
    DEBUG Cache is currently in use, proceeding due to `--force`
    Clearing cache at: [CACHE_DIR]/
    Removed [N] files ([SIZE])
    ");

    Ok(())
}

/// `cache clean iniconfig` should remove a single package (`iniconfig`).
#[test]
fn clean_package_pypi() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units()
        // The cache entry does not have a stable key, so we filter it out.
        .with_filter((
            r"\[CACHE_DIR\](\\|\/)(.+)(\\|\/).*",
            "[CACHE_DIR]/$2/[ENTRY]",
        ));

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("anyio\niniconfig")?;

    // Install a requirement, to populate the cache.
    context
        .pip_sync()
        .arg("requirements.txt")
        .assert()
        .success();

    // Assert that the `.rkyv` file is created for `iniconfig`.
    let rkyv = context
        .cache_dir
        .child("simple-v26")
        .child("pypi")
        .child("iniconfig.rkyv");
    assert!(
        rkyv.exists(),
        "Expected the `.rkyv` file to exist for `iniconfig`"
    );

    uv_snapshot!(context.filters(), context.clean().arg("--verbose").arg("iniconfig"), @"
    exit_code: 0 (success)
    ----- stderr -----
    DEBUG Searching for user configuration in: [UV_USER_CONFIG_DIR]/uv.toml
    DEBUG uv [VERSION] ([COMMIT] DATE)
    DEBUG Removing dangling cache entry: [CACHE_DIR]/archive-v0/[ENTRY]
    Removed [N] files ([SIZE])
    ");

    // Assert that the `.rkyv` file is removed for `iniconfig`.
    assert!(
        !rkyv.exists(),
        "Expected the `.rkyv` file to be removed for `iniconfig`"
    );

    // Running `uv cache prune` should have no effect.
    uv_snapshot!(context.filters(), context.prune().arg("--verbose"), @"
    exit_code: 0 (success)
    ----- stderr -----
    DEBUG Searching for user configuration in: [UV_USER_CONFIG_DIR]/uv.toml
    DEBUG uv [VERSION] ([COMMIT] DATE)
    Pruning cache at: [CACHE_DIR]/
    No unused entries found
    ");

    Ok(())
}

/// `cache clean iniconfig` should remove a single package (`iniconfig`).
#[test]
fn clean_package_index() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units()
        // The cache entry does not have a stable key, so we filter it out.
        .with_filter((
            r"\[CACHE_DIR\](\\|\/)(.+)(\\|\/).*",
            "[CACHE_DIR]/$2/[ENTRY]",
        ));

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("anyio\niniconfig")?;

    // Install a requirement, to populate the cache.
    context
        .pip_sync()
        .arg("requirements.txt")
        .arg("--index-url")
        .arg("https://test.pypi.org/simple")
        .assert()
        .success();

    // Assert that the `.rkyv` file is created for `iniconfig`.
    let rkyv = context
        .cache_dir
        .child("simple-v26")
        .child("index")
        .child("e8208120cae3ba69")
        .child("iniconfig.rkyv");
    assert!(
        rkyv.exists(),
        "Expected the `.rkyv` file to exist for `iniconfig`"
    );

    uv_snapshot!(context.filters(), context.clean().arg("--verbose").arg("iniconfig"), @"
    exit_code: 0 (success)
    ----- stderr -----
    DEBUG Searching for user configuration in: [UV_USER_CONFIG_DIR]/uv.toml
    DEBUG uv [VERSION] ([COMMIT] DATE)
    DEBUG Removing dangling cache entry: [CACHE_DIR]/archive-v0/[ENTRY]
    Removed [N] files ([SIZE])
    ");

    // Assert that the `.rkyv` file is removed for `iniconfig`.
    assert!(
        !rkyv.exists(),
        "Expected the `.rkyv` file to be removed for `iniconfig`"
    );

    Ok(())
}

#[cfg(unix)]
#[test]
fn clean_package_does_not_follow_symlinks() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_sizes_and_units();
    let victim_dir = context.temp_dir.child("victim");
    let archive_entry = context.cache_dir.child("archive-v0").child("archive");
    let package_entry = context
        .cache_dir
        .child("wheels-v7")
        .child("pypi")
        .child("demo");

    victim_dir.create_dir_all()?;
    victim_dir.child("payload.txt").write_str("payload")?;
    archive_entry.create_dir_all()?;
    archive_entry.child("payload.txt").write_str("payload")?;
    package_entry.create_dir_all()?;

    // Preserve external targets while still removing unreferenced entries in the archive bucket.
    fs_err::os::unix::fs::symlink(&victim_dir, package_entry.join("escape"))?;
    fs_err::os::unix::fs::symlink(&archive_entry, package_entry.join("archive"))?;

    let files = context.cache_dir.child("files-v0");
    let shard = files.child("shard");
    shard.child("orphan").write_str("orphan")?;
    shard
        .child("nested")
        .child("orphan")
        .write_str("nested orphan")?;
    fs_err::os::unix::fs::symlink(&victim_dir, files.child("escape"))?;
    fs_err::os::unix::fs::symlink(&victim_dir, shard.child("escape"))?;

    // Keep this shard flat so macOS can prune it with bulk metadata reads.
    let flat_shard = files.child("flat");
    flat_shard.child("orphan").write_str("orphan")?;
    let retained = context.cache_dir.path().with_file_name("retained.bin");
    fs_err::write(&retained, "retained")?;
    fs_err::hard_link(&retained, flat_shard.child("retained"))?;
    fs_err::os::unix::fs::symlink(&victim_dir, flat_shard.child("escape"))?;

    uv_snapshot!(context.filters(), context.clean().args(["demo", "other"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed 6 files ([SIZE])
    ");

    assert!(victim_dir.is_dir());
    assert!(victim_dir.child("payload.txt").is_file());
    assert_path_missing(package_entry);
    assert_path_missing(archive_entry);
    assert!(!shard.child("orphan").exists());
    assert!(!shard.child("nested").exists());
    assert!(fs_err::symlink_metadata(files.child("escape"))?.is_symlink());
    assert!(fs_err::symlink_metadata(shard.child("escape"))?.is_symlink());
    assert!(!flat_shard.child("orphan").exists());
    assert!(retained.is_file());
    assert!(flat_shard.child("retained").is_file());
    assert!(fs_err::symlink_metadata(flat_shard.child("escape"))?.is_symlink());

    Ok(())
}

/// Empty file-cache shards can be removed without search permission.
#[cfg(target_os = "macos")]
#[test]
fn clean_package_empty_shard_without_search_permission() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let shard = context.cache_dir.child("files-v0").child("shard");
    shard.create_dir_all()?;
    fs_err::set_permissions(&shard, Permissions::from_mode(0o600))?;

    uv_snapshot!(context.filters(), context.clean().arg("demo"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed 1 directory (0B)
    ");

    assert!(!shard.exists());

    Ok(())
}

#[tokio::test]
async fn cache_timeout() {
    let context = uv_test::test_context!("3.12");

    // Simulate another uv process running and locking the cache, e.g., with a source build.
    let _cache = Cache::from_path(context.cache_dir.path())
        .with_exclusive_lock()
        .await;

    uv_snapshot!(context.filters(), context.clean().env(EnvVars::UV_LOCK_TIMEOUT, "1"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Cache is currently in-use, waiting for other uv processes to finish (use `--force` to override)
    error: Timeout ([TIME]) when waiting for lock on `[CACHE_DIR]/` at `[CACHE_DIR]/.lock`, is another uv process running? You can set `UV_LOCK_TIMEOUT` to increase the timeout.
    ");
}

/// `cache clean` should handle file paths normally restricted by Win32 path normalization.
#[cfg(windows)]
#[test]
fn clean_handles_verbatim_paths() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Clean slate
    fs_err::remove_dir_all(&context.cache_dir)?;

    // Cached sdist path resembling the uwsgi==2.0.31 build failure.
    let uwsgi_shard = context
        .cache_dir
        .child("sdists-v10")
        .child("pypi")
        .child("uwsgi")
        .child("2.0.31")
        .child("QxDIp0qpjbsWjWURKmegK")
        .child("src")
        .child("core");

    // Attempt to create a file with a trailing dot (we need to make it verbatim to do so)
    uwsgi_shard.create_dir_all()?;
    let invalid_path = uwsgi_shard.child("logging.").to_path_buf();
    let invalid_file = uv_fs::verbatim_path(invalid_path.as_path());
    fs_err::write(&invalid_file, b"")?;

    // Confirm Win32 normalized path causes an os error when attempting to remove
    let remove_err = fs_err::remove_file(&invalid_path).expect_err("expected to fail");
    assert_eq!(remove_err.kind(), std::io::ErrorKind::NotFound);

    // Tests cache clean leverages verbatim conversion
    uv_snapshot!(context.filters(), context.clean().arg("--verbose"), @"
    exit_code: 0 (success)
    ----- stderr -----
    DEBUG Searching for user configuration in: [UV_USER_CONFIG_DIR]/uv.toml
    DEBUG uv [VERSION] ([COMMIT] DATE)
    Clearing cache at: [CACHE_DIR]/
    Removed 1 file (0B)
    ");

    Ok(())
}

struct SilentCacheCleaner;

impl CleanReporter for SilentCacheCleaner {
    fn on_clean(&self) {}
    fn on_complete(&self) {}
}

async fn wait_for_cache_message(
    child: &mut tokio::process::Child,
    message: &str,
) -> Result<tokio::task::JoinHandle<std::io::Result<Vec<u8>>>> {
    let stderr = child
        .stderr
        .take()
        .context("captured cache command stderr")?;
    let mut reader = tokio::io::BufReader::new(stderr);
    let mut output = Vec::new();
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let start = output.len();
            if reader.read_until(b'\n', &mut output).await? == 0 {
                anyhow::bail!("command exited without reporting {message}");
            }
            if String::from_utf8_lossy(&output[start..]).contains(message) {
                return Ok::<_, anyhow::Error>(());
            }
        }
    })
    .await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(err)) => anyhow::bail!(
            "Failed while waiting for {message}: {err}\nCaptured stderr:\n{}",
            String::from_utf8_lossy(&output)
        ),
        Err(err) => anyhow::bail!(
            "Timed out waiting for {message}: {err}\nCaptured stderr:\n{}",
            String::from_utf8_lossy(&output)
        ),
    }
    Ok(tokio::spawn(async move {
        reader.read_to_end(&mut output).await?;
        Ok(output)
    }))
}

#[tokio::test]
async fn clean_preserves_waiting_cache_users() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let root = context.cache_dir.path().to_owned();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let port = listener.local_addr()?.port();
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    let (filename, wheel) = uv_test::packse::generate_wheel(
        &"demo".parse()?,
        &"1.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    fs_err::write(context.temp_dir.join(&filename), wheel)?;
    context.temp_dir.child("backend.py").write_str(&formatdoc! {r#"
        import shutil
        import socket
        from pathlib import Path

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            with socket.create_connection(("127.0.0.1", {port}), timeout=30) as gate:
                gate.sendall(b"ready")
                if gate.recv(1) != b"S":
                    raise RuntimeError("cache lifetime gate closed")
            shutil.copyfile(Path(__file__).with_name("{filename}"), Path(wheel_directory) / "{filename}")
            return "{filename}"
    "#})?;

    // The first cleaner owns the same lock that the waiting build has already opened.
    let cleaner = Cache::from_path(&root).with_exclusive_lock().await?;
    let mut command = context.build();
    command
        .args(["--wheel", "--offline"])
        .env(EnvVars::RUST_LOG, "uv_fs=info");
    let mut user = tokio::process::Command::from(command)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut user_stderr =
        wait_for_cache_message(&mut user, "Waiting to acquire shared lock").await?;
    cleaner.clear(Box::new(SilentCacheCleaner))?;

    let (mut gate, _) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::select! {
            connection = listener.accept() => Ok(connection?),
            status = user.wait() => {
                let stderr = (&mut user_stderr).await??;
                anyhow::bail!("cache user exited before its build hook: {status:?}\n{}", String::from_utf8_lossy(&stderr));
            }
        }
    }).await??;
    let mut ready = [0; 5];
    tokio::time::timeout(Duration::from_secs(30), gate.read_exact(&mut ready)).await??;
    assert_eq!(&ready, b"ready");
    assert!(context.cache_dir.child(".lock").is_file());
    assert!(context.cache_dir.child("CACHEDIR.TAG").is_file());
    assert!(context.cache_dir.child(".gitignore").is_file());
    let source_bucket =
        Cache::from_path(context.cache_dir.path()).bucket(CacheBucket::SourceDistributions);
    assert!(source_bucket.join(".gitignore").is_file());
    assert!(source_bucket.join(".git").is_file());

    // A later cleaner must wait for that user, even though it opened the lock after cleanup.
    let mut later = tokio::process::Command::from(context.clean())
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let later_stderr = wait_for_cache_message(&mut later, "Cache is currently in-use").await?;
    gate.write_all(b"S").await?;
    let mut output = user.wait_with_output().await?;
    output.stderr = user_stderr.await??;
    output.assert().success();
    let mut output =
        tokio::time::timeout(Duration::from_secs(30), later.wait_with_output()).await??;
    output.stderr = later_stderr.await??;
    output.assert().success();
    assert_eq!(fs_err::read_dir(context.cache_dir.path())?.count(), 2);
    assert!(context.cache_dir.child(".lock").is_file());
    assert!(root.join(".gitignore").is_file());
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn clean_preserves_waiting_cache_users_through_alias() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let real_root = context.cache_dir.path().to_owned();
    let alias = context.temp_dir.child("cache-alias");
    fs_err::os::unix::fs::symlink(&real_root, &alias)?;
    let alias_context = context.with_cache_dir(alias.path());
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let port = listener.local_addr()?.port();
    alias_context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    let (filename, wheel) = uv_test::packse::generate_wheel(
        &"demo".parse()?,
        &"1.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    fs_err::write(alias_context.temp_dir.join(&filename), wheel)?;
    alias_context.temp_dir.child("backend.py").write_str(&formatdoc! {r#"
        import shutil
        import socket
        from pathlib import Path

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            with socket.create_connection(("127.0.0.1", {port}), timeout=30) as gate:
                gate.sendall(b"ready")
                if gate.recv(1) != b"S":
                    raise RuntimeError("cache lifetime gate closed")
            shutil.copyfile(Path(__file__).with_name("{filename}"), Path(wheel_directory) / "{filename}")
            return "{filename}"
    "#})?;

    // The first cleaner owns the same lock that the waiting build has already opened.
    let cleaner = Cache::from_path(alias.path()).with_exclusive_lock().await?;
    let mut command = alias_context.build();
    command
        .args(["--wheel", "--offline"])
        .env(EnvVars::RUST_LOG, "uv_fs=info");
    let mut user = tokio::process::Command::from(command)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut user_stderr =
        wait_for_cache_message(&mut user, "Waiting to acquire shared lock").await?;
    cleaner.clear(Box::new(SilentCacheCleaner))?;

    let (mut gate, _) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::select! {
            connection = listener.accept() => Ok(connection?),
            status = user.wait() => {
                let stderr = (&mut user_stderr).await??;
                anyhow::bail!("cache user exited before its build hook: {status:?}\n{}", String::from_utf8_lossy(&stderr));
            }
        }
    }).await??;
    let mut ready = [0; 5];
    tokio::time::timeout(Duration::from_secs(30), gate.read_exact(&mut ready)).await??;
    assert_eq!(&ready, b"ready");
    assert!(alias_context.cache_dir.child(".lock").is_file());
    assert!(alias_context.cache_dir.child("CACHEDIR.TAG").is_file());
    assert!(alias_context.cache_dir.child(".gitignore").is_file());
    let source_bucket =
        Cache::from_path(alias_context.cache_dir.path()).bucket(CacheBucket::SourceDistributions);
    assert!(source_bucket.join(".gitignore").is_file());
    assert!(source_bucket.join(".git").is_file());

    // A later cleaner must wait for that user, even though it opened the lock after cleanup.
    let real_context = alias_context.with_cache_dir(&real_root);
    let mut later = tokio::process::Command::from(real_context.clean())
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let later_stderr = wait_for_cache_message(&mut later, "Cache is currently in-use").await?;
    gate.write_all(b"S").await?;
    let mut output = user.wait_with_output().await?;
    output.stderr = user_stderr.await??;
    output.assert().success();
    let mut output =
        tokio::time::timeout(Duration::from_secs(30), later.wait_with_output()).await??;
    output.stderr = later_stderr.await??;
    output.assert().success();
    assert_eq!(fs_err::read_dir(real_context.cache_dir.path())?.count(), 2);
    assert!(real_context.cache_dir.child(".lock").is_file());
    assert!(alias.path().is_dir());
    assert!(real_root.join(".gitignore").is_file());
    Ok(())
}

#[tokio::test]
async fn cache_init_no_wait_creates_scaffold_only_after_admission() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let root = context.temp_dir.child("new-cache");
    root.create_dir_all()?;
    let cleaner = Cache::from_path(root.path()).with_exclusive_lock().await?;
    assert!(Cache::from_path(root.path()).init_no_wait()?.is_none());
    assert_eq!(fs_err::read_dir(root.path())?.count(), 1);
    drop(cleaner);
    let cache = Cache::from_path(root.path())
        .init_no_wait()?
        .context("uncontended cache")?;
    assert!(root.child("CACHEDIR.TAG").is_file());
    assert!(root.child(".gitignore").is_file());
    assert!(
        cache
            .bucket(CacheBucket::SourceDistributions)
            .join(".git")
            .is_file()
    );
    Ok(())
}

#[tokio::test]
async fn forced_clean_retains_cache_coordination() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let root = context.temp_dir.child("cache-forced");
    let context = context.with_cache_dir(root.path());
    let cache = Cache::from_path(root.path()).init().await?;
    root.child("payload").write_str("payload")?;
    context.clean().arg("--force").assert().success();
    assert_eq!(fs_err::read_dir(root.path())?.count(), 2);
    assert!(root.child(".lock").is_file());
    assert!(
        Cache::from_path(root.path())
            .with_exclusive_lock_no_wait()
            .is_err()
    );
    drop(cache);

    let summary = Cache::from_path(root.path())
        .with_exclusive_lock()
        .await?
        .clear(Box::new(SilentCacheCleaner))?;
    assert_eq!(summary.num_files, 0);
    assert_eq!(summary.num_dirs, 0);

    // Recreate the scaffold after an idle cache was removed externally.
    fs_err::remove_dir_all(root.path())?;
    let cache = Cache::from_path(root.path()).init().await?;
    assert!(cache.root().join("CACHEDIR.TAG").is_file());
    assert!(
        cache
            .bucket(CacheBucket::SourceDistributions)
            .join(".git")
            .is_file()
    );
    Ok(())
}

#[cfg(feature = "test-git")]
#[tokio::test]
async fn clean_keeps_retained_cache_files_ignored() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .external_command("git")
        .args(["init", "--quiet"])
        .assert()
        .success();
    let cache_root = context.temp_dir.child(".uv-cache");
    let context = context.with_cache_dir(cache_root.path());
    drop(Cache::from_path(cache_root.path()).init().await?);
    // A cache may be cleaned before an initializer has supplied its Git marker.
    fs_err::remove_file(cache_root.join(".gitignore"))?;
    context.clean().assert().success();
    context
        .external_command("git")
        .args([
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            ".uv-cache",
        ])
        .assert()
        .success()
        .stdout("");
    assert_eq!(fs_err::read_to_string(cache_root.join(".gitignore"))?, "*");
    assert!(cache_root.join(".lock").is_file());
    Ok(())
}
