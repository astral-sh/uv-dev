use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use indoc::indoc;
use url::Url;

use uv_cache::CacheBucket;
use uv_test::archive::generate_source_archive;
use uv_test::uv_snapshot;

/// Source files with epoch mtimes must be extracted with dates usable by ZIP-based wheel builders.
#[test]
fn tar_dont_preserve_mtime() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let archive = context.temp_dir.child("epoch-1.0.0.tar.gz");
    archive.write_binary(&generate_source_archive(
        &"epoch".parse()?,
        &"1.0.0".parse()?,
        "",
        None,
    )?)?;
    context
        .python_command()
        .arg("-c")
        .arg(indoc! {r#"
            import sys
            import tarfile

            with tarfile.open(sys.argv[1]) as archive:
                member = archive.getmember("epoch-1.0.0/backend.py")
                assert member.mtime == 0, f"fixture member mtime: {member.mtime}"
        "#})
        .arg(archive.path())
        .assert()
        .success();
    let archive_url = Url::from_file_path(archive.path()).expect("absolute archive path");
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&format!("epoch @ {archive_url}"))?;

    uv_snapshot!(context.filters(), context.pip_sync().args(["requirements.txt", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + epoch==1.0.0 (from file://[TEMP_DIR]/epoch-1.0.0.tar.gz)
    ");

    let backend = context
        .cache_files(CacheBucket::SourceDistributions)?
        .into_iter()
        .find(|path| path.file_name().is_some_and(|name| name == "backend.py"))
        .context("source distribution was not extracted into the cache")?;
    let mtime = backend.metadata()?.modified()?;
    let earliest_zip_date = SystemTime::UNIX_EPOCH + Duration::from_secs(315_532_800);
    assert!(
        mtime >= earliest_zip_date,
        "extracted source mtime precedes 1980: {mtime:?}"
    );
    Ok(())
}
