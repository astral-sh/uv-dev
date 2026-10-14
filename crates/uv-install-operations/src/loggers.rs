use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Write;

use itertools::Itertools;
use owo_colors::{AnsiColors, OwoColorize};

use uv_configuration::DryRun;
use uv_distribution_types::Name;
use uv_normalize::PackageName;

use crate::{ChangeEvent, ChangeEventKind, ChangedDist, Changelog, ShortSpecifier};
use uv_command_support::Printer;
use uv_command_support::elapsed;

/// A trait to handle logging during install operations.
pub trait InstallLogger {
    /// Log the completion of the audit phase.
    fn on_check(
        &self,
        count: usize,
        start: std::time::Instant,
        printer: Printer,
        dry_run: DryRun,
    ) -> fmt::Result;

    /// Log the completion of the preparation phase.
    fn on_prepare(
        &self,
        count: usize,
        suffix: Option<&str>,
        start: std::time::Instant,
        printer: Printer,
        dry_run: DryRun,
    ) -> fmt::Result;

    /// Log the completion of the uninstallation phase.
    fn on_uninstall(
        &self,
        count: usize,
        start: std::time::Instant,
        printer: Printer,
        dry_run: DryRun,
    ) -> fmt::Result;

    /// Log the completion of the installation phase.
    fn on_install(
        &self,
        count: usize,
        start: std::time::Instant,
        printer: Printer,
        dry_run: DryRun,
    ) -> fmt::Result;

    /// Log the completion of the operation.
    fn on_complete(&self, changelog: &Changelog, printer: Printer, dry_run: DryRun) -> fmt::Result;
}

/// The default logger for install operations.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultInstallLogger;

/// An installation phase with a package-count summary.
enum InstallPhase<'a> {
    Prepare(Option<&'a str>),
    Uninstall,
    Install,
}

impl InstallPhase<'_> {
    fn write(
        self,
        count: usize,
        start: std::time::Instant,
        printer: Printer,
        dry_run: DryRun,
    ) -> fmt::Result {
        let (completed, pending, suffix) = match self {
            Self::Prepare(suffix) => ("Prepared", "Would download", suffix),
            Self::Uninstall => ("Uninstalled", "Would uninstall", None),
            Self::Install => ("Installed", "Would install", None),
        };
        let plural = if count == 1 { "" } else { "s" };
        let what = if let Some(suffix) = suffix {
            format!("{count} package{plural} {suffix}")
        } else {
            format!("{count} package{plural}")
        };
        let what = what.bold();
        writeln!(
            printer.stderr(),
            "{}",
            if dry_run.enabled() {
                format!("{pending} {what}")
            } else {
                format!(
                    "{completed} {what} {}",
                    format!("in {}", elapsed(start.elapsed())).dimmed()
                )
            }
            .dimmed()
        )
    }
}

impl InstallLogger for DefaultInstallLogger {
    fn on_check(
        &self,
        count: usize,
        start: std::time::Instant,
        printer: Printer,
        dry_run: DryRun,
    ) -> fmt::Result {
        if count == 0 {
            writeln!(
                printer.stderr(),
                "{}",
                format!("Checked in {}", elapsed(start.elapsed())).dimmed()
            )?;
        } else {
            let s = if count == 1 { "" } else { "s" };
            writeln!(
                printer.stderr(),
                "{}",
                format!(
                    "Checked {} {}",
                    format!("{count} package{s}").bold(),
                    format!("in {}", elapsed(start.elapsed())).dimmed()
                )
                .dimmed()
            )?;
        }
        if dry_run.enabled() {
            writeln!(printer.stderr(), "Would make no changes")?;
        }
        Ok(())
    }

    fn on_prepare(
        &self,
        count: usize,
        suffix: Option<&str>,
        start: std::time::Instant,
        printer: Printer,
        dry_run: DryRun,
    ) -> fmt::Result {
        InstallPhase::Prepare(suffix).write(count, start, printer, dry_run)
    }

    fn on_uninstall(
        &self,
        count: usize,
        start: std::time::Instant,
        printer: Printer,
        dry_run: DryRun,
    ) -> fmt::Result {
        InstallPhase::Uninstall.write(count, start, printer, dry_run)
    }

    fn on_install(
        &self,
        count: usize,
        start: std::time::Instant,
        printer: Printer,
        dry_run: DryRun,
    ) -> fmt::Result {
        InstallPhase::Install.write(count, start, printer, dry_run)
    }

    fn on_complete(
        &self,
        changelog: &Changelog,
        printer: Printer,
        _dry_run: DryRun,
    ) -> fmt::Result {
        for event in changelog
            .uninstalled
            .iter()
            .map(|distribution| ChangeEvent {
                dist: distribution,
                kind: ChangeEventKind::Removed,
            })
            .chain(changelog.installed.iter().map(|distribution| ChangeEvent {
                dist: distribution,
                kind: ChangeEventKind::Added,
            }))
            .chain(
                changelog
                    .reinstalled
                    .iter()
                    .map(|distribution| ChangeEvent {
                        dist: distribution,
                        kind: ChangeEventKind::Reinstalled,
                    }),
            )
            .sorted_unstable_by(|a, b| {
                a.dist
                    .name()
                    .cmp(b.dist.name())
                    .then_with(|| a.kind.cmp(&b.kind))
                    .then_with(|| a.dist.long_specifier().cmp(&b.dist.long_specifier()))
            })
        {
            let (symbol, color) = match event.kind {
                ChangeEventKind::Added => ("+", AnsiColors::Green),
                ChangeEventKind::Removed => ("-", AnsiColors::Red),
                ChangeEventKind::Reinstalled => ("~", AnsiColors::Yellow),
            };
            writeln!(
                printer.stderr(),
                " {} {}{}",
                symbol.color(color),
                event.dist.name().bold(),
                event.dist.long_specifier().dimmed()
            )?;
        }
        Ok(())
    }
}

