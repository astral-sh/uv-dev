//! Read these fields from `pyproject.toml`:
//!
//! * `project.{dependencies,optional-dependencies}`
//! * `tool.uv.sources`
//! * `tool.uv.workspace`
//!
//! Convert the fields into a dependency specification.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::Formatter;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use glob::Pattern;
use rustc_hash::{FxBuildHasher, FxHashSet};
use serde::de::SeqAccess;
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;
use tracing::instrument;
use uv_build_backend::BuildBackendSettings;
use uv_configuration::{ExcludeDependency, GitLfsSetting, Override};
use uv_distribution_types::{Index, IndexName, RequirementSource};
use uv_fs::{PortablePathBuf, try_relative_to_if};
use uv_git_types::GitReference;
use uv_macros::OptionsMetadata;
use uv_normalize::{DefaultGroups, ExtraName, GroupName, PackageName};
use uv_options_metadata::{OptionSet, OptionsMetadata, Visit};
use uv_pep440::{Version, VersionSpecifiers};
use uv_pep508::MarkerTree;
use uv_pypi_types::{
    ConflictError, Conflicts, DependencyGroups, SchemaConflicts, SupportedEnvironments,
    VerbatimParsedUrl,
};
use uv_redacted::DisplaySafeUrl;
use uv_toml::deserialize_unique_map;

#[derive(Error, Debug)]
pub enum PyprojectTomlError {
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
    #[error("Failed to parse `tool.uv.sources`")]
    Source(
        #[from]
        #[source]
        SourceError,
    ),
    #[error(
        "`pyproject.toml` is using the `[project]` table, but the required `project.name` field is not set"
    )]
    MissingName,
    #[error(
        "`pyproject.toml` is using the `[project]` table, but the required `project.version` field is neither set nor present in the `project.dynamic` list"
    )]
    MissingVersion,
}

fn deserialize_optional_dependencies<'de, D, V>(
    deserializer: D,
) -> Result<Option<BTreeMap<ExtraName, V>>, D::Error>
where
    D: Deserializer<'de>,
    V: Deserialize<'de>,
{
    deserialize_unique_map(deserializer, |key: &ExtraName| {
        format!("duplicate normalized extra name `{key}`")
    })
    .map(Some)
}

/// A `pyproject.toml` file that follows PEP 517.
#[derive(Deserialize, Debug, Clone)]
#[cfg_attr(test, derive(Serialize))]
#[serde(rename_all = "kebab-case")]
pub struct PyProjectToml {
    /// PEP 621-compliant project metadata.
    pub project: Option<Project>,
    /// Tool-specific metadata.
    pub tool: Option<Tool>,
    /// Non-project dependency groups, as defined in PEP 735.
    pub dependency_groups: Option<DependencyGroups>,
    /// The original document.
    #[serde(skip)]
    pub raw: String,

    /// Record whether the document contains a `build-system` section.
    #[serde(default, skip_serializing)]
    build_system: Option<serde::de::IgnoredAny>,
}

impl PyProjectToml {
    /// Parse a `PyProjectToml` from a raw TOML string.
    #[instrument("toml::from_str workspace", skip_all, fields(path = %_path.as_ref().display()))]
    pub fn from_string(raw: String, _path: impl AsRef<Path>) -> Result<Self, PyprojectTomlError> {
        let pyproject: Self = match toml::from_str(&raw) {
            Ok(pyproject) => pyproject,
            Err(error) => {
                // Preserve the more specific source error if both parses would fail.
                let sources = toml::from_str::<PyProjectTomlSourcesWire>(&raw)
                    .map_err(PyprojectTomlError::Toml)?
                    .tool
                    .and_then(|tool| tool.uv)
                    .and_then(|uv| uv.sources);
                if let Some(sources) = sources {
                    ToolUvSources::try_from(sources)?;
                }
                return Err(PyprojectTomlError::Toml(error));
            }
        };

        Ok(Self { raw, ..pyproject })
    }

    /// Return `true` if the project is a Python package instead of a virtual project.
    pub fn is_package(&self, require_build_system: bool) -> bool {
        // Use the explicit `tool.uv.package` setting if it is present.
        if let Some(is_package) = self.tool_uv_package() {
            return is_package;
        }

        // Otherwise, treat the project as a package if `build-system` is present.
        self.build_system.is_some() || !require_build_system
    }

    /// Return `tool.uv.package` if it is set.
    fn tool_uv_package(&self) -> Option<bool> {
        self.tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.package)
    }

    /// Return whether the project manifest contains a script table.
    pub fn has_scripts(&self) -> bool {
        if let Some(ref project) = self.project {
            project.gui_scripts.is_some() || project.scripts.is_some()
        } else {
            false
        }
    }

    /// Return the project conflicts.
    pub(crate) fn conflicts(&self) -> Result<Conflicts, ConflictError> {
        let empty = Conflicts::empty();
        let Some(project) = self.project.as_ref() else {
            return Ok(empty);
        };
        let Some(tool) = self.tool.as_ref() else {
            return Ok(empty);
        };
        let Some(tooluv) = tool.uv.as_ref() else {
            return Ok(empty);
        };
        let Some(conflicting) = tooluv.conflicts.as_ref() else {
            return Ok(empty);
        };
        conflicting.to_conflicts_with_package_name(&project.name)
    }
}

// Ignore the original document when comparing projects.
impl PartialEq for PyProjectToml {
    fn eq(&self, other: &Self) -> bool {
        self.project.eq(&other.project) && self.tool.eq(&other.tool)
    }
}

impl Eq for PyProjectToml {}

impl AsRef<[u8]> for PyProjectToml {
    fn as_ref(&self) -> &[u8] {
        self.raw.as_bytes()
    }
}

/// PEP 621 project metadata (`project`).
///
/// See <https://packaging.python.org/en/latest/specifications/pyproject-toml>.
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(test, derive(Serialize))]
#[serde(rename_all = "kebab-case", try_from = "ProjectWire")]
pub struct Project {
    /// The project name.
    pub name: PackageName,
    /// The project version.
    version: Option<Version>,
    /// The Python versions that are compatible with this project.
    pub(crate) requires_python: Option<VersionSpecifiers>,
    /// The project dependencies.
    pub dependencies: Option<Vec<String>>,
    /// The optional project dependencies.
    pub optional_dependencies: Option<BTreeMap<ExtraName, Vec<String>>>,

    /// Record whether the document contains a `gui-scripts` section.
    #[serde(default, skip_serializing)]
    gui_scripts: Option<serde::de::IgnoredAny>,
    /// Record whether the document contains a `scripts` section.
    #[serde(default, skip_serializing)]
    scripts: Option<serde::de::IgnoredAny>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
struct ProjectWire {
    name: Option<PackageName>,
    version: Option<Version>,
    dynamic: Option<Vec<String>>,
    requires_python: Option<VersionSpecifiers>,
    dependencies: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_dependencies")]
    optional_dependencies: Option<BTreeMap<ExtraName, Vec<String>>>,
    gui_scripts: Option<serde::de::IgnoredAny>,
    scripts: Option<serde::de::IgnoredAny>,
}

impl TryFrom<ProjectWire> for Project {
    type Error = PyprojectTomlError;

