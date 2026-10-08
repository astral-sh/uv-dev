use std::io;
use std::path::Path;

/// Vacate a running export before releasing its executable-directory lock.
pub(super) fn remove_running_entrypoint(path: &Path) -> io::Result<()> {
    // The unprotected fallback can defer deletion of the original name when the temporary
    // directory is on another filesystem. Relocate beside the export instead, so delayed cleanup
    // targets a unique old filename and cannot remove a later installation's executable.
    self_replace::self_delete_outside_path(path)
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::io::{self, Read, Write};
    use std::process::Stdio;
    use std::time::Duration;

    use anyhow::{Context, Result, bail, ensure};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::process::Command;
    use tokio::time::{sleep, timeout};

    use super::remove_running_entrypoint;

    const CHILD: &str = "UV_TEST_SELF_REMOVAL_CHILD";
    const READY: &str = "self-removal-ready";
    const DEADLINE: Duration = Duration::from_secs(30);
    const TEST: &str =
        "commands::tool::self_removal::tests::fallback_vacates_export_before_process_exit";

    #[tokio::test]
    async fn fallback_vacates_export_before_process_exit() -> Result<()> {
        if env::var_os(CHILD).is_some() {
            // A file in place of the temporary directory forces the initial relocation to fail.
            let temporary = env::temp_dir().components().collect::<std::path::PathBuf>();
            ensure!(fs_err::metadata(temporary)?.is_file());
            remove_running_entrypoint(&env::current_exe()?)?;
            writeln!(io::stdout(), "{READY}")?;
            io::stdout().flush()?;
            // The parent publishes a replacement before releasing this process. EOF also releases
            // the child if its parent goes away; the parent supervises the wait with a deadline.
            io::stdin().read_exact(&mut [0])?;
            // Use ordinary subprocess success when libtest invokes this fixture directly through
            // its panic-abort entry point.
            #[expect(clippy::exit)]
            std::process::exit(0);
        }

        let directory = tempfile::tempdir()?;
        let bin = directory.path().join("bin");
        fs_err::create_dir(&bin)?;
        let exported = bin.join("running-tool.exe");
        fs_err::copy(env::current_exe()?, &exported)?;
        let blocked_temp = directory.path().join("not-a-directory");
        fs_err::write(&blocked_temp, "block temporary relocation")?;

        let mut child = Command::new(&exported)
            .args([TEST, "--exact", "--nocapture", "--test-threads=1"])
            // With panic-abort, libtest otherwise adds an intermediate process that would own the
            // copied executable and outlive the fixture's readiness message.
            .env("__RUST_TEST_INVOKE", TEST)
            .env(CHILD, "1")
            .env("TEMP", &blocked_temp)
            .env("TMP", &blocked_temp)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let mut stdout = BufReader::new(child.stdout.take().context("child stdout missing")?);
        let mut captured = String::new();
        timeout(DEADLINE, async {
            loop {
                let mut line = String::new();
                if stdout.read_line(&mut line).await? == 0 {
                    bail!("child exited before self-removal readiness: {captured}");
                }
                let ready = line.trim_end().ends_with(READY);
                captured.push_str(&line);
                if ready {
                    return Ok::<_, anyhow::Error>(());
                }
            }
        })
        .await
        .context("child did not reach self-removal readiness")??;

        // Return errors while the child is alive so its kill-on-drop guard also runs with
        // panic-abort test harnesses.
        ensure!(
            !exported.exists(),
            "running export was not vacated before readiness"
        );
        fs_err::write(&exported, "replacement export")?;
        child
            .stdin
            .take()
            .context("child stdin missing")?
            .write_all(b"x")
            .await?;
        let output = timeout(DEADLINE, child.wait_with_output())
            .await
            .context("child did not exit after release")??;
        ensure!(
            output.status.success(),
            "self-removal child failed: {output:?}"
        );

        // The helper removes the relocated executable, then its own executable. Wait for those
        // actual cleanup effects before checking the replacement, rather than just the child exit.
        timeout(DEADLINE, async {
            loop {
                let remaining = fs_err::read_dir(&bin)?
                    .map(|entry| entry.map(|entry| entry.path()))
                    .collect::<io::Result<Vec<_>>>()?;
                if remaining.iter().all(|path| *path == exported) {
                    return Ok::<_, io::Error>(());
                }
                sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .context("deferred self-removal cleanup did not finish")??;
        ensure!(fs_err::read(&exported)? == b"replacement export");
        Ok(())
    }
}