/// A logger that only shows installs and uninstalls, the minimal logging necessary to understand
/// environment changes.
#[derive(Debug, Default, Clone, Copy)]
pub struct SummaryInstallLogger;

impl InstallLogger for SummaryInstallLogger {
    fn on_check(
        &self,
        _count: usize,
        _start: std::time::Instant,
        _printer: Printer,
        _dry_run: DryRun,
    ) -> fmt::Result {
        Ok(())
    }

    fn on_prepare(
        &self,
        _count: usize,
        _suffix: Option<&str>,
        _start: std::time::Instant,
        _printer: Printer,
        _dry_run: DryRun,
    ) -> fmt::Result {
        Ok(())
    }

    fn on_uninstall(
        &self,
        count: usize,
        start: std::time::Instant,
        printer: Printer,
        dry_run: DryRun,
    ) -> fmt::Result {
        DefaultInstallLogger.on_uninstall(count, start, printer, dry_run)
    }

    fn on_install(
        &self,
        count: usize,
        start: std::time::Instant,
        printer: Printer,
        dry_run: DryRun,
    ) -> fmt::Result {
        DefaultInstallLogger.on_install(count, start, printer, dry_run)
    }

    fn on_complete(
        &self,
        _changelog: &Changelog,
        _printer: Printer,
        _dry_run: DryRun,
    ) -> fmt::Result {
        Ok(())
    }
}

/// A logger that shows special output for the modification of the given target.
#[derive(Debug, Clone)]
pub struct UpgradeInstallLogger {
    target: PackageName,
}

impl UpgradeInstallLogger {
    /// Create a new logger for the given target.
    pub fn new(target: PackageName) -> Self {
        Self { target }
    }
}

/// The distinct specifiers associated with an upgrade target.
#[derive(Debug, PartialEq, Eq)]
struct TargetSpecifiers<'a>(BTreeSet<ShortSpecifier<'a>>);

impl<'a> TargetSpecifiers<'a> {
    fn for_package(
        package: &PackageName,
        distributions: impl IntoIterator<Item = &'a ChangedDist>,
    ) -> Option<Self> {
        let specifiers = distributions
            .into_iter()
            .filter(|distribution| distribution.name() == package)
            .map(ChangedDist::short_specifier)
            .collect::<BTreeSet<_>>();
        (!specifiers.is_empty()).then_some(Self(specifiers))
    }
}

impl fmt::Display for TargetSpecifiers<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let specifiers = self.0.iter().format_with(", ", |specifier, write| {
            write(&format_args!("v{specifier}"))
        });
        write!(formatter, "{specifiers}")
    }
}

impl InstallLogger for UpgradeInstallLogger {
    fn on_check(
        &self,
        _count: usize,
        _start: std::time::Instant,
        _printer: Printer,
        _dry_run: DryRun,
    ) -> fmt::Result {
        Ok(())
    }

    fn on_prepare(
        &self,
        _count: usize,
        _suffix: Option<&str>,
        _start: std::time::Instant,
        _printer: Printer,
        _dry_run: DryRun,
    ) -> fmt::Result {
        Ok(())
    }

    fn on_uninstall(
        &self,
        _count: usize,
        _start: std::time::Instant,
        _printer: Printer,
        _dry_run: DryRun,
    ) -> fmt::Result {
        Ok(())
    }

    fn on_install(
        &self,
        _count: usize,
        _start: std::time::Instant,
        _printer: Printer,
        _dry_run: DryRun,
    ) -> fmt::Result {
        Ok(())
    }

    fn on_complete(
        &self,
        changelog: &Changelog,
        printer: Printer,
        // TODO(tk): Adjust format for dry_run
        _dry_run: DryRun,
    ) -> fmt::Result {
        let removals = TargetSpecifiers::for_package(&self.target, &changelog.uninstalled);
        let additions = TargetSpecifiers::for_package(&self.target, &changelog.installed);

        // Summarize the change for the target.
        match (removals, additions) {
            (Some(removals), Some(additions)) if removals == additions => {
                writeln!(
                    printer.stderr(),
                    "{} {} {}",
                    "Reinstalled".yellow().bold(),
                    self.target,
                    additions
                )?;
            }
            (Some(removals), Some(additions)) => {
                writeln!(
                    printer.stderr(),
                    "{} {} {} -> {}",
                    "Updated".green().bold(),
                    self.target,
                    removals,
                    additions
                )?;
            }
            (Some(removals), None) => {
                writeln!(
                    printer.stderr(),
                    "{} {} {}",
                    "Removed".red().bold(),
                    self.target,
                    removals
                )?;
            }
            (None, Some(additions)) => {
                writeln!(
                    printer.stderr(),
                    "{} {} {}",
                    "Added".green().bold(),
                    self.target,
                    additions
                )?;
            }
            (None, None) => {
                writeln!(
                    printer.stderr(),
                    "{} {} {}",
                    "Modified".dimmed(),
                    self.target.dimmed().bold(),
                    "environment".dimmed()
                )?;
            }
        }

        // Follow-up with a detailed summary of all changes.
        DefaultInstallLogger.on_complete(changelog, printer, _dry_run)?;

        Ok(())
    }
}
