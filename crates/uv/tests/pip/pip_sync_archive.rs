use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use indoc::indoc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_cache::CacheBucket;
use uv_redacted::SafeUrl;
use uv_static::EnvVars;
use uv_test::archive::generate_source_archive;
use uv_test::packse::generate_wheel;
use uv_test::uv_snapshot;

fn tomli_2_0_1_wheel() -> Result<Vec<u8>> {
    Ok(generate_wheel(
        &"tomli".parse()?,
        &"2.0.1".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    )
    .1)
}

/// Reject a mismatched version unless the filename check is explicitly disabled.
#[test]
fn mismatched_version() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let archive = context.temp_dir.child("tomli-3.7.2-py3-none-any.whl");
    archive.write_binary(&tomli_2_0_1_wheel()?)?;
    let url = SafeUrl::from_file_path(archive.path()).expect("absolute archive path");
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&format!("tomli @ {url}"))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    error: Failed to install: tomli-3.7.2-py3-none-any.whl (tomli==3.7.2 (from file://[TEMP_DIR]/tomli-3.7.2-py3-none-any.whl))
      cause: Wheel version does not match filename (2.0.1 != 3.7.2), which indicates a malformed wheel. If this is intentional, set `UV_SKIP_WHEEL_FILENAME_CHECK=1`.
    ");
    context.assert_not_installed("tomli");

    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict"])
        .env(EnvVars::UV_SKIP_WHEEL_FILENAME_CHECK, "1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + tomli==3.7.2 (from file://[TEMP_DIR]/tomli-3.7.2-py3-none-any.whl)
    ");
    context.assert_installed("tomli", "2.0.1");
    Ok(())
}

/// Reject a wheel whose internal package name differs from the filename.
#[test]
fn mismatched_name() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let archive = context.temp_dir.child("foo-2.0.1-py3-none-any.whl");
    archive.write_binary(&tomli_2_0_1_wheel()?)?;
    let url = SafeUrl::from_file_path(archive.path()).expect("absolute archive path");
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&format!("foo @ {url}"))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--strict"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because foo has an invalid package format and you require foo, we can conclude that your requirements are unsatisfiable.

    hint: The structure of `foo` was invalid
      Caused by: The .dist-info directory `tomli-2.0.1` does not start with the normalized package name: foo
    ");
    context.assert_not_installed("foo");
    context.assert_not_installed("tomli");
    Ok(())
}

/// Skip a wheel member that would escape the extraction directory.
#[tokio::test]
async fn sanitize_streamed_wheel_paths() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let filename = "ok-1.0.0-py3-none-any.whl";
    let wheel = context.temp_dir.child(filename);
    fs_err::copy(
        context.workspace_root.join("test/links").join(filename),
        &wheel,
    )?;
    // Append an unrecorded member, matching a wheel with a valid RECORD and an unsafe ZIP path.
    context
        .python_command()
        .arg("-c")
        .arg(indoc! {r"
            import sys
            from zipfile import ZipFile

            with ZipFile(sys.argv[1], 'a') as archive:
                archive.writestr('../../payload', b'')
        "})
        .arg(wheel.path())
        .assert()
        .success();

    // Wheels are staged in a child of the cache root, so `../../payload` targets its parent.
    let victim = context
        .cache_dir
        .parent()
        .context("cache root must have a parent")?
        .join("payload");
    fs_err::write(&victim, "victim contents")?;

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/{filename}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(fs_err::read(&wheel)?))
        .mount(&server)
        .await;
    context
        .temp_dir
        .child("requirements.txt")
        .write_str(&format!("ok @ {}/{filename}", server.uri()))?;

    uv_snapshot!(context.filters(), context.pip_sync().arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==1.0.0 (from http://[LOCALHOST]/ok-1.0.0-py3-none-any.whl)
    ");
    assert_eq!(fs_err::read_to_string(&victim)?, "victim contents");
    context.assert_installed("ok", "1.0.0");
    Ok(())
}

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
    let archive_url = SafeUrl::from_file_path(archive.path()).expect("absolute archive path");
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
    // ZIP timestamps cannot represent dates before 1980-01-01 UTC.
    let earliest_zip_date = SystemTime::UNIX_EPOCH + Duration::from_hours(87_648);
    assert!(
        mtime >= earliest_zip_date,
        "extracted source mtime precedes 1980: {mtime:?}"
    );
    Ok(())
}

/// Attempt to install a direct URL source distribution with a non-PEP 625-compliant
/// archive format (e.g., `.tar.bz2`). This should hard-error.
#[test]
fn reject_sdist_archive_type_bz2() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "bz2 @ {}",
        context
            .workspace_root
            .join("test/links/bz2-1.0.0.tar.bz2")
            .display()
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Source distribution `[WORKSPACE]/test/links/bz2-1.0.0.tar.bz2` has a non-PEP 625-compliant filename; only `.tar.gz` and `.zip` archives are accepted
    "
    );

    Ok(())
}
