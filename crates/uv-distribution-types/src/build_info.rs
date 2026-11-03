use uv_cache_key::{CacheKey, CacheKeyHasher, cache_digest};

use crate::{BuildVariables, ConfigSettings, ExtraBuildRequirement, Requirement};

/// A digest representing the build settings, such as build dependencies or other build-time
/// configuration.
#[derive(Default, Debug, Clone, Hash, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct BuildInfo {
    #[serde(default, skip_serializing_if = "ConfigSettings::is_empty")]
    config_settings: ConfigSettings,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    extra_build_requires: Vec<ExtraBuildRequirement>,
    #[serde(default, skip_serializing_if = "BuildVariables::is_empty")]
    extra_build_variables: BuildVariables,
}

impl CacheKey for BuildInfo {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        self.config_settings.cache_key(state);
        self.extra_build_requires.cache_key(state);
        self.extra_build_variables.cache_key(state);
    }
}

impl BuildInfo {
    /// Creates a [`BuildInfo`] instance with the given configuration settings, extra build
    /// dependencies, and extra build variables.
    pub fn from_settings(
        config_settings: ConfigSettings,
        extra_build_dependencies: Vec<ExtraBuildRequirement>,
        extra_build_variables: Option<BuildVariables>,
    ) -> Self {
        Self {
            config_settings,
            extra_build_requires: extra_build_dependencies,
            extra_build_variables: extra_build_variables.unwrap_or_default(),
        }
    }

    /// Compare build settings before resolution, verifying each recorded runtime pin against
    /// the installed runtime selection. The remaining declaration and settings must match exactly.
    pub fn matches_with_runtime(
        &self,
        mut expected: Self,
        pin_is_current: impl Fn(&Requirement) -> bool,
    ) -> bool {
        for (declared, recorded) in expected
            .extra_build_requires
            .iter_mut()
            .zip(&self.extra_build_requires)
        {
            if declared.match_runtime && declared.requirement.source.is_empty() {
                if !recorded.match_runtime || !pin_is_current(&recorded.requirement) {
                    return false;
                }
                declared
                    .requirement
                    .source
                    .clone_from(&recorded.requirement.source);
            }
        }
        *self == expected
    }

    /// Returns `true` if the [`BuildInfo`] is empty, meaning it has no configuration settings,
    fn is_empty(&self) -> bool {
        self.config_settings.is_empty()
            && self.extra_build_requires.is_empty()
            && self.extra_build_variables.is_empty()
    }

    /// Return the cache shard for this [`BuildInfo`].
    pub fn cache_shard(&self) -> Option<String> {
        if self.is_empty() {
            None
        } else {
            Some(cache_digest(self))
        }
    }
}
