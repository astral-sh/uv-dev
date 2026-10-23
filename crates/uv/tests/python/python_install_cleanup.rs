use anyhow::Result;
use assert_fs::prelude::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use url::Url;

use uv_static::EnvVars;
use uv_test::archive::write_tar_gz;
use uv_test::uv_snapshot;

/// A failed final rename must retain ownership of a flat archive's temporary directory.
#[test]
fn flat_archive_rename_failure_cleans_temporary_directory() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();
    let key = "cpython-3.12.0-windows-x86_64-none";

    // Two top-level directories exercise the NonSingularArchive extraction path. The Windows
    // installation key skips Unix executable-link repair, and neither file is executable.
    let mut contents = Vec::new();
    write_tar_gz(
        &mut contents,
        &[("first/README", "first\n"), ("second/README", "second\n")],
    )?;
    let archive = context.temp_dir.child(format!("{key}.tar.gz"));
    archive.write_binary(&contents)?;
    let archive_url = Url::from_file_path(archive.path())
        .map_err(|()| anyhow::anyhow!("failed to create the fixture archive URL"))?;
    let downloads = context.temp_dir.child("python-downloads.json");
    downloads.write_str(&serde_json::to_string(&json!({
        (key): {
            "arch": { "family": "x86_64", "variant": null },
            "libc": "none",
            "major": 3,
            "minor": 12,
            "name": "cpython",
            "os": "windows",
            "patch": 0,
            "prerelease": "",
            "sha256": hex::encode(Sha256::digest(&contents)),
            "url": archive_url,
            "variant": null
        }
    }))?)?;

    // On Unix a directory cannot be renamed over a regular file. Discovery ignores this file,
    // so installation reaches the final rename without changing the existing contents.
    let installation = context.temp_dir.child("managed").child(key);
    installation.write_str("existing installation sentinel\n")?;

    let mut command = context.python_install();
    command.env_clear();
    context.add_shared_env(&mut command, false);
    command
        .args(["--no-config", "--offline"])
        .arg(key)
        .arg("--python-downloads-json-url")
        .arg(downloads.path())
        .env(EnvVars::UV_PYTHON_DOWNLOADS, "manual")
        .env(EnvVars::UV_PYTHON_CACHE_DIR, "")
        .env_remove(EnvVars::UV_PYTHON_INSTALL_MIRROR)
        .env_remove(EnvVars::UV_PYPY_INSTALL_MIRROR);

    uv_snapshot!(context.filters(), command, @r"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to install cpython-3.12.0-windows-x86_64-none
      Caused by: Failed to copy to: managed/cpython-3.12.0-windows-x86_64-none
      Caused by: failed to rename file from [TEMP_DIR]/managed/.temp/[TMP] to [TEMP_DIR]/managed/cpython-3.12.0-windows-x86_64-none: Not a directory (os error 20)
    ");

    assert!(installation.is_file());
    assert_eq!(
        fs_err::read_to_string(installation.path())?,
        "existing installation sentinel\n"
    );
    let scratch = context.temp_dir.child("managed/.temp");
    assert!(
        fs_err::read_dir(scratch.path())?
            .collect::<Result<Vec<_>, _>>()?
            .is_empty(),
        "temporary extraction must be removed after the rename failure"
    );
    for executable in ["python3.12", "python3.12.exe"] {
        assert_eq!(
            fs_err::symlink_metadata(context.bin_dir.child(executable).path())
                .expect_err("a failed installation must not publish an executable")
                .kind(),
            std::io::ErrorKind::NotFound,
        );
    }
    assert!(!context.venv.exists());

    Ok(())
}
