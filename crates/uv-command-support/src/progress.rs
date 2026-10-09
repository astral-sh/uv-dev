use std::env;
use std::fmt::{self, Write};
use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use owo_colors::OwoColorize;
use rustc_hash::FxHashMap;
use serde::Serialize;
use uv_console::human_readable_bytes;
use uv_redacted::DisplaySafeUrl;
use uv_static::EnvVars;

use crate::Printer;

/// Since downloads, fetches and builds run in parallel, their message output order is
/// non-deterministic, so can't capture them in test output.
static HAS_UV_INTERNAL__TEST_NO_CLI_PROGRESS: LazyLock<bool> =
    LazyLock::new(|| env::var(EnvVars::UV_INTERNAL__TEST_NO_CLI_PROGRESS).is_ok());
static JSONL_PROGRESS_LOCK: Mutex<()> = Mutex::new(());
static NEXT_PROGRESS_ID: AtomicUsize = AtomicUsize::new(1);

/// The lifecycle of an operation: started, optionally updated, then completed or failed.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressStatus {
    Started,
    Updated,
    Completed,
    Failed,
}

/// Operations represented by the JSONL progress protocol.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressPhase {
    Audit,
    Build,
    Checkout,
    Download,
    Extract,
    Hash,
    Install,
    Prepare,
    Resolve,
    Upload,
}

