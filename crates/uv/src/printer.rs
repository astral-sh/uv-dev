use anstream::{eprint, print};
use indicatif::ProgressDrawTarget;
use uv_cli::ErrorFormat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrinterMode {
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Printer {
    mode: PrinterMode,
    error_format: ErrorFormat,
}

impl Printer {
    /// Create a printer from the global output settings.
    pub(crate) fn new(quiet: u8, verbose: u8, no_progress: bool) -> Self {
        let mode = if quiet == 1 {
            PrinterMode::Quiet
        } else if quiet > 1 {
            PrinterMode::Silent
        } else if verbose > 0 {
            PrinterMode::Verbose
        } else if no_progress {
            PrinterMode::NoProgress
        } else {
            PrinterMode::Default
        };
        Self {
            mode,
            error_format: ErrorFormat::Text,
        }
    }

    /// Suppress every output stream.
    pub(crate) fn silent() -> Self {
        Self::new(2, 0, true)
    }

    /// Select the representation used for error reports.
    pub(crate) fn with_error_format(mut self, format: ErrorFormat) -> Self {
        self.error_format = format;
        self
    }

    pub(crate) fn error_format(self) -> uv_errors::ErrorFormat {
        match self.error_format {
            ErrorFormat::Text => uv_errors::ErrorFormat::Text,
            ErrorFormat::Json => uv_errors::ErrorFormat::Json,
        }
    }

    pub(crate) fn is_quiet(self) -> bool {
        matches!(self.mode, PrinterMode::Quiet | PrinterMode::Silent)
    }

    /// Return whether this printer suppresses progress output.
    pub(crate) const fn suppresses_progress(self) -> bool {
        match self.mode {
            PrinterMode::Silent => true,
            PrinterMode::Quiet => true,
            PrinterMode::Default => false,
            // Confusingly, hide the progress bar when in verbose mode.
            // Otherwise, it gets interleaved with debug messages.
            PrinterMode::Verbose => true,
            PrinterMode::NoProgress => true,
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
        match self.mode {
            PrinterMode::Silent => Stdout::Disabled,
            PrinterMode::Quiet => Stdout::Enabled,
            PrinterMode::Default => Stdout::Enabled,
            PrinterMode::Verbose => Stdout::Enabled,
            PrinterMode::NoProgress => Stdout::Enabled,
        }
    }

    /// Return the [`Stdout`] for this printer.
    pub(crate) fn stdout(self) -> Stdout {
        match self.mode {
            PrinterMode::Silent => Stdout::Disabled,
            PrinterMode::Quiet => Stdout::Disabled,
            PrinterMode::Default => Stdout::Enabled,
            PrinterMode::Verbose => Stdout::Enabled,
            PrinterMode::NoProgress => Stdout::Enabled,
        }
    }

    /// Return the [`Stderr`] for this printer.
    pub(crate) fn stderr_important(self) -> Stderr {
        match self.mode {
            PrinterMode::Silent => Stderr::Disabled,
            PrinterMode::Quiet => Stderr::Enabled,
            PrinterMode::Default => Stderr::Enabled,
            PrinterMode::Verbose => Stderr::Enabled,
            PrinterMode::NoProgress => Stderr::Enabled,
        }
    }

    /// Return the [`Stderr`] for this printer.
    pub(crate) fn stderr(self) -> Stderr {
        match self.mode {
            PrinterMode::Silent => Stderr::Disabled,
            PrinterMode::Quiet => Stderr::Disabled,
            PrinterMode::Default => Stderr::Enabled,
            PrinterMode::Verbose => Stderr::Enabled,
            PrinterMode::NoProgress => Stderr::Enabled,
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
