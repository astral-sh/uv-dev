//! Settings types for project operations.

use uv_configuration::{
    BuildOptions, ConfigSettings, ExcludeNewer, IndexStrategy, KeyringProviderType, LinkMode,
    PackageConfigSettings, SourceStrategy,
};
use uv_distribution_types::{DependencyMetadata, IndexLocations};
use uv_installer::Reinstall;
use uv_normalize::PackageName;

/// Installer settings borrowed for use in project operations.
#[derive(Debug, Clone)]
pub struct InstallerSettingsRef<'a> {
    pub index_locations: &'a IndexLocations,
    pub index_strategy: IndexStrategy,
    pub keyring_provider: KeyringProviderType,
    pub dependency_metadata: &'a DependencyMetadata,
    pub config_setting: &'a ConfigSettings,
    pub config_settings_package: &'a PackageConfigSettings,
    pub no_build_isolation: bool,
    pub no_build_isolation_package: &'a [PackageName],
    pub exclude_newer: Option<ExcludeNewer>,
    pub link_mode: LinkMode,
    pub compile_bytecode: bool,
    pub reinstall: &'a Reinstall,
    pub build_options: &'a BuildOptions,
    pub sources: SourceStrategy,
}

impl<'a> From<&'a uv_cli_common::settings::ResolverInstallerSettings> for InstallerSettingsRef<'a> {
    fn from(settings: &'a uv_cli_common::settings::ResolverInstallerSettings) -> Self {
        Self {
            index_locations: todo!("index_locations"),
            index_strategy: settings.resolver.index_strategy,
            keyring_provider: settings.resolver.keyring_provider,
            dependency_metadata: &settings.resolver.dependency_metadata,
            config_setting: &settings.resolver.config_setting,
            config_settings_package: &settings.resolver.config_settings_package,
            no_build_isolation: todo!("no_build_isolation"),
            no_build_isolation_package: &[],
            exclude_newer: settings.resolver.exclude_newer,
            link_mode: todo!("link_mode"),
            compile_bytecode: settings.compile_bytecode,
            reinstall: &settings.reinstall,
            build_options: &settings.resolver.build_options,
            sources: settings.resolver.source_strategy,
        }
    }
}
