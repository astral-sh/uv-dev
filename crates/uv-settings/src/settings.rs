#[cfg(feature = "schemars")]
use std::borrow::Cow;
use std::{fmt::Debug, num::NonZeroUsize, path::Path, path::PathBuf};

use serde::{Deserialize, Serialize};

use uv_cache_info::CacheKey;
use uv_configuration::{
    BuildIsolation, ExcludeDependency, IndexStrategy, KeyringProviderType, PackageNameSpecifier,
    ProxyUrl, Reinstall, RequiredVersion, TargetTriple, TrustedHost, TrustedPublishing, Upgrade,
};
use uv_distribution_types::{
    ConfigSettings, ExtraBuildVariables, Index, IndexLocations, IndexUrl, IndexUrlError, Origin,
    PackageConfigSettings, PipExtraIndex, PipFindLinks, PipIndex, StaticMetadata,
};
use uv_install_wheel::LinkMode;
use uv_macros::{CombineOptions, OptionsMetadata};
use uv_normalize::{ExtraName, PackageName, PipGroupName};
use uv_pep508::Requirement;
use uv_preview::{MaybePreviewFeature, Preview};
use uv_pypi_types::{SupportedEnvironments, VerbatimParsedUrl};
use uv_python::{PythonDownloads, PythonPreference, PythonVersion};
use uv_redacted::DisplaySafeUrl;
use uv_resolver::{
    AnnotationStyle, ExcludeNewerOverride, ExcludeNewerPackage, ExcludeNewerSpan,
    ExcludeNewerValue, ForkStrategy, PrereleaseMode, PrereleasePackage, ResolutionMode,
    serialize_exclude_newer_package_with_spans,
};
use uv_torch::TorchMode;
use uv_workspace::pyproject::{ExtraBuildDependencies, OverrideDependency};
use uv_workspace::pyproject_mut::AddBoundsKind;

use crate::{EnvironmentOptions, FilesystemOptions};

/// A `pyproject.toml` file with an optional `[tool.uv]` section.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct PyProjectToml {
    pub(crate) tool: Option<Tools>,
}

/// A `[tool]` section.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct Tools {
    pub(crate) uv: Option<Options>,
}

/// A `pyproject.toml` file with an optional `[tool.uv.required-version]` setting.
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct PyProjectRequiredVersionToml {
    pub(crate) tool: Option<RequiredVersionTools>,
}

/// A `[tool]` section with only the fields needed to find `required-version`.
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RequiredVersionTools {
    pub(crate) uv: Option<RequiredVersionOptions>,
}

/// The `[tool.uv]` fields needed to enforce `required-version` before parsing the full file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct RequiredVersionOptions {
    pub(crate) required_version: Option<RequiredVersion>,
}

/// A `uv.toml` file with only the fields needed to find `required-version`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct UvRequiredVersionToml {
    pub(crate) required_version: Option<RequiredVersion>,
}

/// A `[tool.uv]` section.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, Deserialize, CombineOptions, OptionsMetadata)]
#[serde(try_from = "OptionsWire", rename_all = "kebab-case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schemars", schemars(!try_from))]
pub struct Options {
    #[serde(flatten)]
    pub globals: GlobalOptions,

    #[serde(flatten)]
    pub top_level: ResolverInstallerSchema,

    #[serde(flatten)]
    pub install_mirrors: PythonInstallMirrors,

    #[serde(flatten)]
    pub publish: PublishOptions,

    #[serde(flatten)]
    pub add: AddOptions,

    #[option_group]
    pub audit: Option<AuditOptions>,

    #[option_group]
    pub pip: Option<PipOptions>,

    /// The keys that determine when to rebuild the project.
    ///
    /// Use cache keys to select the files and directories that trigger a rebuild when they change.
    /// By default, uv rebuilds a project when `pyproject.toml`, `setup.py`, or `setup.cfg` changes
    /// in the project directory. It also rebuilds the project when a `src` directory is added or
    /// removed:
    ///
    /// ```toml
    /// cache-keys = [{ file = "pyproject.toml" }, { file = "setup.py" }, { file = "setup.cfg" }, { dir = "src" }]
    /// ```
    ///
    /// For example, a project can read dynamic dependency metadata from `requirements.txt`. Set
    /// `cache-keys = [{ file = "requirements.txt" }, { file = "pyproject.toml" }]` to rebuild the
    /// project when either file changes.
    ///
    /// Globs use the syntax of the [`glob`](https://docs.rs/glob/0.3.1/glob/struct.Pattern.html)
    /// crate. Set `cache-keys = [{ file = "**/*.toml" }]` to invalidate the cache when a `.toml`
    /// file changes in the project directory or a subdirectory. Globs can be slow because uv may
    /// need to search the filesystem for changed files.
    ///
    /// Cache keys can include version control information. If a project uses `setuptools_scm` to
    /// read its version from a Git commit, set `cache-keys = [{ git = { commit = true }, { file = "pyproject.toml" }]`
    /// to include the current Git commit hash and `pyproject.toml` in the cache key. To also
    /// include Git tags, use `cache-keys = [{ git = { commit = true, tags = true } }]`.
    ///
    /// Cache keys can also include environment variables. For example, set
    /// `cache-keys = [{ env = "MACOSX_DEPLOYMENT_TARGET" }]` to invalidate the cache when
    /// `MACOSX_DEPLOYMENT_TARGET` changes.
    ///
    /// Cache keys affect only the project defined by their `pyproject.toml`. They do not affect
    /// other workspace members. Paths and globs are relative to the project directory.
    #[option(
        default = r#"[{ file = "pyproject.toml" }, { file = "setup.py" }, { file = "setup.cfg" }]"#,
        value_type = "list[dict]",
        example = r#"
            cache-keys = [{ file = "pyproject.toml" }, { file = "requirements.txt" }, { git = { commit = true } }]
        "#
    )]
    pub cache_keys: Option<Vec<CacheKey>>,

    // NOTE(charlie): These fields are shared with `ToolUv` in
    // `crates/uv-workspace/src/pyproject.rs`, where they are documented.
    // They apply to both `pyproject.toml` and `uv.toml` files.
    #[cfg_attr(feature = "schemars", schemars(skip))]
    pub override_dependencies: Option<Vec<OverrideDependency>>,

    #[cfg_attr(feature = "schemars", schemars(skip))]
    pub exclude_dependencies: Option<Vec<ExcludeDependency>>,

    #[cfg_attr(feature = "schemars", schemars(skip))]
    pub constraint_dependencies: Option<Vec<Requirement<VerbatimParsedUrl>>>,

    #[cfg_attr(feature = "schemars", schemars(skip))]
    pub build_constraint_dependencies: Option<Vec<Requirement<VerbatimParsedUrl>>>,

    #[cfg_attr(feature = "schemars", schemars(skip))]
    pub environments: Option<SupportedEnvironments>,

    #[cfg_attr(feature = "schemars", schemars(skip))]
    pub required_environments: Option<SupportedEnvironments>,

    // NOTE(charlie): Keep these fields in sync with `ToolUv` in
    // `crates/uv-workspace/src/pyproject.rs`, where they are documented.
    // They apply only to `pyproject.toml` files and must be rejected in `uv.toml` files.
    #[cfg_attr(feature = "schemars", schemars(skip))]
    pub(crate) conflicts: Option<serde::de::IgnoredAny>,

    #[cfg_attr(feature = "schemars", schemars(skip))]
    pub(crate) workspace: Option<serde::de::IgnoredAny>,

    #[cfg_attr(feature = "schemars", schemars(skip))]
    pub(crate) sources: Option<serde::de::IgnoredAny>,

    #[cfg_attr(feature = "schemars", schemars(skip))]
    pub(crate) dev_dependencies: Option<serde::de::IgnoredAny>,

    #[cfg_attr(feature = "schemars", schemars(skip))]
    pub(crate) default_groups: Option<serde::de::IgnoredAny>,

    #[cfg_attr(feature = "schemars", schemars(skip))]
    pub(crate) dependency_groups: Option<serde::de::IgnoredAny>,

    #[cfg_attr(feature = "schemars", schemars(skip))]
    pub(crate) managed: Option<serde::de::IgnoredAny>,

    #[cfg_attr(feature = "schemars", schemars(skip))]
    pub(crate) r#package: Option<serde::de::IgnoredAny>,

    #[cfg_attr(feature = "schemars", schemars(skip))]
    pub(crate) build_backend: Option<serde::de::IgnoredAny>,
}

impl Options {
    /// Create [`Options`] from the given global and top-level settings.
    pub fn simple(globals: GlobalOptions, top_level: ResolverInstallerSchema) -> Self {
        Self {
            globals,
            top_level,
            ..Default::default()
        }
    }

    /// Set the [`Origin`] on all indexes without an existing origin.
    #[must_use]
    pub(crate) fn with_origin(mut self, origin: Origin) -> Self {
        if let Some(indexes) = &mut self.top_level.index {
            for index in indexes {
                index.origin.get_or_insert(origin);
            }
        }
        if let Some(index_url) = &mut self.top_level.index_url {
            index_url.try_set_origin(origin);
        }
        if let Some(extra_index_urls) = &mut self.top_level.extra_index_url {
            for index_url in extra_index_urls {
                index_url.try_set_origin(origin);
            }
        }
        if let Some(pip) = &mut self.pip {
            if let Some(indexes) = &mut pip.index {
                for index in indexes {
                    index.origin.get_or_insert(origin);
                }
            }
            if let Some(index_url) = &mut pip.index_url {
                index_url.try_set_origin(origin);
            }
            if let Some(extra_index_urls) = &mut pip.extra_index_url {
                for index_url in extra_index_urls {
                    index_url.try_set_origin(origin);
                }
            }
        }
        self
    }

    /// Resolve the [`Options`] relative to the given root directory.
    pub(crate) fn relative_to(self, root_dir: &Path) -> Result<Self, IndexUrlError> {
        Ok(Self {
            top_level: self.top_level.relative_to(root_dir)?,
            pip: self.pip.map(|pip| pip.relative_to(root_dir)).transpose()?,
            ..self
        })
    }
}