    fn try_from(value: ProjectWire) -> Result<Self, Self::Error> {
        // Report a specific error if `[project.name]` is not present.
        let name = value.name.ok_or(PyprojectTomlError::MissingName)?;

        // Report a specific error if `[project.version]` is absent from both the project and
        // `[project.dynamic]`.
        if value.version.is_none()
            && !value
                .dynamic
                .as_ref()
                .is_some_and(|dynamic| dynamic.iter().any(|field| field == "version"))
        {
            return Err(PyprojectTomlError::MissingVersion);
        }

        Ok(Self {
            name,
            version: value.version,
            requires_python: value.requires_python,
            dependencies: value.dependencies,
            optional_dependencies: value.optional_dependencies,
            gui_scripts: value.gui_scripts,
            scripts: value.scripts,
        })
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(Serialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Tool {
    pub uv: Option<ToolUv>,
}

/// Validate the `tool.uv.index` field.
///
/// Reject duplicate index names and multiple default indexes.
fn deserialize_index_vec<'de, D>(deserializer: D) -> Result<Option<Vec<Index>>, D::Error>
where
    D: Deserializer<'de>,
{
    let indexes = Option::<Vec<Index>>::deserialize(deserializer)?;
    if let Some(indexes) = indexes.as_ref() {
        let mut seen_names = FxHashSet::with_capacity_and_hasher(indexes.len(), FxBuildHasher);
        let mut seen_default = false;
        for index in indexes {
            if let Some(name) = index.name.as_ref() {
                if !seen_names.insert(name) {
                    return Err(serde::de::Error::custom(format!(
                        "duplicate index name `{name}`"
                    )));
                }
            }
            if index.default {
                if seen_default {
                    return Err(serde::de::Error::custom(
                        "found multiple indexes with `default = true`; only one index may be marked as default",
                    ));
                }
                seen_default = true;
            }
        }
    }
    Ok(indexes)
}

/// An override dependency before source lowering.
pub type OverrideDependency = Override<uv_pep508::Requirement<VerbatimParsedUrl>>;

// NOTE(charlie): When adding fields to this struct, also ignore them on `Options` in
// `crates/uv-settings/src/settings.rs`.
#[derive(Deserialize, OptionsMetadata, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(Serialize))]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ToolUv {
    /// The sources to use when resolving dependencies.
    ///
    /// `tool.uv.sources` adds development sources to the dependency metadata. A source can be a
    /// Git repository, a URL, a local path, or another registry.
    ///
    /// For more information, see [Dependencies](../concepts/projects/dependencies.md).
    #[option(
        default = "{}",
        value_type = "dict",
        example = r#"
            [tool.uv.sources]
            httpx = { git = "https://github.com/encode/httpx", tag = "0.27.0" }
            pytest = { url = "https://files.pythonhosted.org/packages/6b/77/7440a06a8ead44c7757a64362dd22df5760f9b12dc5f11b6188cd2fc27a0/pytest-8.3.3-py3-none-any.whl" }
            pydantic = { path = "/path/to/pydantic", editable = true }
        "#
    )]
    pub sources: Option<ToolUvSources>,

