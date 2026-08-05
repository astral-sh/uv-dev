use std::env;
use std::fmt::Write;
use std::ops::Deref;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use owo_colors::OwoColorize;
use rustc_hash::FxHashMap;
use serde::Serialize;

use crate::commands::human_readable_bytes;
use crate::printer::Printer;
use uv_cache::Removal;
use uv_distribution_filename::DistFilename;
use uv_distribution_types::{
    BuildableSource, CachedDist, DistributionMetadata, Name, SourceDist, VersionOrUrlRef,
};
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_python::PythonInstallationKey;
use uv_redacted::DisplaySafeUrl;
use uv_static::EnvVars;

/// Since downloads, fetches and builds run in parallel, their message output order is
/// non-deterministic, so can't capture them in test output.
static HAS_UV_TEST_NO_CLI_PROGRESS: LazyLock<bool> =
    LazyLock::new(|| env::var(EnvVars::UV_TEST_NO_CLI_PROGRESS).is_ok());
static JSONL_PROGRESS_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProgressStatus {
    Started,
    Updated,
    Completed,
}

#[derive(Debug, Serialize)]
struct JsonlProgressEvent {
    #[serde(rename = "type")]
    event_type: &'static str,
    phase: &'static str,
    status: ProgressStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<u64>,
}

impl JsonlProgressEvent {
    fn new(phase: &'static str, status: ProgressStatus) -> Self {
        Self {
            event_type: "progress",
            phase,
            status,
            id: None,
            name: None,
            version: None,
            url: None,
            revision: None,
            bytes: None,
            completed: None,
            total: None,
        }
    }
}

fn emit_jsonl_progress(printer: Printer, event: &JsonlProgressEvent) {
    if !printer.emits_jsonl_progress() {
        return;
    }

    if let Ok(event) = serde_json::to_string(event)
        && let Ok(_guard) = JSONL_PROGRESS_LOCK.lock()
    {
        let _ = writeln!(printer.stdout_important(), "{event}");
    }
}

#[derive(Debug)]
struct ProgressReporter {
    printer: Printer,
    root: ProgressBar,
    mode: ProgressMode,
}

#[derive(Debug)]
enum ProgressMode {
    /// Reports top-level progress.
    Single,
    /// Reports progress of all concurrent download, build, and checkout processes.
    Multi {
        multi_progress: MultiProgress,
        state: Arc<Mutex<BarState>>,
    },
}

#[derive(Debug)]
enum ProgressBarKind {
    /// A progress bar with an increasing value, such as a download.
    Numeric {
        progress: ProgressBar,
        /// The download size in bytes, if known.
        size: Option<u64>,
        /// The operation represented by this progress bar.
        direction: Direction,
    },
    /// A progress spinner for a task, such as a build.
    Spinner { progress: ProgressBar },
}

impl Deref for ProgressBarKind {
    type Target = ProgressBar;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Numeric { progress, .. } => progress,
            Self::Spinner { progress } => progress,
        }
    }
}

#[derive(Debug)]
struct BarState {
    /// The number of bars that precede any download bars (i.e., build/checkout status).
    headers: usize,
    /// A list of download bar sizes, in descending order.
    sizes: Vec<u64>,
    /// A map of progress bars, by ID.
    bars: FxHashMap<usize, ProgressBarKind>,
    /// A monotonic counter for bar IDs.
    id: usize,
    /// The maximum length of all bar names encountered.
    max_len: usize,
}

impl Default for BarState {
    fn default() -> Self {
        Self {
            headers: 0,
            sizes: Vec::default(),
            bars: FxHashMap::default(),
            id: 0,
            // Avoid resizing the progress bar templates too often by starting with a padding
            // that's wider than most package names.
            max_len: 20,
        }
    }
}

