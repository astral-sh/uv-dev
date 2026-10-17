#![cfg_attr(windows, windows_subsystem = "windows")]

use std::convert::Infallible;
use std::path::Path;
use std::process::{Command, ExitCode, ExitStatus};

mod launcher;

use launcher::get_uv_path;

/// Spawns a command exec style.
fn exec_spawn(cmd: &mut Command) -> std::io::Result<Infallible> {
    cfg_select! {
        unix => {
            use std::os::unix::process::CommandExt;
            let err = cmd.exec();
            Err(err)
        },
        windows => uv_windows::spawn_child(cmd, true),
    }
}

/// Assuming the binary is called something like `uvw@1.2.3(.exe)`, compute the `@1.2.3(.exe)` part
/// so that we can preferentially find `uv@1.2.3(.exe)`, for folks who like managing multiple
/// installs in this way.
fn get_uvw_suffix(current_exe: &Path) -> Option<&str> {
    let os_file_name = current_exe.file_name()?;
    let file_name_str = os_file_name.to_str()?;
    file_name_str.strip_prefix("uvw")
}

fn run() -> std::io::Result<ExitStatus> {
    let current_exe = std::env::current_exe()?;
    let Some(bin) = current_exe.parent() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Could not determine the location of the `uvw` binary",
        ));
    };
    let uvw_suffix = get_uvw_suffix(&current_exe);
    let uv = get_uv_path(bin, uvw_suffix)?;
    let args = std::env::args_os()
        // Skip the `uvw` name
        .skip(1)
        .collect::<Vec<_>>();

    let mut cmd = Command::new(uv);
    cmd.args(&args);
    match exec_spawn(&mut cmd)? {}
}

#[expect(clippy::print_stderr)]
fn main() -> ExitCode {
    let result = run();
    match result {
        // Fail with 2 if the status cannot be cast to an exit code
        Ok(status) => u8::try_from(status.code().unwrap_or(2)).unwrap_or(2).into(),
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(2)
        }
    }
}
