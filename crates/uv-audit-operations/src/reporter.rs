use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};
use uv_command_support::Printer;
use uv_command_support::progress::{JsonlProgressEvent, ProgressStatus, emit_jsonl_progress};

#[derive(Debug)]
pub(crate) struct AuditReporter {
    printer: Printer,
    progress: ProgressBar,
}

impl From<Printer> for AuditReporter {
    fn from(printer: Printer) -> Self {
        let progress = ProgressBar::with_draw_target(None, printer.target());
        progress.enable_steady_tick(Duration::from_millis(200));
        progress.set_style(
            ProgressStyle::with_template("{spinner:.white} {wide_msg:.dim}")
                .unwrap()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        progress.set_message("Auditing dependencies...");
        emit_jsonl_progress(
            printer,
            &JsonlProgressEvent::new("audit", ProgressStatus::Started),
        );
        Self { printer, progress }
    }
}

impl AuditReporter {
    pub(crate) fn on_audit_complete(&self) {
        self.progress.set_message("");
        emit_jsonl_progress(
            self.printer,
            &JsonlProgressEvent::new("audit", ProgressStatus::Completed),
        );
        self.progress.finish_and_clear();
    }
}
