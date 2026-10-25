use uv_configuration::NoSources;

/// Whether to show the bundled backend's optional warnings.
///
/// Source restrictions are a proxy for whether the user controls the source distribution.
/// Package-scoped source restrictions disable these warnings.
pub fn warn_for_bundled_backend(sources: &NoSources) -> bool {
    sources.is_none()
}

/// Whether to warn about `uv_build` settings used with a different build backend.
///
/// Package-scoped source restrictions do not suppress this warning.
pub(crate) fn warn_for_unused_settings(sources: &NoSources) -> bool {
    !sources.all()
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
            assert_eq!(
                (
                    warn_for_bundled_backend(&sources),
                    warn_for_unused_settings(&sources),
                ),
                expected,
                "Unexpected warning policy for {sources:?}",
            );
        }
    }
}