impl BarState {
    /// Returns a unique ID for a new progress bar.
    fn id(&mut self) -> usize {
        self.id += 1;
        self.id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Upload,
    Download,
    Extract,
    Hash,
}

impl Direction {
    fn as_str(&self) -> &str {
        match self {
            Self::Download => "Downloading",
            Self::Upload => "Uploading",
            Self::Extract => "Extracting",
            Self::Hash => "Hashing",
        }
    }

    fn phase(self) -> &'static str {
        match self {
            Self::Download => "download",
            Self::Upload => "upload",
            Self::Extract => "extract",
            Self::Hash => "hash",
        }
    }
}

impl From<uv_python::downloads::Direction> for Direction {
    fn from(dir: uv_python::downloads::Direction) -> Self {
        match dir {
            uv_python::downloads::Direction::Download => Self::Download,
            uv_python::downloads::Direction::Extract => Self::Extract,
        }
    }
}

impl ProgressReporter {
    fn new(root: ProgressBar, multi_progress: MultiProgress, printer: Printer) -> Self {
        let mode = if env::var(EnvVars::JPY_SESSION_NAME).is_ok() && !printer.emits_jsonl_progress()
        {
            // Disable concurrent progress bars when running inside a Jupyter notebook
            // because the Jupyter terminal does not support clearing previous lines.
            // See: https://github.com/astral-sh/uv/issues/3887.
            ProgressMode::Single
        } else {
            ProgressMode::Multi {
                state: Arc::default(),
                multi_progress,
            }
        };

        Self {
            printer,
            root,
            mode,
        }
    }

    fn emit_progress(&self, event: &JsonlProgressEvent) {
        emit_jsonl_progress(self.printer, event);
    }

    fn on_build_start(&self, source: &BuildableSource) -> usize {
        let ProgressMode::Multi {
            multi_progress,
            state,
        } = &self.mode
        else {
            return 0;
        };

        let mut state = state.lock().unwrap();
        let id = state.id();

        let progress = multi_progress.insert_before(
            &self.root,
            ProgressBar::with_draw_target(None, self.printer.target()),
        );

        progress.set_style(ProgressStyle::with_template("{wide_msg}").unwrap());
        let message = format!(
            "   {} {}",
            "Building".bold().cyan(),
            source.to_color_string()
        );
        if multi_progress.is_hidden() && !*HAS_UV_TEST_NO_CLI_PROGRESS {
            let _ = writeln!(self.printer.stderr(), "{message}");
        }
        progress.set_message(message);

        state.headers += 1;
        state.bars.insert(id, ProgressBarKind::Spinner { progress });
        if self.printer.emits_jsonl_progress() {
            let mut event = JsonlProgressEvent::new("build", ProgressStatus::Started);
            event.id = Some(id);
            event.name = Some(source.to_string());
            self.emit_progress(&event);
        }
        id
    }

    fn on_build_complete(&self, source: &BuildableSource, id: usize) {
        let ProgressMode::Multi {
            state,
            multi_progress,
        } = &self.mode
        else {
            return;
        };

        let progress = {
            let mut state = state.lock().unwrap();
            state.headers -= 1;
            state.bars.remove(&id).unwrap()
        };

        let message = format!(
            "      {} {}",
            "Built".bold().green(),
            source.to_color_string()
        );
        if multi_progress.is_hidden() && !*HAS_UV_TEST_NO_CLI_PROGRESS {
            let _ = writeln!(self.printer.stderr(), "{message}");
        }
        if self.printer.emits_jsonl_progress() {
            let mut event = JsonlProgressEvent::new("build", ProgressStatus::Completed);
            event.id = Some(id);
            event.name = Some(source.to_string());
            self.emit_progress(&event);
        }
        progress.finish_with_message(message);
    }

