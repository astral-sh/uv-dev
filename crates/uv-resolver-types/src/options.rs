use uv_configuration::{BuildOptions, IndexStrategy};
use uv_distribution_types::MinimumLibcVersion;
use uv_pypi_types::SupportedEnvironments;
use uv_torch::TorchStrategy;

use uv_configuration::ForkStrategy;
use uv_configuration::{DependencyMode, ExcludeNewer, Prerelease, ResolutionMode};

/// Options for resolving a manifest.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Options {
    pub resolution_mode: ResolutionMode,
    pub prerelease: Prerelease,
    pub dependency_mode: DependencyMode,
    pub fork_strategy: ForkStrategy,
    pub exclude_newer: ExcludeNewer,
    pub index_strategy: IndexStrategy,
    pub artifact_environments: SupportedEnvironments,
    pub minimum_libc_version: Option<MinimumLibcVersion>,
    pub flexibility: Flexibility,
    pub build_options: BuildOptions,
    pub torch_backend: Option<TorchStrategy>,
}

/// Whether the [`Options`] are configurable or fixed.
///
/// Applies to the [`ResolutionMode`], [`Prerelease`], and [`DependencyMode`] fields.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Flexibility {
    /// The setting is configurable.
    #[default]
    Configurable,
    /// The setting is fixed.
    Fixed,
}
