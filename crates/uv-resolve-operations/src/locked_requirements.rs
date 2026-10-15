use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use tracing::info_span;

use uv_configuration::Upgrade;
use uv_distribution_types::IndexUrl;
use uv_fs::CWD;
use uv_git::ResolvedRepositoryReference;
use uv_lock::{Lock, LockError, PylockToml, PylockTomlErrorKind};
use uv_normalize::{ExtraName, GroupName};
use uv_pep508::{MarkerEnvironment, MarkerTree, VerbatimUrl};
use uv_requirements_txt::RequirementsTxt;
use uv_resolver::{Preference, PreferenceError, UniversalMarker, UpgradePackages};

#[derive(Debug, Default)]
pub struct LockedRequirements {
    /// The pinned versions from the lockfile.
    pub preferences: Vec<Preference>,
    /// The pinned Git SHAs from the lockfile.
    pub git: Vec<ResolvedRepositoryReference>,
}

impl LockedRequirements {
    /// Retain fallback version preferences outside each package's imported marker domains,
    /// and prepend fallback Git references.
    #[must_use]
    pub fn with_fallback(mut self, fallback: Self) -> Self {
        let mut markers = BTreeMap::new();
        for preference in &self.preferences {
            let marker = markers
                .entry(preference.name().clone())
                .or_insert(MarkerTree::FALSE);
            *marker = marker.or(preference.marker());
        }
        self.preferences
            .extend(fallback.preferences.into_iter().filter_map(|preference| {
                if let Some(marker) = markers.get(preference.name()) {
                    preference.with_marker(marker.negate())
                } else {
                    Some(preference)
                }
            }));
        self.git.splice(0..0, fallback.git);
        self
    }

    /// Create a [`LockedRequirements`] from a list of preferences.
    pub fn from_preferences(preferences: Vec<Preference>) -> Self {
        Self {
            preferences,
            ..Self::default()
        }
    }

    /// Extract version and Git preferences from selected packages in a [`PylockToml`].
    pub fn from_pylock(
        lock: &PylockToml,
        install_path: &Path,
        extras: &[ExtraName],
        groups: &[GroupName],
        upgrade: &Upgrade,
        environment: Option<&MarkerEnvironment>,
    ) -> Result<Self> {
        if upgrade.is_all() {
            return Ok(Self::default());
        }
        let upgrade_packages = UpgradePackages::for_non_project(upgrade);
        let mut preferences = Vec::new();
        let mut git: Vec<ResolvedRepositoryReference> = Vec::new();
        for package in &lock.packages {
            if upgrade_packages.contains(&package.name) {
                continue;
            }
            let marker = package.marker().simplify_pep751(extras, groups);
            if marker.is_false()
                || environment.is_some_and(|environment| !marker.evaluate(environment, &[]))
            {
                continue;
            }
            if let Some(version) = &package.version {
                preferences.push(Preference::from_locked(
                    package.name.clone(),
                    version.clone(),
                    package.index.as_ref().map(|index| {
                        IndexUrl::from(VerbatimUrl::from(index.without_credentials().into_owned()))
                    }),
                    vec![UniversalMarker::from_combined(marker)],
                ));
            }
            if let Some(git_ref) = package.as_git_ref(install_path)? {
                if git.iter().any(|existing| {
                    existing.reference == git_ref.reference && existing.sha != git_ref.sha
                }) {
                    anyhow::bail!(
                        "Cannot import conflicting Git commits for the repository reference used by `{}`",
                        package.name
                    );
                }
                git.push(git_ref);
            }
        }
        Ok(Self { preferences, git })
    }
}