    fn on_request_start(&self, direction: Direction, name: String, size: Option<u64>) -> usize {
        let ProgressMode::Multi {
            multi_progress,
            state,
        } = &self.mode
        else {
            return 0;
        };

        let event_name = self.printer.emits_jsonl_progress().then(|| name.clone());
        let mut state = state.lock().unwrap();

        // Preserve ascending order.
        let position = size.map_or(0, |size| state.sizes.partition_point(|&len| len < size));
        state.sizes.insert(position, size.unwrap_or(0));
        state.max_len = std::cmp::max(state.max_len, name.len());

        let max_len = state.max_len;
        for progress in state.bars.values_mut() {
            // Ignore spinners, such as for builds.
            if let ProgressBarKind::Numeric { progress, .. } = progress {
                let template = format!(
                    "{{msg:{max_len}.dim}} {{bar:30.green/black.dim}} {{binary_bytes:>7}}/{{binary_total_bytes:7}}"
                );
                progress.set_style(
                    ProgressStyle::with_template(&template)
                        .unwrap()
                        .progress_chars("--"),
                );
                progress.tick();
            }
        }

        let progress = multi_progress.insert(
            // Make sure not to reorder the initial "Preparing..." bar, or any previous bars.
            position + 1 + state.headers,
            ProgressBar::with_draw_target(size, self.printer.target()),
        );

        if let Some(size) = size {
            // We're using binary bytes to match `human_readable_bytes`.
            progress.set_style(
                ProgressStyle::with_template(
                    &format!(
                        "{{msg:{}.dim}} {{bar:30.green/black.dim}} {{binary_bytes:>7}}/{{binary_total_bytes:7}}", state.max_len
                    ),
                )
                    .unwrap()
                    .progress_chars("--"),
            );
            // If the file is larger than 1MB, show a message to indicate that this may take
            // a while keeping the log concise.
            if multi_progress.is_hidden() && !*HAS_UV_TEST_NO_CLI_PROGRESS && size > 1024 * 1024 {
                let _ = writeln!(
                    self.printer.stderr(),
                    "{} {} {}",
                    direction.as_str().bold().cyan(),
                    name,
                    format!("({:.1})", human_readable_bytes(size)).dimmed()
                );
            }
            progress.set_message(name);
        } else {
            progress.set_style(ProgressStyle::with_template("{wide_msg:.dim} ....").unwrap());
            if multi_progress.is_hidden() && !*HAS_UV_TEST_NO_CLI_PROGRESS {
                let _ = writeln!(
                    self.printer.stderr(),
                    "{} {}",
                    direction.as_str().bold().cyan(),
                    name
                );
            }
            progress.set_message(name);
            progress.finish();
        }

        let id = state.id();
        state.bars.insert(
            id,
            ProgressBarKind::Numeric {
                progress,
                size,
                direction,
            },
        );
        let mut event = JsonlProgressEvent::new(direction.phase(), ProgressStatus::Started);
        event.id = Some(id);
        event.name = event_name;
        event.total = size;
        self.emit_progress(&event);
        id
    }

    fn on_request_progress(&self, id: usize, bytes: u64) {
        let ProgressMode::Multi { state, .. } = &self.mode else {
            return;
        };

        // Avoid panics due to reads on failed requests.
        // https://github.com/astral-sh/uv/issues/17090
        // TODO(konsti): Add a debug assert once https://github.com/seanmonstar/reqwest/issues/2884
        // is fixed
        if let Some(ProgressBarKind::Numeric {
            progress,
            size,
            direction,
        }) = state.lock().unwrap().bars.get(&id)
        {
            progress.inc(bytes);

            if self.printer.emits_jsonl_progress() {
                let mut event = JsonlProgressEvent::new(direction.phase(), ProgressStatus::Updated);
                event.id = Some(id);
                event.bytes = Some(bytes);
                event.completed = Some(progress.position());
                event.total = *size;
                self.emit_progress(&event);
            }
        }
    }

