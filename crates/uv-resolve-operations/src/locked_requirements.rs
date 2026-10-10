use std::path::Path;

use anyhow::Result;
use tracing::info_span;

use uv_configuration::Upgrade;
use uv_distribution_types::{
    BuiltDist, Dist, IndexUrl, MinimumLibcVersion, ResolvedDist, SourceDist,
};
use uv_fs::CWD;
use uv_git::ResolvedRepositoryReference;
use uv_lock::{Lock, LockError, PylockToml, PylockTomlErrorKind};
use uv_pep508::{MarkerTree, VerbatimUrl};
use uv_requirements_txt::RequirementsTxt;
use uv_resolver::{
    Preference, PreferenceError, ResolverOutput, UpgradePackages, implied_markers_for_wheels,
};

#[derive(Debug, Default)]
pub struct LockedRequirements {
    /// The pinned versions from the lockfile.
    pub preferences: Vec<Preference>,
    /// The pinned Git SHAs from the lockfile.
    pub git: Vec<ResolvedRepositoryReference>,
}

impl LockedRequirements {
    /// Create a [`LockedRequirements`] from a list of preferences.
    pub fn from_preferences(preferences: Vec<Preference>) -> Self {
        Self {
            preferences,
            ..Self::default()
        }
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
    if upgrade.is_all() {
        return Ok(LockedRequirements::default());
    }

    let upgrade_packages = lock.upgrade_packages(upgrade);
    let mut preferences = Vec::new();
    let mut git = Vec::new();
    for package in lock.packages() {
        if upgrade_packages.contains(package.name()) {
            continue;
        }
        if let Some(git_ref) = package.as_git_ref()? {
            git.push(git_ref);
        }
        if let Some(version) = package.version() {
            preferences.push(Preference::from_locked(
                package.name().clone(),
                version.clone(),
                package.index(install_path)?,
                package.fork_markers().to_vec(),
            ));
        }
    }
    Ok(LockedRequirements { preferences, git })
}

/// Remove preferences whose selected registry artifacts do not cover a required environment.
///
/// The resolution must come from the same ordered preference list. Candidate selection records
/// each preference's input position, including reuse on other indexes and through local variants.
/// Preferences absent from this resolution remain available if a retry makes them reachable again.
/// Returns whether any preference was removed.
pub fn retain_wheel_ready_preferences(
    preferences: &mut Vec<Preference>,
    resolution: &ResolverOutput,
    required_environments: &[MarkerTree],
    minimum_libc_version: Option<MinimumLibcVersion>,
) -> bool {
    if preferences.is_empty() || required_environments.is_empty() {
        return false;
    }
    let mut retained = vec![true; preferences.len()];
    for (_, distribution) in resolution.base_dists() {
        if distribution.preferences.is_empty() {
            continue;
        }
        let ResolvedDist::Installable { dist, .. } = &distribution.dist else {
            continue;
        };
        let (index, wheels) = match dist.as_ref() {
            Dist::Built(BuiltDist::Registry(dist)) => (&dist.best_wheel().index, &dist.wheels),
            Dist::Source(SourceDist::Registry(dist)) => (&dist.index, &dist.wheels),
            Dist::Built(BuiltDist::DirectUrl(_) | BuiltDist::Path(_) | BuiltDist::GitPath(_))
            | Dist::Source(
                SourceDist::DirectUrl(_)
                | SourceDist::Path(_)
                | SourceDist::Directory(_)
                | SourceDist::GitPath(_)
                | SourceDist::GitDirectory(_),
            ) => continue,
        };
        let coverage = implied_markers_for_wheels(
            wheels
                .iter()
                .filter(|wheel| wheel.index == *index)
                .map(|wheel| &wheel.filename),
            minimum_libc_version,
        );
        for (preference, marker) in &distribution.preferences {
            let activation = resolution
                .requires_python
                .complexify_markers(marker.pep508());
            if required_environments.iter().any(|required| {
                let applicable = activation.and(*required);
                !applicable.is_false() && coverage.is_disjoint(applicable)
            }) {
                retained[*preference] = false;
            }
        }
    }

    let previous_count = preferences.len();
    let mut index = 0;
    preferences.retain(|_| {
        let keep = retained[index];
        index += 1;
        keep
    });
    preferences.len() != previous_count
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
        if let Some(git_ref) = package.as_git_ref()? {
            git.push(git_ref);
        }
    }

    Ok(LockedRequirements { preferences, git })
}
