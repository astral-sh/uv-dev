//! Construct a namespace's subcommands only when Clap builds that namespace.
//!
//! Each namespace flattens its command enum as an `Args` field. The enum's derived
//! `FromArgMatches` still parses the selected subcommand. Its `Args` implementation below
//! defers registering the children, so a child can shadow an inherited global argument.

use clap::{Args, Command, Subcommand};

use crate::{CacheCommand, PipCommand, PythonCommand, SelfCommand, ToolCommand, WorkspaceCommand};

macro_rules! deferred_subcommands {
    ($($command:ty),+ $(,)?) => {
        $(
            impl Args for $command {
                fn augment_args(command: Command) -> Command {
                    command
                        .defer(Self::augment_subcommands)
                        .subcommand_required(true)
                        .arg_required_else_help(true)
                }

                fn augment_args_for_update(command: Command) -> Command {
                    Self::augment_args(command)
                        .subcommand_required(false)
                        .arg_required_else_help(false)
                }
            }
        )+
    };
}

deferred_subcommands!(
    CacheCommand,
    PipCommand,
    PythonCommand,
    SelfCommand,
    ToolCommand,
    WorkspaceCommand,
);
