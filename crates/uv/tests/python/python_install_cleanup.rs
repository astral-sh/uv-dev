use std::path::Path;
use std::process::Command;
use std::str::FromStr;

use anyhow::{Context, Result};
#[cfg(feature = "test-python-managed")]
use assert_cmd::assert::OutputAssertExt;
use assert_fs::{fixture::ChildPath, prelude::*};
use serde_json::json;
use sha2::{Digest, Sha256};
use url::Url;

use uv_python::PythonInstallationKey;
#[cfg(feature = "test-python-managed")]
use uv_python::downloads::{ManagedPythonDownloadList, PythonDownloadRequest};
use uv_static::EnvVars;
use uv_test::archive::write_tar_gz;
#[cfg(any(unix, feature = "test-python-managed"))]
use uv_test::uv_snapshot;
use uv_test::{TestContext, capture_uv_snapshot};

fn write_cpython_fixture(
    context: &TestContext,
    key: &str,
    entries: &[(&str, &str)],
) -> Result<ChildPath> {
    let parsed = PythonInstallationKey::from_str(key)?;
    let mut contents = Vec::new();
    write_tar_gz(&mut contents, entries)?;
    let archive = context.temp_dir.child(format!("{key}.tar.gz"));
    archive.write_binary(&contents)?;
    let archive_url = Url::from_file_path(archive.path())
        .map_err(|()| anyhow::anyhow!("failed to create the fixture archive URL"))?;
    let downloads = context.temp_dir.child("python-downloads.json");
    downloads.write_str(&serde_json::to_string(&json!({
        (key): {
            "arch": { "family": parsed.arch().to_string(), "variant": null },
            "libc": parsed.libc().to_string(),
            "major": parsed.major(),
            "minor": parsed.minor(),
            "name": "cpython",
            "os": parsed.os().to_string(),
            "patch": parsed.version().patch().context("fixture key must include a patch version")?,
            "prerelease": "",
            "sha256": hex::encode(Sha256::digest(&contents)),
            "url": archive_url,
            "variant": null
        }
    }))?)?;
    Ok(downloads)
}

fn install_fixture(context: &TestContext, key: &str, downloads: &Path) -> Command {
    let mut command = context.python_install();
    context.add_shared_env(&mut command, false);
    command
        .args(["--no-config", "--offline"])
        .arg(key)
        .arg("--python-downloads-json-url")
        .arg(downloads)
        .env(EnvVars::UV_PYTHON_DOWNLOADS, "manual")
        .env(EnvVars::UV_PYTHON_CACHE_DIR, "")
        .env_remove(EnvVars::UV_PYTHON_INSTALL_MIRROR)
        .env_remove(EnvVars::UV_PYPY_INSTALL_MIRROR);
    command
}

fn assert_empty_scratch(context: &TestContext) -> Result<()> {
    let scratch = context.temp_dir.child("managed/.temp");
    assert!(
        fs_err::read_dir(scratch.path())?
            .collect::<Result<Vec<_>, _>>()?
            .is_empty(),
        "temporary extraction must be removed after an installation failure"
    );
    Ok(())
}

fn assert_no_executables(context: &TestContext, minor_version: &str) {
    for executable in [
        format!("python{minor_version}"),
        format!("python{minor_version}.exe"),
    ] {
        assert_eq!(
            fs_err::symlink_metadata(context.bin_dir.child(executable).path())
                .expect_err("a failed installation must not publish an executable")
                .kind(),
            std::io::ErrorKind::NotFound,
        );
    }
}

