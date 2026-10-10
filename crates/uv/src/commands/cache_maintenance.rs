use std::fmt::Write;

use anyhow::Result;
use owo_colors::OwoColorize;
use tracing::debug;

use uv_cache::{Cache, RemovalAccounting};
use uv_command_support::Printer;
use uv_fs::Simplified;
use uv_preview::{Preview, PreviewFeature};

/// Prepare an existing cache for explicit maintenance.
///
/// The returned cache holds any acquired lock until the caller completes removal. `--force`
/// bypasses a contended lock, but still acquires one when it is immediately available.
pub(super) async fn prepare_cache(
    cache: Cache,
    force: bool,
    printer: Printer,
    preview: Preview,
) -> Result<Option<Cache>> {
    if !cache.root().exists() {
        writeln!(
            printer.stderr(),
            "No cache found at: {}",
            cache.root().user_display().cyan()
        )?;
        return Ok(None);
    }

    let cache = match cache.with_exclusive_lock_no_wait() {
        Ok(cache) => cache,
        Err(cache) if force => {
            debug!("Cache is currently in use, proceeding due to `--force`");
            cache
        }
        Err(cache) => {
            writeln!(
                printer.stderr(),
                "Cache is currently in-use, waiting for other uv processes to finish (use `--force` to override)"
            )?;
            cache.with_exclusive_lock().await?
        }
    };

    let removal_accounting = if preview.is_enabled(PreviewFeature::CachePhysicalSpace) {
        RemovalAccounting::Fine
    } else {
        RemovalAccounting::Coarse
    };
    Ok(Some(cache.with_removal_accounting(removal_accounting)))
}
