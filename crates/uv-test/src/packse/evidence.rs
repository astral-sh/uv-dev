//! Command and lockfile artifacts for replaying scenario failures.

use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use anyhow::{Context, Result, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub(super) fn create_directory(directory: &Path) -> Result<()> {
    if let Some(parent) = directory
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs_err::create_dir_all(parent)?;
    }
    fs_err::create_dir(directory)?;
    Ok(())
}

pub(super) fn write_command(directory: &Path, command: &Command, output: &Output) -> Result<()> {
    write_command_with_identity(directory, command, output, None)
}

fn write_command_with_identity(
    directory: &Path,
    command: &Command,
    output: &Output,
    identity: Option<&ProcessIdentity>,
) -> Result<()> {
    fs_err::write(directory.join("stdout.txt"), &output.stdout)?;
    fs_err::write(directory.join("stderr.txt"), &output.stderr)?;
    let digest = match identity {
        Some(identity) => identity.before_sha256.clone(),
        None => hash_executable(Path::new(command.get_program()))?,
    };
    let mut record = serde_json::json!({
        "program": command.get_program().to_string_lossy(),
        "sha256": digest,
        "args": command.get_args().map(|arg| arg.to_string_lossy()).collect::<Vec<_>>(),
        "current_dir": command.get_current_dir().map(|path| path.to_string_lossy()),
        "status": output.status.code(),
    });
    if let Some(identity) = identity {
        record["process_identity"] = serde_json::to_value(identity)?;
    }
    fs_err::write(
        directory.join("command.json"),
        serde_json::to_vec_pretty(&record)?,
    )?;
    Ok(())
}

