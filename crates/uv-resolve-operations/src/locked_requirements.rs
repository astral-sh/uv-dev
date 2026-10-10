use std::path::Path;

use anyhow::Result;
use itertools::Either;
use tracing::info_span;

use uv_configuration::{BuildOptions, Upgrade};
use uv_distribution_types::{IndexUrl, MinimumLibcVersion, RequiresPython};
use uv_fs::CWD;
use uv_git::ResolvedRepositoryReference;
use uv_lock::{Lock, LockError, Package, PylockToml, PylockTomlErrorKind};
use uv_pep508::{MarkerTree, VerbatimUrl};
use uv_requirements_txt::RequirementsTxt;
use uv_resolver::{Preference, PreferenceError, UpgradePackages, implied_markers_for_wheels};

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
///
/// Without current activation information, wheel coverage checks treat every package as active.
pub fn read_lock_requirements(
    lock: &Lock,
    install_path: &Path,
    upgrade: &Upgrade,
    requires_python: &RequiresPython,
    build_options: &BuildOptions,
    required_environments: &[MarkerTree],
    activation: Option<Vec<(&Package, MarkerTree)>>,
    minimum_libc_version: Option<MinimumLibcVersion>,
) -> Result<LockedRequirements, LockError> {
    // As an optimization, skip iterating over the lockfile is we're upgrading all packages anyway.
    if upgrade.is_all() {
        return Ok(LockedRequirements::default());
    }

    let upgrade_packages = lock.upgrade_packages(upgrade);

    let mut candidates = Vec::new();
    let mut git = Vec::new();
    let mut changed_activation = MarkerTree::FALSE;
    let missing_wheels = |package: &Package, activation: MarkerTree| {
        let mut wheel_coverage = None;
        required_environments.iter().copied().any(|marker| {
            let applicable = activation.and(marker);
            !applicable.is_false()
                && wheel_coverage
                    .get_or_insert_with(|| {
                        implied_markers_for_wheels(package.wheel_filenames(), minimum_libc_version)
                    })
                    .is_disjoint(applicable)
        })
    };

    let packages = if let Some(activation) = activation {
        Either::Right(activation.into_iter())
    } else {
        Either::Left(
            lock.packages()
                .iter()
                .map(|package| (package, MarkerTree::TRUE)),
        )
    };
    for (package, activation) in packages {
        // Skip the distribution if it's included in the upgrade strategy (either by explicit
        // package name or via a dependency group).
        if upgrade_packages.contains(package.name()) {
            if !required_environments.is_empty() {
                changed_activation = changed_activation.or(activation);
            }
            continue;
        }

        // Wheel readiness can discard a version preference without upgrading a pinned Git ref.
        if let Some(git_ref) = package.as_git_ref()? {
            git.push(git_ref);
        }

        // Disallowed artifacts can force a replacement release even when its old wheels cover
        // the required environments. Wheel readiness independently discards source-only preferences.
        if (!required_environments.is_empty()
            && !package.satisfies_build_options(requires_python, build_options))
            || missing_wheels(package, activation)
        {
            // Registry selection can move to another release. Fixed URLs, Git pins, and
            // workspace sources retain their dependency metadata when only wheel preference changes.
            if package.index(install_path)?.is_some() {
                changed_activation = changed_activation.or(activation);
            }
            continue;
        }

        // Map each entry in the lockfile to a preference.
        if let Some(version) = package.version() {
            let index = package.index(install_path)?;
            candidates.push((
                package,
                activation,
                index.is_some(),
                Preference::from_locked(
                    package.name().clone(),
                    version.clone(),
                    index,
                    package.fork_markers().to_vec(),
                ),
            ));
        }
    }

    // A replacement release can introduce edges to any locked package. Old adjacency cannot
    // prove those packages inactive, so reconsider their preferences within the unlocked parents'
    // activation domain. Required environments outside that domain retain their existing pins.
    // Newly unlocked registry packages can expand that domain again, so revisit retained candidates
    // until it stops growing. Each expansion removes at least one candidate.
    while !changed_activation.is_false() {
        let previous_activation = changed_activation;
        candidates.retain(|(package, activation, is_registry, _)| {
            if !missing_wheels(package, activation.or(changed_activation)) {
                return true;
            }
            if *is_registry {
                changed_activation = changed_activation.or(*activation);
            }
            false
        });
        if changed_activation == previous_activation {
            break;
        }
    }
    let preferences = candidates
        .into_iter()
        .map(|(_, _, _, preference)| preference)
        .collect();
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