/// Global settings that apply to all commands.
#[derive(Debug, Clone, Default, Deserialize, CombineOptions, OptionsMetadata)]
#[serde(try_from = "GlobalOptionsWire", rename_all = "kebab-case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schemars", schemars(!try_from))]
pub struct GlobalOptions {
    /// Set a version requirement for uv.
    ///
    /// If the uv version does not meet the requirement, uv exits with an error.
    ///
    /// Use a [PEP 440](https://peps.python.org/pep-0440/) specifier, such as `==0.5.0` or `>=0.5.0`.
    #[option(
        default = "null",
        value_type = "str",
        example = r#"
            required-version = ">=0.5.0"
        "#
    )]
    pub required_version: Option<RequiredVersion>,
    /// Whether to load TLS certificates from the platform's native certificate store.
    ///
    /// By default, uv uses bundled Mozilla root certificates. Enable this setting to use
    /// certificates from the platform's native certificate store instead.
    #[option(
        default = "false",
        value_type = "bool",
        uv_toml_only = true,
        example = r#"
            system-certs = true
        "#
    )]
    pub system_certs: Option<bool>,
    /// Whether to load TLS certificates from the platform's native certificate store.
    ///
    /// By default, uv uses bundled Mozilla root certificates. Enable this setting to use
    /// certificates from the platform's native certificate store instead.
    ///
    /// (Deprecated: use `system-certs` instead.)
    #[deprecated(note = "use `system-certs` instead")]
    #[option(
        default = "false",
        value_type = "bool",
        uv_toml_only = true,
        example = r#"
            native-tls = true
        "#
    )]
    pub native_tls: Option<bool>,
    /// Disable network access. Use only cached data and local files.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            offline = true
        "#
    )]
    pub offline: Option<bool>,
    /// Do not read from or write to the cache. Use a temporary directory for the operation.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            no-cache = true
        "#
    )]
    pub no_cache: Option<bool>,
    /// Path to the cache directory.
    ///
    /// Defaults to `$XDG_CACHE_HOME/uv` or `$HOME/.cache/uv` on Linux and macOS, and
    /// `%LOCALAPPDATA%\uv\cache` on Windows.
    #[option(
        default = "None",
        value_type = "str",
        uv_toml_only = true,
        example = r#"
            cache-dir = "./.uv_cache"
        "#
    )]
    pub cache_dir: Option<PathBuf>,

    /// The user's preview configuration.
    #[serde(flatten)]
    pub preview: Option<PreviewOption>,

    /// Whether to prefer existing system Python installations or installations managed by uv.
    #[option(
        default = "\"managed\"",
        value_type = "str",
        example = r#"
            python-preference = "managed"
        "#,
        possible_values = true
    )]
    pub python_preference: Option<PythonPreference>,
    /// Whether to allow Python downloads.
    #[option(
        default = "\"automatic\"",
        value_type = "str",
        example = r#"
            python-downloads = "manual"
        "#,
        possible_values = true
    )]
    pub python_downloads: Option<PythonDownloads>,
    /// The maximum number of downloads that uv runs at the same time.
    #[option(
        default = "50",
        value_type = "int",
        example = r#"
            concurrent-downloads = 4
        "#
    )]
    pub concurrent_downloads: Option<NonZeroUsize>,
    /// The maximum number of source distributions that uv builds at the same time.
    ///
    /// Defaults to the number of available CPU cores.
    #[option(
        default = "None",
        value_type = "int",
        example = r#"
            concurrent-builds = 4
        "#
    )]
    pub concurrent_builds: Option<NonZeroUsize>,
    /// The number of threads that install and unzip packages.
    ///
    /// Defaults to the number of available CPU cores.
    #[option(
        default = "None",
        value_type = "int",
        example = r#"
            concurrent-installs = 4
        "#
    )]
    pub concurrent_installs: Option<NonZeroUsize>,
    /// The URL of the HTTP proxy to use.
    #[option(
        default = "None",
        value_type = "str",
        uv_toml_only = true,
        example = r#"
            http-proxy = "http://proxy.example.com"
        "#
    )]
    pub http_proxy: Option<ProxyUrl>,
    /// The URL of the HTTPS proxy to use.
    #[option(
        default = "None",
        value_type = "str",
        uv_toml_only = true,
        example = r#"
            https-proxy = "https://proxy.example.com"
        "#
    )]
    pub https_proxy: Option<ProxyUrl>,
    /// Hosts that do not use a proxy.
    #[option(
        default = "None",
        value_type = "list[str]",
        uv_toml_only = true,
        example = r#"
            no-proxy = ["localhost", "127.0.0.1"]
        "#
    )]
    pub no_proxy: Option<Vec<String>>,
    /// Allow insecure connections to a host.
    ///
    /// Use a hostname such as `localhost`, a host and port such as `localhost:8080`, or a URL such
    /// as `https://localhost`.
    ///
    /// WARNING: uv does not check these hosts against the system certificate store. Use
    /// `--allow-insecure-host` only on a secure network with verified sources. This setting
    /// bypasses SSL verification and can expose you to MITM attacks.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            allow-insecure-host = ["localhost:8080"]
        "#
    )]
    pub allow_insecure_host: Option<Vec<TrustedHost>>,
}

/// [`GlobalOptions`] with `#[serde(flatten)]` fields inlined.
/// This improves line and column information in error messages.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct GlobalOptionsWire {
    required_version: Option<RequiredVersion>,
    system_certs: Option<bool>,
    native_tls: Option<bool>,
    offline: Option<bool>,
    no_cache: Option<bool>,
    cache_dir: Option<PathBuf>,

    preview: Option<bool>,
    preview_features: Option<PreviewFeaturesOption>,

    python_preference: Option<PythonPreference>,
    python_downloads: Option<PythonDownloads>,
    concurrent_downloads: Option<NonZeroUsize>,
    concurrent_builds: Option<NonZeroUsize>,
    concurrent_installs: Option<NonZeroUsize>,
    http_proxy: Option<ProxyUrl>,
    https_proxy: Option<ProxyUrl>,
    no_proxy: Option<Vec<String>>,
    allow_insecure_host: Option<Vec<TrustedHost>>,
}

impl TryFrom<GlobalOptionsWire> for GlobalOptions {
    type Error = &'static str;

    #[allow(deprecated)]
    fn try_from(value: GlobalOptionsWire) -> Result<Self, Self::Error> {
        let GlobalOptionsWire {
            required_version,
            system_certs,
            native_tls,
            offline,
            no_cache,
            cache_dir,
            preview,
            preview_features,
            python_preference,
            python_downloads,
            concurrent_downloads,
            concurrent_builds,
            concurrent_installs,
            http_proxy,
            https_proxy,
            no_proxy,
            allow_insecure_host,
        } = value;

        Ok(Self {
            required_version,
            system_certs,
            native_tls,
            offline,
            no_cache,
            cache_dir,
            preview: PreviewOption::try_from(preview, preview_features)?,
            python_preference,
            python_downloads,
            concurrent_downloads,
            concurrent_builds,
            concurrent_installs,
            http_proxy,
            https_proxy,
            no_proxy,
            allow_insecure_host,
        })
    }
}

