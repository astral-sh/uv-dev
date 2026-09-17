use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};

use uv_distribution_types::{RequirementSource, ResolutionRecorder};
use uv_normalize::PackageName;
use uv_pep440::Version;

use crate::{DependencyMode, Manifest, ResolverEnvironment};

/// A set of package versions that are permitted, even if they're marked as yanked by the
/// relevant index.
#[derive(Debug, Default, Clone)]
pub struct AllowedYanks {
    versions: Arc<FxHashMap<PackageName, FxHashSet<Version>>>,
    recorder: Option<ResolutionRecorder>,
}

impl AllowedYanks {
    pub fn from_manifest(
        manifest: &Manifest,
        env: &ResolverEnvironment,
        dependencies: DependencyMode,
    ) -> Self {
        let mut allowed_yanks = FxHashMap::<PackageName, FxHashSet<Version>>::default();

        // Allow yanks for any pinned input requirements.
        for requirement in manifest.candidate_selection_requirements(env, dependencies) {
            let RequirementSource::Registry { specifier, .. } = &requirement.source else {
                continue;
            };
            let [specifier] = specifier.as_ref() else {
                continue;
            };
            if matches!(
                specifier.operator(),
                uv_pep440::Operator::Equal | uv_pep440::Operator::ExactEqual
            ) {
                allowed_yanks
                    .entry(requirement.name.clone())
                    .or_default()
                    .insert(specifier.version().clone());
            }
        }

        // Allow yanks for packages explicitly pinned by the current resolution's inputs.
        for (name, preferences) in manifest.preferences.iter() {
            allowed_yanks.entry(name.clone()).or_default().extend(
                preferences
                    .filter(|entry| entry.source().allows_yanked())
                    .map(|entry| entry.pin().version().clone()),
            );
        }

        Self {
            versions: Arc::new(allowed_yanks),
            recorder: manifest.recorder.clone(),
        }
    }

    /// Returns `true` if the package-version is allowed, even if it's marked as yanked.
    pub(crate) fn contains(&self, package_name: &PackageName, version: &Version) -> bool {
        if let Some(recorder) = &self.recorder {
            recorder.candidate_policy(package_name);
        }
        self.versions
            .get(package_name)
            .is_some_and(|versions| versions.contains(version))
    }
}

#[cfg(test)]
mod tests {
    use uv_distribution_types::IndexUrl;

    use crate::{Preference, Preferences};

    use super::*;

    #[test]
    fn inherited_lock_preferences_do_not_allow_yanks() {
        let package_name = "example".parse::<PackageName>().expect("valid test name");
        let index = "https://pypi.org/simple"
            .parse::<IndexUrl>()
            .expect("valid test index");
        let own_version = "1".parse::<Version>().expect("valid test version");
        let inherited_version = "2".parse::<Version>().expect("valid test version");
        let env = ResolverEnvironment::universal(Vec::new());
        let mut manifest = Manifest::simple(Vec::new());
        manifest.preferences = Preferences::from_iter(
            [
                Preference::from_locked(
                    package_name.clone(),
                    own_version.clone(),
                    Some(index.clone()),
                    Vec::new(),
                ),
                Preference::from_inherited_locked(
                    package_name.clone(),
                    inherited_version.clone(),
                    index,
                    Vec::new(),
                ),
            ],
            &env,
        );

        let allowed = AllowedYanks::from_manifest(&manifest, &env, DependencyMode::Transitive);
        assert!(allowed.contains(&package_name, &own_version));
        assert!(!allowed.contains(&package_name, &inherited_version));
    }
}