    /// The indexes to use when resolving dependencies.
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
        default = "[]",
        value_type = "dict",
        example = r#"
            [[tool.uv.index]]
            name = "pytorch"
            url = "https://download.pytorch.org/whl/cu130"
        "#
    )]
    #[serde(deserialize_with = "deserialize_index_vec", default)]
    pub index: Option<Vec<Index>>,

    /// The workspace definition for the project, if any.
    #[option_group]
    pub(crate) workspace: Option<ToolUvWorkspace>,

    /// Whether uv manages the project. If `false`, `uv run` ignores the project.
    #[option(
        default = r#"true"#,
        value_type = "bool",
        example = r#"
            managed = false
        "#
    )]
    pub(crate) managed: Option<bool>,

    /// Whether the project is a Python package or a non-package ("virtual") project.
    ///
    /// uv builds packages and installs them into the virtual environment in editable mode.
    /// Packages therefore require a build backend. uv does _not_ build or install virtual
    /// projects. It installs only their dependencies into the virtual environment.
    ///
    /// A package requires a `build-system` in `pyproject.toml`. Its structure must also meet the
    /// build backend's requirements, such as a `src` layout.
    #[option(
        default = r#"true"#,
        value_type = "bool",
        example = r#"
            package = false
        "#
    )]
    package: Option<bool>,

    /// The list of `dependency-groups` to install by default.
    ///
    /// Set this to `"all"` to enable every group by default.
    #[option(
        default = r#"["dev"]"#,
        value_type = r#"str | list[str]"#,
        example = r#"
            default-groups = ["docs"]
        "#
    )]
    pub default_groups: Option<DefaultGroups>,

    /// Additional settings for `dependency-groups`.
    ///
    /// Use this setting to add `requires-python` constraints to dependency groups. For example,
    /// development tools can require a newer Python version than the project.
    ///
    /// To define dependency groups, use the top-level `[dependency-groups]` table.
    #[option(
        default = "[]",
        value_type = "dict",
        example = r#"
            [tool.uv.dependency-groups]
            my-group = {requires-python = ">=3.12"}
        "#
    )]
    pub(crate) dependency_groups: Option<ToolUvDependencyGroups>,

    /// The project's development dependencies.
    ///
    /// `uv run` and `uv sync` install development dependencies by default. They do not appear in
    /// the project's published metadata.
    ///
    /// Use the standard `dependency-groups.dev` field instead of this field. uv combines
    /// `tool.uv.dev-dependencies` and `dependency-groups.dev` to determine the requirements of the
    /// `dev` dependency group.
    #[cfg_attr(
        feature = "schemars",
        schemars(
            with = "Option<Vec<String>>",
            description = "PEP 508-style requirements, e.g., `ruff==0.5.0`, or `ruff @ https://...`."
        )
    )]
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            dev-dependencies = ["ruff==0.5.0"]
        "#
    )]
    pub dev_dependencies: Option<Vec<uv_pep508::Requirement<VerbatimParsedUrl>>>,

    /// Overrides to apply when resolving the project's dependencies.
    ///
    /// Overrides select a specific package version. They ignore the versions that other packages
    /// request, even if the selected version would normally make the resolution invalid.
    ///
    /// Constraints are _additive_: uv combines them with package requirements. Overrides are
    /// _absolute_: they replace package requirements.
    ///
    /// An override does _not_ install a package by itself. A direct or transitive dependency must
    /// also request the package.
    ///
    /// To override the dependencies of a specific package, use a table with `package` and
    /// `dependencies`. The `package` table identifies the package by `name` and, optionally,
    /// `version`. If you omit `version`, the overrides apply to every version of that package.
    /// Requirements in `dependencies` replace dependencies with the same name and add undeclared
    /// dependencies. Other dependencies do not change.
    ///
    /// Scoped overrides support registry version specifiers only. They do not support direct URL
    /// or path sources, Git sources, or explicit indexes.
    ///
    /// !!! note
    ///     `uv lock`, `uv sync`, and `uv run` read `override-dependencies` only from the workspace
    ///     root's `pyproject.toml`. They ignore declarations in other workspace members and
    ///     `uv.toml` files.
    #[option(
        default = "[]",
        value_type = "list[str | dict]",
        example = r#"
            override-dependencies = [
                # Always install Werkzeug 2.3.0.
                "werkzeug==2.3.0",
                # Use itsdangerous 2.1.2 when requested by Flask 3.0.0.
                { package = { name = "flask", version = "3.0.0" }, dependencies = ["itsdangerous==2.1.2"] },
            ]
        "#
    )]
    pub(crate) override_dependencies: Option<Vec<OverrideDependency>>,

    /// Dependencies to exclude when resolving the project's dependencies.
    ///
    /// Exclusions prevent uv from selecting a package during resolution, even if another package
    /// requests it. uv removes the excluded package from the dependency list.
    ///
    /// An excluded package is not installed, even if a transitive dependency requests it. Use
    /// exclusions to remove optional dependencies or work around broken package dependencies.
    ///
    /// To exclude the dependencies of a specific package, use a table with `package` and
    /// `dependencies`. The `package` table identifies the package by `name` and, optionally,
    /// `version`. If you omit `version`, the exclusions apply to every version of that package.
    /// A version-specific entry takes priority over an entry for all versions.
    ///
    /// !!! note
    ///     `uv lock`, `uv sync`, and `uv run` read `exclude-dependencies` only from the workspace
    ///     root's `pyproject.toml`. They ignore declarations in other workspace members and
    ///     `uv.toml` files.
    #[option(
        default = "[]",
        value_type = "list[str | dict]",
        example = r#"
            # Exclude Werkzeug from being installed, even if transitive dependencies request it.
            exclude-dependencies = [
                "werkzeug",
                { package = { name = "flask", version = "3.0.0" }, dependencies = ["itsdangerous"] },
            ]
        "#
    )]
    pub(crate) exclude_dependencies: Option<Vec<ExcludeDependency>>,

    /// Constraints to apply when resolving the project's dependencies.
    ///
    /// Constraints restrict the dependency versions that uv selects during resolution.
    ///
    /// A constraint does _not_ install a package by itself. A direct or transitive dependency
    /// must also request the package.
    ///
    /// !!! note
    ///     `uv lock`, `uv sync`, and `uv run` read `constraint-dependencies` only from the workspace
    ///     root's `pyproject.toml`. They ignore declarations in other workspace members and
    ///     `uv.toml` files.
    #[cfg_attr(
        feature = "schemars",
        schemars(
            with = "Option<Vec<String>>",
            description = "PEP 508-style requirements, e.g., `ruff==0.5.0`, or `ruff @ https://...`."
        )
    )]
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            # Ensure that the grpcio version is always less than 1.65, if it's requested by a
            # direct or transitive dependency.
            constraint-dependencies = ["grpcio<1.65"]
        "#
    )]
    pub(crate) constraint_dependencies: Option<Vec<uv_pep508::Requirement<VerbatimParsedUrl>>>,

    /// Constraints to apply when solving build dependencies.
    ///
    /// Build constraints restrict the build dependency versions that uv selects when it builds a
    /// package during resolution or installation.
    ///
    /// A build constraint does _not_ install a package by itself. The project's build dependency
    /// graph must also request the package.
    ///
    /// !!! note
    ///     `uv lock`, `uv sync`, and `uv run` read `build-constraint-dependencies` only from the
    ///     workspace root's `pyproject.toml`. They ignore declarations in other workspace members
    ///     and `uv.toml` files.
    #[cfg_attr(
        feature = "schemars",
        schemars(
            with = "Option<Vec<String>>",
            description = "PEP 508-style requirements, e.g., `ruff==0.5.0`, or `ruff @ https://...`."
        )
    )]
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            # Ensure that the setuptools v60.0.0 is used whenever a package has a build dependency
            # on setuptools.
            build-constraint-dependencies = ["setuptools==60.0.0"]
        "#
    )]
    pub(crate) build_constraint_dependencies:
        Option<Vec<uv_pep508::Requirement<VerbatimParsedUrl>>>,

    /// The supported environments for dependency resolution.
    ///
    /// By default, `uv lock` resolves dependencies for every possible environment. Restrict the
    /// supported environments to improve performance and avoid unsatisfiable branches.
    ///
    /// `uv pip compile --universal` also uses these environments.
    #[cfg_attr(
        feature = "schemars",
        schemars(
            with = "Option<Vec<String>>",
            description = "A list of environment markers, e.g., `python_version >= '3.6'`."
        )
    )]
    #[option(
        default = "[]",
        value_type = "str | list[str]",
        example = r#"
            # Resolve for macOS, but not for Linux or Windows.
            environments = ["sys_platform == 'darwin'"]
        "#
    )]
    pub(crate) environments: Option<SupportedEnvironments>,

    /// Required platforms for packages that do not have source distributions.
    ///
    /// Without a source distribution, a package is available only on the platforms that its wheels
    /// support. For example, a package that publishes only Linux wheels cannot be installed on
    /// macOS or Windows.
    ///
    /// By default, uv requires each package to include at least one wheel that is compatible with
    /// the selected Python version. Use `required-environments` to require wheels for specific
    /// platforms. Resolution fails if those wheels are not available.
    ///
    /// The `environments` setting _limits_ the environments that uv considers during resolution.
    /// The `required-environments` setting _expands_ the platforms that uv _must_ support.
    ///
    /// For example, `environments = ["sys_platform == 'darwin'"]` limits resolution to macOS and
    /// ignores Linux and Windows. In contrast, `required-environments = ["sys_platform == 'darwin'"]`
    /// _requires_ each package without a source distribution to include a macOS wheel.
    #[cfg_attr(
        feature = "schemars",
        schemars(
            with = "Option<Vec<String>>",
            description = "A list of environment markers, e.g., `sys_platform == 'darwin'."
        )
    )]
    #[option(
        default = "[]",
        value_type = "str | list[str]",
        example = r#"
            # Require that the package is available on the following platforms:
            required-environments = [
                # macOS on Apple Silicon (ARM)
                "sys_platform == 'darwin' and platform_machine == 'arm64'",
                # Linux on x86_64 (Intel/AMD)
                "sys_platform == 'linux' and platform_machine == 'x86_64'",
                # Windows on x86_64 (Intel/AMD)
                "sys_platform == 'win32' and platform_machine == 'AMD64'",
            ]
        "#
    )]
    pub(crate) required_environments: Option<SupportedEnvironments>,

    /// Declare extras or dependency groups that conflict with each other.
    ///
    /// Declare a conflict when extras have incompatible dependencies but are not intended to be
    /// active together. For example, extra `foo` can require `numpy==2.0.0`, while extra `bar`
    /// requires `numpy==2.1.0`. uv can still create a universal resolution if the extras are
    /// mutually exclusive.
    ///
    /// When you declare the conflict, uv accounts for the mutually exclusive extras and groups.
    /// Installation fails if a user activates conflicting extras together.
    #[cfg_attr(
        feature = "schemars",
        schemars(description = "A list of sets of conflicting groups or extras.")
    )]
    #[option(
        default = r#"[]"#,
        value_type = "list[list[dict]]",
        example = r#"
            # Require that `package[extra1]` and `package[extra2]` are resolved
            # in different forks so that they cannot conflict with one another.
            conflicts = [
                [
                    { extra = "extra1" },
                    { extra = "extra2" },
                ]
            ]

            # Require that the dependency groups `group1` and `group2`
            # are resolved in different forks so that they cannot conflict
            # with one another.
            conflicts = [
                [
                    { group = "group1" },
                    { group = "group2" },
                ]
            ]
        "#
    )]
    pub(crate) conflicts: Option<SchemaConflicts>,

    // Keep this field only for schema and documentation generation. The backend reads its settings
    // separately, and workspace configuration never merges them.
    /// Configuration for the uv build backend.
    ///
    /// These settings apply only to the `uv_build` backend. Other backends, such as hatchling,
    /// have their own configuration.
    #[option_group]
    build_backend: Option<BuildBackendSettingsSchema>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(Serialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ToolUvSources(BTreeMap<PackageName, Sources>);

#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
struct PyProjectTomlSourcesWire {
    tool: Option<ToolSourcesWire>,
}

#[derive(Deserialize, Debug)]
struct ToolSourcesWire {
    uv: Option<ToolUvSourcesOnlyWire>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
struct ToolUvSourcesOnlyWire {
    sources: Option<ToolUvSourcesWire>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
struct ToolUvSourcesWire(BTreeMap<PackageName, SourcesWire>);

impl ToolUvSources {
    /// Return the `BTreeMap` that maps package names to sources.
    pub fn inner(&self) -> &BTreeMap<PackageName, Sources> {
        &self.0
    }

    /// Convert [`ToolUvSources`] into its `BTreeMap`.
    #[must_use]
    pub(crate) fn into_inner(self) -> BTreeMap<PackageName, Sources> {
        self.0
    }
}

impl TryFrom<ToolUvSourcesWire> for ToolUvSources {
    type Error = SourceError;

    fn try_from(wire: ToolUvSourcesWire) -> Result<Self, Self::Error> {
        wire.0
            .into_iter()
            .map(|(name, sources)| Sources::try_from(sources).map(|sources| (name, sources)))
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map(Self)
    }
}

/// Ensure that all keys in the TOML table are unique.
impl<'de> serde::de::Deserialize<'de> for ToolUvSourcesWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_unique_map(deserializer, |key: &PackageName| {
            format!("duplicate sources for package `{key}`")
        })
        .map(ToolUvSourcesWire)
    }
}

/// Ensure that all keys in the TOML table are unique.
impl<'de> serde::de::Deserialize<'de> for ToolUvSources {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_unique_map(deserializer, |key: &PackageName| {
            format!("duplicate sources for package `{key}`")
        })
        .map(ToolUvSources)
    }
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(Serialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub(crate) struct ToolUvDependencyGroups(BTreeMap<GroupName, DependencyGroupSettings>);

impl ToolUvDependencyGroups {
    /// Return the `BTreeMap` that maps group names to settings.
    pub(crate) fn inner(&self) -> &BTreeMap<GroupName, DependencyGroupSettings> {
        &self.0
    }
}

/// Ensure that all keys in the TOML table are unique.
impl<'de> serde::de::Deserialize<'de> for ToolUvDependencyGroups {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_unique_map(deserializer, |key: &GroupName| {
            format!("duplicate settings for dependency group `{key}`")
        })
        .map(ToolUvDependencyGroups)
    }
}

#[derive(Deserialize, Default, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(Serialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub(crate) struct DependencyGroupSettings {
    /// The Python version required to install this group.
    #[cfg_attr(feature = "schemars", schemars(with = "Option<String>"))]
    pub(crate) requires_python: Option<VersionSpecifiers>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, rename_all = "kebab-case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
enum ExtraBuildDependencyWire {
    Unannotated(uv_pep508::Requirement<VerbatimParsedUrl>),
    #[serde(rename_all = "kebab-case")]
    Annotated {
        requirement: uv_pep508::Requirement<VerbatimParsedUrl>,
        match_runtime: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    deny_unknown_fields,
    from = "ExtraBuildDependencyWire",
    into = "ExtraBuildDependencyWire"
)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ExtraBuildDependency {
    pub requirement: uv_pep508::Requirement<VerbatimParsedUrl>,
    pub match_runtime: bool,
}

impl From<ExtraBuildDependency> for uv_pep508::Requirement<VerbatimParsedUrl> {
    fn from(value: ExtraBuildDependency) -> Self {
        value.requirement
    }
}

impl From<ExtraBuildDependencyWire> for ExtraBuildDependency {
    fn from(wire: ExtraBuildDependencyWire) -> Self {
        match wire {
            ExtraBuildDependencyWire::Unannotated(requirement) => Self {
                requirement,
                match_runtime: false,
            },
            ExtraBuildDependencyWire::Annotated {
                requirement,
                match_runtime,
            } => Self {
                requirement,
                match_runtime,
            },
        }
    }
}

impl From<ExtraBuildDependency> for ExtraBuildDependencyWire {
    fn from(item: ExtraBuildDependency) -> Self {
        Self::Annotated {
            requirement: item.requirement,
            match_runtime: item.match_runtime,
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ExtraBuildDependencies(BTreeMap<PackageName, Vec<ExtraBuildDependency>>);

impl std::ops::Deref for ExtraBuildDependencies {
    type Target = BTreeMap<PackageName, Vec<ExtraBuildDependency>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for ExtraBuildDependencies {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl IntoIterator for ExtraBuildDependencies {
    type Item = (PackageName, Vec<ExtraBuildDependency>);
    type IntoIter = std::collections::btree_map::IntoIter<PackageName, Vec<ExtraBuildDependency>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl FromIterator<(PackageName, Vec<ExtraBuildDependency>)> for ExtraBuildDependencies {
    fn from_iter<T: IntoIterator<Item = (PackageName, Vec<ExtraBuildDependency>)>>(
        iter: T,
    ) -> Self {
        Self(iter.into_iter().collect())
    }
}

/// Ensure that all keys in the TOML table are unique.
impl<'de> serde::de::Deserialize<'de> for ExtraBuildDependencies {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_unique_map(deserializer, |key: &PackageName| {
            format!("duplicate extra-build-dependencies for `{key}`")
        })
        .map(ExtraBuildDependencies)
    }
}

#[derive(Deserialize, OptionsMetadata, Default, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(Serialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct ToolUvWorkspace {
    /// Packages to include as workspace members.
    ///
    /// Use globs or explicit paths.
    ///
    /// For the glob syntax, see the [`glob` documentation](https://docs.rs/glob/latest/glob/struct.Pattern.html).
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            members = ["member1", "path/to/member2", "libs/*"]
        "#
    )]
    pub(crate) members: Option<Vec<SerdePattern>>,
    /// Packages to exclude as workspace members. If a package matches both `members` and
    /// `exclude`, uv excludes it.
    ///
    /// Use globs or explicit paths.
    ///
    /// For the glob syntax, see the [`glob` documentation](https://docs.rs/glob/latest/glob/struct.Pattern.html).
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            exclude = ["member1", "path/to/member2", "libs/*"]
        "#
    )]
    pub(crate) exclude: Option<Vec<SerdePattern>>,
}

/// Serialize and deserialize globs as strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SerdePattern(Pattern);

impl serde::ser::Serialize for SerdePattern {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        self.0.as_str().serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for SerdePattern {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = SerdePattern;

            fn expecting(&self, f: &mut Formatter) -> std::fmt::Result {
                f.write_str("a string")
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Pattern::from_str(v)
                    .map(SerdePattern)
                    .map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_str(Visitor)
    }
}

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for SerdePattern {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("SerdePattern")
    }

    fn json_schema(generator: &mut schemars::generate::SchemaGenerator) -> schemars::Schema {
        <String as schemars::JsonSchema>::json_schema(generator)
    }
}

impl Deref for SerdePattern {
    type Target = Pattern;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case", try_from = "SourcesWire")]
pub struct Sources(#[cfg_attr(feature = "schemars", schemars(with = "SourcesWire"))] Vec<Source>);

impl Sources {
    /// Return an [`Iterator`] over the sources.
    ///
    /// Multiple entries always use disjoint markers.
    ///
    /// The iterator contains at most one registry source.
    pub fn iter(&self) -> impl Iterator<Item = &Source> {
        self.0.iter()
    }
}

impl FromIterator<Source> for Sources {
    fn from_iter<T: IntoIterator<Item = Source>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl IntoIterator for Sources {
    type Item = Source;
    type IntoIter = std::vec::IntoIter<Source>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema), schemars(untagged))]
enum SourcesWire {
    One(Source),
    Many(Vec<Source>),
}

impl<'de> serde::de::Deserialize<'de> for SourcesWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = SourcesWire;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a single source (as a map) or list of sources")
            }

            fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let sources = serde::de::Deserialize::deserialize(
                    serde::de::value::SeqAccessDeserializer::new(seq),
                )?;
                Ok(SourcesWire::Many(sources))
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: serde::de::MapAccess<'de>,
            {
                let source = serde::de::Deserialize::deserialize(
                    serde::de::value::MapAccessDeserializer::new(&mut map),
                )?;
                Ok(SourcesWire::One(source))
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

impl TryFrom<SourcesWire> for Sources {
    type Error = SourceError;

    fn try_from(wire: SourcesWire) -> Result<Self, Self::Error> {
        match wire {
            SourcesWire::One(source) => Ok(Self(vec![source])),
            SourcesWire::Many(sources) => {
                for [lhs, rhs] in sources.array_windows() {
                    if lhs.extra() != rhs.extra() {
                        continue;
                    }
                    if lhs.group() != rhs.group() {
                        continue;
                    }

                    let lhs = lhs.marker();
                    let rhs = rhs.marker();
                    if !lhs.is_disjoint(rhs) {
                        let Some(left) = lhs.contents().map(|contents| contents.to_string()) else {
                            return Err(SourceError::MissingMarkers);
                        };

                        let Some(right) = rhs.contents().map(|contents| contents.to_string())
                        else {
                            return Err(SourceError::MissingMarkers);
                        };

                        let hint = lhs.negate().and(rhs);
                        let hint = hint
                            .contents()
                            .map(|contents| contents.to_string())
                            .unwrap_or_else(|| "true".to_string());

                        return Err(SourceError::OverlappingMarkers(left, right, hint));
                    }
                }

                // Require at least one source.
                if sources.is_empty() {
                    return Err(SourceError::EmptySources);
                }

                Ok(Self(sources))
            }
        }
    }
}

/// A `tool.uv.sources` value.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case", untagged, deny_unknown_fields)]
pub enum Source {
    /// A remote Git repository that uses HTTPS or SSH.
    ///
    /// Example:
    /// ```toml
    /// flask = { git = "https://github.com/pallets/flask", tag = "3.0.0" }
    /// ```
    Git {
        /// The repository URL (without the `git+` prefix).
        git: DisplaySafeUrl,
        /// The directory that contains `pyproject.toml`, if it is not in the repository root.
        subdirectory: Option<PortablePathBuf>,
        /// The path to an archive in the repository.
        path: Option<PortablePathBuf>,
        // Only one field may be set. Validate this later and report a custom error.
        rev: Option<String>,
        tag: Option<String>,
        branch: Option<String>,
        /// Whether to use Git LFS when cloning the repository.
        lfs: Option<bool>,
        #[serde(
            skip_serializing_if = "uv_pep508::marker::ser::is_empty",
            serialize_with = "uv_pep508::marker::ser::serialize",
            default
        )]
        marker: MarkerTree,
        extra: Option<ExtraName>,
        group: Option<GroupName>,
    },
    /// A remote `http://` or `https://` URL for a wheel (`.whl`) or source distribution
    /// (`.zip`, `.tar.gz`).
    ///
    /// Example:
    /// ```toml
    /// flask = { url = "https://files.pythonhosted.org/packages/61/80/ffe1da13ad9300f87c93af113edd0638c75138c42a0994becfacac078c06/flask-3.0.3-py3-none-any.whl" }
    /// ```
    Url {
        url: DisplaySafeUrl,
        /// For a source distribution, the directory that contains `pyproject.toml`, if it is not
        /// in the archive root.
        subdirectory: Option<PortablePathBuf>,
        #[serde(
            skip_serializing_if = "uv_pep508::marker::ser::is_empty",
            serialize_with = "uv_pep508::marker::ser::serialize",
            default
        )]
        marker: MarkerTree,
        extra: Option<ExtraName>,
        group: Option<GroupName>,
    },
    /// The path to a wheel (`.whl`), a source distribution (`.zip` or `.tar.gz`), or a source
    /// tree. A source tree contains a `pyproject.toml` or `setup.py` file in its root.
    Path {
        path: PortablePathBuf,
        /// `false` by default.
        editable: Option<bool>,
        /// Whether to treat the dependency as a buildable Python package (`true`) or as a virtual
        /// package (`false`). If `false`, uv does not build or install the package. It installs the
        /// package's dependencies into the virtual environment.
        ///
        /// If omitted, uv infers the package status from `[build-system]` in the project's
        /// `pyproject.toml`.
        package: Option<bool>,
        #[serde(
            skip_serializing_if = "uv_pep508::marker::ser::is_empty",
            serialize_with = "uv_pep508::marker::ser::serialize",
            default
        )]
        marker: MarkerTree,
        extra: Option<ExtraName>,
        group: Option<GroupName>,
    },
    /// A dependency pinned to a specific index, such as `torch` pinned to
    /// `https://download.pytorch.org/whl/cu118`.
    Registry {
        index: IndexName,
        #[serde(
            skip_serializing_if = "uv_pep508::marker::ser::is_empty",
            serialize_with = "uv_pep508::marker::ser::serialize",
            default
        )]
        marker: MarkerTree,
        extra: Option<ExtraName>,
        group: Option<GroupName>,
    },
    /// A dependency on another package in the workspace.
    Workspace {
        /// `true` selects the current workspace. A string selects a workspace at the given path.
        ///
        /// If `false`, uv gets the package from the remote index instead of the workspace.
        workspace: WorkspaceReference,
        /// Whether to install the package as editable. Defaults to `true`.
        editable: Option<bool>,
        #[serde(
            skip_serializing_if = "uv_pep508::marker::ser::is_empty",
            serialize_with = "uv_pep508::marker::ser::serialize",
            default
        )]
        marker: MarkerTree,
        extra: Option<ExtraName>,
        group: Option<GroupName>,
    },
}

