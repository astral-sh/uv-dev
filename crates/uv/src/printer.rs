use anstream::{eprint, print};
use indicatif::ProgressDrawTarget;
use serde::Serialize;

/// Serialize an object-valued command result as a JSONL event.
pub(crate) fn jsonl_result<T: Serialize>(result: &T) -> serde_json::Result<String> {
    #[derive(Serialize)]
    struct ResultEvent<'a, T> {
        #[serde(rename = "type")]
        event_type: &'static str,
        #[serde(flatten)]
        result: &'a T,
    }

    serde_json::to_string(&ResultEvent {
        event_type: "result",
        result,
    })
}

/// Serialize an array-valued command result as a JSONL event.
pub(crate) fn jsonl_result_data<T: Serialize>(result: &T) -> serde_json::Result<String> {
    #[derive(Serialize)]
    struct ResultEvent<'a, T> {
        #[serde(rename = "type")]
        event_type: &'static str,
        data: &'a T,
    }

    serde_json::to_string(&ResultEvent {
        event_type: "result",
        data: result,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Printer {
    /// A printer that suppresses all output.
    Silent,
    /// A printer that suppresses most output, but preserves "important" stdout.
    Quiet,
    /// A printer that prints to standard streams (e.g., stdout).
    Default,
    /// A printer that prints all output, including debug messages.
    Verbose,
    /// A printer that prints to standard streams, excluding all progress outputs
    NoProgress,
    /// A printer that streams progress updates to stdout as newline-delimited JSON.
    Jsonl,
}

impl Printer {
    /// Create a printer from the global output settings.
    pub(crate) fn new(quiet: u8, verbose: u8, no_progress: bool) -> Self {
        if quiet == 1 {
            Self::Quiet
        } else if quiet > 1 {
            Self::Silent
        } else if verbose > 0 {
            Self::Verbose
        } else if no_progress {
            Self::NoProgress
        } else {
            Self::Default
        }
    }

    /// Enable structured progress unless progress output has been explicitly suppressed.
    pub(crate) fn with_jsonl_progress(self) -> Self {
        match self {
            Self::Default | Self::Verbose | Self::Jsonl => Self::Jsonl,
            Self::Silent | Self::Quiet | Self::NoProgress => self,
        }
    }

    /// Whether progress updates should be streamed as newline-delimited JSON.
    pub(crate) fn emits_jsonl_progress(self) -> bool {
        matches!(self, Self::Jsonl)
    }

    /// Return the [`ProgressDrawTarget`] for this printer.
    pub(crate) fn target(self) -> ProgressDrawTarget {
        match self {
            Self::Silent => ProgressDrawTarget::hidden(),
            Self::Quiet => ProgressDrawTarget::hidden(),
            Self::Default => ProgressDrawTarget::stderr(),
            // Confusingly, hide the progress bar when in verbose mode.
            // Otherwise, it gets interleaved with debug messages.
            Self::Verbose => ProgressDrawTarget::hidden(),
            Self::NoProgress => ProgressDrawTarget::hidden(),
            Self::Jsonl => ProgressDrawTarget::hidden(),
        }
    }

    /// Return the [`Stdout`] for this printer.
    #[allow(dead_code, reason = "to be adopted incrementally")]
    pub(crate) fn stdout_important(self) -> Stdout {
        match self {
            Self::Silent => Stdout::Disabled,
            Self::Quiet => Stdout::Enabled,
            Self::Default => Stdout::Enabled,
            Self::Verbose => Stdout::Enabled,
            Self::NoProgress => Stdout::Enabled,
            Self::Jsonl => Stdout::Enabled,
        }
    }

    /// Return the [`Stdout`] for this printer.
    pub(crate) fn stdout(self) -> Stdout {
        match self {
            Self::Silent => Stdout::Disabled,
            Self::Quiet => Stdout::Disabled,
            Self::Default => Stdout::Enabled,
            Self::Verbose => Stdout::Enabled,
            Self::NoProgress => Stdout::Enabled,
            Self::Jsonl => Stdout::Enabled,
        }
    }

    /// Return the [`Stderr`] for this printer.
    #[allow(dead_code)] // Only used with the optional self-update feature.
    pub(crate) fn stderr_important(self) -> Stderr {
        match self {
            Self::Silent => Stderr::Disabled,
            Self::Quiet => Stderr::Enabled,
            Self::Default => Stderr::Enabled,
            Self::Verbose => Stderr::Enabled,
            Self::NoProgress => Stderr::Enabled,
            Self::Jsonl => Stderr::Enabled,
        }
    }

    /// Return the [`Stderr`] for this printer.
    pub(crate) fn stderr(self) -> Stderr {
        match self {
            Self::Silent => Stderr::Disabled,
            Self::Quiet => Stderr::Disabled,
            Self::Default => Stderr::Enabled,
            Self::Verbose => Stderr::Enabled,
            Self::NoProgress => Stderr::Enabled,
            Self::Jsonl => Stderr::Enabled,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stdout {
    Enabled,
    Disabled,
}

impl std::fmt::Write for Stdout {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        match self {
            Self::Enabled => {
                print!("{s}");
            }
            Self::Disabled => {}
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stderr {
    Enabled,
    Disabled,
}

impl std::fmt::Write for Stderr {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        match self {
            Self::Enabled => {
                eprint!("{s}");
            }
            Self::Disabled => {}
        }

        Ok(())
    }
}
