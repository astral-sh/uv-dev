use std::fmt::{self, Display, Formatter};
use std::path::PathBuf;

use serde::Deserialize;
use toml_edit::{Array, Item, Table, Value, value};

use uv_configuration::ExcludeDependency;
use uv_distribution_types::{
    IndexLocations, IndexMetadata, NameRequirementSpecification, Requirement, RequirementSource,
    SourceIndexError,
};
use uv_fs::{PortablePath, Simplified};
use uv_pypi_types::VerbatimParsedUrl;
use uv_python_types::PythonRequest;
use uv_settings::{ResolverInstallerOptions, ToolOptions, ToolOptionsWire};

/// The requirements and validated options for a new or reconfigured tool receipt.
#[derive(Debug, Clone)]
pub struct ToolConfiguration {
    /// The tool target followed by any `--with` requirements.
    requirements: Vec<Requirement>,
    constraints: Vec<Requirement>,
    overrides: Vec<Requirement>,
    excludes: Vec<ExcludeDependency>,
    build_constraints: Vec<NameRequirementSpecification>,
    options: ToolOptions,
}

impl ToolConfiguration {
    /// Promote credentials and validate the policies for every persisted index pin.
    pub fn new(
        requirements: Vec<Requirement>,
        constraints: Vec<Requirement>,
        overrides: Vec<Requirement>,
        excludes: Vec<ExcludeDependency>,
        build_constraints: Vec<NameRequirementSpecification>,
        options: ResolverInstallerOptions,
    ) -> Result<Self, SourceIndexError> {
        let configuration = Self {
            requirements,
            constraints,
            overrides,
            excludes,
            build_constraints,
            options: ToolOptions::from(options),
        };
        let indexes = IndexLocations::from(
            ResolverInstallerOptions::from(configuration.options.clone()).indexes,
        );
        configuration.validate_pinned_indexes(&indexes, &indexes)?;
        Ok(configuration)
    }

    /// The validated options stored in this configuration.
    pub fn options(&self) -> &ToolOptions {
        &self.options
    }

    fn pinned_indexes(&self) -> impl Iterator<Item = &IndexMetadata> {
        self.requirements
            .iter()
            .chain(&self.constraints)
            .chain(&self.overrides)
            .chain(
                self.build_constraints
                    .iter()
                    .map(|constraint| &constraint.requirement),
            )
            .filter_map(|requirement| match &requirement.source {
                RequirementSource::Registry { index, .. } => index.as_ref(),
                RequirementSource::Url { .. }
                | RequirementSource::GitDirectory { .. }
                | RequirementSource::GitPath { .. }
                | RequirementSource::Path { .. }
                | RequirementSource::Directory { .. } => None,
            })
    }

    fn validate_pinned_indexes(
        &self,
        locations: &IndexLocations,
        original: &IndexLocations,
    ) -> Result<(), SourceIndexError> {
        locations.validate_pinned_indexes(original, self.pinned_indexes())
    }
}

/// A tool entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "ToolWire", into = "ToolWire")]
pub struct Tool {
    configuration: ToolConfiguration,
    /// The Python requested by the user during installation.
    python: Option<PythonRequest>,
    /// A mapping of entry point names to their metadata.
    entrypoints: Vec<ToolEntrypoint>,
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
        let configuration = tool.configuration;
        Self {
            requirements: configuration
                .requirements
                .into_iter()
                .map(RequirementWire::Requirement)
                .collect(),
            constraints: configuration.constraints,
            overrides: configuration.overrides,
            excludes: configuration.excludes,
            build_constraint_dependencies: configuration.build_constraints,
            python: tool.python,
            entrypoints: tool.entrypoints,
            options: configuration.options.into(),
        }
    }
}

impl TryFrom<ToolWire> for Tool {
    type Error = serde::de::value::Error;