/// Resolve registry indexes and find-links relative to the given root directory.
fn rebase_indexes(
    root_dir: &Path,
    indexes: &mut Option<Vec<Index>>,
    index_url: &mut Option<PipIndex>,
    extra_index_urls: &mut Option<Vec<PipExtraIndex>>,
    find_links: &mut Option<Vec<PipFindLinks>>,
) -> Result<(), IndexUrlError> {
    *indexes = indexes
        .take()
        .map(|indexes| {
            indexes
                .into_iter()
                .map(|index| index.relative_to(root_dir))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    *index_url = index_url
        .take()
        .map(|index| index.relative_to(root_dir))
        .transpose()?;
    *extra_index_urls = extra_index_urls
        .take()
        .map(|indexes| {
            indexes
                .into_iter()
                .map(|index| index.relative_to(root_dir))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    *find_links = find_links
        .take()
        .map(|find_links| {
            find_links
                .into_iter()
                .map(|find_link| find_link.relative_to(root_dir))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;

    Ok(())
}

/// Settings for all package installation operations.
#[derive(Debug, Clone, Default, CombineOptions)]
pub struct InstallerOptions {
    index: Option<Vec<Index>>,
    index_url: Option<PipIndex>,
    extra_index_url: Option<Vec<PipExtraIndex>>,
    no_index: Option<bool>,
    find_links: Option<Vec<PipFindLinks>>,
    index_strategy: Option<IndexStrategy>,
    keyring_provider: Option<KeyringProviderType>,
    config_settings: Option<ConfigSettings>,
    exclude_newer: Option<ExcludeNewerOverride>,
    link_mode: Option<LinkMode>,
    compile_bytecode: Option<bool>,
    reinstall: Option<Reinstall>,
    build_isolation: Option<BuildIsolation>,
    no_build: Option<bool>,
    no_build_package: Option<Vec<PackageName>>,
    no_binary: Option<bool>,
    no_binary_package: Option<Vec<PackageName>>,
    no_sources: Option<bool>,
    no_sources_package: Option<Vec<PackageName>>,
}

/// Settings shared by all operations that use package indexes.
#[derive(Debug, Clone, Default, CombineOptions)]
pub struct IndexOptions {
    pub index: Option<Vec<Index>>,
    pub index_url: Option<PipIndex>,
    pub extra_index_url: Option<Vec<PipExtraIndex>>,
    pub no_index: Option<bool>,
    pub find_links: Option<Vec<PipFindLinks>>,
}

impl IndexOptions {
    /// Resolve the [`IndexOptions`] relative to the given root directory.
    pub fn relative_to(mut self, root_dir: &Path) -> Result<Self, IndexUrlError> {
        rebase_indexes(
            root_dir,
            &mut self.index,
            &mut self.index_url,
            &mut self.extra_index_url,
            &mut self.find_links,
        )?;

        Ok(self)
    }
}

impl From<IndexOptions> for IndexLocations {
    fn from(value: IndexOptions) -> Self {
        let IndexOptions {
            index,
            index_url,
            extra_index_url,
            no_index,
            find_links,
        } = value;

        Self::new(
            index
                .into_iter()
                .flatten()
                .chain(extra_index_url.into_iter().flatten().map(Index::from))
                .chain(index_url.into_iter().map(Index::from))
                .collect(),
            find_links.into_iter().flatten().map(Index::from).collect(),
            no_index.unwrap_or_default(),
        )
    }
}

impl From<IndexOptions> for PipOptions {
    fn from(value: IndexOptions) -> Self {
        let IndexOptions {
            index,
            index_url,
            extra_index_url,
            no_index,
            find_links,
        } = value;

        Self {
            index,
            index_url,
            extra_index_url,
            no_index,
            find_links,
            ..Self::default()
        }
    }
}

/// Settings for all dependency resolution operations.
#[derive(Debug, Clone, Default, CombineOptions)]
pub struct ResolverOptions {
    pub indexes: IndexOptions,
    pub index_strategy: Option<IndexStrategy>,
    pub keyring_provider: Option<KeyringProviderType>,
    pub resolution: Option<ResolutionMode>,
    pub prerelease: Option<PrereleaseMode>,
    pub prerelease_package: Option<PrereleasePackage>,
    pub fork_strategy: Option<ForkStrategy>,
    pub dependency_metadata: Option<Vec<StaticMetadata>>,
    pub config_settings: Option<ConfigSettings>,
    pub config_settings_package: Option<PackageConfigSettings>,
    pub exclude_newer: Option<ExcludeNewerOverride>,
    pub exclude_newer_package: Option<ExcludeNewerPackage>,
    pub link_mode: Option<LinkMode>,
    pub torch_backend: Option<TorchMode>,
    pub upgrade: Option<Upgrade>,
    pub build_isolation: Option<BuildIsolation>,
    pub no_build: Option<bool>,
    pub no_build_package: Option<Vec<PackageName>>,
    pub no_binary: Option<bool>,
    pub no_binary_package: Option<Vec<PackageName>>,
    pub extra_build_dependencies: Option<ExtraBuildDependencies>,
    pub extra_build_variables: Option<ExtraBuildVariables>,
    pub no_sources: Option<bool>,
    pub no_sources_package: Option<Vec<PackageName>>,
}

impl ResolverOptions {
    /// Resolve the [`ResolverOptions`] relative to the given root directory.
    pub fn relative_to(mut self, root_dir: &Path) -> Result<Self, IndexUrlError> {
        self.indexes = self.indexes.relative_to(root_dir)?;
        Ok(self)
    }
}

/// Settings for operations that resolve and install dependencies.
/// Combines [`InstallerOptions`] and [`ResolverOptions`].
#[derive(Debug, Clone, Default, CombineOptions)]
pub struct ResolverInstallerOptions {
    pub indexes: IndexOptions,
    pub index_strategy: Option<IndexStrategy>,
    pub keyring_provider: Option<KeyringProviderType>,
    pub resolution: Option<ResolutionMode>,
    pub prerelease: Option<PrereleaseMode>,
    pub prerelease_package: Option<PrereleasePackage>,
    pub fork_strategy: Option<ForkStrategy>,
    pub dependency_metadata: Option<Vec<StaticMetadata>>,
    pub config_settings: Option<ConfigSettings>,
    pub config_settings_package: Option<PackageConfigSettings>,
    pub build_isolation: Option<BuildIsolation>,
    pub extra_build_dependencies: Option<ExtraBuildDependencies>,
    pub extra_build_variables: Option<ExtraBuildVariables>,
    pub exclude_newer: Option<ExcludeNewerOverride>,
    pub exclude_newer_package: Option<ExcludeNewerPackage>,
    pub link_mode: Option<LinkMode>,
    pub torch_backend: Option<TorchMode>,
    pub compile_bytecode: Option<bool>,
    pub no_sources: Option<bool>,
    pub no_sources_package: Option<Vec<PackageName>>,
    pub upgrade: Option<Upgrade>,
    pub reinstall: Option<Reinstall>,
    pub no_build: Option<bool>,
    pub no_build_package: Option<Vec<PackageName>>,
    pub no_binary: Option<bool>,
    pub no_binary_package: Option<Vec<PackageName>>,
}

impl ResolverInstallerOptions {
    /// Resolve the [`ResolverInstallerOptions`] relative to the given root directory.
    pub fn relative_to(mut self, root_dir: &Path) -> Result<Self, IndexUrlError> {
        self.indexes = self.indexes.relative_to(root_dir)?;
        Ok(self)
    }
}

impl From<ResolverInstallerSchema> for ResolverInstallerOptions {
    fn from(value: ResolverInstallerSchema) -> Self {
        let ResolverInstallerSchema {
            index,
            index_url,
            extra_index_url,
            no_index,
            find_links,
            index_strategy,
            keyring_provider,
            resolution,
            prerelease,
            prerelease_package,
            fork_strategy,
            dependency_metadata,
            config_settings,
            config_settings_package,
            no_build_isolation,
            no_build_isolation_package,
            extra_build_dependencies,
            extra_build_variables,
            exclude_newer,
            exclude_newer_package,
            link_mode,
            torch_backend,
            compile_bytecode,
            no_sources,
            no_sources_package,
            upgrade,
            upgrade_package,
            reinstall,
            reinstall_package,
            no_build,
            no_build_package,
            no_binary,
            no_binary_package,
        } = value;
        Self {
            indexes: IndexOptions {
                index,
                index_url,
                extra_index_url,
                no_index,
                find_links,
            },
            index_strategy,
            keyring_provider,
            resolution,
            prerelease,
            prerelease_package,
            fork_strategy,
            dependency_metadata,
            config_settings,
            config_settings_package,
            build_isolation: BuildIsolation::from_args(
                no_build_isolation,
                no_build_isolation_package.into_iter().flatten().collect(),
            ),
            extra_build_dependencies,
            extra_build_variables,
            exclude_newer,
            exclude_newer_package,
            link_mode,
            torch_backend,
            compile_bytecode,
            no_sources,
            no_sources_package,
            upgrade: Upgrade::from_args(
                upgrade,
                upgrade_package
                    .into_iter()
                    .flatten()
                    .map(Into::into)
                    .collect(),
                Vec::new(),
            ),
            reinstall: Reinstall::from_args(reinstall, reinstall_package.unwrap_or_default()),
            no_build,
            no_build_package,
            no_binary,
            no_binary_package,
        }
    }
}

impl ResolverInstallerSchema {
    /// Resolve the [`ResolverInstallerSchema`] relative to the given root directory.
    fn relative_to(mut self, root_dir: &Path) -> Result<Self, IndexUrlError> {
        rebase_indexes(
            root_dir,
            &mut self.index,
            &mut self.index_url,
            &mut self.extra_index_url,
            &mut self.find_links,
        )?;

        Ok(self)
    }
}

/// The JSON schema for the `[tool.uv]` section of a `pyproject.toml` file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, CombineOptions, OptionsMetadata)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ResolverInstallerSchema {
    /// The package indexes to use when resolving dependencies.
    ///
    /// Use a repository that follows [PEP 503](https://peps.python.org/pep-0503/) (the simple
    /// repository API), or a local directory that uses the same format.
    ///
    /// uv searches indexes in their defined order. The first index has the highest priority.
    /// These indexes have higher priority than indexes from [`index_url`](#index-url) or
    /// [`extra_index_url`](#extra-index-url). Unless you select another
    /// [index strategy](#index-strategy), uv uses only the first index that contains a package.
    ///
    /// If an index has `explicit = true`, uv uses it only for dependencies that select it in
    /// `[tool.uv.sources]`:
    ///
    /// ```toml
    /// [[tool.uv.index]]
    /// name = "pytorch"
    /// url = "https://download.pytorch.org/whl/cu130"
    /// explicit = true
    ///
    /// [tool.uv.sources]
    /// torch = { index = "pytorch" }
    /// ```
    ///
    /// If an index has `default = true`, uv moves it to the end of the list. The index then has the
    /// lowest priority. A default index also disables the default PyPI index.
    #[option(
        default = "\"[]\"",
        value_type = "dict",
        example = r#"
            [[tool.uv.index]]
            name = "pytorch"
            url = "https://download.pytorch.org/whl/cu130"
        "#
    )]
    pub index: Option<Vec<Index>>,
    /// The Python package index URL. Defaults to <https://pypi.org/simple>.
    ///
    /// Use a repository that follows [PEP 503](https://peps.python.org/pep-0503/) (the simple
    /// repository API), or a local directory that uses the same format.
    ///
    /// This index has lower priority than indexes from [`extra_index_url`](#extra-index-url) or
    /// [`index`](#index).
    ///
    /// (Deprecated: use `index` instead.)
    #[option(
        default = "\"https://pypi.org/simple\"",
        value_type = "str",
        example = r#"
            index-url = "https://test.pypi.org/simple"
        "#
    )]
    pub index_url: Option<PipIndex>,
    /// Additional package index URLs to use with `--index-url`.
    ///
    /// Use a repository that follows [PEP 503](https://peps.python.org/pep-0503/) (the simple
    /// repository API), or a local directory that uses the same format.
    ///
    /// These indexes have higher priority than [`index_url`](#index-url) and any [`index`](#index)
    /// with `default = true`. Earlier indexes have higher priority.
    ///
    /// Use [`index_strategy`](#index-strategy) to control how uv searches multiple indexes.
    ///
    /// (Deprecated: use `index` instead.)
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            extra-index-url = ["https://download.pytorch.org/whl/cpu"]
        "#
    )]
    pub extra_index_url: Option<Vec<PipExtraIndex>>,
    /// Ignore all registry indexes, including PyPI. Use only direct URL dependencies and
    /// dependencies from `--find-links`.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            no-index = true
        "#
    )]
    pub no_index: Option<bool>,
    /// Additional locations to search for distributions outside the registry indexes.
    ///
    /// A path must point to a directory with wheels (`.whl`) or source distributions (`.tar.gz` or
    /// `.zip`) at its top level.
    ///
    /// A URL must point to a page with a flat list of links to those package file formats.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            find-links = ["https://download.pytorch.org/whl/torch_stable.html"]
        "#
    )]
    pub find_links: Option<Vec<PipFindLinks>>,
    /// The strategy for resolving packages from multiple indexes.
    ///
    /// By default, uv uses only the first index that contains a package (`first-index`). This
    /// prevents "dependency confusion" attacks, in which an attacker uploads a malicious package
    /// with the same name to another index.
    #[option(
        default = "\"first-index\"",
        value_type = "str",
        example = r#"
            index-strategy = "unsafe-best-match"
        "#,
        possible_values = true
    )]
    pub index_strategy: Option<IndexStrategy>,
    /// Use `keyring` to authenticate with package indexes.
    ///
    /// Only `--keyring-provider subprocess` is supported. It uses the `keyring` CLI for
    /// authentication.
    #[option(
        default = "\"disabled\"",
        value_type = "str",
        example = r#"
            keyring-provider = "subprocess"
        "#
    )]
    pub keyring_provider: Option<KeyringProviderType>,
    /// The strategy for selecting a compatible package version.
    ///
    /// By default, uv uses the latest compatible version of each package (`highest`).
    #[option(
        default = "\"highest\"",
        value_type = "str",
        example = r#"
            resolution = "lowest-direct"
        "#,
        possible_values = true
    )]
    pub resolution: Option<ResolutionMode>,
    /// The strategy to use when considering pre-release versions.
    ///
    /// By default, uv prefers stable versions. It selects a pre-release only after every stable
    /// version that meets the active constraints is rejected (`if-necessary`).
    #[option(
        default = "\"if-necessary\"",
        value_type = "str",
        example = r#"
            prerelease = "allow"
        "#,
        possible_values = true
    )]
    pub prerelease: Option<PrereleaseMode>,
    /// The strategy to use when considering pre-release versions for specific packages.
    ///
    /// Package-specific modes take priority over the global [`prerelease`](#prerelease) mode.
    /// Use a dictionary that maps package names to supported pre-release modes.
    #[option(
        default = "{}",
        value_type = "dict",
        example = r#"
            prerelease-package = { numpy = "allow", scipy = "disallow" }
        "#
    )]
    pub prerelease_package: Option<PrereleasePackage>,
    /// The strategy to use when selecting multiple versions of a given package across Python
    /// versions and platforms.
    ///
    /// By default, uv selects the latest package version for each supported Python version
    /// (`requires-python`). It also minimizes the number of versions across platforms.
    ///
    /// With `fewest`, uv minimizes the number of versions for each package. It prefers older
    /// versions that support more Python versions or platforms.
    #[option(
        default = "\"requires-python\"",
        value_type = "str",
        example = r#"
            fork-strategy = "fewest"
        "#,
        possible_values = true
    )]
    pub fork_strategy: Option<ForkStrategy>,
    /// Static metadata for direct or transitive project dependencies. uv uses this metadata
    /// instead of querying the registry or building the package from source.
    ///
    /// The metadata should follow the [Metadata 2.3](https://packaging.python.org/en/latest/specifications/core-metadata/)
    /// standard. uv uses only these fields:
    ///
    /// - `name`: The name of the package.
    /// - (Optional) `version`: The package version. If omitted, the metadata applies to all
    ///   versions of the package.
    /// - (Optional) `requires-dist`: The dependencies of the package (e.g., `werkzeug>=0.14`).
    /// - (Optional) `requires-python`: The Python version required by the package (e.g., `>=3.10`).
    /// - (Optional) `provides-extra`: The extras provided by the package.
    #[option(
        default = r#"[]"#,
        value_type = "list[dict]",
        example = r#"
            dependency-metadata = [
                { name = "flask", version = "1.0.0", requires-dist = ["werkzeug"], requires-python = ">=3.6" },
            ]
        "#
    )]
    pub dependency_metadata: Option<Vec<StaticMetadata>>,
    /// Settings to pass to the [PEP 517](https://peps.python.org/pep-0517/) build backend as
    /// `KEY=VALUE` pairs.
    #[option(
        default = "{}",
        value_type = "dict",
        example = r#"
            config-settings = { editable_mode = "compat" }
        "#
    )]
    pub config_settings: Option<ConfigSettings>,
    /// Settings to pass to the [PEP 517](https://peps.python.org/pep-0517/) build backend for
    /// specific packages as `KEY=VALUE` pairs.
    ///
    /// Use a map from package names to string key-value pairs.
    #[option(
        default = "{}",
        value_type = "dict",
        example = r#"
            config-settings-package = { numpy = { editable_mode = "compat" } }
        "#
    )]
    pub config_settings_package: Option<PackageConfigSettings>,
    /// Disable isolation when building source distributions.
    ///
    /// Requires the build dependencies from [PEP 518](https://peps.python.org/pep-0518/) to
    /// already be installed.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            no-build-isolation = true
        "#
    )]
    pub no_build_isolation: Option<bool>,
    /// Disable isolation when building source distributions for a specific package.
    ///
    /// Requires the packages' build dependencies from [PEP 518](https://peps.python.org/pep-0518/)
    /// to already be installed.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
        no-build-isolation-package = ["package1", "package2"]
    "#
    )]
    pub no_build_isolation_package: Option<Vec<PackageName>>,
    /// Additional build dependencies for packages.
    ///
    /// Add packages to the PEP 517 build environments of project dependencies. Use this for
    /// packages that require dependencies such as `pip` but do not declare them.
    #[option(
        default = "[]",
        value_type = "dict",
        example = r#"
            extra-build-dependencies = { pytest = ["setuptools"] }
        "#
    )]
    pub extra_build_dependencies: Option<ExtraBuildDependencies>,
    /// Extra environment variables to set when building certain packages.
    ///
    /// uv adds these variables to the environment when it builds the specified packages.
    #[option(
        default = r#"{}"#,
        value_type = r#"dict[str, dict[str, str]]"#,
        example = r#"
            extra-build-variables = { flash-attn = { FLASH_ATTENTION_SKIP_CUDA_BUILD = "TRUE" } }
        "#
    )]
    pub extra_build_variables: Option<ExtraBuildVariables>,
    /// Select only package files uploaded before the given date.
    ///
    /// uv compares the date with the upload time of each distribution file. It does not use the
    /// release date of the package version.
    ///
    /// Use an RFC 3339 timestamp such as `2006-12-02T02:07:43Z`, a duration such as `24 hours`,
    /// `1 week`, or `30 days`, or an ISO 8601 duration such as `PT24H`, `P7D`, or `P30D`.
    ///
    /// uv converts durations to a fixed number of seconds and treats each day as 24 hours.
    /// It ignores local time zones and daylight saving time. Months and years are not allowed.
    ///
    /// Set to `false` to disable `exclude-newer`.
    #[option(
        default = "None",
        value_type = "str | false",
        example = r#"
            exclude-newer = "2006-12-02T02:07:43Z"
        "#
    )]
    pub exclude_newer: Option<ExcludeNewerOverride>,
    /// For specific packages, select only package files uploaded before the given date.
    ///
    /// Use a dictionary of `PACKAGE = "DATE"` pairs. `DATE` can be an RFC 3339 timestamp such as
    /// `2006-12-02T02:07:43Z`, a duration such as `24 hours`, `1 week`, or `30 days`, or an ISO
    /// 8601 duration such as `PT24H`, `P7D`, or `P30D`.
    ///
    /// uv converts durations to a fixed number of seconds and treats each day as 24 hours.
    /// It ignores local time zones and daylight saving time. Months and years are not allowed.
    ///
    /// Set a package to `false` to exempt it from the global [`exclude-newer`](#exclude-newer)
    /// constraint.
    #[option(
        default = "None",
        value_type = "dict",
        example = r#"
            exclude-newer-package = { tqdm = "2022-04-04T00:00:00Z", markupsafe = false }
        "#
    )]
    pub exclude_newer_package: Option<ExcludeNewerPackage>,
    /// The method to use when installing packages from the global cache.
    ///
    /// Defaults to `clone` (Copy-on-Write) on macOS and Linux, and `hardlink` on Windows.
    ///
    /// WARNING: Symlinks connect the target environment to the cache. If you clear the cache with
    /// `uv cache clean`, uv removes the source files and breaks the installed packages. Use
    /// symlinks with caution.
    #[option(
        default = "\"clone\" (macOS, Linux) or \"hardlink\" (Windows)",
        value_type = "str",
        example = r#"
            link-mode = "copy"
        "#,
        possible_values = true
    )]
    pub link_mode: Option<LinkMode>,
    /// Compile Python files to bytecode after installation.
    ///
    /// By default, uv does not compile Python (`.py`) files to bytecode (`__pycache__/*.pyc`).
    /// Python compiles each module when it is first imported. Enable this setting to trade longer
    /// installation times for faster startup in CLI applications and Docker containers.
    ///
    /// When enabled, uv processes the entire site-packages directory. This includes packages that
    /// the current operation does not change. Like pip, uv ignores errors.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            compile-bytecode = true
        "#
    )]
    pub compile_bytecode: Option<bool>,
    /// Ignore `tool.uv.sources` when resolving dependencies. Lock against standards-compliant,
    /// publishable package metadata instead of local or Git sources.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            no-sources = true
        "#
    )]
    pub no_sources: Option<bool>,
    /// Ignore `tool.uv.sources` for the specified packages.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            no-sources-package = ["ruff"]
        "#
    )]
    pub no_sources_package: Option<Vec<PackageName>>,
    /// Allow package upgrades and ignore pinned versions in an existing output file.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            upgrade = true
        "#
    )]
    pub upgrade: Option<bool>,
    /// Allow upgrades for a specific package and ignore pinned versions in an existing output
    /// file.
    ///
    /// Use a package name such as `ruff` or a version specifier such as `ruff<0.5.0`.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            upgrade-package = ["ruff"]
        "#
    )]
    pub upgrade_package: Option<Vec<Requirement<VerbatimParsedUrl>>>,
    /// Reinstall all packages, including installed packages. Implies `refresh`.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            reinstall = true
        "#
    )]
    pub reinstall: Option<bool>,
    /// Reinstall a specific package, even if it is already installed. Implies `refresh-package`.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            reinstall-package = ["ruff"]
        "#
    )]
    pub reinstall_package: Option<Vec<PackageName>>,
    /// Do not build source distributions.
    ///
    /// uv reuses cached wheels from previous source builds. Operations that require a new source
    /// build exit with an error. uv still builds first-party packages, such as projects in the
    /// workspace. uv may also build editable requirements, and their build backends may run
    /// arbitrary Python code.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            no-build = true
        "#
    )]
    pub no_build: Option<bool>,
    /// Do not build source distributions for a specific package.
    ///
    /// uv still builds first-party packages, such as projects in the workspace.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            no-build-package = ["ruff"]
        "#
    )]
    pub no_build_package: Option<Vec<PackageName>>,
    /// Do not install pre-built wheels.
    ///
    /// uv builds and installs the packages from source. The resolver still uses available
    /// pre-built wheels to extract package metadata.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            no-binary = true
        "#
    )]
    pub no_binary: Option<bool>,
    /// Do not install pre-built wheels for a specific package.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            no-binary-package = ["ruff"]
        "#
    )]
    pub no_binary_package: Option<Vec<PackageName>>,
    /// The backend for packages in the PyTorch ecosystem.
    ///
    /// When set, uv ignores the configured index URLs for PyTorch packages and uses this backend.
    ///
    /// For example, `cpu` uses the CPU-only PyTorch index, and `cu126` uses the PyTorch index for
    /// CUDA 12.6.
    ///
    /// The `auto` mode tries to detect the PyTorch index from the installed CUDA drivers.
    ///
    /// This setting applies only to `uv pip` commands.
    ///
    /// This option is in preview and may change in any future release.
    #[option(
        default = "null",
        value_type = "str",
        example = r#"
            torch-backend = "auto"
        "#
    )]
    pub torch_backend: Option<TorchMode>,
}

