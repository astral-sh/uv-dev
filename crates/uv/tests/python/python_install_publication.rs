use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::indoc;
use insta::allow_duplicates;

use uv_python::managed::platform_key_from_env;
use uv_test::{TestContext, uv_snapshot, venv_bin_path};

#[derive(Debug)]
struct InstallNameCall {
    install_name: PathBuf,
    dylib: PathBuf,
}

/// Pause a real Python installation at its final macOS metadata update.
struct InstallNameToolBarrier {
    child: Option<Child>,
    started: PathBuf,
    release: PathBuf,
    calls: PathBuf,
}

impl InstallNameToolBarrier {
    fn spawn(context: &TestContext, mut command: Command) -> Result<Self> {
        let real_tool = which::which("install_name_tool")?;
        let started = context.temp_dir.join("install-name-started");
        let release = context.temp_dir.join("install-name-release");
        let calls = context.temp_dir.join("install-name-calls");
        let wrapper = context.bin_dir.child("install_name_tool");
        wrapper.write_str(indoc! {r#"
            #!/bin/sh
            set -eu
            parent=$PPID
            printf '%s\0' "$@" >> "$UV_TEST_INSTALL_NAME_CALLS"
            : > "$UV_TEST_INSTALL_NAME_STARTED"
            while [ ! -f "$UV_TEST_INSTALL_NAME_RELEASE" ]; do
                if ! kill -0 "$parent" 2>/dev/null; then
                    exit 1
                fi
                sleep 0.01
            done
            exec "$UV_TEST_REAL_INSTALL_NAME_TOOL" "$@"
        "#})?;
        let mut permissions = fs_err::metadata(wrapper.path())?.permissions();
        permissions.set_mode(0o755);
        fs_err::set_permissions(wrapper.path(), permissions)?;

        let child = command
            .arg("--no-config")
            .env("UV_TEST_REAL_INSTALL_NAME_TOOL", real_tool)
            .env("UV_TEST_INSTALL_NAME_STARTED", &started)
            .env("UV_TEST_INSTALL_NAME_RELEASE", &release)
            .env("UV_TEST_INSTALL_NAME_CALLS", &calls)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        Ok(Self {
            child: Some(child),
            started,
            release,
            calls,
        })
    }

    fn wait_until_paused(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !self.started.is_file() {
            if self
                .child
                .as_mut()
                .context("installation process is missing")?
                .try_wait()?
                .is_some()
            {
                let output = self
                    .child
                    .take()
                    .context("installation process is missing")?
                    .wait_with_output()?;
                bail!("installation exited before finalization: {output:?}");
            }
            ensure!(
                Instant::now() < deadline,
                "timed out waiting for finalization"
            );
            thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    fn calls(&self) -> Result<Vec<InstallNameCall>> {
        let contents = fs_err::read(&self.calls)?;
        let fields = contents.split(|byte| *byte == 0).collect::<Vec<_>>();
        let (last, fields) = fields.split_last().context("missing tool arguments")?;
        let (calls, remainder) = fields.as_chunks::<3>();
        ensure!(
            last.is_empty() && remainder.is_empty(),
            "incomplete `install_name_tool` arguments"
        );
        calls
            .iter()
            .map(|args| {
                ensure!(
                    args[0] == b"-id",
                    "unexpected `install_name_tool` arguments"
                );
                Ok(InstallNameCall {
                    install_name: PathBuf::from(OsString::from_vec(args[1].to_vec())),
                    dylib: PathBuf::from(OsString::from_vec(args[2].to_vec())),
                })
            })
            .collect()
    }

    fn finish(&mut self) -> Result<Output> {
        fs_err::write(&self.release, b"")?;
        Ok(self
            .child
            .take()
            .context("installation process is missing")?
            .wait_with_output()?)
    }
}

impl Drop for InstallNameToolBarrier {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = fs_err::write(&self.release, b"");
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn assert_private_until_finalized(context: &TestContext, command: Command) -> Result<()> {
    let installation = context
        .temp_dir
        .child("managed")
        .child(format!("cpython-3.13.1-{}", platform_key_from_env()?));
    let dylib = installation.child("lib/libpython3.13.dylib");
    let scratch = context.temp_dir.child("managed/.temp");
    let mut barrier = InstallNameToolBarrier::spawn(context, command)?;
    barrier.wait_until_paused()?;

    // Discovery is a separate uv process and does not take the installation lock.
    uv_snapshot!(context.filters(), context.python_find()
        .args(["--no-config", "--offline", "--system", "--managed-python", "--no-python-downloads", "--show-version", "3.13.1"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.13.1 in managed installations
    ");
    assert_eq!(
        fs_err::symlink_metadata(installation.path())
            .expect_err("the installation must not be published before finalization")
            .kind(),
        std::io::ErrorKind::NotFound,
    );

    let calls = barrier.calls()?;
    let [call] = calls.as_slice() else {
        bail!("expected one paused dylib update, got {calls:?}");
    };
    assert_eq!(call.install_name, dylib.path());
    assert!(call.dylib.starts_with(scratch.path()), "{call:?}");
    assert!(call.dylib.is_file(), "{call:?}");

    barrier.finish()?.assert().success();
    let calls = barrier.calls()?;
    assert_eq!(
        calls.len(),
        1,
        "metadata was rewritten after publication: {calls:?}"
    );

    uv_snapshot!(context.filters(), context.python_find()
        .args(["--no-config", "--offline", "--system", "--managed-python", "--no-python-downloads", "--show-version", "3.13.1"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.13.1
    ");

    uv_snapshot!(context.filters(), Command::new("otool").arg("-D").arg(dylib.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    [TEMP_DIR]/managed/cpython-3.13.1-[PLATFORM]/lib/libpython3.13.dylib:
    [TEMP_DIR]/managed/cpython-3.13.1-[PLATFORM]/lib/libpython3.13.dylib
    ");

    let venv = context.temp_dir.child("after-publication");
    context
        .venv()
        .arg(venv.path())
        .args([
            "--no-config",
            "--offline",
            "--python",
            "3.13.1",
            "--managed-python",
            "--no-python-downloads",
        ])
        .assert()
        .success();
    let probe = indoc! {r"
        import pathlib
        import sys
        import sysconfig

        print(sys.version.split()[0])
        print(sysconfig.get_config_var('LIBDIR'))
        print((pathlib.Path(sysconfig.get_path('stdlib')) / 'EXTERNALLY-MANAGED').is_file())
        print((pathlib.Path(sys.base_prefix) / 'BUILD').is_file())
    "};
    uv_snapshot!(context.filters(), Command::new(venv_bin_path(venv.path()).join("python"))
        .args(["-I", "-c", probe]), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.13.1
    [TEMP_DIR]/managed/cpython-3.13.1-[PLATFORM]/lib
    True
    True
    ");
    Ok(())
}

#[test]
fn explicit_install_is_private_until_finalized() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_managed_python_dirs()
        .with_filtered_python_keys();
    let mut command = context.python_install();
    command.arg("3.13.1");
    allow_duplicates! {
        assert_private_until_finalized(&context, command)
    }
}

#[test]
fn automatic_install_is_private_until_finalized() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_managed_python_dirs()
        .with_filtered_python_keys();
    let mut command = context.venv();
    command.args(["--python", "3.13.1", "--managed-python"]);
    allow_duplicates! {
        assert_private_until_finalized(&context, command)
    }
}
