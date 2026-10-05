use uv_client::Connectivity;
use uv_configuration::NoSources;
use uv_configuration::{BuildOptions, IndexStrategy, KeyringProviderType, Reinstall, Upgrade};
use uv_distribution_types::{
    ConfigSettings, DependencyMetadata, IndexLocations, PackageConfigSettings,
};
use uv_resolver::{
    AnnotationStyle, DependencyMode, ExcludeNewer, FlatIndex, PrereleaseMode, ResolutionMode,
};

/// Network-related settings shared across commands.
#[derive(Debug)]
pub struct NetworkSettings {
    pub connectivity: Connectivity,
    pub native_tls: bool,
    pub allow_insecure_host: Vec<uv_configuration::TrustedHost>,
}

/// Resolver settings shared across commands.
#[derive(Debug, Clone)]
pub struct ResolverSettings {
    pub build_options: BuildOptions,
    pub config_setting: ConfigSettings,
    pub config_settings_package: PackageConfigSettings,
    pub dependency_metadata: DependencyMetadata,
    pub exclude_newer: Option<ExcludeNewer>,
    pub dependency_mode: DependencyMode,
    pub flat_index: FlatIndex,
    pub index_locations: IndexLocations,
    pub index_strategy: IndexStrategy,
    pub keyring_provider: KeyringProviderType,
    pub prerelease_mode: PrereleaseMode,
    pub resolution_mode: ResolutionMode,
    pub annotation_style: AnnotationStyle,
    pub sources: NoSources,
    pub upgrade: Option<Upgrade>,
    // TODO: ExtrasResolver requires a generic BuildContext parameter
    // This needs to be handled differently in this crate
}

/// Combined resolver and installer settings.
#[derive(Debug, Clone)]
pub struct ResolverInstallerSettings {
    pub resolver: ResolverSettings,
    pub compile_bytecode: bool,
    pub reinstall: Reinstall,
}