/// Settings for operations that create managed Python installations.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, CombineOptions, OptionsMetadata)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PythonInstallMirrors {
    /// Mirror URL for downloading managed Python installations.
    ///
    /// By default, uv downloads managed Python installations from
    /// [`python-build-standalone`](https://github.com/astral-sh/python-build-standalone).
    /// Set this variable to a mirror URL to use another source. The URL replaces
    /// `https://github.com/astral-sh/python-build-standalone/releases/download` in URLs such as
    /// `https://github.com/astral-sh/python-build-standalone/releases/download/20240713/cpython-3.12.4%2B20240713-aarch64-apple-darwin-install_only.tar.gz`.
    ///
    /// Use a `file://` URL to read distributions from a local directory.
    #[option(
        default = "None",
        value_type = "str",
        uv_toml_only = true,
        example = r#"
            python-install-mirror = "https://github.com/astral-sh/python-build-standalone/releases/download"
        "#
    )]
    pub python_install_mirror: Option<String>,
    /// Mirror URL to use for downloading managed PyPy installations.
    ///
    /// By default, uv downloads managed PyPy installations from
    /// [downloads.python.org](https://downloads.python.org/). Set this variable to a mirror URL to
    /// use another source. The URL replaces `https://downloads.python.org/pypy` in URLs such as
    /// `https://downloads.python.org/pypy/pypy3.8-v7.3.7-osx64.tar.bz2`.
    ///
    /// Use a `file://` URL to read distributions from a local directory.
    #[option(
        default = "None",
        value_type = "str",
        uv_toml_only = true,
        example = r#"
            pypy-install-mirror = "https://downloads.python.org/pypy"
        "#
    )]
    pub pypy_install_mirror: Option<String>,

    /// The URL of a JSON file that defines custom Python installations.
    #[option(
        default = "None",
        value_type = "str",
        uv_toml_only = true,
        example = r#"
            python-downloads-json-url = "/etc/uv/python-downloads.json"
        "#
    )]
    pub python_downloads_json_url: Option<String>,
}

impl PythonInstallMirrors {
    #[must_use]
    pub fn combine(self, other: Self) -> Self {
        Self {
            python_install_mirror: self.python_install_mirror.or(other.python_install_mirror),
            pypy_install_mirror: self.pypy_install_mirror.or(other.pypy_install_mirror),
            python_downloads_json_url: self
                .python_downloads_json_url
                .or(other.python_downloads_json_url),
        }
    }
}

