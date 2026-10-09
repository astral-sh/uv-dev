use std::sync::Arc;

use uv_distribution_types::BuildableSource;
use uv_normalize::PackageName;
use uv_redacted::DisplaySafeUrl;

pub trait Reporter: Send + Sync {
    /// Callback to invoke when a source distribution build is kicked off.
    fn on_build_start(&self, source: &BuildableSource) -> usize;

    /// Callback to invoke when a source distribution build is complete.
    fn on_build_complete(&self, source: &BuildableSource, id: usize);

    /// Callback to invoke when a repository checkout begins.
    fn on_checkout_start(&self, url: &DisplaySafeUrl, rev: &str) -> usize;

    /// Callback to invoke when a repository checkout completes.
    fn on_checkout_complete(&self, url: &DisplaySafeUrl, rev: &str, id: usize);

    /// Callback to invoke when a download is kicked off.
    fn on_download_start(&self, name: &PackageName, size: Option<u64>) -> usize;

    /// Callback to invoke when a download makes progress (i.e. some number of bytes are
    /// downloaded).
    fn on_download_progress(&self, id: usize, inc: u64);

    /// Callback to invoke when a download is complete.
    fn on_download_complete(&self, name: &PackageName, id: usize);

    /// Callback to invoke when a download attempt is abandoned or fails.
    fn on_download_failed(&self, _name: &PackageName, _id: usize) {}
}

/// A started download attempt that reports failure if it does not complete.
pub(crate) struct DownloadGuard<'a> {
    pub(crate) reporter: &'a dyn Reporter,
    pub(crate) id: usize,
    name: &'a PackageName,
    completed: bool,
}

impl<'a> DownloadGuard<'a> {
    pub(crate) fn new(
        reporter: &'a dyn Reporter,
        name: &'a PackageName,
        size: Option<u64>,
    ) -> Self {
        Self {
            reporter,
            id: reporter.on_download_start(name, size),
            name,
            completed: false,
        }
    }

    pub(crate) fn complete(mut self) {
        self.reporter.on_download_complete(self.name, self.id);
        self.completed = true;
    }
}

impl Drop for DownloadGuard<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.reporter.on_download_failed(self.name, self.id);
        }
    }
}

impl dyn Reporter {
    /// Converts this reporter to a [`uv_git::Reporter`].
    pub(crate) fn into_git_reporter(self: Arc<dyn Reporter>) -> Arc<dyn uv_git::Reporter> {
        Arc::new(Facade { reporter: self })
    }
}

/// A facade for converting from [`Reporter`] to [`uv_git::Reporter`].
struct Facade {
    reporter: Arc<dyn Reporter>,
}

impl uv_git::Reporter for Facade {
    fn on_checkout_start(&self, url: &DisplaySafeUrl, rev: &str) -> usize {
        self.reporter.on_checkout_start(url, rev)
    }

    fn on_checkout_complete(&self, url: &DisplaySafeUrl, rev: &str, id: usize) {
        self.reporter.on_checkout_complete(url, rev, id);
    }
}