/// A reference to the current workspace or a workspace at a given path.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema), schemars(untagged))]
#[serde(untagged)]
pub enum WorkspaceReference {
    Bool(bool),
    Path(PortablePathBuf),
}

/// Deserialize [`Source`] like `#[serde(untagged)]`, but report more detailed errors.
impl<'de> Deserialize<'de> for Source {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize, Debug, Clone)]
        #[serde(rename_all = "kebab-case", deny_unknown_fields)]
        struct CatchAll {
            git: Option<DisplaySafeUrl>,
            subdirectory: Option<PortablePathBuf>,
            rev: Option<String>,
            tag: Option<String>,
            branch: Option<String>,
            lfs: Option<bool>,
            url: Option<DisplaySafeUrl>,
            path: Option<PortablePathBuf>,
            editable: Option<bool>,
            package: Option<bool>,
            index: Option<IndexName>,
            workspace: Option<WorkspaceReference>,
            #[serde(
                skip_serializing_if = "uv_pep508::marker::ser::is_empty",
                serialize_with = "uv_pep508::marker::ser::serialize",
                default
            )]
            marker: MarkerTree,
            extra: Option<ExtraName>,
            group: Option<GroupName>,
        }

        // Try to deserialize as `CatchAll`.
        let CatchAll {
            git,
            subdirectory,
            rev,
            tag,
            branch,
            lfs,
            url,
            path,
            editable,
            package,
            index,
            workspace,
            marker,
            extra,
            group,
        } = CatchAll::deserialize(deserializer)?;

        // Return an error if both `extra` and `group` are set.
        if extra.is_some() && group.is_some() {
            return Err(serde::de::Error::custom(
                "cannot specify both `extra` and `group`",
            ));
        }

        // A `git` field identifies a Git source.
        if let Some(git) = git {
            if index.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `git` and `index`",
                ));
            }
            if workspace.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `git` and `workspace`",
                ));
            }
            if url.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `git` and `url`",
                ));
            }
            if editable.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `git` and `editable`",
                ));
            }
            if package.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `git` and `package`",
                ));
            }
            if subdirectory.is_some() && path.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `subdirectory` and `path`",
                ));
            }

            // At most one of `rev`, `tag`, and `branch` may be set.
            match (rev.as_ref(), tag.as_ref(), branch.as_ref()) {
                (None, None, None) => {}
                (Some(_), None, None) => {}
                (None, Some(_), None) => {}
                (None, None, Some(_)) => {}
                _ => {
                    return Err(serde::de::Error::custom(
                        "expected at most one of `rev`, `tag`, or `branch`",
                    ));
                }
            }

            // Remove the `git+` prefix from the URL if it is present.
            let git = if let Some(git) = git.as_str().strip_prefix("git+") {
                DisplaySafeUrl::parse(git).map_err(serde::de::Error::custom)?
            } else {
                git
            };

            return Ok(Self::Git {
                git,
                subdirectory,
                path,
                rev,
                tag,
                branch,
                lfs,
                marker,
                extra,
                group,
            });
        }

        // A `url` field identifies a URL source.
        if let Some(url) = url {
            if index.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `url` and `index`",
                ));
            }
            if workspace.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `url` and `workspace`",
                ));
            }
            if path.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `url` and `path`",
                ));
            }
            if git.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `url` and `git`",
                ));
            }
            if rev.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `url` and `rev`",
                ));
            }
            if tag.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `url` and `tag`",
                ));
            }
            if branch.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `url` and `branch`",
                ));
            }
            if editable.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `url` and `editable`",
                ));
            }
            if package.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `url` and `package`",
                ));
            }

            return Ok(Self::Url {
                url,
                subdirectory,
                marker,
                extra,
                group,
            });
        }

        // A `path` field identifies a path source.
        if let Some(path) = path {
            if index.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `path` and `index`",
                ));
            }
            if workspace.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `path` and `workspace`",
                ));
            }
            if git.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `path` and `git`",
                ));
            }
            if url.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `path` and `url`",
                ));
            }
            if rev.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `path` and `rev`",
                ));
            }
            if tag.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `path` and `tag`",
                ));
            }
            if branch.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `path` and `branch`",
                ));
            }

            // A project must be packaged to support an editable installation.
            if editable == Some(true) && package == Some(false) {
                return Err(serde::de::Error::custom(
                    "cannot specify both `editable = true` and `package = false`",
                ));
            }

            return Ok(Self::Path {
                path,
                editable,
                package,
                marker,
                extra,
                group,
            });
        }

        // An `index` field identifies a registry source.
        if let Some(index) = index {
            if workspace.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `index` and `workspace`",
                ));
            }
            if git.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `index` and `git`",
                ));
            }
            if url.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `index` and `url`",
                ));
            }
            if path.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `index` and `path`",
                ));
            }
            if rev.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `index` and `rev`",
                ));
            }
            if tag.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `index` and `tag`",
                ));
            }
            if branch.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `index` and `branch`",
                ));
            }
            if editable.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `index` and `editable`",
                ));
            }
            if package.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `index` and `package`",
                ));
            }

            return Ok(Self::Registry {
                index,
                marker,
                extra,
                group,
            });
        }

        // A `workspace` field identifies a workspace source.
        if let Some(workspace) = workspace {
            if index.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `workspace` and `index`",
                ));
            }
            if git.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `workspace` and `git`",
                ));
            }
            if url.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `workspace` and `url`",
                ));
            }
            if path.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `workspace` and `path`",
                ));
            }
            if rev.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `workspace` and `rev`",
                ));
            }
            if tag.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `workspace` and `tag`",
                ));
            }
            if branch.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `workspace` and `branch`",
                ));
            }
            if package.is_some() {
                return Err(serde::de::Error::custom(
                    "cannot specify both `workspace` and `package`",
                ));
            }

            return Ok(Self::Workspace {
                workspace,
                editable,
                marker,
                extra,
                group,
            });
        }

        // Return an error if no source field is set.
        Err(serde::de::Error::custom(
            "expected one of `git`, `url`, `path`, `index`, or `workspace`",
        ))
    }
}

