use std::sync::Mutex;
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use uv_command_support::{
    Printer,
    progress::{
        JsonlProgressEvent, ProgressPhase, ProgressReporter, ProgressStatus, emit_jsonl_progress,
    },
};
use uv_distribution_types::BuildableSource;
use uv_distribution_types::CachedDist;
use uv_normalize::PackageName;
use uv_redacted::DisplaySafeUrl;

#[derive(Debug)]
pub(super) struct PrepareReporter {
    reporter: ProgressReporter,
}

impl PrepareReporter {
    pub(super) fn new(printer: Printer, length: u64) -> Self {
        let multi_progress = MultiProgress::with_draw_target(printer.target());
        let root = multi_progress.add(ProgressBar::with_draw_target(None, printer.target()));
        root.enable_steady_tick(Duration::from_millis(200));
        root.set_style(
            ProgressStyle::with_template("{spinner:.white} {msg:.dim} ({pos}/{len})")
                .unwrap()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        root.set_message("Preparing packages...");

        root.set_length(length);
        let reporter = ProgressReporter::new(root, multi_progress, printer);
        let mut event = JsonlProgressEvent::new(ProgressPhase::Prepare, ProgressStatus::Started);
        event.total = Some(length);
        reporter.emit_progress(&event);
        Self { reporter }
    }

    pub(super) fn on_failed(&self) {
        self.finish(ProgressStatus::Failed);
    }

    fn finish(&self, status: ProgressStatus) {
        // Need an extra call to `set_message` here to fully clear avoid leaving ghost output
        // in Jupyter notebooks.
        self.reporter.root.set_message("");
        if self.reporter.printer.emits_jsonl_progress() {
            let mut event = JsonlProgressEvent::new(ProgressPhase::Prepare, status);
            event.completed = Some(self.reporter.root.position());
            event.total = self.reporter.root.length();
            self.reporter.emit_progress(&event);
        }
        self.reporter.root.finish_and_clear();
    }
}

impl uv_installer::PrepareReporter for PrepareReporter {
    fn on_progress(&self, dist: &CachedDist) {
        self.reporter.root.inc(1);
        if self.reporter.printer.emits_jsonl_progress() {
            let mut event =
                JsonlProgressEvent::new(ProgressPhase::Prepare, ProgressStatus::Updated);
            event.name = Some(dist.to_string());
            event.completed = Some(self.reporter.root.position());
            event.total = self.reporter.root.length();
            self.reporter.emit_progress(&event);
        }
    }

    fn on_complete(&self) {
        self.finish(ProgressStatus::Completed);
    }

    fn on_build_start(&self, source: &BuildableSource) -> usize {
        self.reporter
            .on_build_start(source, &source.color_display())
    }

    fn on_build_complete(&self, source: &BuildableSource, id: usize) {
        self.reporter
            .on_build_complete(source, &source.color_display(), id);
    }

    fn on_build_failed(&self, source: &BuildableSource, id: usize) {
        self.reporter.on_build_failed(source, id);
    }

    fn on_download_start(&self, name: &PackageName, size: Option<u64>) -> usize {
        self.reporter.on_download_start(name.to_string(), size)
    }

    fn on_download_progress(&self, id: usize, bytes: u64) {
        self.reporter.on_download_progress(id, bytes);
    }

    fn on_download_complete(&self, _name: &PackageName, id: usize) {
        self.reporter.on_download_complete(id);
    }

    fn on_download_failed(&self, _name: &PackageName, id: usize) {
        self.reporter.on_download_failed(id);
    }

    fn on_checkout_start(&self, url: &DisplaySafeUrl, rev: &str) -> usize {
        self.reporter.on_checkout_start(url, rev)
    }

    fn on_checkout_complete(&self, url: &DisplaySafeUrl, rev: &str, id: usize) {
        self.reporter.on_checkout_complete(url, rev, id);
    }
}

#[derive(Debug)]
pub(super) struct InstallReporter {
    printer: Printer,
    progress: ProgressBar,
    progress_lock: Mutex<()>,
}

impl InstallReporter {
    pub(super) fn new(printer: Printer, length: u64) -> Self {
        let progress = ProgressBar::with_draw_target(None, printer.target());
        progress.set_style(
            ProgressStyle::with_template("{bar:20} [{pos}/{len}] {wide_msg:.dim}").unwrap(),
        );
        progress.set_message("Installing wheels...");
        progress.set_length(length);
        let mut event = JsonlProgressEvent::new(ProgressPhase::Install, ProgressStatus::Started);
        event.total = Some(length);
        emit_jsonl_progress(printer, &event);
        Self {
            printer,
            progress,
            progress_lock: Mutex::new(()),
        }
    }

    pub(super) fn on_failed(&self) {
        self.finish(ProgressStatus::Failed);
    }

    fn finish(&self, status: ProgressStatus) {
        let _guard = self.progress_lock.lock().unwrap();
        self.progress.set_message("");
        if self.printer.emits_jsonl_progress() {
            let mut event = JsonlProgressEvent::new(ProgressPhase::Install, status);
            event.completed = Some(self.progress.position());
            event.total = self.progress.length();
            emit_jsonl_progress(self.printer, &event);
        }
        self.progress.finish_and_clear();
    }
}

impl uv_installer::InstallReporter for InstallReporter {
    fn on_install_progress(&self, wheel: &CachedDist) {
        let _guard = self.progress_lock.lock().unwrap();
        self.progress.set_message(format!("{wheel}"));
        self.progress.inc(1);
        if self.printer.emits_jsonl_progress() {
            let mut event =
                JsonlProgressEvent::new(ProgressPhase::Install, ProgressStatus::Updated);
            event.name = Some(wheel.to_string());
            event.completed = Some(self.progress.position());
            event.total = self.progress.length();
            emit_jsonl_progress(self.printer, &event);
        }
    }

    fn on_install_complete(&self) {
        self.finish(ProgressStatus::Completed);
    }
}