pub(super) fn hash_executable(path: &Path) -> Result<String> {
    let mut executable = fs_err::File::open(path)?;
    ensure!(
        executable.metadata()?.is_file(),
        "the executable is not a regular file"
    );
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = executable.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

/// Executable and child identity observed at the actual spawn, not when artifacts are written.
#[derive(Clone, Debug, Serialize)]
pub(super) struct ProcessIdentity {
    pub(super) executable: PathBuf,
    pub(super) producer_pid: u32,
    pub(super) before_sha256: String,
    pub(super) after_sha256: Option<String>,
    pub(super) after_error: Option<String>,
}

impl ProcessIdentity {
    pub(super) fn verify(&self) -> Result<()> {
        ensure!(self.producer_pid != 0, "the child PID is invalid");
        ensure!(
            self.after_sha256.as_ref() == Some(&self.before_sha256) && self.after_error.is_none(),
            "the selected executable changed during the lock command"
        );
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct LockTrace {
    commands: Vec<LockCommand>,
}

struct LockCommand {
    label: &'static str,
    command: Command,
    output: Output,
    lockfile: Option<Vec<u8>>,
    identity: Option<ProcessIdentity>,
}

impl LockTrace {
    pub(super) fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Run a real command and retain the resulting lockfile, including after failed commands.
    pub(super) fn run(
        &mut self,
        label: &'static str,
        mut command: Command,
        lock_path: &Path,
    ) -> Result<Output> {
        let output = command
            .output()
            .with_context(|| format!("failed to run scenario command `{label}`"))?;
        self.record(label, command, output.clone(), lock_path, None)?;
        Ok(output)
    }

    /// Spawn a source-bound initial command and retain its actual PID and pre/post binary hashes.
    pub(super) fn run_bound(
        &mut self,
        label: &'static str,
        mut command: Command,
        lock_path: &Path,
        observe: impl FnOnce(ProcessIdentity, &Output) -> Result<()>,
    ) -> Result<Output> {
        let executable = fs_err::canonicalize(command.get_program())?;
        ensure!(
            command.get_program() == executable.as_os_str(),
            "the source-bound executable path is not canonical"
        );
        ensure!(
            fs_err::symlink_metadata(&executable)?.file_type().is_file(),
            "the source-bound executable is not a regular non-symlink file"
        );
        let before_sha256 = hash_executable(&executable)?;
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn scenario command `{label}`"))?;
        let producer_pid = child.id();
        let output = child
            .wait_with_output()
            .with_context(|| format!("failed to wait for scenario command `{label}`"))?;
        let after = (|| {
            ensure!(
                fs_err::symlink_metadata(&executable)?.file_type().is_file(),
                "the source-bound executable is no longer a regular non-symlink file"
            );
            hash_executable(&executable)
        })();
        let (after_sha256, after_error) = match after {
            Ok(digest) => (Some(digest), None),
            Err(error) => (None, Some(format!("{error:#}"))),
        };
        let identity = ProcessIdentity {
            executable,
            producer_pid,
            before_sha256,
            after_sha256,
            after_error,
        };
        // Retain completed-child observations before a separate lockfile read can fail. The
        // command trace is still recorded if the observation itself is rejected.
        let observation = observe(identity.clone(), &output);
        let snapshot = self.record(label, command, output.clone(), lock_path, Some(identity));
        observation?;
        snapshot?;
        Ok(output)
    }

    fn record(
        &mut self,
        label: &'static str,
        command: Command,
        output: Output,
        lock_path: &Path,
        identity: Option<ProcessIdentity>,
    ) -> Result<()> {
        self.commands.push(LockCommand {
            label,
            command,
            output,
            lockfile: None,
            identity,
        });
        let lockfile = match fs_err::read(lock_path) {
            Ok(contents) => Some(contents),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        self.commands
            .last_mut()
            .expect("the command was recorded")
            .lockfile = lockfile;
        Ok(())
    }

    pub(super) fn write(&self, directory: &Path) -> Result<()> {
        let commands_dir = directory.join("commands");
        fs_err::create_dir(&commands_dir)?;
        for (index, record) in self.commands.iter().enumerate() {
            let command_dir = commands_dir.join(format!("{:02}-{}", index + 1, record.label));
            fs_err::create_dir(&command_dir)?;
            write_command_with_identity(
                &command_dir,
                &record.command,
                &record.output,
                record.identity.as_ref(),
            )?;
            if let Some(lockfile) = &record.lockfile {
                fs_err::write(command_dir.join("uv.lock"), lockfile)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_each_command_and_lockfile() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let lock_path = directory.path().join("uv.lock");
        let executable = std::env::current_exe()?;
        let mut trace = LockTrace::default();
        for (label, contents) in [("first", "first lock"), ("second", "second lock")] {
            fs_err::write(&lock_path, contents)?;
            let mut command = Command::new(&executable);
            command.arg("--help");
            assert!(trace.run(label, command, &lock_path)?.status.success());
        }
        let evidence = directory.path().join("evidence");
        create_directory(&evidence)?;
        trace.write(&evidence)?;
        for (index, label, contents) in [(1, "first", "first lock"), (2, "second", "second lock")] {
            let command_dir = evidence.join(format!("commands/{index:02}-{label}"));
            assert_eq!(
                fs_err::read_to_string(command_dir.join("uv.lock"))?,
                contents
            );
            let command: serde_json::Value =
                serde_json::from_slice(&fs_err::read(command_dir.join("command.json"))?)?;
            assert_eq!(command["status"], 0);
            assert_eq!(command["args"], serde_json::json!(["--help"]));
            assert_eq!(command["sha256"].as_str().expect("binary digest").len(), 64);
        }
        assert!(create_directory(&evidence).is_err());
        Ok(())
    }

    #[test]
    fn bound_command_records_its_spawned_child_and_binary() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let executable = fs_err::canonicalize(std::env::current_exe()?)?;
        let expected_hash = hash_executable(&executable)?;
        let mut command = Command::new(&executable);
        command.arg("--help");
        let mut trace = LockTrace::default();
        let mut observed_identity = None;
        let output = trace.run_bound(
            "bound",
            command,
            &directory.path().join("uv.lock"),
            |identity, _| {
                observed_identity = Some(identity);
                Ok(())
            },
        )?;
        let identity = observed_identity.context("the child identity was observed")?;
        assert!(output.status.success());
        assert_ne!(identity.producer_pid, std::process::id());
        assert_eq!(identity.before_sha256, expected_hash);
        assert_eq!(identity.after_sha256.as_ref(), Some(&expected_hash));
        identity.verify()?;

        let evidence = directory.path().join("evidence");
        create_directory(&evidence)?;
        trace.write(&evidence)?;
        let command: serde_json::Value = serde_json::from_slice(&fs_err::read(
            evidence.join("commands/01-bound/command.json"),
        )?)?;
        assert_eq!(command["sha256"], expected_hash);
        assert_eq!(
            command["process_identity"],
            serde_json::to_value(&identity)?
        );

        let mut changed = identity.clone();
        changed.after_sha256 = Some("different".to_owned());
        assert!(changed.verify().is_err());
        let mut changed = identity.clone();
        changed.after_error = Some("unreadable".to_owned());
        assert!(changed.verify().is_err());
        let mut changed = identity;
        changed.producer_pid = 0;
        assert!(changed.verify().is_err());
        Ok(())
    }
}