/// Settings for the `uv pip` command-line interface.
///
/// Other commands, such as `uv lock` and `uvx`, ignore these settings.
#[derive(Debug, Clone, Default, Deserialize, CombineOptions, OptionsMetadata)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PipOptions {
    /// The Python interpreter for package installation.
    ///
    /// By default, uv installs into a virtual environment in the current directory or a parent
    /// directory. Use `--python` to select another interpreter for continuous integration (CI)
    /// environments or other automated workflows.
    ///
    /// Supported formats:
    /// - `3.10` looks for an installed Python 3.10 in the registry on Windows (see
    ///   `py --list-paths`), or `python3.10` on Linux and macOS.
    /// - `python3.10` or `python.exe` looks for a binary with the given name in `PATH`.
    /// - `/home/ferris/.local/bin/python3.10` uses the exact Python at the given path.
    #[option(
        default = "None",
        value_type = "str",
        example = r#"
            python = "3.10"
        "#
    )]
    pub python: Option<String>,
    /// Install packages into the system Python environment.
    ///
    /// By default, uv installs into a virtual environment in the current directory or a parent
    /// directory. Use `--system` to select the first Python interpreter in the system `PATH`.
    ///
    /// WARNING: `--system` is intended for continuous integration (CI) environments. Use it with
    /// caution because it can modify the system Python installation.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            system = true
        "#
    )]
    pub system: Option<bool>,
    /// Allow uv to modify an `EXTERNALLY-MANAGED` Python installation.
    ///
    /// WARNING: `--break-system-packages` is intended for continuous integration (CI) environments
    /// that use Python installations managed by tools such as `apt`. Use it with caution. These
    /// installations recommend against changes from other package managers, including uv and pip.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            break-system-packages = true
        "#
    )]
    pub break_system_packages: Option<bool>,
    /// Install packages at the top level of the specified directory instead of a virtual or
    /// system Python environment.
    #[option(
        default = "None",
        value_type = "str",
        example = r#"
            target = "./target"
        "#
    )]
    pub target: Option<PathBuf>,
    /// Install packages into `lib`, `bin`, and other top-level directories under the specified
    /// path, as if it contained a virtual environment.
    ///
    /// Prefer `--python` when installing into another environment. Scripts and other artifacts
    /// installed with `--prefix` reference the installing interpreter, not an interpreter in the
    /// prefix directory. This makes them non-portable.
    #[option(
        default = "None",
        value_type = "str",
        example = r#"
            prefix = "./prefix"
        "#
    )]
    pub prefix: Option<PathBuf>,
    #[serde(skip)]
    #[cfg_attr(feature = "schemars", schemars(skip))]
    pub index: Option<Vec<Index>>,
    /// The Python package index URL. Defaults to <https://pypi.org/simple>.
    ///
    /// Use a repository that follows [PEP 503](https://peps.python.org/pep-0503/) (the simple
    /// repository API), or a local directory that uses the same format.
    ///
    /// This index has lower priority than indexes from [`extra_index_url`](#extra-index-url).
    #[option(
        default = "\"https://pypi.org/simple\"",
        value_type = "str",
        example = r#"
            index-url = "https://test.pypi.org/simple"
        "#
    )]
    pub index_url: Option<PipIndex>,
    /// Additional package index URLs to use with `--index-url`.
    ///
    /// Use a repository that follows [PEP 503](https://peps.python.org/pep-0503/) (the simple
    /// repository API), or a local directory that uses the same format.
    ///
    /// These indexes have higher priority than [`index_url`](#index-url). Earlier indexes have
    /// higher priority.
    ///
    /// Use [`index_strategy`](#index-strategy) to control how uv searches multiple indexes.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            extra-index-url = ["https://download.pytorch.org/whl/cpu"]
        "#
    )]
    pub extra_index_url: Option<Vec<PipExtraIndex>>,
    /// Ignore all registry indexes, including PyPI. Use only direct URL dependencies and
    /// dependencies from `--find-links`.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            no-index = true
        "#
    )]
    pub no_index: Option<bool>,
    /// Additional locations to search for distributions outside the registry indexes.
    ///
    /// A path must point to a directory with wheels (`.whl`) or source distributions (`.tar.gz` or
    /// `.zip`) at its top level.
    ///
    /// A URL must point to a page with a flat list of links to those package file formats.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            find-links = ["https://download.pytorch.org/whl/torch_stable.html"]
        "#
    )]
    pub find_links: Option<Vec<PipFindLinks>>,
    /// The strategy for resolving packages from multiple indexes.
    ///
    /// By default, uv uses only the first index that contains a package (`first-index`). This
    /// prevents "dependency confusion" attacks, in which an attacker uploads a malicious package
    /// with the same name to another index.
    #[option(
        default = "\"first-index\"",
        value_type = "str",
        example = r#"
            index-strategy = "unsafe-best-match"
        "#,
        possible_values = true
    )]
    pub index_strategy: Option<IndexStrategy>,
    /// Use `keyring` to authenticate with package indexes.
    ///
    /// Only `--keyring-provider subprocess` is supported. It uses the `keyring` CLI for
    /// authentication.
    #[option(
        default = "disabled",
        value_type = "str",
        example = r#"
            keyring-provider = "subprocess"
        "#
    )]
    pub keyring_provider: Option<KeyringProviderType>,
    /// Do not build source distributions.
    ///
    /// uv reuses cached wheels from previous source builds. Operations that require a new source
    /// build exit with an error. uv may still build editable requirements, and their build
    /// backends may run arbitrary Python code.
    ///
    /// Alias for `--only-binary :all:`.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            no-build = true
        "#
    )]
    pub no_build: Option<bool>,
    /// Do not install pre-built wheels.
    ///
    /// uv builds and installs the packages from source. The resolver still uses available
    /// pre-built wheels to extract package metadata.
    ///
    /// You may specify multiple packages. Use `:all:` to disable binaries for every package. Use
    /// `:none:` to clear previous package selections.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            no-binary = ["ruff"]
        "#
    )]
    pub no_binary: Option<Vec<PackageNameSpecifier>>,
    /// Use only pre-built wheels. Do not build source distributions.
    ///
    /// uv reuses cached wheels from previous source builds. Operations that require a new source
    /// build for the selected packages exit with an error. uv may still build editable
    /// requirements, and their build backends may run arbitrary Python code.
    ///
    /// You may specify multiple packages. Use `:all:` to disable binaries for every package. Use
    /// `:none:` to clear previous package selections.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            only-binary = ["ruff"]
        "#
    )]
    pub only_binary: Option<Vec<PackageNameSpecifier>>,
    /// Disable isolation when building source distributions.
    ///
    /// Requires the build dependencies from [PEP 518](https://peps.python.org/pep-0518/) to
    /// already be installed.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            no-build-isolation = true
        "#
    )]
    pub no_build_isolation: Option<bool>,
    /// Disable isolation when building source distributions for a specific package.
    ///
    /// Requires the packages' build dependencies from [PEP 518](https://peps.python.org/pep-0518/)
    /// to already be installed.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            no-build-isolation-package = ["package1", "package2"]
        "#
    )]
    pub no_build_isolation_package: Option<Vec<PackageName>>,
    /// Additional build dependencies for packages.
    ///
    /// Add packages to the PEP 517 build environments of project dependencies. Use this for
    /// packages that require dependencies such as `pip` but do not declare them.
    #[option(
        default = "[]",
        value_type = "dict",
        example = r#"
            extra-build-dependencies = { pytest = ["setuptools"] }
        "#
    )]
    pub extra_build_dependencies: Option<ExtraBuildDependencies>,
    /// Extra environment variables to set when building certain packages.
    ///
    /// uv adds these variables to the environment when it builds the specified packages.
    #[option(
        default = r#"{}"#,
        value_type = r#"dict[str, dict[str, str]]"#,
        example = r#"
            extra-build-variables = { flash-attn = { FLASH_ATTENTION_SKIP_CUDA_BUILD = "TRUE" } }
        "#
    )]
    pub extra_build_variables: Option<ExtraBuildVariables>,
    /// Check the Python environment for missing package dependencies and other issues.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            strict = true
        "#
    )]
    pub strict: Option<bool>,
    /// Include optional dependencies from the specified extra. You may specify multiple extras.
    ///
    /// Only applies to `pyproject.toml`, `setup.py`, and `setup.cfg` sources.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            extra = ["dev", "docs"]
        "#
    )]
    pub extra: Option<Vec<ExtraName>>,
    /// Include all optional dependencies.
    ///
    /// Only applies to `pyproject.toml`, `setup.py`, and `setup.cfg` sources.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            all-extras = true
        "#
    )]
    pub all_extras: Option<bool>,
    /// Exclude the specified optional dependencies when `all-extras` is set.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            all-extras = true
            no-extra = ["dev", "docs"]
        "#
    )]
    pub no_extra: Option<Vec<ExtraName>>,
    /// Ignore package dependencies. Add only packages from the command line to the requirements
    /// file.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            no-deps = true
        "#
    )]
    pub no_deps: Option<bool>,
    /// Include the specified dependency groups.
    #[option(
        default = "None",
        value_type = "list[str]",
        example = r#"
            group = ["dev", "docs"]
        "#
    )]
    pub group: Option<Vec<PipGroupName>>,
    /// Allow `uv pip sync` to accept empty requirements and remove all packages from the
    /// environment.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            allow-empty-requirements = true
        "#
    )]
    pub allow_empty_requirements: Option<bool>,
    /// The strategy for selecting a compatible package version.
    ///
    /// By default, uv uses the latest compatible version of each package (`highest`).
    #[option(
        default = "\"highest\"",
        value_type = "str",
        example = r#"
            resolution = "lowest-direct"
        "#,
        possible_values = true
    )]
    pub resolution: Option<ResolutionMode>,
    /// The strategy to use when considering pre-release versions.
    ///
    /// By default, uv prefers stable versions. It selects a pre-release only after every stable
    /// version that meets the active constraints is rejected (`if-necessary`).
    #[option(
        default = "\"if-necessary\"",
        value_type = "str",
        example = r#"
            prerelease = "allow"
        "#,
        possible_values = true
    )]
    pub prerelease: Option<PrereleaseMode>,
    #[serde(skip)]
    #[cfg_attr(feature = "schemars", schemars(skip))]
    pub prerelease_package: Option<PrereleasePackage>,
    /// The strategy to use when selecting multiple versions of a given package across Python
    /// versions and platforms.
    ///
    /// By default, uv selects the latest package version for each supported Python version
    /// (`requires-python`). It also minimizes the number of versions across platforms.
    ///
    /// With `fewest`, uv minimizes the number of versions for each package. It prefers older
    /// versions that support more Python versions or platforms.
    #[option(
        default = "\"requires-python\"",
        value_type = "str",
        example = r#"
            fork-strategy = "fewest"
        "#,
        possible_values = true
    )]
    pub fork_strategy: Option<ForkStrategy>,
    /// Static metadata for direct or transitive project dependencies. uv uses this metadata
    /// instead of querying the registry or building the package from source.
    ///
    /// The metadata should follow the [Metadata 2.3](https://packaging.python.org/en/latest/specifications/core-metadata/)
    /// standard. uv uses only these fields:
    ///
    /// - `name`: The name of the package.
    /// - (Optional) `version`: The package version. If omitted, the metadata applies to all
    ///   versions of the package.
    /// - (Optional) `requires-dist`: The dependencies of the package (e.g., `werkzeug>=0.14`).
    /// - (Optional) `requires-python`: The Python version required by the package (e.g., `>=3.10`).
    /// - (Optional) `provides-extra`: The extras provided by the package.
    #[option(
        default = r#"[]"#,
        value_type = "list[dict]",
        example = r#"
            dependency-metadata = [
                { name = "flask", version = "1.0.0", requires-dist = ["werkzeug"], requires-python = ">=3.6" },
            ]
        "#
    )]
    pub dependency_metadata: Option<Vec<StaticMetadata>>,
    /// Write the requirements generated by `uv pip compile` to the given `requirements.txt` file.
    ///
    /// If the file exists, uv prefers its package versions unless `--upgrade` is set.
    #[option(
        default = "None",
        value_type = "str",
        example = r#"
            output-file = "requirements.txt"
        "#
    )]
    pub output_file: Option<PathBuf>,
    /// Include extras in the output file.
    ///
    /// By default, uv removes extras because their packages already appear as dependencies in the
    /// output file. Files created with `--no-strip-extras` cannot be used as constraint files with
    /// `install` or `sync`.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            no-strip-extras = true
        "#
    )]
    pub no_strip_extras: Option<bool>,
    /// Include environment markers in the output file generated by `uv pip compile`.
    ///
    /// By default, uv removes environment markers. The resolution is guaranteed to be correct
    /// only for the target environment.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            no-strip-markers = true
        "#
    )]
    pub no_strip_markers: Option<bool>,
    /// Omit package source comments from the file generated by `uv pip compile`.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            no-annotate = true
        "#
    )]
    pub no_annotate: Option<bool>,
    /// Omit the header comment from the file generated by `uv pip compile`.
    #[option(
        default = r#"false"#,
        value_type = "bool",
        example = r#"
            no-header = true
        "#
    )]
    pub no_header: Option<bool>,
    /// The header comment for the file generated by `uv pip compile`.
    ///
    /// Use this to identify a custom build script or command that wraps `uv pip compile`.
    #[option(
        default = "None",
        value_type = "str",
        example = r#"
            custom-compile-command = "./custom-uv-compile.sh"
        "#
    )]
    pub custom_compile_command: Option<String>,
    /// Include distribution hashes in the output file.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            generate-hashes = true
        "#
    )]
    pub generate_hashes: Option<bool>,
    /// Settings to pass to the [PEP 517](https://peps.python.org/pep-0517/) build backend as
    /// `KEY=VALUE` pairs.
    #[option(
        default = "{}",
        value_type = "dict",
        example = r#"
            config-settings = { editable_mode = "compat" }
        "#
    )]
    pub config_settings: Option<ConfigSettings>,
    /// Settings to pass to the [PEP 517](https://peps.python.org/pep-0517/) build backend for
    /// specific packages as `KEY=VALUE` pairs.
    #[option(
        default = "{}",
        value_type = "dict",
        example = r#"
            config-settings-package = { numpy = { editable_mode = "compat" } }
        "#
    )]
    pub config_settings_package: Option<PackageConfigSettings>,
    /// The minimum Python version that the resolved requirements support, such as `3.8` or
    /// `3.8.17`.
    ///
    /// If you omit the patch version, uv uses the minimum patch version. For example, `3.8` means
    /// `3.8.0`.
    #[option(
        default = "None",
        value_type = "str",
        example = r#"
            python-version = "3.8"
        "#
    )]
    pub python_version: Option<PythonVersion>,
    /// The target platform for dependency resolution.
    ///
    /// Use a "target triple" that identifies the CPU, vendor, and operating system. Examples
    /// include `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin`.
    #[option(
        default = "None",
        value_type = "str",
        example = r#"
            python-platform = "x86_64-unknown-linux-gnu"
        "#
    )]
    pub python_platform: Option<TargetTriple>,
    /// Create one `requirements.txt` file that works across operating systems, architectures, and
    /// Python implementations.
    ///
    /// In universal mode, the current Python version or `--python-version` is the lower bound.
    /// For example, `--universal --python-version 3.7` resolves for Python 3.7 and later.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            universal = true
        "#
    )]
    pub universal: Option<bool>,
    /// Select only package files uploaded before the given time.
    ///
    /// uv compares the time with the upload time of each distribution file. It does not use the
    /// release date of the package version.
    ///
    /// Use an RFC 3339 timestamp such as `2006-12-02T02:07:43Z`, a duration such as `24 hours`,
    /// `1 week`, or `30 days`, or an ISO 8601 duration such as `PT24H`, `P7D`, or `P30D`.
    ///
    /// uv converts durations to a fixed number of seconds and treats each day as 24 hours.
    /// It ignores local time zones and daylight saving time. Months and years are not allowed.
    ///
    /// Set to `false` to disable `exclude-newer`.
    #[option(
        default = "None",
        value_type = "str | false",
        example = r#"
            exclude-newer = "2006-12-02T02:07:43Z"
        "#
    )]
    pub exclude_newer: Option<ExcludeNewerOverride>,
    /// For specific packages, select only package files uploaded before the given date.
    ///
    /// Use a dictionary of `PACKAGE = "DATE"` pairs. `DATE` can be an RFC 3339 timestamp such as
    /// `2006-12-02T02:07:43Z`, a duration such as `24 hours`, `1 week`, or `30 days`, or an ISO
    /// 8601 duration such as `PT24H`, `P7D`, or `P30D`.
    ///
    /// uv converts durations to a fixed number of seconds and treats each day as 24 hours.
    /// It ignores local time zones and daylight saving time. Months and years are not allowed.
    ///
    /// Set a package to `false` to exempt it from the global [`exclude-newer`](#exclude-newer)
    /// constraint.
    #[option(
        default = "None",
        value_type = "dict",
        example = r#"
            exclude-newer-package = { tqdm = "2022-04-04T00:00:00Z", markupsafe = false }
        "#
    )]
    pub exclude_newer_package: Option<ExcludeNewerPackage>,
    /// Omit a package from the output resolution but keep its dependencies. Equivalent to the
    /// pip-compile `--unsafe-package` option.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            no-emit-package = ["ruff"]
        "#
    )]
    pub no_emit_package: Option<Vec<PackageName>>,
    /// Include `--index-url` and `--extra-index-url` entries in the output file generated by `uv pip compile`.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            emit-index-url = true
        "#
    )]
    pub emit_index_url: Option<bool>,
    /// Include `--find-links` entries in the output file generated by `uv pip compile`.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            emit-find-links = true
        "#
    )]
    pub emit_find_links: Option<bool>,
    /// Include `--no-binary` and `--only-binary` entries in the output file generated by `uv pip compile`.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            emit-build-options = true
        "#
    )]
    pub emit_build_options: Option<bool>,
    /// Whether to emit a marker that identifies when the pinned dependencies are valid.
    ///
    /// The pinned dependencies may also be valid when the marker is false. When the marker is
    /// true, the requirements are guaranteed to be correct.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            emit-marker-expression = true
        "#
    )]
    pub emit_marker_expression: Option<bool>,
    /// Include a comment that identifies each package's index, such as
    /// `# from https://pypi.org/simple`.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            emit-index-annotation = true
        "#
    )]
    pub emit_index_annotation: Option<bool>,
    /// The comment style for package sources in the output file.
    #[option(
        default = "\"split\"",
        value_type = "str",
        example = r#"
            annotation-style = "line"
        "#,
        possible_values = true
    )]
    pub annotation_style: Option<AnnotationStyle>,
    /// The method to use when installing packages from the global cache.
    ///
    /// Defaults to `clone` (Copy-on-Write) on macOS and Linux, and `hardlink` on Windows.
    ///
    /// WARNING: Symlinks connect the target environment to the cache. If you clear the cache with
    /// `uv cache clean`, uv removes the source files and breaks the installed packages. Use
    /// symlinks with caution.
    #[option(
        default = "\"clone\" (macOS, Linux) or \"hardlink\" (Windows)",
        value_type = "str",
        example = r#"
            link-mode = "copy"
        "#,
        possible_values = true
    )]
    pub link_mode: Option<LinkMode>,
    /// Compile Python files to bytecode after installation.
    ///
    /// By default, uv does not compile Python (`.py`) files to bytecode (`__pycache__/*.pyc`).
    /// Python compiles each module when it is first imported. Enable this setting to trade longer
    /// installation times for faster startup in CLI applications and Docker containers.
    ///
    /// When enabled, uv processes the entire site-packages directory. This includes packages that
    /// the current operation does not change. Like pip, uv ignores errors.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            compile-bytecode = true
        "#
    )]
    pub compile_bytecode: Option<bool>,
    /// Require a matching hash for each requirement.
    ///
    /// Hash-checking mode applies to _all_ requirements. Each requirement must have one or more
    /// matching hashes. Each requirement must also use an exact version such as `==1.0.0` or a
    /// direct URL.
    ///
    /// Hash-checking mode has these additional constraints:
    ///
    /// - Git dependencies are not supported.
    /// - Editable installations are not supported.
    /// - Local dependencies must point to a wheel (`.whl`) or source archive (`.zip`, `.tar.gz`),
    ///   not a directory.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            require-hashes = true
        "#
    )]
    pub require_hashes: Option<bool>,
    /// Check hashes in the requirements file.
    ///
    /// Unlike `--require-hashes`, `--verify-hashes` does not require every requirement to have a
    /// hash. It checks only requirements that include hashes.
    #[option(
        default = "true",
        value_type = "bool",
        example = r#"
            verify-hashes = true
        "#
    )]
    pub verify_hashes: Option<bool>,
    /// Ignore `tool.uv.sources` when resolving dependencies. Lock against standards-compliant,
    /// publishable package metadata instead of local or Git sources.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            no-sources = true
        "#
    )]
    pub no_sources: Option<bool>,
    /// Ignore `tool.uv.sources` for the specified packages.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            no-sources-package = ["ruff"]
        "#
    )]
    pub no_sources_package: Option<Vec<PackageName>>,
    /// Allow package upgrades and ignore pinned versions in an existing output file.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            upgrade = true
        "#
    )]
    pub upgrade: Option<bool>,
    /// Allow upgrades for a specific package and ignore pinned versions in an existing output
    /// file.
    ///
    /// Use a package name such as `ruff` or a version specifier such as `ruff<0.5.0`.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            upgrade-package = ["ruff"]
        "#
    )]
    pub upgrade_package: Option<Vec<Requirement<VerbatimParsedUrl>>>,
    /// Reinstall all packages, including installed packages. Implies `refresh`.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            reinstall = true
        "#
    )]
    pub reinstall: Option<bool>,
    /// Reinstall a specific package, even if it is already installed. Implies `refresh-package`.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            reinstall-package = ["ruff"]
        "#
    )]
    pub reinstall_package: Option<Vec<PackageName>>,
    /// The backend for packages in the PyTorch ecosystem.
    ///
    /// When set, uv ignores the configured index URLs for PyTorch packages and uses this backend.
    ///
    /// For example, `cpu` uses the CPU-only PyTorch index, and `cu126` uses the PyTorch index for
    /// CUDA 12.6.
    ///
    /// The `auto` mode tries to detect the PyTorch index from the installed CUDA drivers.
    ///
    /// This setting applies only to `uv pip` commands.
    ///
    /// This option is in preview and may change in any future release.
    #[option(
        default = "null",
        value_type = "str",
        example = r#"
            torch-backend = "auto"
        "#
    )]
    pub torch_backend: Option<TorchMode>,
}

