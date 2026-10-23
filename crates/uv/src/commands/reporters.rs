use std::fmt::{self, Write};

use indicatif::{ProgressBar, ProgressStyle};
use owo_colors::OwoColorize;
use uv_cache::Removal;
use uv_command_support::Printer;
use uv_console::human_readable_bytes;

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
/// Write the final summary for a cache cleanup operation.
pub(super) fn write_cache_removal_summary(
    writer: &mut impl Write,
    summary: &Removal,
    empty_message: &str,
) -> fmt::Result {
    // Write a summary of the number of files and directories removed.
    match (summary.num_files, summary.num_dirs) {
        (0, 0) => {
            write!(writer, "{empty_message}")?;
        }
        (0, 1) => {
            write!(writer, "Removed 1 directory")?;
        }
        (0, num_dirs_removed) => {
            write!(writer, "Removed {num_dirs_removed} directories")?;
        }
        (1, _) => {
            write!(writer, "Removed 1 file")?;
        }
        (num_files_removed, _) => {
            write!(writer, "Removed {num_files_removed} files")?;
        }
    }

    // Prefer the fine-grained estimate, falling back to coarse accounting.
    let reported_bytes = summary.fine_bytes.unwrap_or(summary.coarse_bytes);
    if summary.num_files > 0 || summary.num_dirs > 0 {
        let bytes = human_readable_bytes(reported_bytes);
        if summary.fine_bytes_incomplete {
            write!(writer, " (at least {:.1})", bytes.green())?;
        } else {
            write!(writer, " ({:.1})", bytes.green())?;
        }
    }

    writeln!(writer)
}

#[cfg(test)]
mod cache_removal_tests {
    use std::fmt;

    use uv_cache::Removal;

    use super::write_cache_removal_summary;

    fn assert_summary(empty_message: &str) -> fmt::Result {
        for (summary, expected) in [
            (Removal::default(), format!("{empty_message}\n")),
            (
                Removal {
                    coarse_bytes: 1024,
                    fine_bytes: Some(512),
                    fine_bytes_incomplete: true,
                    ..Removal::default()
                },
                format!("{empty_message}\n"),
            ),
            (
                Removal {
                    num_dirs: 1,
                    ..Removal::default()
                },
                "Removed 1 directory (\x1b[32m0B\x1b[39m)\n".to_string(),
            ),
            (
                Removal {
                    num_dirs: 2,
                    coarse_bytes: 1536,
                    ..Removal::default()
                },
                "Removed 2 directories (\x1b[32m1.5KiB\x1b[39m)\n".to_string(),
            ),
            (
                Removal {
                    num_files: 1,
                    num_dirs: 7,
                    coarse_bytes: 2048,
                    ..Removal::default()
                },
                "Removed 1 file (\x1b[32m2.0KiB\x1b[39m)\n".to_string(),
            ),
            (
                Removal {
                    num_files: 2,
                    num_dirs: 1,
                    coarse_bytes: 4096,
                    fine_bytes: Some(512),
                    ..Removal::default()
                },
                "Removed 2 files (\x1b[32m512B\x1b[39m)\n".to_string(),
            ),
            (
                Removal {
                    num_files: 2,
                    coarse_bytes: 4096,
                    fine_bytes: Some(1536),
                    fine_bytes_incomplete: true,
                    ..Removal::default()
                },
                "Removed 2 files (at least \x1b[32m1.5KiB\x1b[39m)\n".to_string(),
            ),
            (
                Removal {
                    num_files: 2,
                    coarse_bytes: 1024,
                    ..Removal::default()
                },
                "Removed 2 files (\x1b[32m1.0KiB\x1b[39m)\n".to_string(),
            ),
            (
                Removal {
                    num_files: 2,
                    coarse_bytes: 2048,
                    fine_bytes: Some(0),
                    ..Removal::default()
                },
                "Removed 2 files (\x1b[32m0B\x1b[39m)\n".to_string(),
            ),
        ] {
            let mut output = String::new();
            write_cache_removal_summary(&mut output, &summary, empty_message)?;
            assert_eq!(output, expected, "{summary:?}");
        }
        Ok(())
    }

    #[test]
    fn clean_summary() -> fmt::Result {
        assert_summary("No cache entries found")
    }

    #[test]
    fn prune_summary() -> fmt::Result {
        assert_summary("No unused entries found")
    }
}
