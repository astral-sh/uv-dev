use anstream::{eprint, print};
use indicatif::ProgressDrawTarget;
use serde::Serialize;

/// A single record in a command's preview JSONL stream.
#[derive(Debug, Serialize)]
#[serde(untagged)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub(crate) enum JsonlRecord<P, R> {
    Progress(P),
    Result(R),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
enum ResultType {
    Result,
}

/// An object-valued command result with the JSONL discriminator.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub(crate) struct JsonlResult<T> {
    /// Distinguishes the final command result from progress updates.
    #[serde(rename = "type")]
    event_type: ResultType,
    #[serde(flatten)]
    result: T,
}

/// An array-valued command result with the JSONL discriminator.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
struct JsonlResultData<T> {
    /// Distinguishes the final command result from progress updates.
    #[serde(rename = "type")]
    event_type: ResultType,
    data: T,
}

/// Serialize an object-valued command result as a JSONL event.
pub(crate) fn jsonl_result<T: Serialize>(result: &T) -> serde_json::Result<String> {
    serde_json::to_string(&JsonlRecord::<(), _>::Result(JsonlResult {
        event_type: ResultType::Result,
        result,
    }))
}

/// Serialize an array-valued command result as a JSONL event.
pub(crate) fn jsonl_result_data<T: Serialize>(result: &T) -> serde_json::Result<String> {
    serde_json::to_string(&JsonlRecord::<(), _>::Result(JsonlResultData {
        event_type: ResultType::Result,
        data: result,
    }))
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

    /// Return whether this printer suppresses progress output.
    pub(crate) const fn suppresses_progress(self) -> bool {
        match self {
            Self::Silent => true,
            Self::Quiet => true,
            Self::Default => false,
            // Confusingly, hide the progress bar when in verbose mode.
            // Otherwise, it gets interleaved with debug messages.
            Self::Verbose => true,
            Self::NoProgress => true,
            Self::Jsonl => true,
        }
    }

    /// Return the [`ProgressDrawTarget`] for this printer.
    pub(crate) fn target(self) -> ProgressDrawTarget {
        if self.suppresses_progress() {
            ProgressDrawTarget::hidden()
        } else {
            ProgressDrawTarget::stderr()
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

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use super::{jsonl_result, jsonl_result_data};

    #[test]
    fn jsonl_result_envelopes_keep_record_shape() -> serde_json::Result<()> {
        #[derive(Serialize)]
        struct Report<'a> {
            message: &'a str,
            values: &'a [u8],
        }

        let report = Report {
            message: "first\nsecond",
            values: &[1, 2],
        };
        assert_eq!(
            jsonl_result(&report)?,
            r#"{"type":"result","message":"first\nsecond","values":[1,2]}"#
        );
        assert_eq!(
            jsonl_result_data(&[report])?,
            r#"{"type":"result","data":[{"message":"first\nsecond","values":[1,2]}]}"#
        );
        Ok(())
    }
}
