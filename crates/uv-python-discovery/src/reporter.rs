//! Reporting for Python discovery and managed downloads.

use std::fmt::{self, Write};

use crate::PythonInstallation;
use indicatif::{MultiProgress, ProgressBar};
use owo_colors::OwoColorize;
use uv_command_support::Printer;
use uv_command_support::progress::{Direction, ProgressReporter};
use uv_fs::Simplified;
use uv_python_types::PythonInstallationKey;

#[derive(Debug)]
pub struct PythonDownloadReporter {
    reporter: ProgressReporter,
}

impl PythonDownloadReporter {
    /// Initialize a [`PythonDownloadReporter`] for a single Python download.
    pub fn single(printer: Printer) -> Self {
        Self::new(printer, None)
    }

    /// Initialize a [`PythonDownloadReporter`] for multiple Python downloads.
    pub fn new(printer: Printer, length: Option<u64>) -> Self {
        let multi_progress = MultiProgress::with_draw_target(printer.target());
        let root = multi_progress.add(ProgressBar::with_draw_target(length, printer.target()));
        let reporter = ProgressReporter::new(root, multi_progress, printer);
        Self { reporter }
    }
}

impl uv_python_managed::downloads::Reporter for PythonDownloadReporter {
    fn on_request_start(
        &self,
        direction: uv_python_managed::downloads::Direction,
        name: &PythonInstallationKey,
        size: Option<u64>,
    ) -> usize {
        self.reporter.on_request_start(
            progress_direction(direction),
            format!("{name} ({direction})"),
            size,
        )
    }

    fn on_request_progress(&self, id: usize, inc: u64) {
        self.reporter.on_request_progress(id, inc);
    }

    fn on_request_complete(&self, direction: uv_python_managed::downloads::Direction, id: usize) {
        self.reporter
            .on_request_complete(progress_direction(direction), id);
    }
}

/// Map Python's download phases to the shared progress renderer.
fn progress_direction(direction: uv_python_managed::downloads::Direction) -> Direction {
    match direction {
        uv_python_managed::downloads::Direction::Download => Direction::Download,
        uv_python_managed::downloads::Direction::Extract => Direction::Extract,
    }
}

/// Display a message about the interpreter that was selected for the operation.
pub fn report_interpreter(
    python: &PythonInstallation,
    dimmed: bool,
    printer: Printer,
) -> fmt::Result {
    let managed = python.source().is_managed();
    let implementation = python.implementation();
    let interpreter = python.interpreter();

    let message = fmt::from_fn(|formatter| {
        write!(formatter, "Using {} ", implementation.pretty())?;
        let version = interpreter.python_version();
        let suffix = interpreter.variant().display_suffix();
        if managed && !dimmed {
            write!(formatter, "{}{}", version.cyan(), suffix.cyan())?;
        } else {
            write!(formatter, "{version}{suffix}")?;
        }
        if !managed {
            let executable = interpreter.sys_executable();
            let executable = executable.user_display();
            if dimmed {
                write!(formatter, " interpreter at: {executable}")?;
            } else {
                write!(formatter, " interpreter at: {}", executable.cyan())?;
            }
        }
        Ok(())
    });

    if dimmed {
        writeln!(printer.stderr(), "{}", message.dimmed())
    } else {
        writeln!(printer.stderr(), "{message}")
    }
}
