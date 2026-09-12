use std::env;
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use uv_cli::ExternalCommand;

use super::{ParsedRunCommand, RunCommand};

const CHILD_CASE: &str = "UV_INTERNAL_TEST_MODULE_STDIN_CASE";
const CHILD_TEST: &str = "commands::project::run::module_stdin_tests::stdin_parser_child";
const READY: &[u8] = b"module-stdin-ready\n";
const CONTENTS: &[u8] = b"authored\0\xff\r\n";

fn wait_for_exit(child: &mut std::process::Child) -> io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn run_child(case: &str, contents: Option<&[u8]>) -> Result<()> {
    let mut command = Command::new(env::current_exe()?);
    command.args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"]);
    if cfg!(panic = "abort") {
        // The abort-mode harness otherwise spawns another process with null stdin.
        command.args(["-Z", "unstable-options", "--force-run-in-process"]);
    }
    let mut child = command
        .env(CHILD_CASE, case)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let Some(stdout) = child.stdout.take() else {
        child.kill()?;
        child.wait()?;
        bail!("missing parser child stdout");
    };
    let (ready_sender, ready_receiver) = mpsc::channel();
    let reader = thread::spawn(move || -> io::Result<Vec<u8>> {
        let mut reader = BufReader::new(stdout);
        let mut output = Vec::new();
        let mut line = Vec::new();
        while reader.read_until(b'\n', &mut line)? != 0 {
            if line == READY {
                let _ = ready_sender.send(());
            }
            output.extend_from_slice(&line);
            line.clear();
        }
        Ok(output)
    });

    let result = (|| -> Result<Option<ExitStatus>> {
        ready_receiver
            .recv_timeout(Duration::from_secs(30))
            .context("parser child did not become ready")?;
        if let Some(contents) = contents {
            child
                .stdin
                .as_mut()
                .context("missing parser child stdin")?
                .write_all(contents)?;
            drop(child.stdin.take());
        }
        wait_for_exit(&mut child).map_err(Into::into)
    })();
    if !matches!(&result, Ok(Some(_))) {
        let _ = child.kill();
    }
    drop(child.stdin.take());
    let output = child.wait_with_output()?;
    let stdout = reader
        .join()
        .map_err(|_| anyhow!("parser child stdout reader panicked"))??;
    let message = if contents.is_none() {
        "module/stdin rejection waited for EOF"
    } else {
        "stdin parser child did not finish"
    };
    let status = result?.context(message)?;
    ensure!(
        status.success(),
        "parser child {case} failed:\n{}{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
fn rejects_module_stdin_before_eof() -> Result<()> {
    for case in ["module", "module-script", "module-gui", "module-script-gui"] {
        run_child(case, None)?;
    }
    Ok(())
}

#[test]
fn preserves_plain_and_gui_stdin() -> Result<()> {
    for (case, contents) in [
        ("plain-empty", b"".as_slice()),
        ("plain", CONTENTS),
        ("gui", CONTENTS),
    ] {
        run_child(case, Some(contents))?;
    }
    Ok(())
}

#[test]
fn stdin_parser_child() -> Result<()> {
    let Ok(case) = env::var(CHILD_CASE) else {
        return Ok(());
    };
    let (module, script, gui_script) = match case.as_str() {
        "module" => (true, false, false),
        "module-script" => (true, true, false),
        "module-gui" => (true, false, true),
        "module-script-gui" => (true, true, true),
        "plain-empty" | "plain" => (false, false, false),
        "gui" => (false, false, true),
        _ => bail!("unknown parser child case: {case}"),
    };
    let arguments = vec!["--inert".into(), "β".into()];
    let mut command = vec!["-".into()];
    command.extend(arguments.iter().cloned());
    let command = ExternalCommand::Cmd(command);
    {
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "\nmodule-stdin-ready")?;
        stdout.flush()?;
    }

    let parsed = ParsedRunCommand::from_args(&command, module, script, gui_script);
    if module {
        let Err(error) = parsed else {
            bail!("module/stdin unexpectedly parsed");
        };
        assert_eq!(error.to_string(), "Cannot run a Python module from stdin");
        return Ok(());
    }
    let (contents, actual_arguments) = match parsed? {
        ParsedRunCommand::Ready(RunCommand::PythonStdin(contents, arguments)) if !gui_script => {
            (contents, arguments)
        }
        ParsedRunCommand::Ready(RunCommand::PythonGuiStdin(contents, arguments)) if gui_script => {
            (contents, arguments)
        }
        command => bail!("unexpected parsed command: {command:?}"),
    };
    let expected = if case == "plain-empty" { b"" } else { CONTENTS };
    assert_eq!(contents, expected);
    assert_eq!(actual_arguments, arguments);
    Ok(())
}