    fn on_request_complete(&self, direction: Direction, id: usize) {
        let ProgressMode::Multi {
            state,
            multi_progress,
        } = &self.mode
        else {
            return;
        };

        let mut state = state.lock().unwrap();
        if let ProgressBarKind::Numeric { progress, size, .. } = state.bars.remove(&id).unwrap() {
            if multi_progress.is_hidden()
                && !*HAS_UV_TEST_NO_CLI_PROGRESS
                && size.is_none_or(|size| size > 1024 * 1024)
            {
                let _ = writeln!(
                    self.printer.stderr(),
                    " {} {}",
                    match direction {
                        Direction::Download => "Downloaded",
                        Direction::Upload => "Uploaded",
                        Direction::Extract => "Extracted",
                        Direction::Hash => "Hashed",
                    }
                    .bold()
                    .cyan(),
                    progress.message()
                );
            }

            if self.printer.emits_jsonl_progress() {
                let mut event =
                    JsonlProgressEvent::new(direction.phase(), ProgressStatus::Completed);
                event.id = Some(id);
                event.name = Some(progress.message());
                event.completed = Some(progress.position());
                event.total = size;
                self.emit_progress(&event);
            }
            progress.finish_and_clear();
        } else {
            debug_assert!(false, "Request progress bars are numeric");
        }
    }

    fn on_download_progress(&self, id: usize, bytes: u64) {
        self.on_request_progress(id, bytes);
    }

    fn on_download_complete(&self, id: usize) {
        self.on_request_complete(Direction::Download, id);
    }

    fn on_download_start(&self, name: String, size: Option<u64>) -> usize {
        self.on_request_start(Direction::Download, name, size)
    }

    fn on_upload_progress(&self, id: usize, bytes: u64) {
        self.on_request_progress(id, bytes);
    }

    fn on_upload_complete(&self, id: usize) {
        self.on_request_complete(Direction::Upload, id);
    }

    fn on_upload_start(&self, name: String, size: Option<u64>) -> usize {
        self.on_request_start(Direction::Upload, name, size)
    }

    fn on_hash_progress(&self, id: usize, bytes: u64) {
        self.on_request_progress(id, bytes);
    }

    fn on_hash_complete(&self, id: usize) {
        self.on_request_complete(Direction::Hash, id);
    }

    fn on_hash_start(&self, name: String, size: Option<u64>) -> usize {
        self.on_request_start(Direction::Hash, name, size)
    }

    fn on_checkout_start(&self, url: &DisplaySafeUrl, rev: &str) -> usize {
        let ProgressMode::Multi {
            multi_progress,
            state,
        } = &self.mode
        else {
            return 0;
        };

        let mut state = state.lock().unwrap();
        let id = state.id();

        let progress = multi_progress.insert_before(
            &self.root,
            ProgressBar::with_draw_target(None, self.printer.target()),
        );

        progress.set_style(ProgressStyle::with_template("{wide_msg}").unwrap());
        let message = format!("   {} {} ({})", "Updating".bold().cyan(), url, rev.dimmed());
        if multi_progress.is_hidden() && !*HAS_UV_TEST_NO_CLI_PROGRESS {
            let _ = writeln!(self.printer.stderr(), "{message}");
        }
        progress.set_message(message);
        progress.finish();

        state.headers += 1;
        state.bars.insert(id, ProgressBarKind::Spinner { progress });
        if self.printer.emits_jsonl_progress() {
            let mut event = JsonlProgressEvent::new("checkout", ProgressStatus::Started);
            event.id = Some(id);
            event.url = Some(url.to_string());
            event.revision = Some(rev.to_string());
            self.emit_progress(&event);
        }
        id
    }

