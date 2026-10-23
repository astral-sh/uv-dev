use std::fmt::{self, Write};

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use owo_colors::OwoColorize;
use uv_command_support::Printer;
use uv_command_support::progress::ProgressReporter;
use uv_console::human_readable_bytes;
use uv_distribution_filename::DistFilename;

#[derive(Debug)]
pub(super) struct PublishReporter {
    reporter: ProgressReporter,
    dry_run: bool,
}

impl PublishReporter {
    /// Initialize a [`PublishReporter`] for a single upload.
    pub(super) fn single(printer: Printer, dry_run: bool) -> Self {
        let multi_progress = MultiProgress::with_draw_target(printer.target());
        let root = multi_progress.add(ProgressBar::with_draw_target(None, printer.target()));
        let reporter = ProgressReporter::new(root, multi_progress, printer);
        Self { reporter, dry_run }
    }

    /// Finish the publication status after the registry has accepted or rejected the upload.
    pub(super) fn finish_upload(&self, uploaded: bool) {
        if !uploaded {
            self.reporter.root.finish_and_clear();
            return;
        }

        let message = self.reporter.root.message();
        let prefix = "Uploaded".bold().green().to_string();
        if self.reporter.should_write_progress() {
            let _ = writeln!(self.reporter.printer.stderr(), "{prefix} {message}");
        }
        self.reporter.root.set_prefix(prefix);
        self.reporter.root.finish_with_message(message);
    }
}

impl uv_publish::Reporter for PublishReporter {
    fn on_validation_start(&self, name: &DistFilename, size: u64) -> Result<(), fmt::Error> {
        let bytes = human_readable_bytes(size);
        if self.dry_run {
            writeln!(
                self.reporter.printer.stderr(),
                "{} {name} {}",
                "Checking".bold().cyan(),
                format!("({bytes:.1})").dimmed()
            )
        } else {
            writeln!(
                self.reporter.printer.stderr(),
                "{} {name} {}",
                "Hashing".bold().green(),
                format!("({bytes:.1})").dimmed()
            )
        }
    }

    fn on_upload_ready(&self, name: &DistFilename, size: u64) -> Result<(), fmt::Error> {
        let bytes = human_readable_bytes(size);
        let message = format!("{name} {}", format!("({bytes:.1})").dimmed());
        let prefix = "Uploading".bold().green().to_string();
        if self.reporter.should_write_progress() {
            writeln!(self.reporter.printer.stderr(), "{prefix} {message}")?;
        }
        self.reporter
            .root
            .set_style(ProgressStyle::with_template("{prefix} {wide_msg}").unwrap());
        self.reporter.root.set_prefix(prefix);
        self.reporter.root.set_message(message);
        Ok(())
    }

    fn on_progress(&self, _name: &str, id: usize) {
        self.reporter.on_download_complete(id);
    }

    fn on_upload_start(&self, name: &str, size: Option<u64>) -> usize {
        self.reporter.on_upload_start(name.to_string(), size)
    }

    fn on_upload_progress(&self, id: usize, inc: u64) {
        self.reporter.on_upload_progress(id, inc);
    }

    fn on_upload_complete(&self, id: usize) {
        self.reporter.on_upload_complete(id);
    }

    fn on_hash_start(&self, name: &DistFilename, size: Option<u64>) -> usize {
        self.reporter.on_hash_start(name.to_string(), size)
    }

    fn on_hash_progress(&self, id: usize, inc: u64) {
        self.reporter.on_hash_progress(id, inc);
    }

    fn on_hash_complete(&self, id: usize) {
        self.reporter.on_hash_complete(id);
    }
}

#[cfg(test)]
mod tests {
    use indicatif::ProgressDrawTarget;
    use uv_publish::Reporter as _;

    use super::*;

    #[test]
    fn publish_status_waits_for_registry_result() -> Result<(), fmt::Error> {
        let name =
            DistFilename::try_from_normalized_filename("publish_progress-1.0.0-py3-none-any.whl")
                .expect("valid wheel filename");

        for uploaded in [false, true] {
            let multi_progress = MultiProgress::with_draw_target(ProgressDrawTarget::hidden());
            let root = multi_progress.add(ProgressBar::hidden());
            let reporter = PublishReporter {
                reporter: ProgressReporter::new(
                    root.clone(),
                    multi_progress,
                    Printer::new(2, 0, true),
                ),
                dry_run: false,
            };

            reporter.on_upload_ready(&name, 2 * 1024 * 1024)?;
            assert_eq!(root.prefix(), "Uploading".bold().green().to_string());
            let message = root.message();
            assert_eq!(message, format!("{name} {}", "(2.0MiB)".dimmed()));

            let id = reporter.on_upload_start(&name.to_string(), Some(2 * 1024 * 1024));
            reporter.on_upload_progress(id, 17);
            reporter.on_upload_complete(id);
            assert!(!root.is_finished());
            assert_eq!(root.prefix(), "Uploading".bold().green().to_string());

            reporter.finish_upload(uploaded);
            assert!(root.is_finished());
            assert_eq!(root.message(), message);
            let verb = if uploaded { "Uploaded" } else { "Uploading" };
            assert_eq!(root.prefix(), verb.bold().green().to_string());
        }
        Ok(())
    }
}
