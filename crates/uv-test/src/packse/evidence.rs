//! Command and lockfile artifacts for replaying scenario failures.

use std::io::{ErrorKind, Read};
use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Context, Result};
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
    fs_err::write(directory.join("stdout.txt"), &output.stdout)?;
    fs_err::write(directory.join("stderr.txt"), &output.stderr)?;

    let mut executable = fs_err::File::open(command.get_program())?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = executable.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    fs_err::write(
        directory.join("command.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "program": command.get_program().to_string_lossy(),
            "sha256": hex::encode(digest.finalize()),
            "args": command.get_args().map(|arg| arg.to_string_lossy()).collect::<Vec<_>>(),
            "current_dir": command.get_current_dir().map(|path| path.to_string_lossy()),
            "status": output.status.code(),
        }))?,
    )?;
    Ok(())
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
        self.commands.push(LockCommand {
            label,
            command,
            output: output.clone(),
            lockfile: None,
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
        Ok(output)
    }

    pub(super) fn write(&self, directory: &Path) -> Result<()> {
        let commands_dir = directory.join("commands");
        fs_err::create_dir(&commands_dir)?;
        for (index, record) in self.commands.iter().enumerate() {
            let command_dir = commands_dir.join(format!("{:02}-{}", index + 1, record.label));
            fs_err::create_dir(&command_dir)?;
            write_command(&command_dir, &record.command, &record.output)?;
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
}