#[derive(Error, Debug)]
pub enum SourceError {
    #[error("Failed to resolve Git reference: `{0}`")]
    UnresolvedReference(String),
    #[error("Workspace dependency `{0}` must refer to local directory, not a Git repository")]
    WorkspacePackageGit(String),
    #[error("Workspace dependency `{0}` must refer to local directory, not a URL")]
    WorkspacePackageUrl(String),
    #[error("Workspace dependency `{0}` must refer to local directory, not a file")]
    WorkspacePackageFile(String),
    #[error(
        "`{0}` did not resolve to a Git repository, but a Git reference (`--rev {1}`) was provided."
    )]
    UnusedRev(String, String),
    #[error(
        "`{0}` did not resolve to a Git repository, but a Git reference (`--tag {1}`) was provided."
    )]
    UnusedTag(String, String),
    #[error(
        "`{0}` did not resolve to a Git repository, but a Git reference (`--branch {1}`) was provided."
    )]
    UnusedBranch(String, String),
    #[error(
        "`{0}` did not resolve to a Git repository, but a Git extension (`--lfs`) was provided."
    )]
    UnusedLfs(String),
    #[error(
        "`{0}` did not resolve to a local directory, but the `--editable` flag was provided. Editable installs are only supported for local directories."
    )]
    UnusedEditable(String),
    #[error("Failed to resolve absolute path")]
    Absolute(#[from] std::io::Error),
    #[error("Path contains invalid characters: `{}`", _0.display())]
    NonUtf8Path(PathBuf),
    #[error("Source markers must be disjoint, but the following markers overlap: `{0}` and `{1}`.")]
    OverlappingMarkers(String, String, String),
    #[error(
        "When multiple sources are provided, each source must include a platform marker (e.g., `marker = \"sys_platform == 'linux'\"`)"
    )]
    MissingMarkers,
    #[error("Must provide at least one source")]
    EmptySources,
}

