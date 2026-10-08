#[cfg(not(windows))]
use std::path::Path;

use anyhow::Result;
use assert_fs::prelude::*;
use uv_test::uv_snapshot;

#[test]
fn clean_cache_directory_link_absolute_plain() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_cache_dir("cache-link")
        .with_filtered_sizes_and_units();
    let target = context.temp_dir.child("cache-real");
    target.child("payload").write_str("cache contents")?;
    let external = context.temp_dir.child("external");
    external.child("sentinel").write_str("external contents")?;
    uv_fs::create_symlink(external.path(), target.child("external-link"))?;
    let link_target = target.path();
    let link = context.cache_dir.path().to_path_buf();
    uv_fs::create_symlink(link_target, &link)?;
    let context = context.with_cache_dir(link.clone());

    uv_snapshot!(context.filters(), context.clean(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Clearing cache at: cache-link
    Removed 3 files ([SIZE])
    ");

    assert!(fs_err::symlink_metadata(&link)?.is_symlink());
    assert_eq!(fs_err::read_dir(target.path())?.count(), 0);
    assert_eq!(
        fs_err::read(external.child("sentinel"))?,
        b"external contents"
    );

    context
        .cache_dir
        .child("next-payload")
        .write_str("next contents")?;
    uv_snapshot!(context.filters(), context.clean(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Clearing cache at: cache-link
    Removed 2 files ([SIZE])
    ");
    assert!(context.cache_dir.is_dir());
    Ok(())
}

#[test]
fn clean_cache_directory_link_absolute_trailing_separator() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_cache_dir("cache-link")
        .with_filtered_sizes_and_units();
    let target = context.temp_dir.child("cache-real");
    target.child("payload").write_str("cache contents")?;
    let external = context.temp_dir.child("external");
    external.child("sentinel").write_str("external contents")?;
    uv_fs::create_symlink(external.path(), target.child("external-link"))?;
    let link_target = target.path();
    let link = context.cache_dir.path().to_path_buf();
    uv_fs::create_symlink(link_target, &link)?;
    let context = context.with_cache_dir(format!("{}/", link.display()));

    uv_snapshot!(context.filters(), context.clean(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Clearing cache at: cache-link
    Removed 3 files ([SIZE])
    ");

    assert!(fs_err::symlink_metadata(&link)?.is_symlink());
    assert_eq!(fs_err::read_dir(target.path())?.count(), 0);
    assert_eq!(
        fs_err::read(external.child("sentinel"))?,
        b"external contents"
    );

    context
        .cache_dir
        .child("next-payload")
        .write_str("next contents")?;
    uv_snapshot!(context.filters(), context.clean(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Clearing cache at: cache-link
    Removed 2 files ([SIZE])
    ");
    assert!(context.cache_dir.is_dir());
    Ok(())
}

#[test]
fn clean_cache_directory_link_absolute_trailing_dot() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_cache_dir("cache-link")
        .with_filtered_sizes_and_units();
    let target = context.temp_dir.child("cache-real");
    target.child("payload").write_str("cache contents")?;
    let external = context.temp_dir.child("external");
    external.child("sentinel").write_str("external contents")?;
    uv_fs::create_symlink(external.path(), target.child("external-link"))?;
    let link_target = target.path();
    let link = context.cache_dir.path().to_path_buf();
    uv_fs::create_symlink(link_target, &link)?;
    let context = context.with_cache_dir(format!("{}/.", link.display()));

    uv_snapshot!(context.filters(), context.clean(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Clearing cache at: cache-link
    Removed 3 files ([SIZE])
    ");

    assert!(fs_err::symlink_metadata(&link)?.is_symlink());
    assert_eq!(fs_err::read_dir(target.path())?.count(), 0);
    assert_eq!(
        fs_err::read(external.child("sentinel"))?,
        b"external contents"
    );

    context
        .cache_dir
        .child("next-payload")
        .write_str("next contents")?;
    uv_snapshot!(context.filters(), context.clean(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Clearing cache at: cache-link
    Removed 2 files ([SIZE])
    ");
    assert!(context.cache_dir.is_dir());
    Ok(())
}

#[cfg(not(windows))]
#[test]
fn clean_cache_directory_link_relative_plain() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_cache_dir("cache-link")
        .with_filtered_sizes_and_units();
    let target = context.temp_dir.child("cache-real");
    target.child("payload").write_str("cache contents")?;
    let external = context.temp_dir.child("external");
    external.child("sentinel").write_str("external contents")?;
    uv_fs::create_symlink(external.path(), target.child("external-link"))?;
    let link_target = Path::new("cache-real");
    let link = context.cache_dir.path().to_path_buf();
    uv_fs::create_symlink(link_target, &link)?;
    let context = context.with_cache_dir(link.clone());

    uv_snapshot!(context.filters(), context.clean(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Clearing cache at: cache-link
    Removed 3 files ([SIZE])
    ");

    assert!(fs_err::symlink_metadata(&link)?.is_symlink());
    assert_eq!(fs_err::read_dir(target.path())?.count(), 0);
    assert_eq!(
        fs_err::read(external.child("sentinel"))?,
        b"external contents"
    );

    context
        .cache_dir
        .child("next-payload")
        .write_str("next contents")?;
    uv_snapshot!(context.filters(), context.clean(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Clearing cache at: cache-link
    Removed 2 files ([SIZE])
    ");
    assert!(context.cache_dir.is_dir());
    Ok(())
}

#[cfg(not(windows))]
#[test]
fn clean_cache_directory_link_relative_trailing_separator() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_cache_dir("cache-link")
        .with_filtered_sizes_and_units();
    let target = context.temp_dir.child("cache-real");
    target.child("payload").write_str("cache contents")?;
    let external = context.temp_dir.child("external");
    external.child("sentinel").write_str("external contents")?;
    uv_fs::create_symlink(external.path(), target.child("external-link"))?;
    let link_target = Path::new("cache-real");
    let link = context.cache_dir.path().to_path_buf();
    uv_fs::create_symlink(link_target, &link)?;
    let context = context.with_cache_dir(format!("{}/", link.display()));

    uv_snapshot!(context.filters(), context.clean(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Clearing cache at: cache-link
    Removed 3 files ([SIZE])
    ");

    assert!(fs_err::symlink_metadata(&link)?.is_symlink());
    assert_eq!(fs_err::read_dir(target.path())?.count(), 0);
    assert_eq!(
        fs_err::read(external.child("sentinel"))?,
        b"external contents"
    );

    context
        .cache_dir
        .child("next-payload")
        .write_str("next contents")?;
    uv_snapshot!(context.filters(), context.clean(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Clearing cache at: cache-link
    Removed 2 files ([SIZE])
    ");
    assert!(context.cache_dir.is_dir());
    Ok(())
}

#[cfg(not(windows))]
#[test]
fn clean_cache_directory_link_relative_trailing_dot() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_cache_dir("cache-link")
        .with_filtered_sizes_and_units();
    let target = context.temp_dir.child("cache-real");
    target.child("payload").write_str("cache contents")?;
    let external = context.temp_dir.child("external");
    external.child("sentinel").write_str("external contents")?;
    uv_fs::create_symlink(external.path(), target.child("external-link"))?;
    let link_target = Path::new("cache-real");
    let link = context.cache_dir.path().to_path_buf();
    uv_fs::create_symlink(link_target, &link)?;
    let context = context.with_cache_dir(format!("{}/.", link.display()));

    uv_snapshot!(context.filters(), context.clean(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Clearing cache at: cache-link
    Removed 3 files ([SIZE])
    ");

    assert!(fs_err::symlink_metadata(&link)?.is_symlink());
    assert_eq!(fs_err::read_dir(target.path())?.count(), 0);
    assert_eq!(
        fs_err::read(external.child("sentinel"))?,
        b"external contents"
    );

    context
        .cache_dir
        .child("next-payload")
        .write_str("next contents")?;
    uv_snapshot!(context.filters(), context.clean(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Clearing cache at: cache-link
    Removed 2 files ([SIZE])
    ");
    assert!(context.cache_dir.is_dir());
    Ok(())
}

#[cfg(unix)]
#[test]
fn force_clean_cache_absolute_link_to_file() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_cache_dir("cache-link");
    let target = context.temp_dir.child("cache-file");
    target.write_str("retained contents")?;
    let link_target = target.path();
    uv_fs::create_symlink(link_target, &context.cache_dir)?;

    uv_snapshot!(context.filters(), context.clean().arg("--force"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Clearing cache at: cache-link
    error: Failed to clear cache at: cache-link
      cause: Cache link target is not a directory: [TEMP_DIR]/cache-file
    ");

    assert!(fs_err::symlink_metadata(&context.cache_dir)?.is_symlink());
    assert_eq!(fs_err::read(target)?, b"retained contents");
    Ok(())
}

#[cfg(unix)]
#[test]
fn force_clean_cache_relative_link_to_file() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_cache_dir("cache-link");
    let target = context.temp_dir.child("cache-file");
    target.write_str("retained contents")?;
    let link_target = Path::new("cache-file");
    uv_fs::create_symlink(link_target, &context.cache_dir)?;

    uv_snapshot!(context.filters(), context.clean().arg("--force"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Clearing cache at: cache-link
    error: Failed to clear cache at: cache-link
      cause: Cache link target is not a directory: [TEMP_DIR]/cache-file
    ");

    assert!(fs_err::symlink_metadata(&context.cache_dir)?.is_symlink());
    assert_eq!(fs_err::read(target)?, b"retained contents");
    Ok(())
}
