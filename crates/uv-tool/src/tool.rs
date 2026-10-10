use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use toml_edit::{Array, Item, Table, Value, value};

use uv_configuration::{ExcludeDependency, Override};
use uv_distribution_types::{
    ExtraBuildRequires, GitDirectorySourceUrl, IndexUrl, NameRequirementSpecification, Requirement,
    RequirementSource,
};
use uv_fs::{PortablePath, Simplified};
use uv_git_types::GitUrl;
use uv_pep508::VerbatimUrl;
use uv_pypi_types::VerbatimParsedUrl;
use uv_python_types::PythonRequest;
use uv_settings::{ToolOptions, ToolOptionsWire};

/// A tool entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "ToolWire", into = "ToolWire")]
pub struct Tool {
    /// The requirements requested by the user during installation.
    ///
    /// The first requirement is the tool target itself; any remaining requirements come from
    /// `--with`.
    requirements: Vec<Requirement>,
    /// The constraints requested by the user during installation.
    constraints: Vec<Requirement>,
    /// The overrides requested by the user during installation.
    overrides: Vec<Override<Requirement>>,
    /// The excludes requested by the user during installation.
    excludes: Vec<ExcludeDependency>,
    /// The build constraints requested by the user during installation.
    build_constraints: Vec<NameRequirementSpecification>,
    /// Build sources resolved from the source project during installation.
    extra_build_requires: ExtraBuildRequires,
    /// Source trees for repository-local indexes persisted in the tool options.
    index_sources: Vec<ToolIndexSource>,
    /// The Python requested by the user during installation.
    python: Option<PythonRequest>,
    /// A mapping of entry point names to their metadata.
    entrypoints: Vec<ToolEntrypoint>,
    /// The [`ToolOptions`] used to install this tool.
    options: ToolOptions,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct ToolWire {
    #[serde(default)]
    requirements: Vec<RequirementWire>,
    #[serde(default)]
    constraints: Vec<Requirement>,
    #[serde(default)]
    overrides: Vec<Override<Requirement>>,
    #[serde(default)]
    excludes: Vec<ExcludeDependency>,
    #[serde(default)]
    build_constraint_dependencies: Vec<NameRequirementSpecification>,
    #[serde(default)]
    extra_build_requires: ExtraBuildRequires,
    #[serde(default)]
    index_sources: Vec<ToolIndexSource>,
    python: Option<PythonRequest>,
    entrypoints: Vec<ToolEntrypoint>,
    #[serde(default)]
    options: ToolOptionsWire,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum RequirementWire {
    /// A [`Requirement`] following our uv-specific schema.
    Requirement(Requirement),
    /// A PEP 508-compatible requirement. We no longer write these, but there might be receipts out
    /// there that still use them.
    Deprecated(uv_pep508::Requirement<VerbatimParsedUrl>),
}

impl From<Tool> for ToolWire {
    fn from(tool: Tool) -> Self {
        Self {
            requirements: tool
                .requirements
                .into_iter()
                .map(RequirementWire::Requirement)
                .collect(),
            constraints: tool.constraints,
            overrides: tool.overrides,
            excludes: tool.excludes,
            build_constraint_dependencies: tool.build_constraints,
            extra_build_requires: tool.extra_build_requires,
            index_sources: tool.index_sources,
            python: tool.python,
            entrypoints: tool.entrypoints,
            options: tool.options.into(),
        }
    }
}

impl TryFrom<ToolWire> for Tool {
    type Error = serde::de::value::Error;

    fn try_from(tool: ToolWire) -> Result<Self, Self::Error> {
        Ok(Self {
            requirements: tool
                .requirements
                .into_iter()
                .map(|req| match req {
                    RequirementWire::Requirement(requirements) => requirements,
                    RequirementWire::Deprecated(requirement) => Requirement::from(requirement),
                })
                .collect(),
            constraints: tool.constraints,
            overrides: tool.overrides,
            excludes: tool.excludes,
            build_constraints: tool.build_constraint_dependencies,
            extra_build_requires: tool.extra_build_requires,
            index_sources: tool.index_sources,
            python: tool.python,
            entrypoints: tool.entrypoints,
            options: tool.options.into(),
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ToolEntrypoint {
    pub name: String,
    pub install_path: PathBuf,
    pub from: Option<String>,
}

impl Display for ToolEntrypoint {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        cfg_select! {
            windows => {
                write!(
                    f,
                    "{} ({})",
                    self.name,
                    self.install_path
                        .simplified_display()
                        .to_string()
                        .replace('/', "\\")
                )
            },
            unix => {
                write!(
                    f,
                    "{} ({})",
                    self.name,
                    self.install_path.simplified_display()
                )
            },
        }
    }
}

/// The durable source of an index whose on-disk location belongs to a Git checkout.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "ToolIndexSourceWire", into = "ToolIndexSourceWire")]
pub struct ToolIndexSource {
    index: IndexUrl,
    git: GitUrl,
    subdirectory: Option<Box<Path>>,
    url: VerbatimUrl,
}

#[derive(Serialize, Deserialize)]
struct ToolIndexSourceWire {
    index: IndexUrl,
    source: RequirementSource,
}

impl ToolIndexSource {
    pub fn new(
        index: IndexUrl,
        source: RequirementSource,
    ) -> Result<Self, serde::de::value::Error> {
        let RequirementSource::GitDirectory {
            git,
            subdirectory,
            url,
        } = source
        else {
            return Err(serde::de::Error::custom(
                "tool index provenance must be a Git directory",
            ));
        };
        Ok(Self {
            index,
            git,
            subdirectory,
            url,
        })
    }

    pub fn index(&self) -> &IndexUrl {
        &self.index
    }

    pub fn source(&self) -> GitDirectorySourceUrl<'_> {
        GitDirectorySourceUrl {
            git: &self.git,
            subdirectory: self.subdirectory.as_deref(),
            url: &self.url,
        }
    }

    #[must_use]
    pub fn with_index(self, index: IndexUrl) -> Self {
        Self { index, ..self }
    }
}

impl TryFrom<ToolIndexSourceWire> for ToolIndexSource {
    type Error = serde::de::value::Error;

    fn try_from(source: ToolIndexSourceWire) -> Result<Self, Self::Error> {
        Self::new(source.index, source.source)
    }
}

impl From<ToolIndexSource> for ToolIndexSourceWire {
    fn from(source: ToolIndexSource) -> Self {
        Self {
            index: source.index,
            source: RequirementSource::GitDirectory {
                git: source.git,
                subdirectory: source.subdirectory,
                url: source.url,
            },
        }
    }
}

/// Format an array so that each element is on its own line and has a trailing comma.
///
/// Example:
///
/// ```toml
/// requirements = [
///     "foo",
///     "bar",
/// ]
/// ```
fn each_element_on_its_line_array(elements: impl Iterator<Item = impl Into<Value>>) -> Array {
    let mut array = elements
        .map(Into::into)
        .map(|mut value| {
            // Each dependency is on its own line and indented.
            value.decor_mut().set_prefix("\n    ");
            value
        })
        .collect::<Array>();
    // With a trailing comma, inserting another entry doesn't change the preceding line,
    // reducing the diff noise.
    array.set_trailing_comma(true);
    // The line break between the last element's comma and the closing square bracket.
    array.set_trailing("\n");
    array
}

impl Tool {
    /// Create a new `Tool`.
    pub fn new(
        requirements: Vec<Requirement>,
        constraints: Vec<Requirement>,
        overrides: Vec<Override<Requirement>>,
        excludes: Vec<ExcludeDependency>,
        build_constraints: Vec<NameRequirementSpecification>,
        python: Option<PythonRequest>,
        entrypoints: impl IntoIterator<Item = ToolEntrypoint>,
        options: ToolOptions,
    ) -> Self {
        let mut entrypoints: Vec<_> = entrypoints.into_iter().collect();
        entrypoints.sort();
        Self {
            requirements,
            constraints,
            overrides,
            excludes,
            build_constraints,
            extra_build_requires: ExtraBuildRequires::default(),
            index_sources: Vec::new(),
            python,
            entrypoints,
            options,
        }
    }

    /// Create a new [`Tool`] with the given [`ToolOptions`].
    #[must_use]
    pub fn with_options(self, options: ToolOptions) -> Self {
        Self { options, ..self }
    }

    /// Retain source-project build dependencies for subsequent upgrades.
    #[must_use]
    pub fn with_extra_build_requires(self, extra_build_requires: ExtraBuildRequires) -> Self {
        Self {
            extra_build_requires,
            ..self
        }
    }

    /// Build requirements whose source mappings were resolved during installation.
    pub fn extra_build_requires(&self) -> &ExtraBuildRequires {
        &self.extra_build_requires
    }

    /// Retain index sources for subsequent upgrades.
    #[must_use]
    pub fn with_index_sources(self, index_sources: Vec<ToolIndexSource>) -> Self {
        Self {
            index_sources,
            ..self
        }
    }

    pub fn index_sources(&self) -> &[ToolIndexSource] {
        &self.index_sources
    }

    /// Update repository-local index bindings in every persisted requirement.
    pub fn replace_requirement_indexes(&mut self, replacements: &BTreeMap<IndexUrl, IndexUrl>) {
        let requirements = self
            .requirements
            .iter_mut()
            .chain(self.constraints.iter_mut())
            .chain(self.overrides.iter_mut().flat_map(|entry| match entry {
                Override::Requirement(requirement) => std::slice::from_mut(requirement),
                Override::Package(package) => package.dependencies.as_mut(),
            }))
            .chain(
                self.build_constraints
                    .iter_mut()
                    .map(|entry| &mut entry.requirement),
            )
            .chain(
                self.extra_build_requires
                    .values_mut()
                    .flatten()
                    .map(|entry| &mut entry.requirement),
            );
        for requirement in requirements {
            if let RequirementSource::Registry {
                index: Some(index), ..
            } = &mut requirement.source
                && let Some(url) = replacements.get(&index.url)
            {
                index.url = url.clone();
            }
        }
    }

    /// Returns the TOML table for this tool.
    pub(crate) fn to_toml(&self) -> Result<Table, toml_edit::ser::Error> {
        let mut table = Table::new();

        if !self.requirements.is_empty() {
            table.insert("requirements", {
                let requirements = self
                    .requirements
                    .iter()
                    .map(|requirement| {
                        serde::Serialize::serialize(
                            &requirement,
                            toml_edit::ser::ValueSerializer::new(),
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                let requirements = match requirements.as_slice() {
                    [] => Array::new(),
                    [requirement] => Array::from_iter([requirement]),
                    requirements => each_element_on_its_line_array(requirements.iter()),
                };
                value(requirements)
            });
        }

        if !self.constraints.is_empty() {
            table.insert("constraints", {
                let constraints = self
                    .constraints
                    .iter()
                    .map(|constraint| {
                        serde::Serialize::serialize(
                            &constraint,
                            toml_edit::ser::ValueSerializer::new(),
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                let constraints = match constraints.as_slice() {
                    [] => Array::new(),
                    [constraint] => Array::from_iter([constraint]),
                    constraints => each_element_on_its_line_array(constraints.iter()),
                };
                value(constraints)
            });
        }

        if !self.overrides.is_empty() {
            table.insert("overrides", {
                let overrides = self
                    .overrides
                    .iter()
                    .map(|r#override| {
                        serde::Serialize::serialize(
                            &r#override,
                            toml_edit::ser::ValueSerializer::new(),
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                let overrides = match overrides.as_slice() {
                    [] => Array::new(),
                    [r#override] => Array::from_iter([r#override]),
                    overrides => each_element_on_its_line_array(overrides.iter()),
                };
                value(overrides)
            });
        }

        if !self.excludes.is_empty() {
            table.insert("excludes", {
                let excludes = self
                    .excludes
                    .iter()
                    .map(|r#exclude| {
                        serde::Serialize::serialize(
                            &r#exclude,
                            toml_edit::ser::ValueSerializer::new(),
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                let excludes = match excludes.as_slice() {
                    [] => Array::new(),
                    [r#exclude] => Array::from_iter([r#exclude]),
                    excludes => each_element_on_its_line_array(excludes.iter()),
                };
                value(excludes)
            });
        }

        if !self.build_constraints.is_empty() {
            table.insert("build-constraint-dependencies", {
                let build_constraints = self
                    .build_constraints
                    .iter()
                    .map(|r#build_constraint| {
                        serde::Serialize::serialize(
                            &r#build_constraint,
                            toml_edit::ser::ValueSerializer::new(),
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                let build_constraints = match build_constraints.as_slice() {
                    [] => Array::new(),
                    [r#build_constraint] => Array::from_iter([r#build_constraint]),
                    build_constraints => each_element_on_its_line_array(build_constraints.iter()),
                };
                value(build_constraints)
            });
        }

        if !self.extra_build_requires.is_empty() {
            table.insert(
                "extra-build-requires",
                value(serde::Serialize::serialize(
                    &self.extra_build_requires,
                    toml_edit::ser::ValueSerializer::new(),
                )?),
            );
        }

        if !self.index_sources.is_empty() {
            table.insert(
                "index-sources",
                value(serde::Serialize::serialize(
                    &self.index_sources,
                    toml_edit::ser::ValueSerializer::new(),
                )?),
            );
        }

        if let Some(ref python) = self.python {
            table.insert(
                "python",
                value(serde::Serialize::serialize(
                    &python,
                    toml_edit::ser::ValueSerializer::new(),
                )?),
            );
        }

        table.insert("entrypoints", {
            let entrypoints = each_element_on_its_line_array(
                self.entrypoints
                    .iter()
                    .map(ToolEntrypoint::to_toml)
                    .map(Table::into_inline_table),
            );
            value(entrypoints)
        });

        if self.options != ToolOptions::default() {
            let serialized = serde::Serialize::serialize(
                &ToolOptionsWire::from(self.options.clone()),
                toml_edit::ser::ValueSerializer::new(),
            )?;
            let Value::InlineTable(serialized) = serialized else {
                return Err(toml_edit::ser::Error::Custom(
                    "Expected an inline table".to_string(),
                ));
            };
            table.insert("options", Item::Table(serialized.into_table()));
        }

        Ok(table)
    }

    pub fn entrypoints(&self) -> &[ToolEntrypoint] {
        &self.entrypoints
    }

    pub fn requirements(&self) -> &[Requirement] {
        &self.requirements
    }

    pub fn constraints(&self) -> &[Requirement] {
        &self.constraints
    }

    pub fn overrides(&self) -> &[Override<Requirement>] {
        &self.overrides
    }

    pub fn excludes(&self) -> &[ExcludeDependency] {
        &self.excludes
    }

    pub fn build_constraints(&self) -> &[NameRequirementSpecification] {
        &self.build_constraints
    }

    pub fn python(&self) -> &Option<PythonRequest> {
        &self.python
    }

    pub fn options(&self) -> &ToolOptions {
        &self.options
    }
}

impl ToolEntrypoint {
    /// Create a new [`ToolEntrypoint`].
    pub fn new(name: &str, install_path: PathBuf, from: String) -> Self {
        let name = name
            .trim_end_matches(std::env::consts::EXE_SUFFIX)
            .to_string();
        Self {
            name,
            install_path,
            from: Some(from),
        }
    }

    /// Returns the TOML table for this entrypoint.
    fn to_toml(&self) -> Table {
        let mut table = Table::new();
        table.insert("name", value(&self.name));
        table.insert(
            "install-path",
            // Use cross-platform slashes so the toml string type does not change
            value(PortablePath::from(&self.install_path).to_string()),
        );
        if let Some(from) = &self.from {
            table.insert("from", value(from));
        }
        table
    }
}
