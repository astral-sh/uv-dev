use std::fmt::Write;

use anstream::stream::IsTerminal;
use anyhow::Result;

use crate::commands::{ExitStatus, human_readable_bytes};
use crate::printer::Printer;
use uv_cache::Cache;
use uv_cli::CacheSizeOutputFormat;
use uv_preview::{Preview, PreviewFeature};
use uv_warnings::warn_user;

/// Display the total size of the cache.
pub(crate) fn cache_size(
    cache: &Cache,
    output_format: CacheSizeOutputFormat,
    printer: Printer,
    preview: Preview,
) -> Result<ExitStatus> {
    if !preview.is_enabled(PreviewFeature::CacheSize) {
        warn_user!(
            "`uv cache size` is experimental and may change without warning. Pass `--preview-features {}` to disable this warning.",
            PreviewFeature::CacheSize
        );
    }

    let human_readable = match output_format {
        CacheSizeOutputFormat::Auto => std::io::stdout().is_terminal(),
        CacheSizeOutputFormat::Human => true,
        CacheSizeOutputFormat::Machine => false,
    };

    if !cache.root().exists() {
        if human_readable {
            writeln!(printer.stdout_important(), "0B")?;
        } else {
            writeln!(printer.stdout_important(), "0")?;
        }
        return Ok(ExitStatus::Success);
    }

    let total_bytes: u64 = walkdir::WalkDir::new(cache.root())
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(|entry| match entry.metadata() {
            Ok(metadata) if metadata.is_file() => Some(metadata.len()),
            _ => None,
        })
        .sum();

    if human_readable {
        let bytes = human_readable_bytes(total_bytes);
        writeln!(printer.stdout_important(), "{bytes:.1}")?;
    } else {
        writeln!(printer.stdout_important(), "{total_bytes}")?;
    }

    Ok(ExitStatus::Success)
}
