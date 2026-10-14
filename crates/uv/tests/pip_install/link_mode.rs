use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use anyhow::Result;
use assert_cmd::prelude::*;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use futures::executor::block_on;
use uv_test::{TestContext, venv_bin_path};

const ARCHIVE_SCRIPT: &str = "#!/bin/sh\necho 'archive payload'\n";

/// Install a wheel with both an executable archive member and a generated console script.
fn install_executable(context: &TestContext, link_mode: Option<&str>) -> Result<()> {
    let wheel = context
        .temp_dir
        .join("executable_file-1.0.0-py3-none-any.whl");
    write_executable_wheel(&wheel)?;

    let mut command = context.pip_install();
    command.arg("--no-index").arg(&wheel);
    if let Some(link_mode) = link_mode {
        command.arg("--link-mode").arg(link_mode);
    }
    command.assert().success();

    let script = context.site_packages().join("executable_file/run.sh");
    assert_eq!(fs_err::read_to_string(&script)?, ARCHIVE_SCRIPT);
    #[cfg(unix)]
    Command::new(&script)
        .assert()
        .success()
        .stdout("archive payload\n");

    let launcher = venv_bin_path(&context.venv)
        .join(format!("executable-file{}", std::env::consts::EXE_SUFFIX));
    Command::new(launcher)
        .assert()
        .success()
        .stdout("console script");
    Ok(())
}

#[test]
fn executable_default() -> Result<()> {
    install_executable(&uv_test::test_context!("3.12"), None)
}

#[test]
fn executable_copy() -> Result<()> {
    install_executable(&uv_test::test_context!("3.12"), Some("copy"))
}

#[test]
fn executable_hardlink() -> Result<()> {
    install_executable(&uv_test::test_context!("3.12"), Some("hardlink"))
}

/// Cloning data blocks must retain executable permissions.
///
/// Requires `UV_INTERNAL__TEST_COW_FS`.
/// See <https://github.com/astral-sh/uv/issues/18181>.
#[test]
#[cfg(unix)]
fn executable_clone_on_cow_filesystem() -> Result<()> {
    let Some(context) = uv_test::test_context!("3.12").with_cache_on_cow_fs()? else {
        return Ok(());
    };
    let Some(context) = context.with_working_dir_on_cow_fs()? else {
        return Ok(());
    };
    context.venv().assert().success();
    install_executable(&context, Some("clone"))
}

fn write_executable_wheel(path: &Path) -> Result<()> {
    let entries = [
        (
            "executable_file/__init__.py",
            "def main():\n    print('console script', end='')\n",
            0o644,
        ),
        ("executable_file/run.sh", ARCHIVE_SCRIPT, 0o755),
        (
            "executable_file-1.0.0.dist-info/METADATA",
            "Metadata-Version: 2.1\nName: executable-file\nVersion: 1.0.0\n",
            0o644,
        ),
        (
            "executable_file-1.0.0.dist-info/WHEEL",
            "Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
            0o644,
        ),
        (
            "executable_file-1.0.0.dist-info/entry_points.txt",
            "[console_scripts]\nexecutable-file = executable_file:main\n",
            0o644,
        ),
    ];
    let mut writer = ZipFileWriter::new(Vec::new());
    let mut record = String::new();
    for (name, contents, permissions) in entries {
        let entry =
            ZipEntryBuilder::new(name.into(), Compression::Stored).unix_permissions(permissions);
        block_on(writer.write_entry_whole(entry, contents.as_bytes()))?;
        writeln!(record, "{name},,")?;
    }
    let name = "executable_file-1.0.0.dist-info/RECORD";
    writeln!(record, "{name},,")?;
    let entry = ZipEntryBuilder::new(name.into(), Compression::Stored).unix_permissions(0o644);
    block_on(writer.write_entry_whole(entry, record.as_bytes()))?;
    fs_err::write(path, block_on(writer.close())?)?;
    Ok(())
}
