use std::path::Path;

use anyhow::Result;
use assert_fs::prelude::*;
use uv_test::uv_snapshot;

#[test]
fn clean_cache_directory_link() -> Result<()> {
    for (relative, suffix) in [false, true]
        .into_iter()
        .flat_map(|relative| ["", "/", "/."].map(|suffix| (relative, suffix)))
    {
        if cfg!(windows) && relative {
            continue;
        }
        let context = uv_test::test_context_with_versions!(&[])
            .with_cache_dir("cache-link")
            .with_filtered_sizes_and_units();
        let target = context.temp_dir.child("cache-real");
        target.child("payload").write_str("cache contents")?;
        let external = context.temp_dir.child("external");
        external.child("sentinel").write_str("external contents")?;
        uv_fs::create_symlink(external.path(), target.child("external-link"))?;
        let link_target = if relative {
            Path::new("cache-real")
        } else {
            target.path()
        };
        let link = context.cache_dir.path().to_path_buf();
        uv_fs::create_symlink(link_target, &link)?;
        let context = context.with_cache_dir(format!("{}{suffix}", link.display()));

        insta::allow_duplicates! {
            uv_snapshot!(context.filters(), context.clean(), @"
            exit_code: 0 (success)
            ----- stderr -----
            Clearing cache at: cache-link
            Removed 3 files ([SIZE])
            ");
        }

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
        insta::allow_duplicates! {
            uv_snapshot!(context.filters(), context.clean(), @"
            exit_code: 0 (success)
            ----- stderr -----
            Clearing cache at: cache-link
            Removed 2 files ([SIZE])
            ");
        }
        assert!(context.cache_dir.is_dir());
    }
    Ok(())
}