impl PipOptions {
    /// Resolve the [`PipOptions`] relative to the given root directory.
    fn relative_to(mut self, root_dir: &Path) -> Result<Self, IndexUrlError> {
        rebase_indexes(
            root_dir,
            &mut self.index,
            &mut self.index_url,
            &mut self.extra_index_url,
            &mut self.find_links,
        )?;

        Ok(self)
    }
}

impl From<ResolverInstallerSchema> for ResolverOptions {
    fn from(value: ResolverInstallerSchema) -> Self {
        Self {
            indexes: IndexOptions {
                index: value.index,
                index_url: value.index_url,
                extra_index_url: value.extra_index_url,
                no_index: value.no_index,
                find_links: value.find_links,
            },
            index_strategy: value.index_strategy,
            keyring_provider: value.keyring_provider,
            resolution: value.resolution,
            prerelease: value.prerelease,
            prerelease_package: value.prerelease_package,
            fork_strategy: value.fork_strategy,
            dependency_metadata: value.dependency_metadata,
            config_settings: value.config_settings,
            config_settings_package: value.config_settings_package,
            exclude_newer: value.exclude_newer,
            exclude_newer_package: value.exclude_newer_package,
            link_mode: value.link_mode,
            upgrade: Upgrade::from_args(
                value.upgrade,
                value
                    .upgrade_package
                    .into_iter()
                    .flatten()
                    .map(Into::into)
                    .collect(),
                Vec::new(),
            ),
            no_build: value.no_build,
            no_build_package: value.no_build_package,
            no_binary: value.no_binary,
            no_binary_package: value.no_binary_package,
            build_isolation: BuildIsolation::from_args(
                value.no_build_isolation,
                value.no_build_isolation_package.unwrap_or_default(),
            ),
            extra_build_dependencies: value.extra_build_dependencies,
            extra_build_variables: value.extra_build_variables,
            no_sources: value.no_sources,
            no_sources_package: value.no_sources_package,
            torch_backend: value.torch_backend,
        }
    }
}

impl From<ResolverInstallerSchema> for InstallerOptions {
    fn from(value: ResolverInstallerSchema) -> Self {
        Self {
            index: value.index,
            index_url: value.index_url,
            extra_index_url: value.extra_index_url,
            no_index: value.no_index,
            find_links: value.find_links,
            index_strategy: value.index_strategy,
            keyring_provider: value.keyring_provider,
            config_settings: value.config_settings,
            exclude_newer: value.exclude_newer,
            link_mode: value.link_mode,
            compile_bytecode: value.compile_bytecode,
            reinstall: Reinstall::from_args(
                value.reinstall,
                value.reinstall_package.unwrap_or_default(),
            ),
            build_isolation: BuildIsolation::from_args(
                value.no_build_isolation,
                value.no_build_isolation_package.unwrap_or_default(),
            ),
            no_build: value.no_build,
            no_build_package: value.no_build_package,
            no_binary: value.no_binary,
            no_binary_package: value.no_binary_package,
            no_sources: value.no_sources,
            no_sources_package: value.no_sources_package,
        }
    }
}

/// The options persisted alongside an installed tool.
///
/// A mirror of [`ResolverInstallerSchema`], without upgrades and reinstalls, which shouldn't be
/// persisted in a tool receipt.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, CombineOptions, OptionsMetadata,
)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ToolOptions {
    index: Option<Vec<Index>>,
    index_url: Option<PipIndex>,
    extra_index_url: Option<Vec<PipExtraIndex>>,
    no_index: Option<bool>,
    find_links: Option<Vec<PipFindLinks>>,
    index_strategy: Option<IndexStrategy>,
    keyring_provider: Option<KeyringProviderType>,
    resolution: Option<ResolutionMode>,
    prerelease: Option<PrereleaseMode>,
    prerelease_package: Option<PrereleasePackage>,
    fork_strategy: Option<ForkStrategy>,
    dependency_metadata: Option<Vec<StaticMetadata>>,
    config_settings: Option<ConfigSettings>,
    config_settings_package: Option<PackageConfigSettings>,
    build_isolation: Option<BuildIsolation>,
    extra_build_dependencies: Option<ExtraBuildDependencies>,
    extra_build_variables: Option<ExtraBuildVariables>,
    exclude_newer: Option<ExcludeNewerOverride>,
    exclude_newer_package: Option<ExcludeNewerPackage>,
    link_mode: Option<LinkMode>,
    compile_bytecode: Option<bool>,
    no_sources: Option<bool>,
    no_sources_package: Option<Vec<PackageName>>,
    no_build: Option<bool>,
    no_build_package: Option<Vec<PackageName>>,
    no_binary: Option<bool>,
    no_binary_package: Option<Vec<PackageName>>,
    torch_backend: Option<TorchMode>,
}

