//! Experimental daemon bootstrap. This runs before uv initializes process-global state.

use std::ffi::OsString;
use std::process::ExitCode;

use uv_cli::Cli;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix;

pub(crate) enum Bootstrap {
    Continue(Vec<OsString>),
    Exit(ExitCode),
}

pub(crate) fn bootstrap(args: Vec<OsString>) -> anyhow::Result<Bootstrap> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    return unix::bootstrap(args);

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        if args.len() == 2 {
            if args[1] == "--daemon" || args[1] == "--daemon-status" {
                anyhow::bail!("The experimental daemon is only available on Linux and macOS");
            }
            if args[1] == "--no-daemon" {
                return Ok(Bootstrap::Exit(ExitCode::SUCCESS));
            }
        }
        Ok(Bootstrap::Continue(args))
    }
}

pub(crate) fn dispatch(args: &[OsString], cli: &Cli) -> anyhow::Result<Option<ExitCode>> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    return unix::dispatch(args, cli);

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = args;
        if cli.top_level.global_args.daemon {
            anyhow::bail!("The experimental daemon is only available on Linux and macOS");
        }
        Ok(None)
    }
}
