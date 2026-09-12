use std::path::PathBuf;

use anyhow::{Context, Result};
use assert_cmd::assert::OutputAssertExt;

use uv_static::EnvVars;
use uv_test::venv_bin_path;

#[test]
fn python_find_retargeted_cpython_venv_launcher() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"]);
    let python311 = &context.python_versions[0].1;
    let python312 = &context.python_versions[1].1;
    let environment = context.temp_dir.join("redirector");

    // CPython's venv redirector has the creator's file version, but the interpreter it launches
    // is selected by `pyvenv.cfg`. Its version resource is not authoritative for discovery.
    context
        .external_command(python311)
        .args(["-I", "-m", "venv", "--without-pip"])
        .arg(&environment)
        .assert()
        .success();
    fs_err::write(
        environment.join("pyvenv.cfg"),
        format!(
            "home = {}\ninclude-system-site-packages = false\nversion = 3.12\n",
            python312
                .parent()
                .context("Python executable has a parent")?
                .display(),
        ),
    )?;

    let scripts = venv_bin_path(&environment);
    let redirector = scripts.join("python.exe");
    context
        .external_command(&redirector)
        .args([
            "-I",
            "-c",
            "import sys; sys.stdout.write('.'.join(map(str, sys.version_info[:2])))",
        ])
        .assert()
        .success()
        .stdout("3.12");

    let found = context
        .python_find()
        .env(EnvVars::UV_PYTHON_SEARCH_PATH, &scripts)
        .arg("3.12")
        .assert()
        .success();
    let found = PathBuf::from(String::from_utf8_lossy(&found.get_output().stdout).trim());
    assert_eq!(
        fs_err::canonicalize(found)?,
        fs_err::canonicalize(redirector)?
    );

    Ok(())
}