/// Load the preferred requirements from an existing `requirements.txt`, applying the upgrade strategy.
pub async fn read_requirements_txt(
    output_file: &Path,
    upgrade: &Upgrade,
) -> Result<Vec<Preference>> {
    // As an optimization, skip reading the lockfile is we're upgrading all packages anyway.
    if upgrade.is_all() {
        return Ok(Vec::new());
    }

    // Parse the requirements from the lockfile.
    let requirements_txt = RequirementsTxt::parse(output_file, &*CWD).await?;

    // Map each entry in the lockfile to a preference.
    let preferences = requirements_txt
        .requirements
        .into_iter()
        .map(Preference::from_entry)
        .filter_map(Result::transpose)
        .collect::<Result<Vec<_>, PreferenceError>>()?;

    // Apply the upgrade strategy to the requirements.
    let upgrade_packages = UpgradePackages::for_non_project(upgrade);

    Ok(if upgrade.is_none() {
        // Respect all pinned versions from the existing lockfile.
        preferences
    } else {
        // Ignore all pinned versions for packages that should be upgraded.
        preferences
            .into_iter()
            .filter(|preference| !upgrade_packages.contains(preference.name()))
            .collect()
    })
}

/// Load the preferred requirements from an existing lockfile, applying the upgrade strategy.
pub fn read_lock_requirements(
    lock: &Lock,
    install_path: &Path,
    upgrade: &Upgrade,
) -> Result<LockedRequirements, LockError> {
    // As an optimization, skip iterating over the lockfile is we're upgrading all packages anyway.
    if upgrade.is_all() {
        return Ok(LockedRequirements::default());
    }

    let upgrade_packages = lock.upgrade_packages(upgrade);

    let mut preferences = Vec::new();
    let mut git = Vec::new();

    for package in lock.packages() {
        // Skip the distribution if it's included in the upgrade strategy (either by explicit
        // package name or via a dependency group).
        if upgrade_packages.contains(package.name()) {
            continue;
        }

        // Map each entry in the lockfile to a preference.
        if let Some(version) = package.version() {
            preferences.push(Preference::from_locked(
                package.name().clone(),
                version.clone(),
                package.index(install_path)?,
                package.fork_markers().to_vec(),
            ));
        }

        // Map each entry in the lockfile to a Git SHA.
        if let Some(git_ref) = package.as_git_ref()? {
            git.push(git_ref);
        }
    }

    Ok(LockedRequirements { preferences, git })
}

/// Load the preferred requirements from an existing `pylock.toml` file, applying the upgrade strategy.
pub async fn read_pylock_toml_requirements(
    output_file: &Path,
    upgrade: &Upgrade,
) -> Result<LockedRequirements, PylockTomlErrorKind> {
    // As an optimization, skip iterating over the lockfile is we're upgrading all packages anyway.
    if upgrade.is_all() {
        return Ok(LockedRequirements::default());
    }

    // Read the `pylock.toml` from disk, and deserialize it from TOML.
    let content = fs_err::tokio::read_to_string(&output_file).await?;
    let lock = info_span!("toml::from_str upgrade", path = %output_file.display())
        .in_scope(|| toml::from_str::<PylockToml>(&content))?;
    let absolute = std::path::absolute(output_file)?;
    let install_path = absolute.parent().unwrap_or(&CWD);

    let upgrade_packages = UpgradePackages::for_non_project(upgrade);

    let mut preferences = Vec::new();
    let mut git = Vec::new();

    for package in &lock.packages {
        // Skip the distribution if it's not included in the upgrade strategy.
        if upgrade_packages.contains(&package.name) {
            continue;
        }

        // Map each entry in the lockfile to a preference.
        if let Some(version) = package.version.as_ref() {
            preferences.push(Preference::from_locked(
                package.name.clone(),
                version.clone(),
                package
                    .index
                    .as_ref()
                    .map(|index| IndexUrl::from(VerbatimUrl::from(index.clone()))),
                vec![],
            ));
        }

        // Map each entry in the lockfile to a Git SHA.
        if let Some(git_ref) = package.as_git_ref(install_path)? {
            git.push(git_ref);
        }
    }

    Ok(LockedRequirements { preferences, git })
}