impl uv_errors::Hint for SourceError {
    fn hints(&self) -> uv_errors::Hints<'_> {
        match self {
            Self::OverlappingMarkers(_, rhs, replacement) => {
                uv_errors::Hints::from(format!("replace `{rhs}` with `{replacement}`"))
            }
            _ => uv_errors::Hints::none(),
        }
    }
}

impl Source {
    pub fn from_requirement(
        name: &PackageName,
        source: RequirementSource,
        workspace: bool,
        editable: Option<bool>,
        index: Option<IndexName>,
        rev: Option<String>,
        tag: Option<String>,
        branch: Option<String>,
        lfs: GitLfsSetting,
        root: &Path,
        existing_sources: Option<&BTreeMap<PackageName, Sources>>,
    ) -> Result<Option<Self>, SourceError> {
        // If a non-Git source has a Git reference, check existing Git sources before returning an
        // error.
        if !matches!(
            source,
            RequirementSource::GitDirectory { .. } | RequirementSource::GitPath { .. }
        ) && (branch.is_some()
            || tag.is_some()
            || rev.is_some()
            || matches!(lfs, GitLfsSetting::Enabled { .. }))
        {
            if let Some(sources) = existing_sources
                && let Some(package_sources) = sources.get(name)
            {
                for existing_source in package_sources.iter() {
                    if let Self::Git {
                        git,
                        subdirectory,
                        path,
                        marker,
                        extra,
                        group,
                        ..
                    } = existing_source
                    {
                        return Ok(Some(Self::Git {
                            git: git.clone(),
                            subdirectory: subdirectory.clone(),
                            rev,
                            tag,
                            branch,
                            lfs: lfs.into(),
                            marker: *marker,
                            path: path.clone(),
                            extra: extra.clone(),
                            group: group.clone(),
                        }));
                    }
                }
            }
            if let Some(rev) = rev {
                return Err(SourceError::UnusedRev(name.to_string(), rev));
            }
            if let Some(tag) = tag {
                return Err(SourceError::UnusedTag(name.to_string(), tag));
            }
            if let Some(branch) = branch {
                return Err(SourceError::UnusedBranch(name.to_string(), branch));
            }
            if matches!(lfs, GitLfsSetting::Enabled { from_env: false }) {
                return Err(SourceError::UnusedLfs(name.to_string()));
            }
        }

        // Reject `--editable` for a non-path source.
        if !workspace {
            if !matches!(source, RequirementSource::Directory { .. }) {
                if editable == Some(true) {
                    return Err(SourceError::UnusedEditable(name.to_string()));
                }
            }
        }

        // Reject explicit sources for a workspace package.
        if workspace {
            return match source {
                RequirementSource::Registry { .. } | RequirementSource::Directory { .. } => {
                    Ok(Some(Self::Workspace {
                        workspace: WorkspaceReference::Bool(true),
                        editable,
                        marker: MarkerTree::TRUE,
                        extra: None,
                        group: None,
                    }))
                }
                RequirementSource::Url { .. } => {
                    Err(SourceError::WorkspacePackageUrl(name.to_string()))
                }
                RequirementSource::GitDirectory { .. } => {
                    Err(SourceError::WorkspacePackageGit(name.to_string()))
                }
                RequirementSource::GitPath { .. } => {
                    Err(SourceError::WorkspacePackageGit(name.to_string()))
                }
                RequirementSource::Path { .. } => {
                    Err(SourceError::WorkspacePackageFile(name.to_string()))
                }
            };
        }

        let source = match source {
            RequirementSource::Registry { index: Some(_), .. } => {
                return Ok(None);
            }
            RequirementSource::Registry { index: None, .. } if let Some(index) = index => {
                Self::Registry {
                    index,
                    marker: MarkerTree::TRUE,
                    extra: None,
                    group: None,
                }
            }
            RequirementSource::Registry { index: None, .. } => return Ok(None),
            RequirementSource::Path {
                install_path, url, ..
            } => Self::Path {
                editable: None,
                package: None,
                path: PortablePathBuf::from(
                    try_relative_to_if(&install_path, root, !url.was_given_absolute())
                        .map_err(SourceError::Absolute)?
                        .into_boxed_path(),
                ),
                marker: MarkerTree::TRUE,
                extra: None,
                group: None,
            },
            RequirementSource::Directory {
                install_path,
                editable: is_editable,
                url,
                ..
            } => Self::Path {
                editable: editable.or(is_editable),
                package: None,
                path: PortablePathBuf::from(
                    try_relative_to_if(&install_path, root, !url.was_given_absolute())
                        .map_err(SourceError::Absolute)?
                        .into_boxed_path(),
                ),
                marker: MarkerTree::TRUE,
                extra: None,
                group: None,
            },
            RequirementSource::Url {
                location,
                subdirectory,
                ..
            } => Self::Url {
                url: location,
                subdirectory: subdirectory.map(PortablePathBuf::from),
                marker: MarkerTree::TRUE,
                extra: None,
                group: None,
            },
            RequirementSource::GitDirectory {
                git, subdirectory, ..
            } => {
                if rev.is_none() && tag.is_none() && branch.is_none() {
                    let rev = match git.reference() {
                        GitReference::Branch(rev) => Some(rev),
                        GitReference::Tag(rev) => Some(rev),
                        GitReference::BranchOrTag(rev) => Some(rev),
                        GitReference::BranchOrTagOrCommit(rev) => Some(rev),
                        GitReference::NamedRef(rev) => Some(rev),
                        GitReference::DefaultBranch => None,
                    };
                    Self::Git {
                        rev: rev.cloned(),
                        tag,
                        branch,
                        lfs: lfs.into(),
                        git: git.url().clone(),
                        subdirectory: subdirectory.map(PortablePathBuf::from),
                        path: None,
                        marker: MarkerTree::TRUE,
                        extra: None,
                        group: None,
                    }
                } else {
                    Self::Git {
                        rev,
                        tag,
                        branch,
                        lfs: lfs.into(),
                        git: git.url().clone(),
                        subdirectory: subdirectory.map(PortablePathBuf::from),
                        path: None,
                        marker: MarkerTree::TRUE,
                        extra: None,
                        group: None,
                    }
                }
            }
            RequirementSource::GitPath {
                git, install_path, ..
            } => {
                if rev.is_none() && tag.is_none() && branch.is_none() {
                    let rev = match git.reference() {
                        GitReference::Branch(rev) => Some(rev),
                        GitReference::Tag(rev) => Some(rev),
                        GitReference::BranchOrTag(rev) => Some(rev),
                        GitReference::BranchOrTagOrCommit(rev) => Some(rev),
                        GitReference::NamedRef(rev) => Some(rev),
                        GitReference::DefaultBranch => None,
                    };
                    Self::Git {
                        rev: rev.cloned(),
                        tag,
                        branch,
                        lfs: lfs.into(),
                        git: git.url().clone(),
                        subdirectory: None,
                        path: Some(PortablePathBuf::from(install_path.as_path())),
                        marker: MarkerTree::TRUE,
                        extra: None,
                        group: None,
                    }
                } else {
                    Self::Git {
                        rev,
                        tag,
                        branch,
                        lfs: lfs.into(),
                        git: git.url().clone(),
                        subdirectory: None,
                        path: Some(PortablePathBuf::from(install_path.as_path())),
                        marker: MarkerTree::TRUE,
                        extra: None,
                        group: None,
                    }
                }
            }
        };

        Ok(Some(source))
    }

