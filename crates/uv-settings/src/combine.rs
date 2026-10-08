use std::path::PathBuf;
use std::{collections::BTreeMap, num::NonZeroUsize};

use url::Url;

use uv_configuration::{
    AddBoundsKind, AnnotationStyle, BuildIsolation, ExcludeNewer, ExcludeNewerPackage,
    ExportFormat, ForkStrategy, IndexStrategy, KeyringProviderType, NoSources, PrereleaseMode,
    PrereleasePackage, ProxyUrl, Reinstall, RequiredVersion, ResolutionMode, TargetTriple,
    TrustedPublishing, Upgrade,
};
use uv_distribution_types::{
    ConfigSettings, ExcludeNewerOverride, ExcludeNewerValue, ExtraBuildVariables, Index, IndexUrl,
    MinimumLibcVersion, PackageConfigSettings, PipExtraIndex, PipFindLinks, PipIndex,
};
use uv_install_wheel::LinkMode;
use uv_normalize::PackageName;
use uv_pypi_types::{SchemaConflicts, SupportedEnvironments};
use uv_python_types::{PythonDownloads, PythonPreference, PythonVersion};
use uv_redacted::DisplaySafeUrl;
use uv_torch::TorchMode;
use uv_workspace::pyproject::ExtraBuildDependencies;

use crate::{
    AuditOptions, FilesystemOptions, Options, PipOptions, PreviewOption, ResolverInstallerOptions,
    ResolverOptions,
};

pub trait Combine {
    /// Combine two values, preferring the values in `self`.
    ///
    /// The logic should follow that of Cargo's `config.toml`:
    ///
    /// > If a key is specified in multiple config files, the values will get merged together.
    /// > Numbers, strings, and booleans will use the value in the deeper config directory taking
    /// > precedence over ancestor directories, where the home directory is the lowest priority.
    /// > Arrays will be joined together with higher precedence items being placed later in the
    /// > merged array.
    ///
    /// ...with one exception: we place items with higher precedence earlier in the merged array.
    #[must_use]
    fn combine(self, other: Self) -> Self;
}

impl ResolverOptions {
    /// Combine CLI options above an environment layer and the remaining configuration.
    #[must_use]
    pub fn combine_with_environment(self, environment: Self, other: Self) -> Self {
        let no_build = environment.no_build;
        let no_binary = environment.no_binary;
        let no_sources = environment.no_sources;
        let mut other = environment.combine(other);
        preserve_cli_packages_after_environment_reset(
            self.no_build,
            self.no_build_package.as_deref(),
            no_build,
            &mut other.no_build,
            &mut other.no_build_package,
        );
        preserve_cli_packages_after_environment_reset(
            self.no_binary,
            self.no_binary_package.as_deref(),
            no_binary,
            &mut other.no_binary,
            &mut other.no_binary_package,
        );
        preserve_cli_packages_after_environment_reset(
            self.no_sources,
            self.no_sources_package.as_deref(),
            no_sources,
            &mut other.no_sources,
            &mut other.no_sources_package,
        );
        self.combine(other)
    }
}

impl ResolverInstallerOptions {
    /// Combine CLI options above an environment layer and the remaining configuration.
    #[must_use]
    pub fn combine_with_environment(self, environment: Self, other: Self) -> Self {
        let no_build = environment.no_build;
        let no_binary = environment.no_binary;
        let no_sources = environment.no_sources;
        let mut other = environment.combine(other);
        preserve_cli_packages_after_environment_reset(
            self.no_build,
            self.no_build_package.as_deref(),
            no_build,
            &mut other.no_build,
            &mut other.no_build_package,
        );
        preserve_cli_packages_after_environment_reset(
            self.no_binary,
            self.no_binary_package.as_deref(),
            no_binary,
            &mut other.no_binary,
            &mut other.no_binary_package,
        );
        preserve_cli_packages_after_environment_reset(
            self.no_sources,
            self.no_sources_package.as_deref(),
            no_sources,
            &mut other.no_sources,
            &mut other.no_sources_package,
        );
        self.combine(other)
    }
}