    fn on_checkout_complete(&self, url: &DisplaySafeUrl, rev: &str, id: usize) {
        let ProgressMode::Multi {
            state,
            multi_progress,
        } = &self.mode
        else {
            return;
        };

        let progress = {
            let mut state = state.lock().unwrap();
            state.headers -= 1;
            state.bars.remove(&id).unwrap()
        };

        let message = format!(
            "    {} {} ({})",
            "Updated".bold().green(),
            url,
            rev.dimmed()
        );
        if multi_progress.is_hidden() && !*HAS_UV_TEST_NO_CLI_PROGRESS {
            let _ = writeln!(self.printer.stderr(), "{message}");
        }
        if self.printer.emits_jsonl_progress() {
            let mut event = JsonlProgressEvent::new("checkout", ProgressStatus::Completed);
            event.id = Some(id);
            event.url = Some(url.to_string());
            event.revision = Some(rev.to_string());
            self.emit_progress(&event);
        }
        progress.finish_with_message(message);
    }
}

#[derive(Debug)]
pub(crate) struct PrepareReporter {
    reporter: ProgressReporter,
}

impl From<Printer> for PrepareReporter {
    fn from(printer: Printer) -> Self {
        let multi_progress = MultiProgress::with_draw_target(printer.target());
        let root = multi_progress.add(ProgressBar::with_draw_target(None, printer.target()));
        root.enable_steady_tick(Duration::from_millis(200));
        root.set_style(
            ProgressStyle::with_template("{spinner:.white} {msg:.dim} ({pos}/{len})")
                .unwrap()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        root.set_message("Preparing packages...");

        let reporter = ProgressReporter::new(root, multi_progress, printer);
        Self { reporter }
    }
}

impl PrepareReporter {
    #[must_use]
    pub(crate) fn with_length(self, length: u64) -> Self {
        self.reporter.root.set_length(length);
        let mut event = JsonlProgressEvent::new("prepare", ProgressStatus::Started);
        event.total = Some(length);
        self.reporter.emit_progress(&event);
        self
    }
}

impl uv_installer::PrepareReporter for PrepareReporter {
    fn on_progress(&self, dist: &CachedDist) {
        self.reporter.root.inc(1);
        if self.reporter.printer.emits_jsonl_progress() {
            let mut event = JsonlProgressEvent::new("prepare", ProgressStatus::Updated);
            event.name = Some(dist.to_string());
            event.completed = Some(self.reporter.root.position());
            event.total = self.reporter.root.length();
            self.reporter.emit_progress(&event);
        }
    }

    fn on_complete(&self) {
        // Need an extra call to `set_message` here to fully clear avoid leaving ghost output
        // in Jupyter notebooks.
        self.reporter.root.set_message("");
        if self.reporter.printer.emits_jsonl_progress() {
            let mut event = JsonlProgressEvent::new("prepare", ProgressStatus::Completed);
            event.completed = Some(self.reporter.root.position());
            event.total = self.reporter.root.length();
            self.reporter.emit_progress(&event);
        }
        self.reporter.root.finish_and_clear();
    }

    fn on_build_start(&self, source: &BuildableSource) -> usize {
        self.reporter.on_build_start(source)
    }

    fn on_build_complete(&self, source: &BuildableSource, id: usize) {
        self.reporter.on_build_complete(source, id);
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

    fn on_checkout_start(&self, url: &DisplaySafeUrl, rev: &str) -> usize {
        self.reporter.on_checkout_start(url, rev)
    }

    fn on_checkout_complete(&self, url: &DisplaySafeUrl, rev: &str, id: usize) {
        self.reporter.on_checkout_complete(url, rev, id);
    }
}

#[derive(Debug)]
pub(crate) struct ResolverReporter {
    reporter: ProgressReporter,
    started: AtomicBool,
}

impl ResolverReporter {
    fn start(&self) {
        if self.reporter.printer.emits_jsonl_progress()
            && !self.started.swap(true, Ordering::Relaxed)
        {
            self.reporter
                .emit_progress(&JsonlProgressEvent::new("resolve", ProgressStatus::Started));
        }
    }

