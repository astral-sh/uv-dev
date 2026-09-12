use uv_configuration::NoSources;

/// Whether to show warnings about `uv_build` settings that the user can act on.
///
/// The source strategy is a proxy for whether the user controls the source distribution. The
/// bundled backend and unused-settings warning intentionally use different historical checks for
/// package-scoped source restrictions.
#[derive(Debug, Clone, Copy)]
pub struct UvBuildWarningPolicy {
    bundled_backend: bool,
    unused_settings: bool,
}

impl UvBuildWarningPolicy {
    /// Determine the warning policy for the given source strategy.
    pub fn from_sources(sources: &NoSources) -> Self {
        match sources {
            NoSources::None => Self {
                bundled_backend: true,
                unused_settings: true,
            },
            NoSources::All => Self {
                bundled_backend: false,
                unused_settings: false,
            },
            NoSources::Packages(_) => Self {
                bundled_backend: false,
                unused_settings: true,
            },
        }
    }

    /// Whether to show the bundled backend's optional warnings.
    pub fn warn_for_bundled_backend(self) -> bool {
        self.bundled_backend
    }

    /// Whether to warn about `uv_build` settings used with a different build backend.
    pub(crate) fn warn_for_unused_settings(self) -> bool {
        self.unused_settings
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use uv_normalize::PackageName;

    use super::*;

    #[test]
    fn source_strategy_warning_policy() {
        let package = PackageName::from_str("project").expect("Valid package name");
        for (sources, expected) in [
            (NoSources::None, (true, true)),
            (NoSources::All, (false, false)),
            (NoSources::Packages(Vec::new()), (false, true)),
            (NoSources::Packages(vec![package]), (false, true)),
        ] {
            let policy = UvBuildWarningPolicy::from_sources(&sources);
            assert_eq!(
                (
                    policy.warn_for_bundled_backend(),
                    policy.warn_for_unused_settings(),
                ),
                expected,
                "Unexpected warning policy for {sources:?}",
            );
        }
    }
}