/// The on-disk representation of [`ToolOptions`] in a tool receipt.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct ToolOptionsWire {
    index: Option<Vec<Index>>,
    index_url: Option<PipIndex>,
    extra_index_url: Option<Vec<PipExtraIndex>>,
    no_index: Option<bool>,
    find_links: Option<Vec<PipFindLinks>>,
    index_strategy: Option<IndexStrategy>,
    keyring_provider: Option<KeyringProviderType>,
    resolution: Option<ResolutionMode>,
    prerelease: Option<PrereleaseMode>,
    prerelease_package: Option<PrereleasePackage>,
    fork_strategy: Option<ForkStrategy>,
    dependency_metadata: Option<Vec<StaticMetadata>>,
    config_settings: Option<ConfigSettings>,
    config_settings_package: Option<PackageConfigSettings>,
    build_isolation: Option<BuildIsolation>,
    extra_build_dependencies: Option<ExtraBuildDependencies>,
    extra_build_variables: Option<ExtraBuildVariables>,
    exclude_newer: Option<ExcludeNewerOverride>,
    exclude_newer_span: Option<ExcludeNewerSpan>,
    #[serde(serialize_with = "serialize_exclude_newer_package_with_spans")]
    exclude_newer_package: Option<ExcludeNewerPackage>,
    link_mode: Option<LinkMode>,
    compile_bytecode: Option<bool>,
    no_sources: Option<bool>,
    no_sources_package: Option<Vec<PackageName>>,
    no_build: Option<bool>,
    no_build_package: Option<Vec<PackageName>>,
    no_binary: Option<bool>,
    no_binary_package: Option<Vec<PackageName>>,
    torch_backend: Option<TorchMode>,
}

impl From<ResolverInstallerOptions> for ToolOptions {
    fn from(value: ResolverInstallerOptions) -> Self {
        Self {
            index: value.indexes.index.map(|indexes| {
                indexes
                    .into_iter()
                    .map(Index::with_promoted_auth_policy)
                    .collect()
            }),
            index_url: value.indexes.index_url,
            extra_index_url: value.indexes.extra_index_url,
            no_index: value.indexes.no_index,
            find_links: value.indexes.find_links,
            index_strategy: value.index_strategy,
            keyring_provider: value.keyring_provider,
            resolution: value.resolution,
            prerelease: value.prerelease,
            prerelease_package: value.prerelease_package,
            fork_strategy: value.fork_strategy,
            dependency_metadata: value.dependency_metadata,
            config_settings: value.config_settings,
            config_settings_package: value.config_settings_package,
            build_isolation: value.build_isolation,
            extra_build_dependencies: value.extra_build_dependencies,
            extra_build_variables: value.extra_build_variables,
            exclude_newer: value.exclude_newer,
            exclude_newer_package: value.exclude_newer_package,
            link_mode: value.link_mode,
            compile_bytecode: value.compile_bytecode,
            no_sources: value.no_sources,
            no_sources_package: value.no_sources_package,
            no_build: value.no_build,
            no_build_package: value.no_build_package,
            no_binary: value.no_binary,
            no_binary_package: value.no_binary_package,
            torch_backend: value.torch_backend,
        }
    }
}

impl From<ToolOptionsWire> for ToolOptions {
    fn from(value: ToolOptionsWire) -> Self {
        let exclude_newer = value
            .exclude_newer
            .map(|exclude_newer| match exclude_newer {
                ExcludeNewerOverride::Disabled => ExcludeNewerOverride::Disabled,
                ExcludeNewerOverride::Enabled(exclude_newer) => {
                    let exclude_newer = *exclude_newer;
                    if let Some(span) = value.exclude_newer_span
                        && exclude_newer.span().is_none()
                    {
                        ExcludeNewerValue::relative(span).into()
                    } else {
                        exclude_newer.into()
                    }
                }
            });

        Self {
            index: value.index,
            index_url: value.index_url,
            extra_index_url: value.extra_index_url,
            no_index: value.no_index,
            find_links: value.find_links,
            index_strategy: value.index_strategy,
            keyring_provider: value.keyring_provider,
            resolution: value.resolution,
            prerelease: value.prerelease,
            prerelease_package: value.prerelease_package,
            fork_strategy: value.fork_strategy,
            dependency_metadata: value.dependency_metadata,
            config_settings: value.config_settings,
            config_settings_package: value.config_settings_package,
            build_isolation: value.build_isolation,
            extra_build_dependencies: value.extra_build_dependencies,
            extra_build_variables: value.extra_build_variables,
            exclude_newer,
            exclude_newer_package: value.exclude_newer_package,
            link_mode: value.link_mode,
            compile_bytecode: value.compile_bytecode,
            no_sources: value.no_sources,
            no_sources_package: value.no_sources_package,
            no_build: value.no_build,
            no_build_package: value.no_build_package,
            no_binary: value.no_binary,
            no_binary_package: value.no_binary_package,
            torch_backend: value.torch_backend,
        }
    }
}

impl From<ToolOptions> for ToolOptionsWire {
    fn from(value: ToolOptions) -> Self {
        let (exclude_newer, exclude_newer_span) = match &value.exclude_newer {
            Some(ExcludeNewerOverride::Disabled) => (Some(ExcludeNewerOverride::Disabled), None),
            Some(ExcludeNewerOverride::Enabled(value)) => match value.as_ref() {
                ExcludeNewerValue::Absolute(_) => {
                    (Some(ExcludeNewerOverride::Enabled(value.clone())), None)
                }
                ExcludeNewerValue::Relative(span) => (
                    Some(ExcludeNewerValue::absolute(value.timestamp()).into()),
                    Some(*span),
                ),
            },
            None => (None, None),
        };

        Self {
            index: value.index,
            index_url: value.index_url,
            extra_index_url: value.extra_index_url,
            no_index: value.no_index,
            find_links: value.find_links,
            index_strategy: value.index_strategy,
            keyring_provider: value.keyring_provider,
            resolution: value.resolution,
            prerelease: value.prerelease,
            prerelease_package: value.prerelease_package,
            fork_strategy: value.fork_strategy,
            dependency_metadata: value.dependency_metadata,
            config_settings: value.config_settings,
            config_settings_package: value.config_settings_package,
            build_isolation: value.build_isolation,
            extra_build_dependencies: value.extra_build_dependencies,
            extra_build_variables: value.extra_build_variables,
            exclude_newer,
            exclude_newer_span,
            exclude_newer_package: value.exclude_newer_package,
            link_mode: value.link_mode,
            compile_bytecode: value.compile_bytecode,
            no_sources: value.no_sources,
            no_sources_package: value.no_sources_package,
            no_build: value.no_build,
            no_build_package: value.no_build_package,
            no_binary: value.no_binary,
            no_binary_package: value.no_binary_package,
            torch_backend: value.torch_backend,
        }
    }
}

impl From<ToolOptions> for ResolverInstallerOptions {
    fn from(value: ToolOptions) -> Self {
        Self {
            indexes: IndexOptions {
                index: value.index,
                index_url: value.index_url,
                extra_index_url: value.extra_index_url,
                no_index: value.no_index,
                find_links: value.find_links,
            },
            index_strategy: value.index_strategy,
            keyring_provider: value.keyring_provider,
            resolution: value.resolution,
            prerelease: value.prerelease,
            prerelease_package: value.prerelease_package,
            fork_strategy: value.fork_strategy,
            dependency_metadata: value.dependency_metadata,
            config_settings: value.config_settings,
            config_settings_package: value.config_settings_package,
            build_isolation: value.build_isolation,
            extra_build_dependencies: value.extra_build_dependencies,
            extra_build_variables: value.extra_build_variables,
            exclude_newer: value.exclude_newer,
            exclude_newer_package: value.exclude_newer_package,
            link_mode: value.link_mode,
            compile_bytecode: value.compile_bytecode,
            no_sources: value.no_sources,
            no_sources_package: value.no_sources_package,
            upgrade: None,
            reinstall: None,
            no_build: value.no_build,
            no_build_package: value.no_build_package,
            no_binary: value.no_binary,
            no_binary_package: value.no_binary_package,
            torch_backend: value.torch_backend,
        }
    }
}

/// Like [`Options]`, but with any `#[serde(flatten)]` fields inlined. This leads to far, far
/// better error messages when deserializing.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct OptionsWire {
    // #[serde(flatten)]
    // globals: GlobalOptions
    required_version: Option<RequiredVersion>,
    system_certs: Option<bool>,
    native_tls: Option<bool>,
    offline: Option<bool>,
    no_cache: Option<bool>,
    cache_dir: Option<PathBuf>,
    preview: Option<bool>,
    preview_features: Option<PreviewFeaturesOption>,
    python_preference: Option<PythonPreference>,
    python_downloads: Option<PythonDownloads>,
    concurrent_downloads: Option<NonZeroUsize>,
    concurrent_builds: Option<NonZeroUsize>,
    concurrent_installs: Option<NonZeroUsize>,

    // #[serde(flatten)]
    // top_level: ResolverInstallerOptions
    index: Option<Vec<Index>>,
    index_url: Option<PipIndex>,
    extra_index_url: Option<Vec<PipExtraIndex>>,
    no_index: Option<bool>,
    find_links: Option<Vec<PipFindLinks>>,
    index_strategy: Option<IndexStrategy>,
    keyring_provider: Option<KeyringProviderType>,
    http_proxy: Option<ProxyUrl>,
    https_proxy: Option<ProxyUrl>,
    no_proxy: Option<Vec<String>>,
    allow_insecure_host: Option<Vec<TrustedHost>>,
    resolution: Option<ResolutionMode>,
    prerelease: Option<PrereleaseMode>,
    prerelease_package: Option<PrereleasePackage>,
    fork_strategy: Option<ForkStrategy>,
    dependency_metadata: Option<Vec<StaticMetadata>>,
    config_settings: Option<ConfigSettings>,
    config_settings_package: Option<PackageConfigSettings>,
    no_build_isolation: Option<bool>,
    no_build_isolation_package: Option<Vec<PackageName>>,
    extra_build_dependencies: Option<ExtraBuildDependencies>,
    extra_build_variables: Option<ExtraBuildVariables>,
    exclude_newer: Option<ExcludeNewerOverride>,
    exclude_newer_package: Option<ExcludeNewerPackage>,
    link_mode: Option<LinkMode>,
    compile_bytecode: Option<bool>,
    no_sources: Option<bool>,
    no_sources_package: Option<Vec<PackageName>>,
    upgrade: Option<bool>,
    upgrade_package: Option<Vec<Requirement<VerbatimParsedUrl>>>,
    reinstall: Option<bool>,
    reinstall_package: Option<Vec<PackageName>>,
    no_build: Option<bool>,
    no_build_package: Option<Vec<PackageName>>,
    no_binary: Option<bool>,
    no_binary_package: Option<Vec<PackageName>>,
    torch_backend: Option<TorchMode>,

    // #[serde(flatten)]
    // install_mirror: PythonInstallMirrors,
    python_install_mirror: Option<String>,
    pypy_install_mirror: Option<String>,
    python_downloads_json_url: Option<String>,

    // #[serde(flatten)]
    // publish: PublishOptions
    publish_url: Option<DisplaySafeUrl>,
    trusted_publishing: Option<TrustedPublishing>,
    check_url: Option<IndexUrl>,

    // #[serde(flatten)]
    // add: AddOptions
    add_bounds: Option<AddBoundsKind>,

    audit: Option<AuditOptions>,
    pip: Option<PipOptions>,
    cache_keys: Option<Vec<CacheKey>>,

    // NOTE(charlie): These fields are shared with `ToolUv` in
    // `crates/uv-workspace/src/pyproject.rs`. The documentation lives on that struct.
    // They're respected in both `pyproject.toml` and `uv.toml` files.
    override_dependencies: Option<Vec<OverrideDependency>>,
    exclude_dependencies: Option<Vec<ExcludeDependency>>,
    constraint_dependencies: Option<Vec<Requirement<VerbatimParsedUrl>>>,
    build_constraint_dependencies: Option<Vec<Requirement<VerbatimParsedUrl>>>,
    environments: Option<SupportedEnvironments>,
    required_environments: Option<SupportedEnvironments>,

    // NOTE(charlie): These fields should be kept in-sync with `ToolUv` in
    // `crates/uv-workspace/src/pyproject.rs`. The documentation lives on that struct.
    // They're only respected in `pyproject.toml` files, and should be rejected in `uv.toml` files.
    conflicts: Option<serde::de::IgnoredAny>,
    workspace: Option<serde::de::IgnoredAny>,
    sources: Option<serde::de::IgnoredAny>,
    managed: Option<serde::de::IgnoredAny>,
    r#package: Option<serde::de::IgnoredAny>,
    default_groups: Option<serde::de::IgnoredAny>,
    dependency_groups: Option<serde::de::IgnoredAny>,
    dev_dependencies: Option<serde::de::IgnoredAny>,

    // Build backend
    build_backend: Option<serde::de::IgnoredAny>,
}