/// A progress update emitted before a command's final JSONL result.
///
/// Concurrent operations are correlated using their process-wide `id`. Top-level
/// phases omit `id`, since only one instance of each phase is active at a time.
/// Operations can complete without an intermediate update.
#[derive(Debug, Serialize)]
pub struct JsonlProgressEvent {
    /// Distinguishes progress updates from the final command result.
    #[serde(rename = "type")]
    event_type: &'static str,
    /// The operation being reported, such as `download`, `build`, or `install`.
    phase: ProgressPhase,
    /// The operation's current lifecycle state.
    status: ProgressStatus,
    /// A process-wide identifier shared by all events for one concurrent operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<usize>,
    /// The package, distribution, or source currently being processed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The selected package version, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The source URL associated with a resolution or checkout operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The Git revision associated with a checkout operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
    /// Completed bytes for transfers, or completed packages for package phases.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<u64>,
    /// The fixed total bytes or packages for this operation, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

impl JsonlProgressEvent {
    pub fn new(phase: ProgressPhase, status: ProgressStatus) -> Self {
        Self {
            event_type: "progress",
            phase,
            status,
            id: None,
            name: None,
            version: None,
            url: None,
            revision: None,
            completed: None,
            total: None,
        }
    }
}

pub fn emit_jsonl_progress(printer: Printer, event: &JsonlProgressEvent) {
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
pub struct ProgressReporter {
    pub printer: Printer,
    pub root: ProgressBar,
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
    /// The maximum length of all bar names encountered.
    max_len: usize,
}

impl Default for BarState {
    fn default() -> Self {
        Self {
            headers: 0,
            sizes: Vec::default(),
            bars: FxHashMap::default(),
            // Avoid resizing the progress bar templates too often by starting with a padding
            // that's wider than most package names.
            max_len: 20,
        }
    }
}

impl BarState {
    /// Returns a process-wide unique ID for a new progress bar.
    fn next_id() -> usize {
        NEXT_PROGRESS_ID.fetch_add(1, Ordering::Relaxed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
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

    fn phase(self) -> ProgressPhase {
        match self {
            Self::Download => ProgressPhase::Download,
            Self::Upload => ProgressPhase::Upload,
            Self::Extract => ProgressPhase::Extract,
            Self::Hash => ProgressPhase::Hash,
        }
    }
}

impl ProgressReporter {
    pub fn new(root: ProgressBar, multi_progress: MultiProgress, printer: Printer) -> Self {
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

    pub fn emit_progress(&self, event: &JsonlProgressEvent) {
        emit_jsonl_progress(self.printer, event);
    }

    /// Start reporting a build using the caller's source display.
    pub fn on_build_start(&self, source: &dyn fmt::Display, styled: &dyn fmt::Display) -> usize {
        let ProgressMode::Multi {
            multi_progress,
            state,
        } = &self.mode
        else {
            return 0;
        };

        let mut state = state.lock().unwrap();
        let id = BarState::next_id();

        let progress = multi_progress.insert_before(
            &self.root,
            ProgressBar::with_draw_target(None, self.printer.target()),
        );

        progress.set_style(ProgressStyle::with_template("{wide_msg}").unwrap());
        let message = format!("   {} {}", "Building".bold().cyan(), styled);
        if multi_progress.is_hidden() && !*HAS_UV_INTERNAL__TEST_NO_CLI_PROGRESS {
            let _ = writeln!(self.printer.stderr(), "{message}");
        }
        progress.set_message(message);

        state.headers += 1;
        state.bars.insert(id, ProgressBarKind::Spinner { progress });
        if self.printer.emits_jsonl_progress() {
            let mut event = JsonlProgressEvent::new(ProgressPhase::Build, ProgressStatus::Started);
            event.id = Some(id);
            event.name = Some(source.to_string());
            self.emit_progress(&event);
        }
        id
    }

    /// Finish reporting a build using the caller's source display.
    pub fn on_build_complete(
        &self,
        source: &dyn fmt::Display,
        styled: &dyn fmt::Display,
        id: usize,
    ) {
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

        let message = format!("      {} {}", "Built".bold().green(), styled);
        if multi_progress.is_hidden() && !*HAS_UV_INTERNAL__TEST_NO_CLI_PROGRESS {
            let _ = writeln!(self.printer.stderr(), "{message}");
        }
        if self.printer.emits_jsonl_progress() {
            let mut event =
                JsonlProgressEvent::new(ProgressPhase::Build, ProgressStatus::Completed);
            event.id = Some(id);
            event.name = Some(source.to_string());
            self.emit_progress(&event);
        }
        progress.finish_with_message(message);
    }

    /// Close an abandoned build without presenting it as completed.
    pub fn on_build_failed(&self, source: &dyn fmt::Display, id: usize) {
        let ProgressMode::Multi { state, .. } = &self.mode else {
            return;
        };
        let progress = {
            let mut state = state.lock().unwrap();
            let Some(progress) = state.bars.remove(&id) else {
                return;
            };
            state.headers -= 1;
            progress
        };
        if self.printer.emits_jsonl_progress() {
            let mut event = JsonlProgressEvent::new(ProgressPhase::Build, ProgressStatus::Failed);
            event.id = Some(id);
            event.name = Some(source.to_string());
            self.emit_progress(&event);
        }
        progress.finish_and_clear();
    }

    pub fn on_request_start(&self, direction: Direction, name: String, size: Option<u64>) -> usize {
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
            if multi_progress.is_hidden()
                && !*HAS_UV_INTERNAL__TEST_NO_CLI_PROGRESS
                && size > 1024 * 1024
            {
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
            if multi_progress.is_hidden() && !*HAS_UV_INTERNAL__TEST_NO_CLI_PROGRESS {
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

        let id = BarState::next_id();
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

    pub fn on_request_progress(&self, id: usize, bytes: u64) {
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

            if bytes > 0 && self.printer.emits_jsonl_progress() {
                let mut event = JsonlProgressEvent::new(direction.phase(), ProgressStatus::Updated);
                event.id = Some(id);
                event.completed = Some(progress.position());
                event.total = *size;
                self.emit_progress(&event);
            }
        }
    }

    pub fn on_request_complete(&self, id: usize) {
        self.finish_request(id, ProgressStatus::Completed);
    }

    pub fn on_request_failed(&self, id: usize) {
        self.finish_request(id, ProgressStatus::Failed);
    }

    fn finish_request(&self, id: usize, status: ProgressStatus) {
        let ProgressMode::Multi {
            state,
            multi_progress,
        } = &self.mode
        else {
            return;
        };

        let mut state = state.lock().unwrap();
        if let ProgressBarKind::Numeric {
            progress,
            size,
            direction,
        } = state.bars.remove(&id).unwrap()
        {
            if matches!(status, ProgressStatus::Completed)
                && multi_progress.is_hidden()
                && !*HAS_UV_INTERNAL__TEST_NO_CLI_PROGRESS
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
                let mut event = JsonlProgressEvent::new(direction.phase(), status);
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

    pub fn on_download_progress(&self, id: usize, bytes: u64) {
        self.on_request_progress(id, bytes);
    }

    pub fn on_download_complete(&self, id: usize) {
        self.on_request_complete(id);
    }

    pub fn on_download_failed(&self, id: usize) {
        self.on_request_failed(id);
    }

    pub fn on_download_start(&self, name: String, size: Option<u64>) -> usize {
        self.on_request_start(Direction::Download, name, size)
    }

    pub fn on_upload_progress(&self, id: usize, bytes: u64) {
        self.on_request_progress(id, bytes);
    }

    pub fn on_upload_complete(&self, id: usize) {
        self.on_request_complete(id);
    }

    pub fn on_upload_start(&self, name: String, size: Option<u64>) -> usize {
        self.on_request_start(Direction::Upload, name, size)
    }

    pub fn on_hash_progress(&self, id: usize, bytes: u64) {
        self.on_request_progress(id, bytes);
    }

    pub fn on_hash_complete(&self, id: usize) {
        self.on_request_complete(id);
    }

    pub fn on_hash_start(&self, name: String, size: Option<u64>) -> usize {
        self.on_request_start(Direction::Hash, name, size)
    }

    pub fn on_checkout_start(&self, url: &DisplaySafeUrl, rev: &str) -> usize {
        let ProgressMode::Multi {
            multi_progress,
            state,
        } = &self.mode
        else {
            return 0;
        };

        let mut state = state.lock().unwrap();
        let id = BarState::next_id();

        let progress = multi_progress.insert_before(
            &self.root,
            ProgressBar::with_draw_target(None, self.printer.target()),
        );

        progress.set_style(ProgressStyle::with_template("{wide_msg}").unwrap());
        let message = format!("   {} {} ({})", "Updating".bold().cyan(), url, rev.dimmed());
        if multi_progress.is_hidden() && !*HAS_UV_INTERNAL__TEST_NO_CLI_PROGRESS {
            let _ = writeln!(self.printer.stderr(), "{message}");
        }
        progress.set_message(message);
        progress.finish();

        state.headers += 1;
        state.bars.insert(id, ProgressBarKind::Spinner { progress });
        if self.printer.emits_jsonl_progress() {
            let mut event =
                JsonlProgressEvent::new(ProgressPhase::Checkout, ProgressStatus::Started);
            event.id = Some(id);
            event.url = Some(url.to_string());
            event.revision = Some(rev.to_string());
            self.emit_progress(&event);
        }
        id
    }

    pub fn on_checkout_complete(&self, url: &DisplaySafeUrl, rev: &str, id: usize) {
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
        if multi_progress.is_hidden() && !*HAS_UV_INTERNAL__TEST_NO_CLI_PROGRESS {
            let _ = writeln!(self.printer.stderr(), "{message}");
        }
        if self.printer.emits_jsonl_progress() {
            let mut event =
                JsonlProgressEvent::new(ProgressPhase::Checkout, ProgressStatus::Completed);
            event.id = Some(id);
            event.url = Some(url.to_string());
            event.revision = Some(rev.to_string());
            self.emit_progress(&event);
        }
        progress.finish_with_message(message);
    }
}