    /// Return the [`MarkerTree`] for the source.
    pub fn marker(&self) -> MarkerTree {
        match self {
            Self::Git { marker, .. } => *marker,
            Self::Url { marker, .. } => *marker,
            Self::Path { marker, .. } => *marker,
            Self::Registry { marker, .. } => *marker,
            Self::Workspace { marker, .. } => *marker,
        }
    }

    /// Return the extra name for the source.
    pub fn extra(&self) -> Option<&ExtraName> {
        match self {
            Self::Git { extra, .. } => extra.as_ref(),
            Self::Url { extra, .. } => extra.as_ref(),
            Self::Path { extra, .. } => extra.as_ref(),
            Self::Registry { extra, .. } => extra.as_ref(),
            Self::Workspace { extra, .. } => extra.as_ref(),
        }
    }

    /// Return the dependency group name for the source.
    pub fn group(&self) -> Option<&GroupName> {
        match self {
            Self::Git { group, .. } => group.as_ref(),
            Self::Url { group, .. } => group.as_ref(),
            Self::Path { group, .. } => group.as_ref(),
            Self::Registry { group, .. } => group.as_ref(),
            Self::Workspace { group, .. } => group.as_ref(),
        }
    }
}

/// The type of a dependency in a `pyproject.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyType {
    /// A dependency in `project.dependencies`.
    Production,
    /// A dependency in `tool.uv.dev-dependencies`.
    Dev,
    /// A dependency in `project.optional-dependencies.{0}`.
    Optional(ExtraName),
    /// A dependency in `dependency-groups.{0}`.
    Group(GroupName),
}