    #[must_use]
    pub(crate) fn with_length(self, length: u64) -> Self {
        self.reporter.root.set_length(length);
        self.start();
        let mut event = JsonlProgressEvent::new("resolve", ProgressStatus::Updated);
        event.total = Some(length);
        self.reporter.emit_progress(&event);
        self
    }
}

impl From<Printer> for ResolverReporter {
    fn from(printer: Printer) -> Self {
        let multi_progress = MultiProgress::with_draw_target(printer.target());
        let root = multi_progress.add(ProgressBar::with_draw_target(None, printer.target()));
        root.enable_steady_tick(Duration::from_millis(200));
        root.set_style(
            ProgressStyle::with_template("{spinner:.white} {wide_msg:.dim}")
                .unwrap()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        root.set_message("Resolving dependencies...");

        Self {
            reporter: ProgressReporter::new(root, multi_progress, printer),
            started: AtomicBool::new(false),
        }
    }
}

impl uv_resolver::ResolverReporter for ResolverReporter {
    fn on_progress(&self, name: &PackageName, version_or_url: &VersionOrUrlRef) {
        self.start();
        match version_or_url {
            VersionOrUrlRef::Version(version) => {
                self.reporter.root.set_message(format!("{name}=={version}"));
            }
            VersionOrUrlRef::Url(url) => {
                self.reporter.root.set_message(format!("{name} @ {url}"));
            }
        }

        if self.reporter.printer.emits_jsonl_progress() {
            let mut event = JsonlProgressEvent::new("resolve", ProgressStatus::Updated);
            event.name = Some(name.to_string());
            match version_or_url {
                VersionOrUrlRef::Version(version) => event.version = Some(version.to_string()),
                VersionOrUrlRef::Url(url) => event.url = Some(url.to_string()),
            }
            self.reporter.emit_progress(&event);
        }
    }

    fn on_complete(&self) {
        self.start();
        self.reporter.root.set_message("");
        self.reporter.emit_progress(&JsonlProgressEvent::new(
            "resolve",
            ProgressStatus::Completed,
        ));
        self.reporter.root.finish_and_clear();
    }

    fn on_build_start(&self, source: &BuildableSource) -> usize {
        self.reporter.on_build_start(source)
    }

    fn on_build_complete(&self, source: &BuildableSource, id: usize) {
        self.reporter.on_build_complete(source, id);
    }

    fn on_checkout_start(&self, url: &DisplaySafeUrl, rev: &str) -> usize {
        self.reporter.on_checkout_start(url, rev)
    }

    fn on_checkout_complete(&self, url: &DisplaySafeUrl, rev: &str, id: usize) {
        self.reporter.on_checkout_complete(url, rev, id);
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
}

impl uv_distribution::Reporter for ResolverReporter {
    fn on_build_start(&self, source: &BuildableSource) -> usize {
        self.reporter.on_build_start(source)
    }

    fn on_build_complete(&self, source: &BuildableSource, id: usize) {
        self.reporter.on_build_complete(source, id);
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

    fn on_checkout_start(&self, url: &DisplaySafeUrl, rev: &str) -> usize {
        self.reporter.on_checkout_start(url, rev)
    }

    fn on_checkout_complete(&self, url: &DisplaySafeUrl, rev: &str, id: usize) {
        self.reporter.on_checkout_complete(url, rev, id);
    }
}

#[derive(Debug)]
pub(crate) struct InstallReporter {
    printer: Printer,
    progress: ProgressBar,
}

impl From<Printer> for InstallReporter {
    fn from(printer: Printer) -> Self {
        let progress = ProgressBar::with_draw_target(None, printer.target());
        progress.set_style(
            ProgressStyle::with_template("{bar:20} [{pos}/{len}] {wide_msg:.dim}").unwrap(),
        );
        progress.set_message("Installing wheels...");
        Self { printer, progress }
    }
}

impl InstallReporter {
    #[must_use]
    pub(crate) fn with_length(self, length: u64) -> Self {
        self.progress.set_length(length);
        let mut event = JsonlProgressEvent::new("install", ProgressStatus::Started);
        event.total = Some(length);
        emit_jsonl_progress(self.printer, &event);
        self
    }
}

impl uv_installer::InstallReporter for InstallReporter {
    fn on_install_progress(&self, wheel: &CachedDist) {
        self.progress.set_message(format!("{wheel}"));
        self.progress.inc(1);
        if self.printer.emits_jsonl_progress() {
            let mut event = JsonlProgressEvent::new("install", ProgressStatus::Updated);
            event.name = Some(wheel.to_string());
            event.completed = Some(self.progress.position());
            event.total = self.progress.length();
            emit_jsonl_progress(self.printer, &event);
        }
    }

