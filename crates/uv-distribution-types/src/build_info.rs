use uv_cache_key::{CacheKey, CacheKeyHasher, cache_digest};

use crate::{BuildVariables, ConfigSettings, ExtraBuildRequirement};

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

#[cfg(test)]
mod tests {
    use uv_cache_key::cache_digest;
    use uv_pep440::Version;
    use uv_pep508::Requirement as Pep508Requirement;
    use uv_pypi_types::VerbatimParsedUrl;

    use crate::{BuildInfo, BuildVariables, ConfigSettings, ExtraBuildRequirement, Requirement};

    #[test]
    fn wildcard_build_requirements_leave_legacy_cache_shard() {
        let version = Version::new([1, 2]);
        for (operator, exact, wildcard) in [
            ("==", "foo==1.2", "foo==1.2.*"),
            ("!=", "foo!=1.2", "foo!=1.2.*"),
        ] {
            // The old registry-requirement encoding omitted the wildcard flag. Describe it
            // independently with typed fields so this also works on different pointer widths.
            let legacy_requirement = (
                "foo", 0usize, 0usize, 0u8, 0u8, 1usize, operator, &version, 0u8,
            );
            let legacy_key = cache_digest(&legacy_requirement);
            let legacy_shard = cache_digest(&(
                ConfigSettings::default(),
                1usize,
                legacy_requirement,
                false,
                BuildVariables::default(),
            ));

            let shards = [(exact, false), (wildcard, true)].map(|(input, is_star)| {
                let requirement = Requirement::from(
                    input
                        .parse::<Pep508Requirement<VerbatimParsedUrl>>()
                        .expect("valid test requirement"),
                );
                let expected_requirement = (
                    "foo",
                    0usize,
                    0usize,
                    0u8,
                    0u8,
                    1usize,
                    (operator, is_star, &version),
                    0u8,
                );
                assert_eq!(
                    cache_digest(&requirement),
                    cache_digest(&expected_requirement),
                );
                assert_ne!(cache_digest(&requirement), legacy_key);

                let build_info = BuildInfo::from_settings(
                    ConfigSettings::default(),
                    vec![ExtraBuildRequirement {
                        requirement,
                        match_runtime: false,
                    }],
                    None,
                );
                let shard = build_info
                    .cache_shard()
                    .expect("build requirements create a cache shard");
                assert_eq!(
                    shard,
                    cache_digest(&(
                        ConfigSettings::default(),
                        1usize,
                        expected_requirement,
                        false,
                        BuildVariables::default(),
                    )),
                );
                assert_ne!(shard, legacy_shard);
                shard
            });
            assert_ne!(shards[0], shards[1]);
        }
    }
}