impl TryFrom<OptionsWire> for Options {
    type Error = &'static str;

    #[allow(deprecated)]
    fn try_from(value: OptionsWire) -> Result<Self, Self::Error> {
        let OptionsWire {
            required_version,
            system_certs,
            native_tls,
            offline,
            no_cache,
            cache_dir,
            preview,
            preview_features,
            python_preference,
            python_downloads,
            python_install_mirror,
            pypy_install_mirror,
            python_downloads_json_url,
            concurrent_downloads,
            concurrent_builds,
            concurrent_installs,
            index,
            index_url,
            extra_index_url,
            no_index,
            find_links,
            index_strategy,
            keyring_provider,
            http_proxy,
            https_proxy,
            no_proxy,
            allow_insecure_host,
            resolution,
            prerelease,
            prerelease_package,
            fork_strategy,
            dependency_metadata,
            config_settings,
            config_settings_package,
            no_build_isolation,
            no_build_isolation_package,
            exclude_newer,
            exclude_newer_package,
            link_mode,
            compile_bytecode,
            no_sources,
            no_sources_package,
            upgrade,
            upgrade_package,
            reinstall,
            reinstall_package,
            no_build,
            no_build_package,
            no_binary,
            no_binary_package,
            torch_backend,
            audit,
            pip,
            cache_keys,
            override_dependencies,
            exclude_dependencies,
            constraint_dependencies,
            build_constraint_dependencies,
            environments,
            required_environments,
            conflicts,
            publish_url,
            trusted_publishing,
            check_url,
            workspace,
            sources,
            default_groups,
            dependency_groups,
            extra_build_dependencies,
            extra_build_variables,
            dev_dependencies,
            managed,
            package,
            add_bounds: bounds,
            // Used by the build backend
            build_backend,
        } = value;

        Ok(Self {
            globals: GlobalOptions {
                required_version,
                system_certs,
                native_tls,
                offline,
                no_cache,
                cache_dir,
                preview: PreviewOption::try_from(preview, preview_features)?,
                python_preference,
                python_downloads,
                concurrent_downloads,
                concurrent_builds,
                concurrent_installs,
                http_proxy,
                https_proxy,
                no_proxy,
                // Used twice for backwards compatibility
                allow_insecure_host: allow_insecure_host.clone(),
            },
            top_level: ResolverInstallerSchema {
                index,
                index_url,
                extra_index_url,
                no_index,
                find_links,
                index_strategy,
                keyring_provider,
                resolution,
                prerelease,
                prerelease_package,
                fork_strategy,
                dependency_metadata,
                config_settings,
                config_settings_package,
                no_build_isolation,
                no_build_isolation_package,
                extra_build_dependencies,
                extra_build_variables,
                exclude_newer,
                exclude_newer_package,
                link_mode,
                compile_bytecode,
                no_sources,
                no_sources_package,
                upgrade,
                upgrade_package,
                reinstall,
                reinstall_package,
                no_build,
                no_build_package,
                no_binary,
                no_binary_package,
                torch_backend,
            },
            pip,
            cache_keys,
            build_backend,
            override_dependencies,
            exclude_dependencies,
            constraint_dependencies,
            build_constraint_dependencies,
            environments,
            required_environments,
            install_mirrors: PythonInstallMirrors {
                python_install_mirror,
                pypy_install_mirror,
                python_downloads_json_url,
            },
            conflicts,
            publish: PublishOptions {
                publish_url,
                trusted_publishing,
                check_url,
            },
            add: AddOptions { add_bounds: bounds },
            audit,
            workspace,
            sources,
            dev_dependencies,
            default_groups,
            dependency_groups,
            managed,
            package,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, CombineOptions, OptionsMetadata)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PublishOptions {
    /// The URL for publishing packages. Defaults to <https://upload.pypi.org/legacy/>.
    #[option(
        default = "\"https://upload.pypi.org/legacy/\"",
        value_type = "str",
        example = r#"
            publish-url = "https://test.pypi.org/legacy/"
        "#
    )]
    pub publish_url: Option<DisplaySafeUrl>,

    /// Configure trusted publishing.
    ///
    /// By default, uv checks for trusted publishing in supported environments. If trusted
    /// publishing is not configured, uv ignores it.
    ///
    /// Supported environments include GitHub Actions and GitLab CI/CD.
    #[option(
        default = "automatic",
        value_type = "str",
        example = r#"
            trusted-publishing = "always"
        "#
    )]
    pub trusted_publishing: Option<TrustedPublishing>,

    /// Check an index URL for existing files to skip duplicate uploads.
    ///
    /// Use this option to retry a partial upload. It also handles concurrent uploads of the same
    /// file.
    ///
    /// Before each upload, uv checks the index. If the same file is already present, uv skips it.
    /// If an upload fails, uv checks the index again because another process may have uploaded the
    /// same file.
    ///
    /// Behavior depends on the index. PyPI accepts an identical upload without `--check-url`, but
    /// most other indexes return an error.
    ///
    /// The index must provide one of the supported hashes (SHA-256, SHA-384, or SHA-512).
    #[option(
        default = "None",
        value_type = "str",
        example = r#"
            check-url = "https://test.pypi.org/simple"
        "#
    )]
    pub check_url: Option<IndexUrl>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, CombineOptions, OptionsMetadata)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AddOptions {
    /// The default version specifier when adding a dependency.
    ///
    /// If a dependency has no constraint or URL, uv adds a constraint based on its latest
    /// compatible version. By default, uv uses a lower bound such as `>=1.2.3`.
    ///
    /// With `--frozen`, uv does not resolve dependencies and adds them without constraints.
    ///
    /// This option is in preview and may change in any future release.
    #[option(
        default = "\"lower\"",
        value_type = "str",
        example = r#"
            add-bounds = "major"
        "#,
        possible_values = true
    )]
    pub add_bounds: Option<AddBoundsKind>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, CombineOptions, OptionsMetadata)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AuditOptions {
    /// Whether to run the automatic malware check during sync operations.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            malware-check = true
        "#
    )]
    pub malware_check: Option<bool>,

    /// The vulnerability service URL to use for automatic malware checks.
    #[option(
        default = "\"https://api.osv.dev/\"",
        value_type = "str",
        example = r#"
            malware-check-url = "https://example.com"
        "#
    )]
    pub malware_check_url: Option<DisplaySafeUrl>,

    /// Vulnerability IDs to ignore during an audit.
    ///
    /// uv excludes vulnerabilities that match these IDs or their aliases from the audit results.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            ignore = ["PYSEC-2022-43017", "GHSA-5239-wwwm-4pmq"]
        "#
    )]
    pub ignore: Option<Vec<String>>,

    /// Vulnerability IDs to ignore during an audit until a fix is available.
    ///
    /// uv excludes vulnerabilities that match these IDs or their aliases while no fixed version is
    /// available. When a fixed version becomes available, uv reports the vulnerability again.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            ignore-until-fixed = ["PYSEC-2022-43017"]
        "#
    )]
    pub ignore_until_fixed: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct MalwareCheckSettings {
    /// Whether the malware check is enabled.
    pub enabled: bool,
    /// The URL of the OSV-compatible service for malware checks.
    pub malware_check_url: Option<DisplaySafeUrl>,
}

impl MalwareCheckSettings {
    pub fn resolve(
        filesystem: Option<&FilesystemOptions>,
        environment: &EnvironmentOptions,
    ) -> Self {
        let audit = filesystem.and_then(|options| options.audit.as_ref());

        Self {
            enabled: environment
                .malware_check
                .value
                .or(audit.and_then(|audit| audit.malware_check))
                .unwrap_or_default(),
            malware_check_url: environment
                .malware_check_url
                .clone()
                .or_else(|| audit.and_then(|audit| audit.malware_check_url.clone())),
        }
    }
}

/// The `preview-features` configuration option.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schemars", schemars(untagged))]
pub enum PreviewFeaturesOption {
    Toggle(bool),
    Features(Vec<MaybePreviewFeature>),
}

// A derived `#[serde(untagged)]` implementation replaces detailed type and element errors with
// "data did not match any variant". Use a type-directed visitor to preserve specific errors.
impl<'de> Deserialize<'de> for PreviewFeaturesOption {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        serde_untagged::UntaggedEnumVisitor::new()
            .expecting("a boolean or a list of preview feature names")
            .bool(|value| Ok(Self::Toggle(value)))
            .seq(|sequence| sequence.deserialize().map(Self::Features))
            .deserialize(deserializer)
    }
}

#[expect(
    dead_code,
    reason = "Fields are only used by the OptionsMetadata and JsonSchema derives"
)]
#[derive(OptionsMetadata)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schemars", schemars(rename_all = "kebab-case"))]
struct PreviewOptionsDefinition {
    // Keep this legacy setting in the JSON schema, but omit it from option metadata. The generated
    // settings reference then documents only `preview-features`.
    /// Whether to enable all experimental preview features.
    ///
    /// Use `preview-features` instead.
    #[deprecated(note = "use `preview-features` instead")]
    preview: Option<bool>,
    /// Whether to enable specific preview features or all preview features.
    ///
    /// uv ignores unknown feature names and reports a warning.
    #[option(
        default = "false",
        value_type = "bool | list[str]",
        example = r#"
            preview-features = true
            # or
            preview-features = ["json-output"]
        "#
    )]
    preview_features: Option<PreviewFeaturesOption>,
}

/// The user's preview configuration from `preview` or `preview-features`.
#[derive(Debug, Clone)]
pub enum PreviewOption {
    /// Whether to enable all experimental preview features.
    Preview(bool),
    /// Whether to enable specific preview features or all preview features.
    PreviewFeatures(PreviewFeaturesOption),
}

impl uv_options_metadata::OptionsMetadata for PreviewOption {
    fn record(visit: &mut dyn uv_options_metadata::Visit) {
        <PreviewOptionsDefinition as uv_options_metadata::OptionsMetadata>::record(visit);
    }
}

#[cfg(feature = "schemars")]
struct ConflictingPreviewOptions;

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for ConflictingPreviewOptions {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("ConflictingPreviewOptions")
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "object",
            "properties": {
                "preview": {},
                "preview-features": {},
            },
            "required": ["preview", "preview-features"],
        })
    }
}

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for PreviewOption {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("PreviewOption")
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let mut schema = <PreviewOptionsDefinition as schemars::JsonSchema>::json_schema(generator);
        // Keep this constraint in a referenced schema to avoid a fastjsonschema code-generation
        // bug. See: https://github.com/astral-sh/uv/pull/20547.
        schema.insert(
            "not".to_string(),
            generator
                .subschema_for::<ConflictingPreviewOptions>()
                .into(),
        );
        schema
    }
}

impl PreviewOption {
    fn try_from(
        preview: Option<bool>,
        preview_features: Option<PreviewFeaturesOption>,
    ) -> Result<Option<Self>, &'static str> {
        match (preview, preview_features) {
            (Some(_), Some(_)) => Err("cannot specify both `preview` and `preview-features`"),
            (Some(b), None) => Ok(Some(Self::Preview(b))),
            (None, Some(features)) => Ok(Some(Self::PreviewFeatures(features))),
            (None, None) => Ok(None),
        }
    }

    /// Resolve the preview configuration, warning and ignoring unknown feature names.
    pub fn resolve(&self) -> Preview {
        use PreviewFeaturesOption::{Features, Toggle};

        match self {
            Self::Preview(false) | Self::PreviewFeatures(Toggle(false)) => Preview::default(),
            Self::Preview(true) | Self::PreviewFeatures(Toggle(true)) => Preview::all(),
            Self::PreviewFeatures(Features(features)) => Preview::from_feature_names(features),
        }
    }
}