    fn try_from(tool: ToolWire) -> Result<Self, Self::Error> {
        // Reading a receipt does not promote credentials or reject its stored policies. Upgrade
        // needs that configuration to diagnose policy changes before modifying an environment.
        Ok(Self {
            configuration: ToolConfiguration {
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
                options: tool.options.into(),
            },
            python: tool.python,
            entrypoints: tool.entrypoints,
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

impl Tool {
    /// Create a new [`Tool`] from a validated receipt configuration.
    pub fn new(
        configuration: ToolConfiguration,
        python: Option<PythonRequest>,
        entrypoints: impl IntoIterator<Item = ToolEntrypoint>,
    ) -> Self {
        let mut entrypoints: Vec<_> = entrypoints.into_iter().collect();
        entrypoints.sort();
        Self {
            configuration,
            python,
            entrypoints,
        }
    }

    /// Create a new [`Tool`] with a validated receipt configuration.
    #[must_use]
    pub fn with_configuration(&self, configuration: ToolConfiguration) -> Self {
        Self {
            configuration,
            python: self.python.clone(),
            entrypoints: self.entrypoints.clone(),
        }
    }

    /// Validate every stored index pin against a new runtime configuration.
    pub fn validate_pinned_indexes(
        &self,
        locations: &IndexLocations,
        original: &IndexLocations,
    ) -> Result<(), SourceIndexError> {
        self.configuration
            .validate_pinned_indexes(locations, original)
    }

    /// Returns the TOML table for this tool.
    pub(crate) fn to_toml(&self) -> Result<Table, toml_edit::ser::Error> {
        let mut table = Table::new();

        if !self.configuration.requirements.is_empty() {
            table.insert("requirements", {
                let requirements = self
                    .configuration
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

        if !self.configuration.constraints.is_empty() {
            table.insert("constraints", {
                let constraints = self
                    .configuration
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

        if !self.configuration.overrides.is_empty() {
            table.insert("overrides", {
                let overrides = self
                    .configuration
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

        if !self.configuration.excludes.is_empty() {
            table.insert("excludes", {
                let excludes = self
                    .configuration
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

        if !self.configuration.build_constraints.is_empty() {
            table.insert("build-constraint-dependencies", {
                let build_constraints = self
                    .configuration
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

        if self.configuration.options != ToolOptions::default() {
            let serialized = serde::Serialize::serialize(
                &ToolOptionsWire::from(self.configuration.options.clone()),
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
        &self.configuration.requirements
    }

    pub fn constraints(&self) -> &[Requirement] {
        &self.configuration.constraints
    }

    pub fn overrides(&self) -> &[Requirement] {
        &self.configuration.overrides
    }

    pub fn excludes(&self) -> &[ExcludeDependency] {
        &self.configuration.excludes
    }

    pub fn build_constraints(&self) -> &[NameRequirementSpecification] {
        &self.configuration.build_constraints
    }

    pub fn python(&self) -> &Option<PythonRequest> {
        &self.python
    }

    pub fn options(&self) -> &ToolOptions {
        &self.configuration.options
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

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::str::FromStr;

    use uv_distribution_types::{
        Index, NameRequirementSpecification, Requirement, SourceIndexError,
    };
    use uv_settings::ResolverInstallerOptions;

    use super::{Tool, ToolConfiguration, ToolEntrypoint};
    use crate::ToolReceipt;

    type TestResult = Result<(), Box<dyn Error>>;

    fn pinned_requirement(name: &str) -> Result<Requirement, toml::de::Error> {
        toml::from_str(&format!(
            "name = \"{name}\"\nindex = \"https://example.com/simple/\""
        ))
    }

    fn alias_options(consistent: bool) -> Result<ResolverInstallerOptions, Box<dyn Error>> {
        let mut options = ResolverInstallerOptions::default();
        options.indexes.index = Some(vec![
            Index::from_str(
                "receipt-owner-private=https://fixture-user:fixture-secret@example.com/simple/",
            )?,
            Index::from_str(if consistent {
                "receipt-owner-mirror=https://alias-user:alias-secret@example.com/simple/"
            } else {
                "receipt-owner-mirror=https://example.com/simple/"
            })?,
        ]);
        Ok(options)
    }

    fn assert_auth_conflict(result: Result<ToolConfiguration, SourceIndexError>) {
        let error = result.expect_err("promoted aliases must agree for the persisted pin");
        assert!(matches!(
            &error,
            SourceIndexError::ConflictingAuthentication { .. }
        ));
        let diagnostic = error.to_string();
        assert!(!diagnostic.contains("fixture-user"));
        assert!(!diagnostic.contains("fixture-secret"));
    }

    #[test]
    fn configuration_checks_requirement_pins_after_promotion() -> TestResult {
        assert_auth_conflict(ToolConfiguration::new(
            vec![pinned_requirement("demo")?],
            vec![],
            vec![],
            vec![],
            vec![],
            alias_options(false)?,
        ));
        Ok(())
    }

    #[test]
    fn configuration_checks_constraint_pins_after_promotion() -> TestResult {
        assert_auth_conflict(ToolConfiguration::new(
            vec![],
            vec![pinned_requirement("demo")?],
            vec![],
            vec![],
            vec![],
            alias_options(false)?,
        ));
        Ok(())
    }

    #[test]
    fn configuration_checks_override_pins_after_promotion() -> TestResult {
        assert_auth_conflict(ToolConfiguration::new(
            vec![],
            vec![],
            vec![pinned_requirement("demo")?],
            vec![],
            vec![],
            alias_options(false)?,
        ));
        Ok(())
    }

    #[test]
    fn configuration_checks_build_constraint_pins_after_promotion() -> TestResult {
        assert_auth_conflict(ToolConfiguration::new(
            vec![],
            vec![],
            vec![],
            vec![],
            vec![NameRequirementSpecification {
                requirement: pinned_requirement("backend")?,
                hashes: vec![],
            }],
            alias_options(false)?,
        ));
        Ok(())
    }

    #[test]
    fn promoted_configuration_serializes_owned_collections() -> TestResult {
        let configuration = ToolConfiguration::new(
            vec![pinned_requirement("demo")?],
            vec![pinned_requirement("constraint")?],
            vec![pinned_requirement("override")?],
            vec![uv_configuration::ExcludeDependency::Dependency(
                "blocked-package".parse()?,
            )],
            vec![NameRequirementSpecification {
                requirement: pinned_requirement("backend")?,
                hashes: vec![format!("sha256:{}", "0".repeat(64))],
            }],
            alias_options(true)?,
        )?;
        let tool = Tool::new(
            configuration,
            None,
            [
                ToolEntrypoint::new("z-demo", "bin/z-demo".into(), "demo".to_owned()),
                ToolEntrypoint::new("a-demo", "bin/a-demo".into(), "demo".to_owned()),
            ],
        );
        let serialized = ToolReceipt::from(tool).to_toml()?;
        let actual: toml::Value = toml::from_str(&serialized)?;
        let expected: toml::Value = toml::from_str(
            r#"
            [tool]
            requirements = [{ name = "demo", index = "https://example.com/simple/" }]
            constraints = [{ name = "constraint", index = "https://example.com/simple/" }]
            overrides = [{ name = "override", index = "https://example.com/simple/" }]
            excludes = ["blocked-package"]
            build-constraint-dependencies = [{ name = "backend", index = "https://example.com/simple/", hashes = ["sha256:0000000000000000000000000000000000000000000000000000000000000000"] }]
            entrypoints = [
                { name = "a-demo", install-path = "bin/a-demo", from = "demo" },
                { name = "z-demo", install-path = "bin/z-demo", from = "demo" },
            ]
            [tool.options]
            index = [
                { name = "receipt-owner-private", url = "https://example.com/simple/", explicit = false, default = false, format = "simple", authenticate = "always" },
                { name = "receipt-owner-mirror", url = "https://example.com/simple/", explicit = false, default = false, format = "simple", authenticate = "always" },
            ]
            "#,
        )?;
        assert_eq!(actual, expected);
        let restored = ToolReceipt::from_string(serialized.clone())?;
        assert_eq!(restored.to_toml()?, serialized);
        Ok(())
    }

    #[test]
    fn legacy_read_defers_promotion_until_reconfiguration() -> TestResult {
        let legacy: Tool = toml::from_str(
            r#"
            requirements = ["demo==1.0"]
            overrides = [{ name = "demo", index = "https://example.com/simple/" }]
            python = "3.12"
            entrypoints = [{ name = "demo", install-path = "bin/demo", from = "demo" }]
            [options]
            index = [
                { name = "receipt-owner-private", url = "https://fixture-user:fixture-secret@example.com/simple/", authenticate = "auto" },
                { name = "receipt-owner-mirror", url = "https://example.com/simple/", authenticate = "auto" },
            ]
            "#,
        )?;
        let original_options = legacy.options().clone();
        let options = ResolverInstallerOptions::from(original_options.clone());
        let indexes = options.indexes.index.as_ref().ok_or("missing indexes")?;
        assert_eq!(indexes[0].authenticate.to_string(), "auto");
        assert_eq!(indexes[1].authenticate.to_string(), "auto");
        assert_auth_conflict(ToolConfiguration::new(
            legacy.requirements().to_vec(),
            legacy.constraints().to_vec(),
            legacy.overrides().to_vec(),
            legacy.excludes().to_vec(),
            legacy.build_constraints().to_vec(),
            options,
        ));
        let configuration = ToolConfiguration::new(
            legacy.requirements().to_vec(),
            legacy.constraints().to_vec(),
            legacy.overrides().to_vec(),
            legacy.excludes().to_vec(),
            legacy.build_constraints().to_vec(),
            alias_options(true)?,
        )?;
        let updated = legacy.with_configuration(configuration);
        assert_eq!(updated.requirements(), legacy.requirements());
        assert_eq!(updated.overrides(), legacy.overrides());
        assert_eq!(updated.python(), legacy.python());
        assert_eq!(updated.entrypoints(), legacy.entrypoints());
        assert_eq!(legacy.options(), &original_options);
        let serialized = ToolReceipt::from(updated).to_toml()?;
        let receipt: toml::Value = toml::from_str(&serialized)?;
        assert_eq!(
            receipt["tool"]["options"]["index"][0]["authenticate"].as_str(),
            Some("always")
        );
        assert_eq!(
            receipt["tool"]["options"]["index"][1]["authenticate"].as_str(),
            Some("always")
        );
        assert!(!serialized.contains("fixture-secret"));
        assert!(!serialized.contains("alias-secret"));
        Ok(())
    }

    #[test]
    fn legacy_conflicting_policies_remain_readable() -> TestResult {
        let legacy: Tool = toml::from_str(
            r#"
            requirements = [{ name = "demo", index = "https://example.com/simple/" }]
            entrypoints = []
            [options]
            index = [
                { name = "receipt-owner-private", url = "https://example.com/simple/", authenticate = "always" },
                { name = "receipt-owner-mirror", url = "https://example.com/simple/", authenticate = "auto" },
            ]
            "#,
        )?;
        let options = ResolverInstallerOptions::from(legacy.options().clone());
        let indexes = uv_distribution_types::IndexLocations::from(options.indexes.clone());
        assert!(matches!(
            legacy.validate_pinned_indexes(&indexes, &indexes),
            Err(SourceIndexError::ConflictingAuthentication { .. })
        ));
        assert_auth_conflict(ToolConfiguration::new(
            legacy.requirements().to_vec(),
            legacy.constraints().to_vec(),
            legacy.overrides().to_vec(),
            legacy.excludes().to_vec(),
            legacy.build_constraints().to_vec(),
            options,
        ));
        Ok(())
    }
}
