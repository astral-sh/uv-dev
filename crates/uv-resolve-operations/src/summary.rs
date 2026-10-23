use std::fmt::{self, Write};

use owo_colors::OwoColorize;
use uv_command_support::{Printer, elapsed};

/// Whether to display a summary after resolving dependencies.
#[derive(Debug, Clone, Copy)]
pub enum ResolveSummary {
    /// Display the resolved package count and elapsed time.
    Display,
    /// Omit the resolution summary.
    Suppress,
}

impl ResolveSummary {
    /// Display the completion summary, if enabled.
    pub fn on_complete(
        self,
        count: usize,
        start: std::time::Instant,
        printer: Printer,
    ) -> fmt::Result {
        match self {
            Self::Suppress => Ok(()),
            Self::Display => {
                if count == 0 {
                    writeln!(
                        printer.stderr(),
                        "{}",
                        format!("Resolved in {}", elapsed(start.elapsed())).dimmed()
                    )
                } else {
                    let s = if count == 1 { "" } else { "s" };
                    writeln!(
                        printer.stderr(),
                        "{}",
                        format!(
                            "Resolved {} {}",
                            format!("{count} package{s}").bold(),
                            format!("in {}", elapsed(start.elapsed())).dimmed()
                        )
                        .dimmed()
                    )
                }
            }
        }
    }
}