/// A failed final rename must retain ownership of a flat archive's temporary directory.
#[cfg(unix)]
#[test]
fn flat_archive_rename_failure_cleans_temporary_directory() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();
    let key = "cpython-3.12.0-windows-x86_64-none";

    // Two top-level directories exercise the NonSingularArchive extraction path. The Windows
    // installation key skips Unix executable-link repair. The inert stdlib directory lets
    // finalization reach the rename without executing anything from the archive.
    let downloads = write_cpython_fixture(
        &context,
        key,
        &[("Lib/README", "stdlib\n"), ("second/README", "second\n")],
    )?;

    // On Unix a directory cannot be renamed over a regular file. Discovery ignores this file,
    // so installation reaches the final rename without changing the existing contents.
    let installation = context.temp_dir.child("managed").child(key);
    installation.write_str("existing installation sentinel\n")?;

    let command = install_fixture(&context, key, downloads.path());

    uv_snapshot!(context.filters(), command, @r"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to install cpython-3.12.0-windows-x86_64-none
      cause: Failed to copy to: managed/cpython-3.12.0-windows-x86_64-none
      cause: failed to rename file from [TEMP_DIR]/managed/.temp/[TMP] to [TEMP_DIR]/managed/cpython-3.12.0-windows-x86_64-none: Not a directory (os error 20)
    ");

    assert!(installation.is_file());
    assert_eq!(
        fs_err::read_to_string(installation.path())?,
        "existing installation sentinel\n"
    );
    assert_empty_scratch(&context)?;
    assert_no_executables(&context, "3.12");
    assert!(!context.venv.exists());

    Ok(())
}

/// A failed preparation must not leave a discoverable installation directory.
#[test]
fn incomplete_archive_is_not_published() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_managed_python_dirs()
        .with_filtered_missing_file_error();
    let key = "cpython-3.12.0-windows-x86_64-none";
    let downloads = write_cpython_fixture(
        &context,
        key,
        &[("first/README", "first\n"), ("second/README", "second\n")],
    )?;

    let snapshot = capture_uv_snapshot!(
        context.filters(),
        install_fixture(&context, key, downloads.path())
    );
    assert_eq!(
        fs_err::symlink_metadata(context.temp_dir.child("managed").child(key).path())
            .expect_err("an incomplete installation must remain undiscoverable")
            .kind(),
        std::io::ErrorKind::NotFound,
    );
    assert_empty_scratch(&context)?;
    assert_no_executables(&context, "3.12");
    insta::assert_snapshot!(snapshot, @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to install cpython-3.12.0-windows-x86_64-none
      cause: failed to create file `[TEMP_DIR]/managed/.temp/[TMP]/[PYTHON-LIB]/EXTERNALLY-MANAGED`: [OS ERROR 2]
    ");
    Ok(())
}

/// A failed replacement must leave the existing Python and its entry point usable.
#[cfg(feature = "test-python-managed")]
#[test]
fn incomplete_reinstall_keeps_existing_python() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_managed_python_dirs()
        .with_filtered_python_keys()
        .with_filtered_missing_file_error();
    let download_list = ManagedPythonDownloadList::new_only_embedded()?;
    let request = PythonDownloadRequest::from_str("cpython-3.13.1")?.fill_platform()?;
    let key = download_list.find(&request)?.key().to_string();

    context.python_install().arg(&key).assert().success();
    let installation = context.temp_dir.child("managed").child(&key);
    let build = fs_err::read(installation.child("BUILD").path())?;
    let sentinel = installation.child("reinstall-sentinel");
    sentinel.write_str("existing installation\n")?;
    let bin_python = context
        .bin_dir
        .child(format!("python3.13{}", std::env::consts::EXE_SUFFIX));

    // The archive extracts successfully but is missing the stdlib directory needed for the
    // externally-managed marker. A real installed Python must survive that preparation error.
    let downloads = write_cpython_fixture(
        &context,
        &key,
        &[("bin/README", "bin\n"), ("payload/README", "payload\n")],
    )?;
    let snapshot = capture_uv_snapshot!(
        context.filters(),
        install_fixture(&context, &key, downloads.path()).arg("--reinstall")
    );
    assert_eq!(
        fs_err::read_to_string(sentinel.path())?,
        "existing installation\n"
    );
    assert_eq!(fs_err::read(installation.child("BUILD").path())?, build);
    assert_empty_scratch(&context)?;
    uv_snapshot!(context.filters(), Command::new(bin_python.path())
        .args(["-I", "-c", "import sys; print(sys.version.split()[0])"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.13.1
    ");
    insta::assert_snapshot!(snapshot, @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to install cpython-3.13.1-[PLATFORM]
      cause: failed to create file `[TEMP_DIR]/managed/.temp/[TMP]/[PYTHON-LIB]/EXTERNALLY-MANAGED`: [OS ERROR 2]
    ");
    Ok(())
}