impl DependencyType {
    /// Return the TOML table name or names for this dependency type.
    pub fn toml_table_name(&self) -> Cow<'_, str> {
        match self {
            Self::Production => Cow::Borrowed("`project.dependencies`"),
            Self::Dev => {
                Cow::Borrowed("`tool.uv.dev-dependencies` or `tool.uv.dependency-groups.dev`")
            }
            Self::Optional(extra) => Cow::Owned(format!("`project.optional-dependencies.{extra}`")),
            Self::Group(group) => Cow::Owned(format!("`dependency-groups.{group}`")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(Serialize))]
pub(crate) struct BuildBackendSettingsSchema;

impl<'de> Deserialize<'de> for BuildBackendSettingsSchema {
    fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self)
    }
}

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for BuildBackendSettingsSchema {
    fn schema_name() -> Cow<'static, str> {
        BuildBackendSettings::schema_name()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        BuildBackendSettings::json_schema(generator)
    }
}

impl OptionsMetadata for BuildBackendSettingsSchema {
    fn record(visit: &mut dyn Visit) {
        BuildBackendSettings::record(visit);
    }

    fn documentation() -> Option<&'static str> {
        BuildBackendSettings::documentation()
    }

    fn metadata() -> OptionSet
    where
        Self: Sized + 'static,
    {
        BuildBackendSettings::metadata()
    }
}