/// A false environment policy clears lower restrictions before CLI packages add specific ones.
fn preserve_cli_packages_after_environment_reset(
    cli: Option<bool>,
    cli_packages: Option<&[PackageName]>,
    environment: Option<bool>,
    other: &mut Option<bool>,
    other_packages: &mut Option<Vec<PackageName>>,
) {
    if cli.is_none()
        && cli_packages.is_some_and(|packages| !packages.is_empty())
        && environment == Some(false)
    {
        *other = None;
        *other_packages = None;
    }
}

impl Combine for Option<FilesystemOptions> {
    /// Combine the options used in two [`FilesystemOptions`]s. Retains the root of `self`.
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Some(a), Some(b)) => Some(FilesystemOptions(
                a.into_options().combine(b.into_options()),
            )),
            (a, b) => a.or(b),
        }
    }
}

impl Combine for Option<Options> {
    /// Combine the options used in two [`Options`]s. Retains the root of `self`.
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Some(a), Some(b)) => Some(a.combine(b)),
            (a, b) => a.or(b),
        }
    }
}

impl Combine for Option<PipOptions> {
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Some(a), Some(b)) => Some(a.combine(b)),
            (a, b) => a.or(b),
        }
    }
}

impl Combine for Option<AuditOptions> {
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Some(a), Some(b)) => Some(a.combine(b)),
            (a, b) => a.or(b),
        }
    }
}

macro_rules! impl_combine_or {
    ($name:ident) => {
        impl Combine for Option<$name> {
            fn combine(self, other: Option<$name>) -> Option<$name> {
                self.or(other)
            }
        }
    };
}

impl_combine_or!(AddBoundsKind);
impl_combine_or!(AnnotationStyle);
impl_combine_or!(ExcludeNewer);
impl_combine_or!(ExcludeNewerOverride);
impl_combine_or!(ExcludeNewerValue);
impl_combine_or!(ExportFormat);
impl_combine_or!(ForkStrategy);
impl_combine_or!(MinimumLibcVersion);
impl_combine_or!(Index);
impl_combine_or!(IndexStrategy);
impl_combine_or!(IndexUrl);
impl_combine_or!(KeyringProviderType);
impl_combine_or!(LinkMode);
impl_combine_or!(DisplaySafeUrl);
impl_combine_or!(NonZeroUsize);
impl_combine_or!(PathBuf);
impl_combine_or!(PipExtraIndex);
impl_combine_or!(PipFindLinks);
impl_combine_or!(PipIndex);
impl_combine_or!(PrereleaseMode);
impl_combine_or!(PreviewOption);
impl_combine_or!(ProxyUrl);
impl_combine_or!(PythonDownloads);
impl_combine_or!(PythonPreference);
impl_combine_or!(PythonVersion);
impl_combine_or!(RequiredVersion);
impl_combine_or!(ResolutionMode);
impl_combine_or!(SchemaConflicts);
impl_combine_or!(String);
impl_combine_or!(SupportedEnvironments);
impl_combine_or!(TargetTriple);
impl_combine_or!(TorchMode);
impl_combine_or!(TrustedPublishing);
impl_combine_or!(Url);
impl_combine_or!(bool);

impl<T> Combine for Option<Vec<T>> {
    /// Combine two vectors by extending the vector in `self` with the vector in `other`, if they're
    /// both `Some`.
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Some(mut a), Some(b)) => {
                a.extend(b);
                Some(a)
            }
            (a, b) => a.or(b),
        }
    }
}

impl<K: Ord, T> Combine for Option<BTreeMap<K, Vec<T>>> {
    /// Combine two maps of vecs by combining their vecs
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Some(mut a), Some(b)) => {
                for (key, value) in b {
                    a.entry(key).or_default().extend(value);
                }
                Some(a)
            }
            (a, b) => a.or(b),
        }
    }
}

