use std::ffi::OsString;

use uv_cli::Cli;

use crate::GlobalInitialization;
use crate::commands::ExitStatus;

tokio::task_local! {
    static ARGS: Vec<OsString>;
}

pub(crate) fn args() -> impl Iterator<Item = OsString> {
    ARGS.try_with(Clone::clone)
        .unwrap_or_else(|_| std::env::args_os().collect())
        .into_iter()
}

/// Execute a command with its own invocation arguments.
///
/// Other process-global state is still governed by [`GlobalInitialization`]. This is not yet an
/// API for concurrently executing commands with different process environments.
#[doc(hidden)]
pub(crate) async fn run_with_args(
    cli: Cli,
    global_initialization: GlobalInitialization,
    args: Vec<OsString>,
) -> anyhow::Result<ExitStatus> {
    ARGS.scope(args, Box::pin(crate::run(cli, global_initialization)))
        .await
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{ARGS, args};

    #[tokio::test]
    async fn invocation_arguments_are_task_local() {
        async fn read() -> Vec<OsString> {
            tokio::task::yield_now().await;
            args().collect::<Vec<_>>()
        }
        let first = vec![OsString::from("uv"), OsString::from("export")];
        let second = vec![OsString::from("uv"), OsString::from("pip")];
        let (actual_first, actual_second) = tokio::join!(
            ARGS.scope(first.clone(), read()),
            ARGS.scope(second.clone(), read()),
        );
        assert_eq!(actual_first, first);
        assert_eq!(actual_second, second);
    }
}
