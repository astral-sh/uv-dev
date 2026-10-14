use std::fmt::{self, Display, Formatter};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use toml_edit::{Array, Item, Table, Value, value};

use uv_configuration::ExcludeDependency;
use uv_distribution_types::{NameRequirementSpecification, Requirement};
use uv_fs::{PortablePath, Simplified};
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
    overrides: Vec<Requirement>,
    /// The excludes requested by the user during installation.
    excludes: Vec<ExcludeDependency>,
    /// The build constraints requested by the user during installation.
    build_constraints: Vec<NameRequirementSpecification>,
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
    overrides: Vec<Requirement>,
    #[serde(default)]
    excludes: Vec<ExcludeDependency>,
    #[serde(default)]
    build_constraint_dependencies: Vec<NameRequirementSpecification>,
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

/// Insert a nonempty dependency list, keeping a single entry on one line.
fn insert_dependency_array(
    table: &mut Table,
    key: &str,
    dependencies: &[impl Serialize],
) -> Result<(), toml_edit::ser::Error> {
    if dependencies.is_empty() {
        return Ok(());
    }

    let elements = dependencies
        .iter()
        .map(|dependency| dependency.serialize(toml_edit::ser::ValueSerializer::new()))
        .collect::<Result<Vec<_>, _>>()?;
    let array = if elements.len() == 1 {
        Array::from_iter(elements)
    } else {
        each_element_on_its_line_array(elements.into_iter())
    };
    table.insert(key, value(array));
    Ok(())
}

impl Tool {
    /// Create a new `Tool`.
    pub fn new(
        requirements: Vec<Requirement>,
        constraints: Vec<Requirement>,
        overrides: Vec<Requirement>,
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

    /// Returns the TOML table for this tool.
    pub(crate) fn to_toml(&self) -> Result<Table, toml_edit::ser::Error> {
        let mut table = Table::new();

        insert_dependency_array(&mut table, "requirements", &self.requirements)?;
        insert_dependency_array(&mut table, "constraints", &self.constraints)?;
        insert_dependency_array(&mut table, "overrides", &self.overrides)?;
        insert_dependency_array(&mut table, "excludes", &self.excludes)?;
        insert_dependency_array(
            &mut table,
            "build-constraint-dependencies",
            &self.build_constraints,
        )?;

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

    pub fn overrides(&self) -> &[Requirement] {
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