impl Combine for Option<ExcludeNewerPackage> {
    /// Combine two [`ExcludeNewerPackage`] instances by merging them, with the values in `self` taking precedence.
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Some(mut a), Some(b)) => {
                // Extend with values from b, but a takes precedence (we don't overwrite existing keys)
                for (key, value) in b {
                    a.entry(key).or_insert(value);
                }
                Some(a)
            }
            (a, b) => a.or(b),
        }
    }
}

impl Combine for Option<PrereleasePackage> {
    /// Merge package-specific policies, retaining the higher-precedence value for duplicates.
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Some(mut current), Some(fallback)) => {
                for (package, mode) in fallback {
                    current.entry(package).or_insert(mode);
                }
                Some(current)
            }
            (current, fallback) => current.or(fallback),
        }
    }
}

impl Combine for Option<ConfigSettings> {
    /// Combine two maps by merging the map in `self` with the map in `other`, if they're both
    /// `Some`.
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Some(a), Some(b)) => Some(a.merge(b)),
            (a, b) => a.or(b),
        }
    }
}

impl Combine for Option<PackageConfigSettings> {
    /// Combine two maps by merging the map in `self` with the map in `other`, if they're both
    /// `Some`.
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Some(a), Some(b)) => Some(a.merge(b)),
            (a, b) => a.or(b),
        }
    }
}

impl Combine for Option<NoSources> {
    /// Combine two source strategies by using the `combine` method if they're both `Some`.
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Some(a), Some(b)) => Some(a.combine(b)),
            (a, b) => a.or(b),
        }
    }
}

impl Combine for Option<Upgrade> {
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Some(a), Some(b)) => Some(a.combine(b)),
            (a, b) => a.or(b),
        }
    }
}

impl Combine for Option<Reinstall> {
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Some(a), Some(b)) => Some(a.combine(b)),
            (a, b) => a.or(b),
        }
    }
}

impl Combine for Option<BuildIsolation> {
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Some(a), Some(b)) => Some(a.combine(b)),
            (a, b) => a.or(b),
        }
    }
}

impl Combine for serde::de::IgnoredAny {
    fn combine(self, _other: Self) -> Self {
        self
    }
}

impl Combine for Option<serde::de::IgnoredAny> {
    fn combine(self, _other: Self) -> Self {
        self
    }
}

impl Combine for ExcludeNewer {
    fn combine(mut self, other: Self) -> Self {
        self.global = self.global.combine(other.global);

        if !other.package.is_empty() {
            if self.package.is_empty() {
                self.package = other.package;
            } else {
                // Merge package-specific settings, with self taking precedence
                for (pkg, setting) in &other.package {
                    self.package
                        .entry(pkg.clone())
                        .or_insert_with(|| setting.clone());
                }
            }
        }

        self
    }
}

impl Combine for ExtraBuildDependencies {
    fn combine(mut self, other: Self) -> Self {
        for (key, value) in other {
            match self.entry(key) {
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    // Combine the vecs, with self taking precedence
                    let existing = entry.get_mut();
                    existing.extend(value);
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(value);
                }
            }
        }
        self
    }
}

impl Combine for Option<ExtraBuildDependencies> {
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Some(a), Some(b)) => Some(a.combine(b)),
            (a, b) => a.or(b),
        }
    }
}

impl Combine for ExtraBuildVariables {
    fn combine(mut self, other: Self) -> Self {
        for (key, value) in other {
            match self.entry(key) {
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    // Combine the maps, with self taking precedence
                    let existing = entry.get_mut();
                    for (k, v) in value {
                        existing.entry(k).or_insert(v);
                    }
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(value);
                }
            }
        }
        self
    }
}

impl Combine for Option<ExtraBuildVariables> {
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Some(a), Some(b)) => Some(a.combine(b)),
            (a, b) => a.or(b),
        }
    }
}