    fn on_install_complete(&self) {
        self.progress.set_message("");
        if self.printer.emits_jsonl_progress() {
            let mut event = JsonlProgressEvent::new("install", ProgressStatus::Completed);
            event.completed = Some(self.progress.position());
            event.total = self.progress.length();
            emit_jsonl_progress(self.printer, &event);
        }
        self.progress.finish_and_clear();
    }
}

#[derive(Debug)]
pub(crate) struct PythonDownloadReporter {
    reporter: ProgressReporter,
}

impl PythonDownloadReporter {
    /// Initialize a [`PythonDownloadReporter`] for a single Python download.
    pub(crate) fn single(printer: Printer) -> Self {
        Self::new(printer, None)
    }

    /// Initialize a [`PythonDownloadReporter`] for multiple Python downloads.
    pub(crate) fn new(printer: Printer, length: Option<u64>) -> Self {
        let multi_progress = MultiProgress::with_draw_target(printer.target());
        let root = multi_progress.add(ProgressBar::with_draw_target(length, printer.target()));
        let reporter = ProgressReporter::new(root, multi_progress, printer);
        Self { reporter }
    }
}

impl uv_python::downloads::Reporter for PythonDownloadReporter {
    fn on_request_start(
        &self,
        direction: uv_python::downloads::Direction,
        name: &PythonInstallationKey,
        size: Option<u64>,
    ) -> usize {
        self.reporter
            .on_request_start(direction.into(), format!("{name} ({direction})"), size)
    }

    fn on_request_progress(&self, id: usize, inc: u64) {
        self.reporter.on_request_progress(id, inc);
    }

    fn on_request_complete(&self, direction: uv_python::downloads::Direction, id: usize) {
        self.reporter.on_request_complete(direction.into(), id);
    }
}

#[derive(Debug)]
pub(crate) struct PublishReporter {
    reporter: ProgressReporter,
}

impl PublishReporter {
    /// Initialize a [`PublishReporter`] for a single upload.
    pub(crate) fn single(printer: Printer) -> Self {
        Self::new(printer, None)
    }

    /// Initialize a [`PublishReporter`] for multiple uploads.
    fn new(printer: Printer, length: Option<u64>) -> Self {
        let multi_progress = MultiProgress::with_draw_target(printer.target());
        let root = multi_progress.add(ProgressBar::with_draw_target(length, printer.target()));
        let reporter = ProgressReporter::new(root, multi_progress, printer);
        Self { reporter }
    }
}

impl uv_publish::Reporter for PublishReporter {
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

#[derive(Debug)]
pub(crate) struct LatestVersionReporter {
    progress: ProgressBar,
}

impl From<Printer> for LatestVersionReporter {
    fn from(printer: Printer) -> Self {
        let progress = ProgressBar::with_draw_target(None, printer.target());
        progress.set_style(
            ProgressStyle::with_template("{bar:20} [{pos}/{len}] {wide_msg:.dim}").unwrap(),
        );
        progress.set_message("Fetching latest versions...");
        Self { progress }
    }
}

impl LatestVersionReporter {
    #[must_use]
    pub(crate) fn with_length(self, length: u64) -> Self {
        self.progress.set_length(length);
        self
    }

    pub(crate) fn on_fetch_progress(&self) {
        self.progress.inc(1);
    }

