use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Context;
use uv_command_support::Printer;
use uv_configuration::DryRun;
use uv_fs::Simplified;
use uv_install_wheel::{RequestedExtras, RequestedExtrasUpdate};
use uv_installer::SitePackages;
use uv_normalize::PackageName;
use uv_pep440::Version;

use crate::loggers::InstallLogger;
use crate::{Changelog, Error};

/// An installed distribution that may need an extras receipt update.
pub(crate) struct InstalledSelection<'a> {
    name: &'a PackageName,
    version: Version,
    path: PathBuf,
    extras: &'a RequestedExtras,
}

/// Receipt changes for distributions that will remain installed at the same location.
#[derive(Default)]
pub struct RequestedExtrasChanges(Vec<(PackageName, Version, RequestedExtrasUpdate)>);

impl RequestedExtrasChanges {
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn candidates<'a>(
        requested: &'a BTreeMap<PackageName, RequestedExtras>,
        installed: &SitePackages,
    ) -> Vec<InstalledSelection<'a>> {
        requested
            .iter()
            .flat_map(|(name, extras)| {
                installed
                    .get_packages(name)
                    .into_iter()
                    .filter_map(move |dist| {
                        Some(InstalledSelection {
                            name,
                            version: dist.version().clone(),
                            path: dist.dist_info_path()?.to_owned(),
                            extras,
                        })
                    })
            })
            .collect()
    }

    pub(crate) fn plan(
        candidates: Vec<InstalledSelection<'_>>,
        retained: impl Fn(&Path) -> bool,
    ) -> Result<Self, Error> {
        let mut changes = Vec::new();
        for InstalledSelection {
            name,
            version,
            path,
            extras,
        } in candidates
        {
            if retained(&path)
                && let Some(update) = extras.plan(&path).with_context(|| {
                    format!(
                        "Failed to plan requested extras for `{name}` at `{}`",
                        path.user_display()
                    )
                })?
            {
                changes.push((name.clone(), version, update));
            }
        }
        Ok(Self(changes))
    }

    /// Plan changes for a satisfied environment, where no distributions will be replaced.
    pub fn for_satisfied(
        requested: &BTreeMap<PackageName, RequestedExtras>,
        installed: &SitePackages,
    ) -> Result<Self, Error> {
        Self::plan(Self::candidates(requested, installed), |_| true)
    }

    pub(crate) fn apply(self, changelog: &mut Changelog, dry_run: DryRun) -> Result<(), Error> {
        for (name, version, update) in self.0 {
            if !dry_run.enabled() {
                update
                    .apply()
                    .with_context(|| format!("Failed to record requested extras for `{name}`"))?;
            }
            changelog.requested_extras.insert(name, version);
        }
        Ok(())
    }

    /// Finish an installation with no package changes, including a check or dry run.
    pub fn finish(
        self,
        count: usize,
        start: Instant,
        logger: &dyn InstallLogger,
        dry_run: DryRun,
        printer: Printer,
    ) -> Result<Changelog, Error> {
        let mut changelog = Changelog::default();
        self.apply(&mut changelog, dry_run)?;
        if changelog.is_empty() {
            logger.on_check(count, start, printer, dry_run)?;
        } else {
            logger.on_complete(&changelog, printer, dry_run)?;
            if matches!(dry_run, DryRun::Check) {
                return Err(Error::OutdatedEnvironment(Box::new(changelog)));
            }
        }
        Ok(changelog)
    }
}
