use std::sync::Arc;

use uv_distribution_types::{BuildableSource, VersionOrUrlRef};
use uv_normalize::PackageName;
use uv_redacted::DisplaySafeUrl;

pub trait Reporter: Send + Sync {
    /// Callback to invoke when a dependency is resolved.
    fn on_progress(&self, name: &PackageName, version: &VersionOrUrlRef);

    /// Callback to invoke when resolution finishes, including whether it succeeded.
    fn on_complete(&self, success: bool);

    /// Callback to invoke when a source distribution build is kicked off.
    fn on_build_start(&self, source: &BuildableSource) -> usize;

    /// Callback to invoke when a source distribution build is complete.
    fn on_build_complete(&self, source: &BuildableSource, id: usize);

    /// Callback to invoke when a source build attempt fails or is abandoned.
    fn on_build_failed(&self, _source: &BuildableSource, _id: usize) {}

    /// Callback to invoke when a download is kicked off.
    fn on_download_start(&self, name: &PackageName, size: Option<u64>) -> usize;

    /// Callback to invoke when a download makes progress (i.e. some number of bytes are
    /// downloaded).
    fn on_download_progress(&self, id: usize, bytes: u64);

    /// Callback to invoke when a download is complete.
    fn on_download_complete(&self, name: &PackageName, id: usize);

    /// Callback to invoke when a download attempt is abandoned or fails.
    fn on_download_failed(&self, _name: &PackageName, _id: usize) {}

    /// Callback to invoke when a repository checkout begins.
    fn on_checkout_start(&self, url: &DisplaySafeUrl, rev: &str) -> usize;

    /// Callback to invoke when a repository checkout completes.
    fn on_checkout_complete(&self, url: &DisplaySafeUrl, rev: &str, id: usize);

    /// Callback to invoke when a repository checkout fails or is abandoned.
    fn on_checkout_failed(&self, _url: &DisplaySafeUrl, _rev: &str, _id: usize) {}
}

impl dyn Reporter {
    /// Converts this reporter to a [`uv_distribution::Reporter`].
    pub(crate) fn into_distribution_reporter(
        self: Arc<dyn Reporter>,
    ) -> Arc<dyn uv_distribution::Reporter> {
        Arc::new(Facade { reporter: self })
    }
}

/// A facade for converting from [`Reporter`] to [`uv_distribution::Reporter`].
struct Facade {
    reporter: Arc<dyn Reporter>,
}

impl uv_distribution::Reporter for Facade {
    fn on_build_start(&self, source: &BuildableSource) -> usize {
        self.reporter.on_build_start(source)
    }

    fn on_build_complete(&self, source: &BuildableSource, id: usize) {
        self.reporter.on_build_complete(source, id);
    }

    fn on_build_failed(&self, source: &BuildableSource, id: usize) {
        self.reporter.on_build_failed(source, id);
    }

    fn on_checkout_start(&self, url: &DisplaySafeUrl, rev: &str) -> usize {
        self.reporter.on_checkout_start(url, rev)
    }

    fn on_checkout_complete(&self, url: &DisplaySafeUrl, rev: &str, id: usize) {
        self.reporter.on_checkout_complete(url, rev, id);
    }

    fn on_checkout_failed(&self, url: &DisplaySafeUrl, rev: &str, id: usize) {
        self.reporter.on_checkout_failed(url, rev, id);
    }

    fn on_download_start(&self, name: &PackageName, size: Option<u64>) -> usize {
        self.reporter.on_download_start(name, size)
    }

    fn on_download_progress(&self, id: usize, bytes: u64) {
        self.reporter.on_download_progress(id, bytes);
    }

    fn on_download_complete(&self, name: &PackageName, id: usize) {
        self.reporter.on_download_complete(name, id);
    }

    fn on_download_failed(&self, name: &PackageName, id: usize) {
        self.reporter.on_download_failed(name, id);
    }
}