    pub(crate) fn on_fetch_version(&self, name: &PackageName, version: &Version) {
        self.progress.set_message(format!("{name} v{version}"));
        self.progress.inc(1);
    }

    pub(crate) fn on_fetch_complete(&self) {
        self.progress.set_message("");
        self.progress.finish_and_clear();
    }
}

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

#[derive(Debug)]
pub(crate) struct CleaningDirectoryReporter {
    bar: ProgressBar,
}

impl CleaningDirectoryReporter {
    /// Initialize a [`CleaningDirectoryReporter`] for cleaning the cache directory.
    pub(crate) fn new(printer: Printer, max: Option<usize>) -> Self {
        let bar = ProgressBar::with_draw_target(max.map(|m| m as u64), printer.target());
        bar.set_style(
            ProgressStyle::with_template("{prefix} [{bar:20}] {percent}%")
                .unwrap()
                .progress_chars("=> "),
        );
        bar.set_prefix(format!("{}", "Cleaning".bold().cyan()));
        Self { bar }
    }
}

impl uv_cache::CleanReporter for CleaningDirectoryReporter {
    fn on_clean(&self) {
        self.bar.inc(1);
    }

    fn on_complete(&self) {
        self.bar.finish_and_clear();
    }
}

#[derive(Debug)]
pub(crate) struct CleaningPackageReporter {
    bar: ProgressBar,
}

impl CleaningPackageReporter {
    /// Initialize a [`CleaningPackageReporter`] for cleaning packages from the cache.
    pub(crate) fn new(printer: Printer, max: Option<usize>) -> Self {
        let bar = ProgressBar::with_draw_target(max.map(|m| m as u64), printer.target());
        bar.set_style(
            ProgressStyle::with_template("{prefix} [{bar:20}] {pos}/{len}{msg}")
                .unwrap()
                .progress_chars("=> "),
        );
        bar.set_prefix(format!("{}", "Cleaning".bold().cyan()));
        Self { bar }
    }

    pub(crate) fn on_clean(&self, package: &str, removal: &Removal) {
        self.bar.inc(1);
        self.bar.set_message(format!(
            ": {}, {} files {} folders removed",
            package, removal.num_files, removal.num_dirs,
        ));
    }

    pub(crate) fn on_complete(&self) {
        self.bar.finish_and_clear();
    }
}

/// Like [`std::fmt::Display`], but with colors.
trait ColorDisplay {
    fn to_color_string(&self) -> String;
}

impl ColorDisplay for SourceDist {
    fn to_color_string(&self) -> String {
        let name = self.name();
        let version_or_url = self.version_or_url();
        format!("{}{}", name, version_or_url.to_string().dimmed())
    }
}

impl ColorDisplay for BuildableSource<'_> {
    fn to_color_string(&self) -> String {
        match self {
            Self::Dist(dist) => dist.to_color_string(),
            Self::Url(url) => url.to_string(),
        }
    }
}

pub(crate) struct BinaryDownloadReporter {
    reporter: ProgressReporter,
}

impl BinaryDownloadReporter {
    /// Initialize a [`BinaryDownloadReporter`] for a single binary download.
    pub(crate) fn single(printer: Printer) -> Self {
        let multi_progress = MultiProgress::with_draw_target(printer.target());
        let root = multi_progress.add(ProgressBar::with_draw_target(None, printer.target()));
        let reporter = ProgressReporter::new(root, multi_progress, printer);
        Self { reporter }
    }
}

impl uv_bin_install::Reporter for BinaryDownloadReporter {
    fn on_download_start(&self, name: &str, version: &Version, size: Option<u64>) -> usize {
        self.reporter
            .on_request_start(Direction::Download, format!("{name} v{version}"), size)
    }

    fn on_download_progress(&self, id: usize, inc: u64) {
        self.reporter.on_request_progress(id, inc);
    }

    fn on_download_complete(&self, id: usize) {
        self.reporter.on_request_complete(Direction::Download, id);
    }
}
