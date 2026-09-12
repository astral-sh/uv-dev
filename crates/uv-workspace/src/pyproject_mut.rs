use std::fmt::{Display, Formatter};
use std::path::Path;
use std::str::FromStr;
use std::{fmt, iter, mem};

use itertools::Itertools;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use toml_edit::{
    Array, ArrayOfTables, DocumentMut, Formatted, Item, RawString, Table, TomlError, Value,
};

use uv_cache_key::CanonicalUrl;
use uv_distribution_types::{Index, IndexFormat, IndexUrl};
use uv_fs::{PortablePath, is_same_file_allow_missing, try_relative_to_if};
use uv_normalize::{ExtraName, GroupName, PackageName};
use uv_pep440::{Version, VersionParseError, VersionSpecifier, VersionSpecifiers};
use uv_pep508::{MarkerTree, Requirement, VersionOrUrl};

use crate::pyproject::{DependencyType, Source};

/// A mutable `pyproject.toml` document.
///
/// Preserve comments and document structure when commands such as `uv add` or `uv remove` edit
/// an existing `pyproject.toml`.
pub struct PyProjectTomlMut {
    doc: DocumentMut,
    target: DependencyTarget,
}

fn index_locations_equal(existing: &str, incoming: &IndexUrl, root_dir: &Path) -> bool {
    let Ok(existing) = IndexUrl::parse(existing, Some(root_dir)) else {
        return false;
    };

    if let (IndexUrl::Path(existing), IndexUrl::Path(incoming)) = (&existing, incoming)
        && let (Ok(existing), Ok(incoming)) = (existing.to_file_path(), incoming.to_file_path())
        && let Some(equal) = is_same_file_allow_missing(&existing, &incoming)
    {
        return equal;
    }

    CanonicalUrl::new(existing.url().clone()) == CanonicalUrl::new(incoming.url().clone())
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("Failed to parse `pyproject.toml`")]
    Parse(#[from] Box<TomlError>),
    #[error("Failed to serialize `pyproject.toml`")]
    Serialize(#[from] Box<toml::ser::Error>),
    #[error("Failed to deserialize `pyproject.toml`")]
    Deserialize(#[from] Box<toml::de::Error>),
    #[error("Dependencies in `pyproject.toml` are malformed")]
    MalformedDependencies,
    #[error("Sources in `pyproject.toml` are malformed")]
    MalformedSources,
    #[error("Workspace in `pyproject.toml` is malformed")]
    MalformedWorkspace,
    #[error("Expected a dependency at index {0}")]
    MissingDependency(usize),
    #[error("Failed to parse `version` field of `pyproject.toml`")]
    VersionParse(#[from] VersionParseError),
    #[error("Cannot perform ambiguous update; found multiple entries for `{}`:\n{}", package_name, requirements.iter().map(|requirement| format!("- `{requirement}`")).join("\n"))]
    Ambiguous {
        package_name: PackageName,
        requirements: Vec<Requirement>,
    },
    #[error("Unknown bound king {0}")]
    UnknownBoundKind(String),
}

/// The result of editing an array in a TOML document.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ArrayEdit {
    /// Updated an existing entry at the given index.
    Update(usize),
    /// Added an entry at the given index, usually at the end of the array.
    Add(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CommentType {
    /// A comment that appears on its own line.
    OwnLine,
    /// A comment that appears at the end of a line.
    EndOfLine { leading_whitespace: String },
}

#[derive(Debug, Clone)]
struct Comment {
    text: String,
    kind: CommentType,
}

/// The default version specifier when adding a dependency.
// PEP 440 allows any number of version components. The `major` and `minor` bounds assume
// versions usually use two or three components and follow semantic versioning conventions.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum AddBoundsKind {
    /// Set only a lower bound, such as `>=1.2.3`.
    #[default]
    Lower,
    /// Allow the same major version, such as `>=1.2.3, <2.0.0`.
    /// This is similar to a semantic-versioning caret.
    ///
    /// Skip leading zeroes, as in `>=0.1.2, <0.2.0`.
    Major,
    /// Allow the same minor version, such as `>=1.2.3, <1.3.0`.
    /// This is similar to a semantic-versioning tilde.
    ///
    /// Skip leading zeroes, as in `>=0.1.2, <0.1.3`.
    Minor,
    /// Pin the exact version, such as `==1.2.3`.
    ///
    /// Avoid this option because the uv lockfile already pins versions.
    Exact,
}

impl Display for AddBoundsKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lower => write!(f, "lower"),
            Self::Major => write!(f, "major"),
            Self::Minor => write!(f, "minor"),
            Self::Exact => write!(f, "exact"),
        }
    }
}

impl AddBoundsKind {
    fn specifiers(self, version: Version) -> VersionSpecifiers {
        // The major version is the most significant component. The minor version is the next
        // component. Common formats are `major.minor.patch` and `0.major.minor`.
        match self {
            Self::Lower => {
                VersionSpecifiers::from(VersionSpecifier::greater_than_equal_version(version))
            }
            Self::Major => {
                let leading_zeroes = version
                    .release()
                    .iter()
                    .take_while(|digit| **digit == 0)
                    .count();

                // Handle a version that contains only zeroes.
                if leading_zeroes == version.release().len() {
                    let upper_bound = Version::new(
                        [0, 1]
                            .into_iter()
                            .chain(iter::repeat_n(0, version.release().iter().skip(2).len())),
                    );
                    return VersionSpecifiers::from_iter([
                        VersionSpecifier::greater_than_equal_version(version),
                        VersionSpecifier::less_than_version(upper_bound),
                    ]);
                }

                // Increment the major version and preserve the number of components:
                // 1.2.3 -> 2.0.0
                // 1.2 -> 2.0
                // 1 -> 2
                // Skip leading zeroes to apply semantic versioning to `0.x` versions:
                // 0.1.2 -> 0.2.0
                // 0.0.1 -> 0.0.2
                let major = version.release().get(leading_zeroes).copied().unwrap_or(0);
                // Count the components after the incremented component.
                let trailing_zeros = version.release().iter().skip(leading_zeroes + 1).len();
                let upper_bound = Version::new(
                    iter::repeat_n(0, leading_zeroes)
                        .chain(iter::once(major + 1))
                        .chain(iter::repeat_n(0, trailing_zeros)),
                );

                VersionSpecifiers::from_iter([
                    VersionSpecifier::greater_than_equal_version(version),
                    VersionSpecifier::less_than_version(upper_bound),
                ])
            }
            Self::Minor => {
                let leading_zeroes = version
                    .release()
                    .iter()
                    .take_while(|digit| **digit == 0)
                    .count();

                // Handle a version that contains only zeroes.
                if leading_zeroes == version.release().len() {
                    let upper_bound = [0, 0, 1]
                        .into_iter()
                        .chain(iter::repeat_n(0, version.release().iter().skip(3).len()));
                    return VersionSpecifiers::from_iter([
                        VersionSpecifier::greater_than_equal_version(version),
                        VersionSpecifier::less_than_version(Version::new(upper_bound)),
                    ]);
                }

                // If the major and minor versions are zero, increment the next nonzero component.
                // This preserves the number of components, such as the three components in
                // `0.0.1`.
                if leading_zeroes >= 2 {
                    let most_significant =
                        version.release().get(leading_zeroes).copied().unwrap_or(0);
                    // Count the components after the incremented component.
                    let trailing_zeros = version.release().iter().skip(leading_zeroes + 1).len();
                    let upper_bound = Version::new(
                        iter::repeat_n(0, leading_zeroes)
                            .chain(iter::once(most_significant + 1))
                            .chain(iter::repeat_n(0, trailing_zeros)),
                    );
                    return VersionSpecifiers::from_iter([
                        VersionSpecifier::greater_than_equal_version(version),
                        VersionSpecifier::less_than_version(upper_bound),
                    ]);
                }

                // Increment the minor version and preserve the number of components when possible:
                // 1.2.3 -> 1.3.0
                // 1.2 -> 1.3
                // 1 -> 1.1
                // Skip leading zeroes to apply semantic versioning to `0.x` versions:
                // 0.1.2 -> 0.1.3
                // 0.0.1 -> 0.0.2

                // Pad single-component versions and versions with only leading zeroes.
                let major = version.release().get(leading_zeroes).copied().unwrap_or(0);
                let minor = version
                    .release()
                    .get(leading_zeroes + 1)
                    .copied()
                    .unwrap_or(0);
                let upper_bound = Version::new(
                    iter::repeat_n(0, leading_zeroes)
                        .chain(iter::once(major))
                        .chain(iter::once(minor + 1))
                        .chain(iter::repeat_n(
                            0,
                            version.release().iter().skip(leading_zeroes + 2).len(),
                        )),
                );

                VersionSpecifiers::from_iter([
                    VersionSpecifier::greater_than_equal_version(version),
                    VersionSpecifier::less_than_version(upper_bound),
                ])
            }
            Self::Exact => {
                VersionSpecifiers::from_iter([VersionSpecifier::equals_version(version)])
            }
        }
    }
}

/// The file type that receives new dependencies.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DependencyTarget {
    /// A PEP 723 script with inline metadata.
    Script,
    /// A project with a `pyproject.toml`.
    PyProjectToml,
}

impl PyProjectTomlMut {
    /// Create a [`PyProjectTomlMut`] from a [`str`].
    pub fn from_toml(raw: &str, target: DependencyTarget) -> Result<Self, Error> {
        Ok(Self {
            doc: raw.parse().map_err(Box::new)?,
            target,
        })
    }

    /// Add a project to the workspace.
    pub fn add_workspace(&mut self, path: impl AsRef<Path>) -> Result<(), Error> {
        // Get or create `tool.uv.workspace.members`.
        let members = self
            .doc
            .entry("tool")
            .or_insert(implicit())
            .as_table_mut()
            .ok_or(Error::MalformedWorkspace)?
            .entry("uv")
            .or_insert(implicit())
            .as_table_mut()
            .ok_or(Error::MalformedWorkspace)?
            .entry("workspace")
            .or_insert(Item::Table(Table::new()))
            .as_table_mut()
            .ok_or(Error::MalformedWorkspace)?
            .entry("members")
            .or_insert(Item::Value(Value::Array(Array::new())))
            .as_array_mut()
            .ok_or(Error::MalformedWorkspace)?;

        // Add the path to the workspace.
        members.push(PortablePath::from(path.as_ref()).to_string());

        reformat_array_multiline(members);

        Ok(())
    }

    /// Get the mutable `project` [`Table`]. Create the table if it does not exist.
    ///
    /// For a script, return the root table.
    fn project(&mut self) -> Result<&mut Table, Error> {
        let doc = match self.target {
            DependencyTarget::Script => self.doc.as_table_mut(),
            DependencyTarget::PyProjectToml => self
                .doc
                .entry("project")
                .or_insert(Item::Table(Table::new()))
                .as_table_mut()
                .ok_or(Error::MalformedDependencies)?,
        };
        Ok(doc)
    }

    /// Get the mutable `project` [`Table`], or return `None` if it does not exist.
    ///
    /// For a script, return the root table.
    fn project_mut(&mut self) -> Result<Option<&mut Table>, Error> {
        let doc = match self.target {
            DependencyTarget::Script => Some(self.doc.as_table_mut()),
            DependencyTarget::PyProjectToml => self
                .doc
                .get_mut("project")
                .map(|project| project.as_table_mut().ok_or(Error::MalformedSources))
                .transpose()?,
        };
        Ok(doc)
    }

    /// Add a dependency to `project.dependencies`.
    ///
    /// Return [`ArrayEdit::Add`] or [`ArrayEdit::Update`] for the affected dependency.
    pub fn add_dependency(
        &mut self,
        req: &Requirement,
        source: Option<&Source>,
        raw: bool,
    ) -> Result<ArrayEdit, Error> {
        // Get or create `project.dependencies`.
        let dependencies = self
            .project()?
            .entry("dependencies")
            .or_insert(Item::Value(Value::Array(Array::new())))
            .as_array_mut()
            .ok_or(Error::MalformedDependencies)?;

        let edit = add_dependency(req, dependencies, source.is_some(), raw)?;

        if let Some(source) = source {
            self.add_source(&req.name, source)?;
        }

        Ok(edit)
    }

    /// Replace every exact dependency declaration match without modifying its source.
    ///
    /// Return the position of each replaced dependency.
    pub fn replace_dependency_declaration(
        &mut self,
        dependency_type: &DependencyType,
        existing: &Requirement,
        replacement: &Requirement,
    ) -> Result<Vec<ArrayEdit>, Error> {
        let Some(dependencies) = self.dependency_type_array_mut(dependency_type)? else {
            return Ok(Vec::new());
        };

        let replacement = replacement.to_string();
        let mut edits = Vec::new();
        for (index, requirement) in
            find_dependencies(&existing.name, Some(&existing.marker), dependencies)
        {
            if same_requirement_declaration(&requirement, existing) {
                dependencies.replace(index, replacement.clone());
                edits.push(ArrayEdit::Update(index));
            }
        }
        Ok(edits)
    }

    /// Remove every exact string match for a dependency declaration without modifying its source.
    ///
    /// Return the position of each removed dependency.
    pub fn remove_dependency_declaration_text(
        &mut self,
        dependency_type: &DependencyType,
        existing: &str,
    ) -> Result<Vec<ArrayEdit>, Error> {
        let Some(dependencies) = self.dependency_type_array_mut(dependency_type)? else {
            return Ok(Vec::new());
        };

        let mut edits = Vec::new();
        for index in dependencies
            .iter()
            .enumerate()
            .filter_map(|(index, dependency)| {
                (dependency.as_str() == Some(existing)).then_some(index)
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            remove_dependency_at(index, dependencies);
            edits.push(ArrayEdit::Update(index));
        }
        if !edits.is_empty() {
            reformat_array_multiline(dependencies);
        }
        edits.reverse();
        Ok(edits)
    }

    /// Add a development dependency to `tool.uv.dev-dependencies`.
    ///
    /// Return [`ArrayEdit::Add`] or [`ArrayEdit::Update`] for the affected dependency.
    pub fn add_dev_dependency(
        &mut self,
        req: &Requirement,
        source: Option<&Source>,
        raw: bool,
    ) -> Result<ArrayEdit, Error> {
        // Get or create `tool.uv.dev-dependencies`.
        let dev_dependencies = self
            .doc
            .entry("tool")
            .or_insert(implicit())
            .as_table_mut()
            .ok_or(Error::MalformedSources)?
            .entry("uv")
            .or_insert(Item::Table(Table::new()))
            .as_table_mut()
            .ok_or(Error::MalformedSources)?
            .entry("dev-dependencies")
            .or_insert(Item::Value(Value::Array(Array::new())))
            .as_array_mut()
            .ok_or(Error::MalformedDependencies)?;

        let edit = add_dependency(req, dev_dependencies, source.is_some(), raw)?;

        if let Some(source) = source {
            self.add_source(&req.name, source)?;
        }

        Ok(edit)
    }

    /// Add an [`Index`] to `tool.uv.index`.
    pub fn add_index(&mut self, index: &Index, root_dir: &Path) -> Result<(), Error> {
        let size = self.doc.len();
        let existing = self
            .doc
            .entry("tool")
            .or_insert(implicit())
            .as_table_mut()
            .ok_or(Error::MalformedSources)?
            .entry("uv")
            .or_insert(implicit())
            .as_table_mut()
            .ok_or(Error::MalformedSources)?
            .entry("index")
            .or_insert(Item::ArrayOfTables(ArrayOfTables::new()))
            .as_array_of_tables_mut()
            .ok_or(Error::MalformedSources)?;

        // Update an existing index with the same name or URL and move it to the top.
        let mut table = existing
            .iter()
            .find(|table| {
                // If the index has the same name, reuse it.
                if let Some(index) = index.name.as_deref()
                    && table
                        .get("name")
                        .and_then(|name| name.as_str())
                        .is_some_and(|name| name == index)
                {
                    return true;
                }

                // Reuse an existing default index if the new index is also the default.
                if index.default
                    && table
                        .get("default")
                        .is_some_and(|default| default.as_bool() == Some(true))
                {
                    return true;
                }

                // Reuse an index with the same URL.
                if table
                    .get("url")
                    .and_then(|item| item.as_str())
                    .is_some_and(|url| index_locations_equal(url, &index.url, root_dir))
                {
                    return true;
                }

                false
            })
            .cloned()
            .unwrap_or_default();

        // Update the name if necessary.
        if let Some(index) = index.name.as_deref()
            && table
                .get("name")
                .and_then(|name| name.as_str())
                .is_none_or(|name| name != index)
        {
            let mut formatted = Formatted::new(index.to_string());
            if let Some(value) = table.get("name").and_then(Item::as_value) {
                if let Some(prefix) = value.decor().prefix() {
                    formatted.decor_mut().set_prefix(prefix.clone());
                }
                if let Some(suffix) = value.decor().suffix() {
                    formatted.decor_mut().set_suffix(suffix.clone());
                }
            }
            table.insert("name", Value::String(formatted).into());
        }

        let url = if let IndexUrl::Path(url) = &index.url
            && let Ok(path) = url.to_file_path()
            && let Ok(path) = try_relative_to_if(path, root_dir, !url.was_given_absolute())
        {
            PortablePath::from(&path).to_string()
        } else {
            index.url.without_credentials().to_string()
        };
        let existing_url = table.get("url").and_then(|item| item.as_str());

        // Update the stored URL independently of whether the index location changed.
        let url_needs_update = existing_url.is_none_or(|existing| existing != url);
        let index_location_changed = existing_url
            .is_none_or(|existing| !index_locations_equal(existing, &index.url, root_dir));

        // Update the URL if necessary.
        if url_needs_update {
            let mut formatted = Formatted::new(url);
            if let Some(value) = table.get("url").and_then(Item::as_value) {
                if let Some(prefix) = value.decor().prefix() {
                    formatted.decor_mut().set_prefix(prefix.clone());
                }
                if let Some(suffix) = value.decor().suffix() {
                    formatted.decor_mut().set_suffix(suffix.clone());
                }
            }
            table.insert("url", Value::String(formatted).into());
        }

        // Update the default setting if necessary.
        if index.default {
            if !table
                .get("default")
                .and_then(Item::as_bool)
                .is_some_and(|default| default)
            {
                let mut formatted = Formatted::new(true);
                if let Some(value) = table.get("default").and_then(Item::as_value) {
                    if let Some(prefix) = value.decor().prefix() {
                        formatted.decor_mut().set_prefix(prefix.clone());
                    }
                    if let Some(suffix) = value.decor().suffix() {
                        formatted.decor_mut().set_suffix(suffix.clone());
                    }
                }
                table.insert("default", Value::Boolean(formatted).into());
            }
        }

        // If the index location changed, match the new index format.
        if index_location_changed {
            match index.format {
                IndexFormat::Flat => {
                    if table
                        .get("format")
                        .and_then(Item::as_str)
                        .is_none_or(|format| format != "flat")
                    {
                        let mut formatted = Formatted::new("flat".to_string());
                        if let Some(value) = table.get("format").and_then(Item::as_value) {
                            if let Some(prefix) = value.decor().prefix() {
                                formatted.decor_mut().set_prefix(prefix.clone());
                            }
                            if let Some(suffix) = value.decor().suffix() {
                                formatted.decor_mut().set_suffix(suffix.clone());
                            }
                        }
                        table.insert("format", Value::String(formatted).into());
                    }
                }
                IndexFormat::Simple => {
                    // Remove the format key because `Simple` is the default.
                    table.remove("format");
                }
            }
        }

        // Remove any replaced tables.
        existing.retain(|table| {
            // If the index has the same name, skip it.
            if let Some(index) = index.name.as_deref()
                && table
                    .get("name")
                    .and_then(|name| name.as_str())
                    .is_some_and(|name| name == index)
            {
                return false;
            }

            // Skip another default index.
            if index.default
                && table
                    .get("default")
                    .is_some_and(|default| default.as_bool() == Some(true))
            {
                return false;
            }

            // Skip another index with the same URL.
            if table
                .get("url")
                .and_then(|item| item.as_str())
                .is_some_and(|url| index_locations_equal(url, &index.url, root_dir))
            {
                return false;
            }

            true
        });

        // Set the position to the minimum if the index is not already first.
        if let Some(min) = existing.iter().filter_map(Table::position).min() {
            table.set_position(Some(min));

            // Increment the position of all existing elements.
            for table in existing.iter_mut() {
                if let Some(position) = table.position() {
                    table.set_position(Some(position + 1));
                }
            }
        } else {
            let position = isize::try_from(size).expect("TOML table size fits in `isize`");
            table.set_position(Some(position));
        }

        // Push the item to the table.
        existing.push(table);

        Ok(())
    }

    /// Add a dependency to `project.optional-dependencies`.
    ///
    /// Return [`ArrayEdit::Add`] or [`ArrayEdit::Update`] for the affected dependency.
    pub fn add_optional_dependency(
        &mut self,
        group: &ExtraName,
        req: &Requirement,
        source: Option<&Source>,
        raw: bool,
    ) -> Result<ArrayEdit, Error> {
        // Get or create `project.optional-dependencies`.
        let optional_dependencies = self
            .project()?
            .entry("optional-dependencies")
            .or_insert(Item::Table(Table::new()))
            .as_table_like_mut()
            .ok_or(Error::MalformedDependencies)?;

        // Try to find the existing group.
        let existing_group = optional_dependencies.iter_mut().find_map(|(key, value)| {
            if ExtraName::from_str(key.get()).is_ok_and(|g| g == *group) {
                Some(value)
            } else {
                None
            }
        });

        // Create the group if it does not exist.
        let group = match existing_group {
            Some(value) => value,
            None => optional_dependencies
                .entry(group.as_ref())
                .or_insert(Item::Value(Value::Array(Array::new()))),
        }
        .as_array_mut()
        .ok_or(Error::MalformedDependencies)?;

        let added = add_dependency(req, group, source.is_some(), raw)?;

        // Reformat `project.optional-dependencies` if it is an inline table.
        // Inline tables do not permit comments between items, so reformatting cannot remove any.
        if let Some(optional_dependencies) = self
            .project()?
            .get_mut("optional-dependencies")
            .and_then(Item::as_inline_table_mut)
        {
            optional_dependencies.fmt();
        }

        if let Some(source) = source {
            self.add_source(&req.name, source)?;
        }

        Ok(added)
    }

    /// Ensure an optional dependency group exists. Create an empty group if necessary.
    pub fn ensure_optional_dependency(&mut self, extra: &ExtraName) -> Result<(), Error> {
        // Get or create `project.optional-dependencies`.
        let optional_dependencies = self
            .project()?
            .entry("optional-dependencies")
            .or_insert(Item::Table(Table::new()))
            .as_table_like_mut()
            .ok_or(Error::MalformedDependencies)?;

        // Check if the extra already exists.
        let extra_exists = optional_dependencies
            .iter()
            .any(|(key, _value)| ExtraName::from_str(key).is_ok_and(|e| e == *extra));

        // Create the extra if it does not exist.
        if !extra_exists {
            optional_dependencies.insert(extra.as_ref(), Item::Value(Value::Array(Array::new())));
        }

        // Reformat `project.optional-dependencies` if it is an inline table.
        // Inline tables do not permit comments between items, so reformatting cannot remove any.
        if let Some(optional_dependencies) = self
            .project()?
            .get_mut("optional-dependencies")
            .and_then(Item::as_inline_table_mut)
        {
            optional_dependencies.fmt();
        }

        Ok(())
    }

    /// Add a dependency to `dependency-groups`.
    ///
    /// Return [`ArrayEdit::Add`] or [`ArrayEdit::Update`] for the affected dependency.
    pub fn add_dependency_group_requirement(
        &mut self,
        group: &GroupName,
        req: &Requirement,
        source: Option<&Source>,
        raw: bool,
    ) -> Result<ArrayEdit, Error> {
        // Get or create `dependency-groups`.
        let dependency_groups = self
            .doc
            .entry("dependency-groups")
            .or_insert(Item::Table(Table::new()))
            .as_table_like_mut()
            .ok_or(Error::MalformedDependencies)?;

        let was_sorted = dependency_groups
            .get_values()
            .iter()
            .filter_map(|(dotted_ks, _)| dotted_ks.first())
            .map(|k| k.get())
            .is_sorted();

        // Try to find the existing group.
        let existing_group = dependency_groups.iter_mut().find_map(|(key, value)| {
            if GroupName::from_str(key.get()).is_ok_and(|g| g == *group) {
                Some(value)
            } else {
                None
            }
        });

        // Create the group if it does not exist.
        let group = match existing_group {
            Some(value) => value,
            None => dependency_groups
                .entry(group.as_ref())
                .or_insert(Item::Value(Value::Array(Array::new()))),
        }
        .as_array_mut()
        .ok_or(Error::MalformedDependencies)?;

        let added = add_dependency(req, group, source.is_some(), raw)?;

        // Sort new group keys only if existing keys were sorted. This avoids unnecessary changes.
        if was_sorted {
            dependency_groups.sort_values();
        }

        // Reformat `dependency-groups` if it is an inline table.
        // Inline tables do not permit comments between items, so reformatting cannot remove any.
        if let Some(dependency_groups) = self
            .doc
            .get_mut("dependency-groups")
            .and_then(Item::as_inline_table_mut)
        {
            dependency_groups.fmt();
        }

        if let Some(source) = source {
            self.add_source(&req.name, source)?;
        }

        Ok(added)
    }

    /// Ensure a dependency group exists. Create an empty group if necessary.
    pub fn ensure_dependency_group(&mut self, group: &GroupName) -> Result<(), Error> {
        // Get or create `dependency-groups`.
        let dependency_groups = self
            .doc
            .entry("dependency-groups")
            .or_insert(Item::Table(Table::new()))
            .as_table_like_mut()
            .ok_or(Error::MalformedDependencies)?;

        let was_sorted = dependency_groups
            .get_values()
            .iter()
            .filter_map(|(dotted_ks, _)| dotted_ks.first())
            .map(|k| k.get())
            .is_sorted();

        // Check if the group already exists.
        let group_exists = dependency_groups
            .iter()
            .any(|(key, _value)| GroupName::from_str(key).is_ok_and(|g| g == *group));

        // Create the group if it does not exist.
        if !group_exists {
            dependency_groups.insert(group.as_ref(), Item::Value(Value::Array(Array::new())));

            // Sort new group keys only if existing keys were sorted.
            if was_sorted {
                dependency_groups.sort_values();
            }
        }

        // Reformat `dependency-groups` if it is an inline table.
        // Inline tables do not permit comments between items, so reformatting cannot remove any.
        if let Some(dependency_groups) = self
            .doc
            .get_mut("dependency-groups")
            .and_then(Item::as_inline_table_mut)
        {
            dependency_groups.fmt();
        }

        Ok(())
    }

    /// Set the constraint for a requirement for an existing dependency.
    pub fn set_dependency_bound(
        &mut self,
        dependency_type: &DependencyType,
        index: usize,
        version: Version,
        bound_kind: AddBoundsKind,
    ) -> Result<(), Error> {
        let group = match dependency_type {
            DependencyType::Production => self.dependencies_array()?,
            DependencyType::Dev => self.dev_dependencies_array()?,
            DependencyType::Optional(extra) => self.optional_dependencies_array(extra)?,
            DependencyType::Group(group) => self.dependency_groups_array(group)?,
        };

        let Some(req) = group.get(index) else {
            return Err(Error::MissingDependency(index));
        };

        let mut req = req
            .as_str()
            .and_then(try_parse_requirement)
            .ok_or(Error::MalformedDependencies)?;
        req.version_or_url = Some(VersionOrUrl::VersionSpecifier(
            bound_kind.specifiers(version),
        ));
        group.replace(index, req.to_string());

        Ok(())
    }

    /// Get the TOML array for `project.dependencies`.
    fn dependencies_array(&mut self) -> Result<&mut Array, Error> {
        // Get or create `project.dependencies`.
        let dependencies = self
            .project()?
            .entry("dependencies")
            .or_insert(Item::Value(Value::Array(Array::new())))
            .as_array_mut()
            .ok_or(Error::MalformedDependencies)?;

        Ok(dependencies)
    }

    /// Get the TOML array for `tool.uv.dev-dependencies`.
    fn dev_dependencies_array(&mut self) -> Result<&mut Array, Error> {
        // Get or create `tool.uv.dev-dependencies`.
        let dev_dependencies = self
            .doc
            .entry("tool")
            .or_insert(implicit())
            .as_table_mut()
            .ok_or(Error::MalformedSources)?
            .entry("uv")
            .or_insert(Item::Table(Table::new()))
            .as_table_mut()
            .ok_or(Error::MalformedSources)?
            .entry("dev-dependencies")
            .or_insert(Item::Value(Value::Array(Array::new())))
            .as_array_mut()
            .ok_or(Error::MalformedDependencies)?;

        Ok(dev_dependencies)
    }

    /// Get the TOML array for a `project.optional-dependencies` entry.
    fn optional_dependencies_array(&mut self, group: &ExtraName) -> Result<&mut Array, Error> {
        // Get or create `project.optional-dependencies`.
        let optional_dependencies = self
            .project()?
            .entry("optional-dependencies")
            .or_insert(Item::Table(Table::new()))
            .as_table_like_mut()
            .ok_or(Error::MalformedDependencies)?;

        // Try to find the existing extra.
        let existing_key = optional_dependencies.iter().find_map(|(key, _value)| {
            if ExtraName::from_str(key).is_ok_and(|g| g == *group) {
                Some(key.to_string())
            } else {
                None
            }
        });

        // Create the group if it does not exist.
        let group = optional_dependencies
            .entry(existing_key.as_deref().unwrap_or(group.as_ref()))
            .or_insert(Item::Value(Value::Array(Array::new())))
            .as_array_mut()
            .ok_or(Error::MalformedDependencies)?;

        Ok(group)
    }

    /// Get the TOML array for a `dependency-groups` entry.
    fn dependency_groups_array(&mut self, group: &GroupName) -> Result<&mut Array, Error> {
        // Get or create `dependency-groups`.
        let dependency_groups = self
            .doc
            .entry("dependency-groups")
            .or_insert(Item::Table(Table::new()))
            .as_table_like_mut()
            .ok_or(Error::MalformedDependencies)?;

        // Try to find the existing group.
        let existing_key = dependency_groups.iter().find_map(|(key, _value)| {
            if GroupName::from_str(key).is_ok_and(|g| g == *group) {
                Some(key.to_string())
            } else {
                None
            }
        });

        // Create the group if it does not exist.
        let group = dependency_groups
            .entry(existing_key.as_deref().unwrap_or(group.as_ref()))
            .or_insert(Item::Value(Value::Array(Array::new())))
            .as_array_mut()
            .ok_or(Error::MalformedDependencies)?;

        Ok(group)
    }

    /// Get an existing TOML array for a dependency type.
    fn dependency_type_array_mut(
        &mut self,
        dependency_type: &DependencyType,
    ) -> Result<Option<&mut Array>, Error> {
        let dependencies = match dependency_type {
            DependencyType::Production => self
                .project_mut()?
                .and_then(|project| project.get_mut("dependencies"))
                .map(|dependencies| {
                    dependencies
                        .as_array_mut()
                        .ok_or(Error::MalformedDependencies)
                })
                .transpose()?,
            DependencyType::Dev => self
                .doc
                .get_mut("tool")
                .map(|tool| tool.as_table_mut().ok_or(Error::MalformedDependencies))
                .transpose()?
                .and_then(|tool| tool.get_mut("uv"))
                .map(|tool_uv| tool_uv.as_table_mut().ok_or(Error::MalformedDependencies))
                .transpose()?
                .and_then(|tool_uv| tool_uv.get_mut("dev-dependencies"))
                .map(|dependencies| {
                    dependencies
                        .as_array_mut()
                        .ok_or(Error::MalformedDependencies)
                })
                .transpose()?,
            DependencyType::Optional(extra) => self
                .project_mut()?
                .and_then(|project| project.get_mut("optional-dependencies"))
                .map(|extras| {
                    extras
                        .as_table_like_mut()
                        .ok_or(Error::MalformedDependencies)
                })
                .transpose()?
                .and_then(|extras| {
                    extras.iter_mut().find_map(|(key, value)| {
                        if ExtraName::from_str(key.get()).is_ok_and(|name| name == *extra) {
                            Some(value)
                        } else {
                            None
                        }
                    })
                })
                .map(|dependencies| {
                    dependencies
                        .as_array_mut()
                        .ok_or(Error::MalformedDependencies)
                })
                .transpose()?,
            DependencyType::Group(group) => self
                .doc
                .get_mut("dependency-groups")
                .map(|groups| {
                    groups
                        .as_table_like_mut()
                        .ok_or(Error::MalformedDependencies)
                })
                .transpose()?
                .and_then(|groups| {
                    groups.iter_mut().find_map(|(key, value)| {
                        if GroupName::from_str(key.get()).is_ok_and(|name| name == *group) {
                            Some(value)
                        } else {
                            None
                        }
                    })
                })
                .map(|dependencies| {
                    dependencies
                        .as_array_mut()
                        .ok_or(Error::MalformedDependencies)
                })
                .transpose()?,
        };

        Ok(dependencies)
    }

    /// Adds a source to `tool.uv.sources`.
    fn add_source(&mut self, name: &PackageName, source: &Source) -> Result<(), Error> {
        // Get or create `tool.uv.sources`.
        let sources = self
            .doc
            .entry("tool")
            .or_insert(implicit())
            .as_table_mut()
            .ok_or(Error::MalformedSources)?
            .entry("uv")
            .or_insert(implicit())
            .as_table_mut()
            .ok_or(Error::MalformedSources)?
            .entry("sources")
            .or_insert(Item::Table(Table::new()))
            .as_table_mut()
            .ok_or(Error::MalformedSources)?;

        if let Some(key) = find_source(name, sources) {
            sources.remove(&key);
        }
        add_source(name, source, sources)?;

        Ok(())
    }

    /// Removes all occurrences of dependencies with the given name.
    pub fn remove_dependency(&mut self, name: &PackageName) -> Result<Vec<Requirement>, Error> {
        // Try to get `project.dependencies`.
        let Some(dependencies) = self
            .project_mut()?
            .and_then(|project| project.get_mut("dependencies"))
            .map(|dependencies| {
                dependencies
                    .as_array_mut()
                    .ok_or(Error::MalformedDependencies)
            })
            .transpose()?
        else {
            return Ok(Vec::new());
        };

        let requirements = remove_dependency(name, dependencies);
        self.remove_source(name)?;

        Ok(requirements)
    }

    /// Removes all occurrences of development dependencies with the given name.
    pub fn remove_dev_dependency(&mut self, name: &PackageName) -> Result<Vec<Requirement>, Error> {
        // Try to get `tool.uv.dev-dependencies`.
        let Some(dev_dependencies) = self
            .doc
            .get_mut("tool")
            .map(|tool| tool.as_table_mut().ok_or(Error::MalformedDependencies))
            .transpose()?
            .and_then(|tool| tool.get_mut("uv"))
            .map(|tool_uv| tool_uv.as_table_mut().ok_or(Error::MalformedDependencies))
            .transpose()?
            .and_then(|tool_uv| tool_uv.get_mut("dev-dependencies"))
            .map(|dependencies| {
                dependencies
                    .as_array_mut()
                    .ok_or(Error::MalformedDependencies)
            })
            .transpose()?
        else {
            return Ok(Vec::new());
        };

        let requirements = remove_dependency(name, dev_dependencies);
        self.remove_source(name)?;

        Ok(requirements)
    }

    /// Removes all occurrences of optional dependencies in the group with the given name.
    pub fn remove_optional_dependency(
        &mut self,
        name: &PackageName,
        group: &ExtraName,
    ) -> Result<Vec<Requirement>, Error> {
        // Try to get `project.optional-dependencies.<group>`.
        let Some(optional_dependencies) = self
            .project_mut()?
            .and_then(|project| project.get_mut("optional-dependencies"))
            .map(|extras| {
                extras
                    .as_table_like_mut()
                    .ok_or(Error::MalformedDependencies)
            })
            .transpose()?
            .and_then(|extras| {
                extras.iter_mut().find_map(|(key, value)| {
                    if ExtraName::from_str(key.get()).is_ok_and(|g| g == *group) {
                        Some(value)
                    } else {
                        None
                    }
                })
            })
            .map(|dependencies| {
                dependencies
                    .as_array_mut()
                    .ok_or(Error::MalformedDependencies)
            })
            .transpose()?
        else {
            return Ok(Vec::new());
        };

        let requirements = remove_dependency(name, optional_dependencies);
        self.remove_source(name)?;

        Ok(requirements)
    }

    /// Removes all occurrences of the dependency in the group with the given name.
    pub fn remove_dependency_group_requirement(
        &mut self,
        name: &PackageName,
        group: &GroupName,
    ) -> Result<Vec<Requirement>, Error> {
        // Try to get `project.optional-dependencies.<group>`.
        let Some(group_dependencies) = self
            .doc
            .get_mut("dependency-groups")
            .map(|groups| {
                groups
                    .as_table_like_mut()
                    .ok_or(Error::MalformedDependencies)
            })
            .transpose()?
            .and_then(|groups| {
                groups.iter_mut().find_map(|(key, value)| {
                    if GroupName::from_str(key.get()).is_ok_and(|g| g == *group) {
                        Some(value)
                    } else {
                        None
                    }
                })
            })
            .map(|dependencies| {
                dependencies
                    .as_array_mut()
                    .ok_or(Error::MalformedDependencies)
            })
            .transpose()?
        else {
            return Ok(Vec::new());
        };

        let requirements = remove_dependency(name, group_dependencies);
        self.remove_source(name)?;

        Ok(requirements)
    }

    /// Remove a matching source from `tool.uv.sources`, if it exists.
    fn remove_source(&mut self, name: &PackageName) -> Result<(), Error> {
        // If the dependency is still in use, don't remove the source.
        if !self.find_dependency(name, None).is_empty() {
            return Ok(());
        }

        if let Some(sources) = self
            .doc
            .get_mut("tool")
            .map(|tool| tool.as_table_mut().ok_or(Error::MalformedSources))
            .transpose()?
            .and_then(|tool| tool.get_mut("uv"))
            .map(|tool_uv| tool_uv.as_table_mut().ok_or(Error::MalformedSources))
            .transpose()?
            .and_then(|tool_uv| tool_uv.get_mut("sources"))
            .map(|sources| sources.as_table_mut().ok_or(Error::MalformedSources))
            .transpose()?
        {
            if let Some(key) = find_source(name, sources) {
                sources.remove(&key);

                // Remove the `tool.uv.sources` table if it is empty.
                if sources.is_empty() {
                    self.doc
                        .entry("tool")
                        .or_insert(implicit())
                        .as_table_mut()
                        .ok_or(Error::MalformedSources)?
                        .entry("uv")
                        .or_insert(implicit())
                        .as_table_mut()
                        .ok_or(Error::MalformedSources)?
                        .remove("sources");
                }
            }
        }

        Ok(())
    }

    /// Returns `true` if the `tool.uv.dev-dependencies` table is present.
    pub fn has_dev_dependencies(&self) -> bool {
        self.doc
            .get("tool")
            .and_then(Item::as_table)
            .and_then(|tool| tool.get("uv"))
            .and_then(Item::as_table)
            .and_then(|uv| uv.get("dev-dependencies"))
            .is_some()
    }

    /// Returns `true` if the `dependency-groups` table is present and contains the given group.
    pub fn has_dependency_group(&self, group: &GroupName) -> bool {
        self.doc
            .get("dependency-groups")
            .and_then(Item::as_table)
            .and_then(|groups| groups.get(group.as_ref()))
            .is_some()
    }

    /// Returns all the places in this `pyproject.toml` that contain a dependency with the given
    /// name.
    ///
    /// This method searches `project.dependencies`, `tool.uv.dev-dependencies`, and
    /// `tool.uv.optional-dependencies`.
    pub fn find_dependency(
        &self,
        name: &PackageName,
        marker: Option<&MarkerTree>,
    ) -> Vec<DependencyType> {
        let mut types = Vec::new();

        if let Some(project) = self.doc.get("project").and_then(Item::as_table) {
            // Check `project.dependencies`.
            if let Some(dependencies) = project.get("dependencies").and_then(Item::as_array)
                && !find_dependencies(name, marker, dependencies).is_empty()
            {
                types.push(DependencyType::Production);
            }

            // Check `project.optional-dependencies`.
            if let Some(extras) = project
                .get("optional-dependencies")
                .and_then(Item::as_table)
            {
                for (extra, dependencies) in extras {
                    let Some(dependencies) = dependencies.as_array() else {
                        continue;
                    };
                    let Ok(extra) = ExtraName::from_str(extra) else {
                        continue;
                    };

                    if !find_dependencies(name, marker, dependencies).is_empty() {
                        types.push(DependencyType::Optional(extra));
                    }
                }
            }
        }

        // Check `dependency-groups`.
        if let Some(groups) = self.doc.get("dependency-groups").and_then(Item::as_table) {
            for (group, dependencies) in groups {
                let Some(dependencies) = dependencies.as_array() else {
                    continue;
                };
                let Ok(group) = GroupName::from_str(group) else {
                    continue;
                };

                if !find_dependencies(name, marker, dependencies).is_empty() {
                    types.push(DependencyType::Group(group));
                }
            }
        }

        // Check `tool.uv.dev-dependencies`.
        if let Some(dev_dependencies) = self
            .doc
            .get("tool")
            .and_then(Item::as_table)
            .and_then(|tool| tool.get("uv"))
            .and_then(Item::as_table)
            .and_then(|uv| uv.get("dev-dependencies"))
            .and_then(Item::as_array)
            && !find_dependencies(name, marker, dev_dependencies).is_empty()
        {
            types.push(DependencyType::Dev);
        }

        types
    }

    pub fn version(&mut self) -> Result<Version, Error> {
        let version = self
            .doc
            .get("project")
            .and_then(Item::as_table)
            .and_then(|project| project.get("version"))
            .and_then(Item::as_str)
            .ok_or(Error::MalformedWorkspace)?;

        Ok(Version::from_str(version)?)
    }

    pub fn has_dynamic_version(&mut self) -> bool {
        let Some(dynamic) = self
            .doc
            .get("project")
            .and_then(Item::as_table)
            .and_then(|project| project.get("dynamic"))
            .and_then(Item::as_array)
        else {
            return false;
        };

        dynamic.iter().any(|val| val.as_str() == Some("version"))
    }

    pub fn set_version(&mut self, version: &Version) -> Result<(), Error> {
        let project = self
            .doc
            .get_mut("project")
            .and_then(Item::as_table_mut)
            .ok_or(Error::MalformedWorkspace)?;

        if let Some(existing) = project.get_mut("version") {
            if let Some(value) = existing.as_value_mut() {
                let mut formatted = Value::from(version.to_string());
                *formatted.decor_mut() = value.decor().clone();
                *value = formatted;
            } else {
                *existing = Item::Value(Value::from(version.to_string()));
            }
        } else {
            project.insert("version", Item::Value(Value::from(version.to_string())));
        }

        Ok(())
    }
}

/// Return an implicit table.
fn implicit() -> Item {
    let mut table = Table::new();
    table.set_implicit(true);
    Item::Table(table)
}

/// Add a dependency to the given `deps` array.
///
/// Return [`ArrayEdit::Add`] or [`ArrayEdit::Update`] for the affected dependency.
fn add_dependency(
    req: &Requirement,
    deps: &mut Array,
    has_source: bool,
    raw: bool,
) -> Result<ArrayEdit, Error> {
    let mut to_replace = find_dependencies(&req.name, Some(&req.marker), deps);

    match to_replace.as_slice() {
        [] => {
            #[derive(Debug, Copy, Clone)]
            enum Sort {
                /// Sort the list without considering case.
                CaseInsensitive,
                /// Sort complete entries without considering case.
                CaseInsensitiveNaive,
                /// Sort the list while considering case.
                CaseSensitive,
                /// Sort complete entries while considering case.
                CaseSensitiveNaive,
                /// Keep the existing unsorted order.
                Unsorted,
            }

            fn is_sorted<T, I>(items: I) -> bool
            where
                I: IntoIterator<Item = T>,
                T: PartialOrd + Copy,
            {
                items.into_iter().tuple_windows().all(|(a, b)| a <= b)
            }

            // Dependencies contain requirement strings and inline `include-group` tables.
            // Use only requirement strings to determine the sort order.
            let reqs: Vec<_> = deps.iter().filter_map(Value::as_str).collect();
            let reqs_lowercase: Vec<_> = reqs.iter().copied().map(str::to_lowercase).collect();

            // Check whether the original dependency list is sorted. Sort the new list only when
            // the original was sorted. This preserves a user's custom dependency order.
            //
            // Ignore non-string items, such as `{ include-group = "..." }`.
            //
            // Check both case-sensitive and case-insensitive sort orders.
            let sort = if is_sorted(
                reqs_lowercase
                    .iter()
                    .map(String::as_str)
                    .map(split_specifiers),
            ) {
                Sort::CaseInsensitive
            } else if is_sorted(reqs.iter().copied().map(split_specifiers)) {
                Sort::CaseSensitive
            } else if is_sorted(reqs_lowercase.iter().map(String::as_str)) {
                Sort::CaseInsensitiveNaive
            } else if is_sorted(reqs) {
                Sort::CaseSensitiveNaive
            } else {
                Sort::Unsorted
            };

            let req_string = if raw {
                req.displayable_with_credentials().to_string()
            } else {
                req.to_string()
            };
            let index = match sort {
                Sort::CaseInsensitive => deps.iter().position(|dep| {
                    dep.as_str().is_some_and(|dep| {
                        split_specifiers(&dep.to_lowercase())
                            > split_specifiers(&req_string.to_lowercase())
                    })
                }),
                Sort::CaseInsensitiveNaive => deps.iter().position(|dep| {
                    dep.as_str()
                        .is_some_and(|dep| dep.to_lowercase() > req_string.to_lowercase())
                }),
                Sort::CaseSensitive => deps.iter().position(|dep| {
                    dep.as_str()
                        .is_some_and(|dep| split_specifiers(dep) > split_specifiers(&req_string))
                }),
                Sort::CaseSensitiveNaive => deps
                    .iter()
                    .position(|dep| dep.as_str().is_some_and(|dep| *dep > *req_string)),
                Sort::Unsorted => None,
            };
            let index = index.unwrap_or_else(|| {
                // Add the dependency after the last requirement. Keep `include-group` entries at
                // the end when the user has placed them there.
                deps.iter()
                    .enumerate()
                    .filter_map(|(i, dep)| if dep.is_str() { Some(i + 1) } else { None })
                    .last()
                    .unwrap_or(deps.len())
            });

            let mut value = Value::from(req_string.as_str());

            let decor = value.decor_mut();

            // Keep comments on the correct lines after insertion.
            match index {
                val if val == deps.len() => {
                    // At the end of the list, attach trailing comments to the added dependency.
                    //
                    // For example, given:
                    // ```toml
                    // dependencies = [
                    //     "anyio", # trailing comment
                    // ]
                    // ```
                    //
                    // After adding `flask`, keep the comment on `anyio`:
                    // ```toml
                    // dependencies = [
                    //     "anyio", # trailing comment
                    //     "flask",
                    // ]
                    // ```
                    decor.set_prefix(deps.trailing().clone());
                    deps.set_trailing("");
                }
                0 => {
                    // Do nothing when prepending to a nonempty list.
                }
                val => {
                    // Keep end-of-line comments in place when inserting a dependency below them.
                    //
                    // For example, given:
                    // ```toml
                    // dependencies = [
                    //     "anyio", # end-of-line comment
                    //     "flask",
                    // ]
                    // ```
                    //
                    // After inserting `pydantic`, keep the comment on `anyio`:
                    // ```toml
                    // dependencies = [
                    //     "anyio", # end-of-line comment
                    //     "pydantic",
                    //     "flask",
                    // ]
                    // ```
                    let targeted_decor = deps.get_mut(val).unwrap().decor_mut();
                    decor.set_prefix(targeted_decor.prefix().unwrap().clone());
                    targeted_decor.set_prefix(""); // Re-formatted later by `reformat_array_multiline`
                }
            }

            deps.insert_formatted(index, value);

            // `reformat_array_multiline` uses the first dependency's indentation.
            // When prepending to a nonempty list, copy that indentation to the new first entry.
            if deps.len() > 1 && index == 0 {
                let prefix = deps
                    .clone()
                    .get(index + 1)
                    .unwrap()
                    .decor()
                    .prefix()
                    .unwrap()
                    .clone();

                // Do not duplicate comments in the prefix. Keep each comment in place or attach it
                // to the entry that moves to the next line.
                //
                // For example, given:
                // ```toml
                // dependencies = [ # comment
                //     "flask",
                // ]
                // ```
                //
                // After adding `anyio` first, keep the comment on the opening bracket:
                // ```toml
                // dependencies = [ # comment
                //     "anyio",
                //     "flask",
                // ]
                // ```
                //
                // For a comment on its own line:
                // ```toml
                // dependencies = [
                //     # comment
                //     "flask",
                // ]
                // ```
                //
                // After adding `anyio` first, move the comment with the existing entry:
                // ```toml
                // dependencies = [
                //     "anyio",
                //     # comment
                //     "flask",
                // ]
                if let Some(prefix) = prefix.as_str() {
                    // Attach content before the first own-line comment to the new entry.
                    // Attach that comment and later content to the existing entry.
                    //
                    // The new entry uses the first and last lines as its prefix. The existing entry
                    // uses the remaining lines.
                    if let Some((first_line, rest)) = prefix.split_once(['\r', '\n']) {
                        // Determine the appropriate newline character.
                        let newline = {
                            let mut chars = prefix[first_line.len()..].chars();
                            match (chars.next(), chars.next()) {
                                (Some('\r'), Some('\n')) => "\r\n",
                                (Some('\r'), _) => "\r",
                                (Some('\n'), _) => "\n",
                                _ => "\n",
                            }
                        };
                        let last_line = rest.lines().last().unwrap_or_default();
                        let prefix = format!("{first_line}{newline}{last_line}");
                        deps.get_mut(index).unwrap().decor_mut().set_prefix(prefix);

                        let prefix = format!("{newline}{rest}");
                        deps.get_mut(index + 1)
                            .unwrap()
                            .decor_mut()
                            .set_prefix(prefix);
                    } else {
                        deps.get_mut(index).unwrap().decor_mut().set_prefix(prefix);
                    }
                } else {
                    deps.get_mut(index).unwrap().decor_mut().set_prefix(prefix);
                }
            }

            reformat_array_multiline(deps);

            Ok(ArrayEdit::Add(index))
        }
        [_] => {
            let (i, mut old_req) = to_replace.remove(0);
            update_requirement(&mut old_req, req, has_source);
            deps.replace(i, old_req.to_string());
            reformat_array_multiline(deps);
            Ok(ArrayEdit::Update(i))
        }
        // Cannot perform ambiguous updates.
        _ => Err(Error::Ambiguous {
            package_name: req.name.clone(),
            requirements: to_replace
                .into_iter()
                .map(|(_, requirement)| requirement)
                .collect(),
        }),
    }
}

/// Update an existing requirement.
fn update_requirement(old: &mut Requirement, new: &Requirement, has_source: bool) {
    // Add any new extras.
    let mut extras = old.extras.to_vec();
    extras.extend(new.extras.iter().cloned());
    extras.sort_unstable();
    extras.dedup();
    old.extras = extras.into_boxed_slice();

    // Clear the requirement source before adding it to `tool.uv.sources`.
    if has_source {
        old.clear_url();
    }

    // Update the source if a new one was specified.
    match &new.version_or_url {
        None => {}
        Some(VersionOrUrl::VersionSpecifier(specifier)) if specifier.is_empty() => {}
        Some(version_or_url) => old.version_or_url = Some(version_or_url.clone()),
    }

    // Update the marker expression.
    if new.marker.contents().is_some() {
        old.marker = new.marker;
    }
}

/// Remove every dependency with the given name from the `deps` array.
fn remove_dependency(name: &PackageName, deps: &mut Array) -> Vec<Requirement> {
    // Remove entries in reverse order to preserve their indices. Before each removal, move the
    // entry's prefix to the next entry or the array's trailing content. This preserves
    // end-of-line comments that belong to the previous entry.
    //
    // For example, in:
    // ```toml
    // dependencies = [
    //     "numpy>=2.4.3", # essential comment
    //     "requests>=2.32.5",
    // ]
    // ```
    //
    // `toml_edit` stores `# essential comment` in the prefix of `requests`.
    // When removing `requests`, move the comment so it remains on the `numpy` line.
    let removed = find_dependencies(name, None, deps)
        .into_iter()
        .rev()
        .filter_map(|(i, _)| remove_dependency_at(i, deps))
        .collect::<Vec<_>>();

    if !removed.is_empty() {
        reformat_array_multiline(deps);
    }

    removed
}

fn remove_dependency_at(index: usize, deps: &mut Array) -> Option<Requirement> {
    if let Some(prefix) = deps
        .get(index)
        .and_then(|item| item.decor().prefix().and_then(|s| s.as_str()))
        .filter(|s| !s.is_empty())
    {
        let prefix = prefix.to_string();
        if let Some(next) = deps.get(index + 1)
            && let Some(existing) = next.decor().prefix().and_then(|s| s.as_str())
        {
            // Add the removed entry's prefix to the next entry's prefix.
            let existing = existing.to_string();
            if let Some(next) = deps.get_mut(index + 1) {
                next.decor_mut().set_prefix(format!("{prefix}{existing}"));
            }
        } else if let Some(next) = deps.get_mut(index + 1) {
            // The next entry has no prefix. Use the removed entry's prefix.
            next.decor_mut().set_prefix(&prefix);
        } else if let Some(existing) = deps.trailing().as_str() {
            // No next entry exists. Move comments to the array's trailing content.
            deps.set_trailing(format!("{prefix}{existing}"));
        } else {
            deps.set_trailing(&prefix);
        }
    }

    deps.remove(index)
        .as_str()
        .and_then(|req| Requirement::from_str(req).ok())
}

/// Return every dependency with the given name and its position in the array.
fn find_dependencies(
    name: &PackageName,
    marker: Option<&MarkerTree>,
    deps: &Array,
) -> Vec<(usize, Requirement)> {
    let mut to_replace = Vec::new();
    for (i, dep) in deps.iter().enumerate() {
        if let Some(req) = dep.as_str().and_then(try_parse_requirement)
            && marker.is_none_or(|m| *m == req.marker)
            && *name == req.name
        {
            to_replace.push((i, req));
        }
    }
    to_replace
}

/// Check whether two requirements have the same serialized fields, regardless of parsed origin.
fn same_requirement_declaration(left: &Requirement, right: &Requirement) -> bool {
    left.name == right.name
        && left.extras == right.extras
        && left.version_or_url == right.version_or_url
        && left.marker == right.marker
}

/// Return the `tool.uv.sources` key that matches the given package name.
fn find_source(name: &PackageName, sources: &Table) -> Option<String> {
    for (key, _) in sources {
        if PackageName::from_str(key).is_ok_and(|ref key| key == name) {
            return Some(key.to_string());
        }
    }
    None
}

// Add a source to `tool.uv.sources`.
fn add_source(req: &PackageName, source: &Source, sources: &mut Table) -> Result<(), Error> {
    // Serialize as an inline table.
    let mut doc = toml::to_string(&source)
        .map_err(Box::new)?
        .parse::<DocumentMut>()
        .unwrap();
    let table = mem::take(doc.as_table_mut()).into_inline_table();

    sources.insert(req.as_ref(), Item::Value(Value::InlineTable(table)));

    Ok(())
}

impl fmt::Display for PyProjectTomlMut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.doc.fmt(f)
    }
}

fn try_parse_requirement(req: &str) -> Option<Requirement> {
    Requirement::from_str(req).ok()
}

/// Format a TOML array across multiple lines. Preserve its comments and add a trailing comma.
fn reformat_array_multiline(deps: &mut Array) {
    fn find_comments(s: Option<&RawString>) -> Box<dyn Iterator<Item = Comment> + '_> {
        let iter = s
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .lines()
            .scan(
                (false, false),
                |(prev_line_was_empty, prev_line_was_comment), line| {
                    let trimmed_line = line.trim();

                    if let Some((before, comment)) = line.split_once('#') {
                        let comment_text = format!("#{}", comment.trim_end());

                        let comment_kind = if (*prev_line_was_empty) || (*prev_line_was_comment) {
                            CommentType::OwnLine
                        } else {
                            CommentType::EndOfLine {
                                leading_whitespace: before
                                    .chars()
                                    .rev()
                                    .take_while(|c| c.is_whitespace())
                                    .collect::<String>()
                                    .chars()
                                    .rev()
                                    .collect(),
                            }
                        };

                        *prev_line_was_empty = trimmed_line.is_empty();
                        *prev_line_was_comment = true;

                        Some(Some(Comment {
                            text: comment_text,
                            kind: comment_kind,
                        }))
                    } else {
                        *prev_line_was_empty = trimmed_line.is_empty();
                        *prev_line_was_comment = false;
                        Some(None)
                    }
                },
            )
            .flatten();

        Box::new(iter)
    }

    // Without a trailing comma, `toml_edit` stores final comments in the last entry's suffix.
    // After adding a trailing comma, move those comments after the comma.
    if !deps.trailing_comma()
        && let Some(last) = deps.iter_mut().last()
        && let Some(suffix) = last.decor().suffix().and_then(RawString::as_str)
        && suffix.contains('#')
    {
        let suffix = suffix.to_string();
        last.decor_mut().set_suffix("");
        let trailing = deps.trailing().as_str().unwrap_or_default();
        deps.set_trailing(format!("{suffix}{trailing}"));
    }

    let mut indentation_prefix = None;

    // Use the first dependency's indentation as the indentation prefix.
    if let Some(first_item) = deps.iter().next() {
        let decor_prefix = first_item
            .decor()
            .prefix()
            .and_then(|s| s.as_str())
            .and_then(|s| s.lines().last())
            .unwrap_or_default();

        let decor_prefix = decor_prefix
            .split_once('#')
            .map(|(s, _)| s)
            .unwrap_or(decor_prefix);

        indentation_prefix = (!decor_prefix.is_empty()).then_some(decor_prefix.to_string());
    }

    let indentation_prefix_str = format!("\n{}", indentation_prefix.as_deref().unwrap_or("    "));

    for item in deps.iter_mut() {
        let decor = item.decor_mut();
        let mut prefix = String::new();

        for comment in find_comments(decor.prefix()).chain(find_comments(decor.suffix())) {
            match &comment.kind {
                CommentType::OwnLine => {
                    prefix.push_str(&indentation_prefix_str);
                }
                CommentType::EndOfLine { leading_whitespace } => {
                    prefix.push_str(leading_whitespace);
                }
            }
            prefix.push_str(&comment.text);
        }
        prefix.push_str(&indentation_prefix_str);
        decor.set_prefix(prefix);
        decor.set_suffix("");
    }

    deps.set_trailing(&{
        let mut comments = find_comments(Some(deps.trailing())).peekable();
        let mut rv = String::new();
        if comments.peek().is_some() {
            for comment in comments {
                match &comment.kind {
                    CommentType::OwnLine => {
                        let indentation_prefix_str =
                            format!("\n{}", indentation_prefix.as_deref().unwrap_or("    "));
                        rv.push_str(&indentation_prefix_str);
                    }
                    CommentType::EndOfLine { leading_whitespace } => {
                        rv.push_str(leading_whitespace);
                    }
                }
                rv.push_str(&comment.text);
            }
        }
        if !rv.is_empty() || !deps.is_empty() {
            rv.push('\n');
        }
        rv
    });
    deps.set_trailing_comma(true);
}

/// Split a requirement into the package name and its dependency specifiers.
///
/// For `flask>=1.0`, return `("flask", ">=1.0")`.
/// For `Flask>=1.0`, preserve case and return `("Flask", ">=1.0")`.
///
/// Keep extras: `flask[dotenv]>=1.0` returns `("flask[dotenv]", ">=1.0")`.
fn split_specifiers(req: &str) -> (&str, &str) {
    let (name, specifiers) = req
        .find(['>', '<', '=', '~', '!', '@'])
        .map_or((req, ""), |pos| {
            let (name, specifiers) = req.split_at(pos);
            (name, specifiers)
        });
    (name.trim(), specifiers.trim())
}

#[cfg(test)]
mod test {
    use crate::pyproject::DependencyType;

    use super::{
        AddBoundsKind, ArrayEdit, DependencyTarget, PyProjectTomlMut, reformat_array_multiline,
        remove_dependency, split_specifiers,
    };
    use anyhow::Result;
    use insta::assert_snapshot;
    use std::path::Path;
    use std::str::FromStr;
    use toml_edit::DocumentMut;
    use uv_distribution_types::Index;
    use uv_normalize::{ExtraName, GroupName, PackageName};
    use uv_pep440::Version;
    use uv_pep508::{Requirement, RequirementOrigin};

    #[test]
    fn split() {
        assert_eq!(split_specifiers("flask>=1.0"), ("flask", ">=1.0"));
        assert_eq!(split_specifiers("Flask>=1.0"), ("Flask", ">=1.0"));
        assert_eq!(
            split_specifiers("flask[dotenv]>=1.0"),
            ("flask[dotenv]", ">=1.0")
        );
        assert_eq!(split_specifiers("flask[dotenv]"), ("flask[dotenv]", ""));
        assert_eq!(
            split_specifiers(
                "flask @ https://files.pythonhosted.org/packages/af/47/93213ee66ef8fae3b93b3e29206f6b251e65c97bd91d8e1c5596ef15af0a/flask-3.1.0-py3-none-any.whl"
            ),
            (
                "flask",
                "@ https://files.pythonhosted.org/packages/af/47/93213ee66ef8fae3b93b3e29206f6b251e65c97bd91d8e1c5596ef15af0a/flask-3.1.0-py3-none-any.whl"
            )
        );
    }

    #[test]
    fn reformat_preserves_inline_comment_spacing() {
        let mut doc: DocumentMut = r#"
[project]
dependencies = [
    "attrs>=25.4.0",     # comment
]
"#
        .parse()
        .unwrap();

        reformat_array_multiline(
            doc["project"]["dependencies"]
                .as_array_mut()
                .expect("dependencies array"),
        );

        let serialized = doc.to_string();

        assert!(
            serialized.contains("\"attrs>=25.4.0\",     # comment"),
            "inline comment spacing should be preserved:\n{serialized}"
        );
    }

    #[test]
    fn reformat_preserves_inline_comment_without_padding() {
        let mut doc: DocumentMut = r#"
[project]
dependencies = [
    "attrs>=25.4.0",#comment
]
"#
        .parse()
        .unwrap();

        reformat_array_multiline(
            doc["project"]["dependencies"]
                .as_array_mut()
                .expect("dependencies array"),
        );

        let serialized = doc.to_string();

        assert!(
            serialized.contains("\"attrs>=25.4.0\",#comment"),
            "inline comment spacing without padding should be preserved:\n{serialized}"
        );
    }

    #[test]
    fn bound_kind_to_specifiers_exact() {
        let tests = [
            ("0", "==0"),
            ("0.0", "==0.0"),
            ("0.0.0", "==0.0.0"),
            ("0.1", "==0.1"),
            ("0.0.1", "==0.0.1"),
            ("0.0.0.1", "==0.0.0.1"),
            ("1.0.0", "==1.0.0"),
            ("1.2", "==1.2"),
            ("1.2.3", "==1.2.3"),
            ("1.2.3.4", "==1.2.3.4"),
            ("1.2.3.4a1.post1", "==1.2.3.4a1.post1"),
        ];

        for (version, expected) in tests {
            let actual = AddBoundsKind::Exact
                .specifiers(Version::from_str(version).unwrap())
                .to_string();
            assert_eq!(actual, expected, "{version}");
        }
    }

    #[test]
    fn bound_kind_to_specifiers_lower() {
        let tests = [
            ("0", ">=0"),
            ("0.0", ">=0.0"),
            ("0.0.0", ">=0.0.0"),
            ("0.1", ">=0.1"),
            ("0.0.1", ">=0.0.1"),
            ("0.0.0.1", ">=0.0.0.1"),
            ("1", ">=1"),
            ("1.0.0", ">=1.0.0"),
            ("1.2", ">=1.2"),
            ("1.2.3", ">=1.2.3"),
            ("1.2.3.4", ">=1.2.3.4"),
            ("1.2.3.4a1.post1", ">=1.2.3.4a1.post1"),
        ];

        for (version, expected) in tests {
            let actual = AddBoundsKind::Lower
                .specifiers(Version::from_str(version).unwrap())
                .to_string();
            assert_eq!(actual, expected, "{version}");
        }
    }

    #[test]
    fn bound_kind_to_specifiers_major() {
        let tests = [
            ("0", ">=0, <0.1"),
            ("0.0", ">=0.0, <0.1"),
            ("0.0.0", ">=0.0.0, <0.1.0"),
            ("0.0.0.0", ">=0.0.0.0, <0.1.0.0"),
            ("0.1", ">=0.1, <0.2"),
            ("0.0.1", ">=0.0.1, <0.0.2"),
            ("0.0.1.1", ">=0.0.1.1, <0.0.2.0"),
            ("0.0.0.1", ">=0.0.0.1, <0.0.0.2"),
            ("1", ">=1, <2"),
            ("1.0.0", ">=1.0.0, <2.0.0"),
            ("1.2", ">=1.2, <2.0"),
            ("1.2.3", ">=1.2.3, <2.0.0"),
            ("1.2.3.4", ">=1.2.3.4, <2.0.0.0"),
            ("1.2.3.4a1.post1", ">=1.2.3.4a1.post1, <2.0.0.0"),
        ];

        for (version, expected) in tests {
            let actual = AddBoundsKind::Major
                .specifiers(Version::from_str(version).unwrap())
                .to_string();
            assert_eq!(actual, expected, "{version}");
        }
    }

    #[test]
    fn bound_kind_to_specifiers_minor() {
        let tests = [
            ("0", ">=0, <0.0.1"),
            ("0.0", ">=0.0, <0.0.1"),
            ("0.0.0", ">=0.0.0, <0.0.1"),
            ("0.0.0.0", ">=0.0.0.0, <0.0.1.0"),
            ("0.1", ">=0.1, <0.1.1"),
            ("0.0.1", ">=0.0.1, <0.0.2"),
            ("0.0.1.1", ">=0.0.1.1, <0.0.2.0"),
            ("0.0.0.1", ">=0.0.0.1, <0.0.0.2"),
            ("1", ">=1, <1.1"),
            ("1.0.0", ">=1.0.0, <1.1.0"),
            ("1.2", ">=1.2, <1.3"),
            ("1.2.3", ">=1.2.3, <1.3.0"),
            ("1.2.3.4", ">=1.2.3.4, <1.3.0.0"),
            ("1.2.3.4a1.post1", ">=1.2.3.4a1.post1, <1.3.0.0"),
        ];

        for (version, expected) in tests {
            let actual = AddBoundsKind::Minor
                .specifiers(Version::from_str(version).unwrap())
                .to_string();
            assert_eq!(actual, expected, "{version}");
        }
    }

    #[test]
    fn replace_dependency_updates_every_exact_match() -> Result<()> {
        let mut pyproject = PyProjectTomlMut::from_toml(
            r#"[project]
dependencies = ["anyio<=2", "anyio>=1", "anyio<=2"]

[tool.uv.sources]
anyio = { index = "internal" }
            "#,
            DependencyTarget::PyProjectToml,
        )?;
        let existing = Requirement::from_str("anyio<=2")?.with_origin(RequirementOrigin::Workspace);
        let replacement = Requirement::from_str("anyio<3")?;

        let replaced = pyproject.replace_dependency_declaration(
            &DependencyType::Production,
            &existing,
            &replacement,
        )?;
        assert_eq!(replaced, vec![ArrayEdit::Update(0), ArrayEdit::Update(2)]);

        assert_snapshot!(
            pyproject.to_string(),
            @r#"
[project]
dependencies = ["anyio<3", "anyio>=1", "anyio<3"]

[tool.uv.sources]
anyio = { index = "internal" }
"#
        );
        Ok(())
    }

    #[test]
    fn replace_dependency_declaration_updates_selected_type() -> Result<()> {
        let mut pyproject = PyProjectTomlMut::from_toml(
            r#"[project]
dependencies = ["anyio<=2"]

[project.optional-dependencies]
test = ["anyio<=2"]

[dependency-groups]
dev = ["anyio<=2"]
            "#,
            DependencyTarget::PyProjectToml,
        )?;
        let existing = Requirement::from_str("anyio<=2")?;
        let optional_replacement = Requirement::from_str("anyio<3")?;
        let group_replacement = Requirement::from_str("anyio<4")?;

        let replaced = pyproject.replace_dependency_declaration(
            &DependencyType::Optional(ExtraName::from_str("test")?),
            &existing,
            &optional_replacement,
        )?;
        assert_eq!(replaced, vec![ArrayEdit::Update(0)]);

        let replaced = pyproject.replace_dependency_declaration(
            &DependencyType::Group(GroupName::from_str("dev")?),
            &existing,
            &group_replacement,
        )?;
        assert_eq!(replaced, vec![ArrayEdit::Update(0)]);

        assert_snapshot!(
            pyproject.to_string(),
            @r#"
[project]
dependencies = ["anyio<=2"]

[project.optional-dependencies]
test = ["anyio<3"]

[dependency-groups]
dev = ["anyio<4"]
"#
        );
        Ok(())
    }

    #[test]
    fn remove_preserves_end_of_line_comment_on_previous_item() {
        let toml = r#"
[project]
dependencies = [
    "numpy>=2.4.3", # this comment is clearly essential
    "requests>=2.32.5",
]
"#;
        let mut doc: DocumentMut = toml.parse().unwrap();
        let deps = doc["project"]["dependencies"]
            .as_array_mut()
            .expect("dependencies array");

        let name = PackageName::from_str("requests").unwrap();
        remove_dependency(&name, deps);

        assert_snapshot!(
            doc.to_string(),
            @r#"
[project]
dependencies = [
    "numpy>=2.4.3", # this comment is clearly essential
]
"#
        );
    }

    #[test]
    fn remove_preserves_end_of_line_comment_on_previous_item_middle() {
        let toml = r#"
[project]
dependencies = [
    "numpy>=2.4.3", # numpy comment
    "requests>=2.32.5",
    "flask>=3.0.0",
]
"#;
        let mut doc: DocumentMut = toml.parse().unwrap();
        let deps = doc["project"]["dependencies"]
            .as_array_mut()
            .expect("dependencies array");

        let name = PackageName::from_str("requests").unwrap();
        remove_dependency(&name, deps);

        assert_snapshot!(
            doc.to_string(),
            @r#"
[project]
dependencies = [
    "numpy>=2.4.3", # numpy comment
    "flask>=3.0.0",
]
"#
        );
    }

    #[test]
    fn remove_preserves_own_line_comment_above_removed_item() {
        let toml = r#"
[project]
dependencies = [
    "numpy>=2.4.3",
    # This is a comment about requests
    "requests>=2.32.5",
]
"#;
        let mut doc: DocumentMut = toml.parse().unwrap();
        let deps = doc["project"]["dependencies"]
            .as_array_mut()
            .expect("dependencies array");

        let name = PackageName::from_str("requests").unwrap();
        remove_dependency(&name, deps);

        assert_snapshot!(
            doc.to_string(),
            @r#"
[project]
dependencies = [
    "numpy>=2.4.3",
    # This is a comment about requests
]
"#
        );
    }

    #[test]
    fn remove_item_with_trailing_comment_last() {
        // When the removed item itself has an end-of-line comment and is the last item,
        // toml_edit stores the comment in the array trailing. The comment is preserved
        // (as an own-line comment in the trailing section) but moves position since it
        // can no longer be on the removed item's line.
        let toml = r#"
[project]
dependencies = [
    "requests>=2.32.5",
    "numpy>=2.4.3", # comment on numpy
]
"#;
        let mut doc: DocumentMut = toml.parse().unwrap();
        let deps = doc["project"]["dependencies"]
            .as_array_mut()
            .expect("dependencies array");

        let name = PackageName::from_str("numpy").unwrap();
        remove_dependency(&name, deps);

        assert_snapshot!(
            doc.to_string(),
            @r#"
[project]
dependencies = [
    "requests>=2.32.5",
    # comment on numpy
]
"#
        );
    }

    #[test]
    fn remove_last_item_with_trailing_comment_preserves_previous_comment() {
        let toml = r#"
[project]
dependencies = [
    "boto3", # this is boto3
    "requests", # this is requests
]
"#;
        let mut doc: DocumentMut = toml.parse().unwrap();
        let deps = doc["project"]["dependencies"]
            .as_array_mut()
            .expect("dependencies array");

        let name = PackageName::from_str("requests").unwrap();
        remove_dependency(&name, deps);

        assert_snapshot!(
            doc.to_string(),
            @r#"
[project]
dependencies = [
    "boto3", # this is boto3
    # this is requests
]
"#
        );
    }

    #[test]
    fn remove_item_with_trailing_comment_middle() {
        // When the removed item has an end-of-line comment and is in the middle,
        // toml_edit stores the comment in the next item's prefix. After removal,
        // reformat_array_multiline repositions it as an own-line comment.
        let toml = r#"
[project]
dependencies = [
    "requests>=2.32.5",
    "numpy>=2.4.3", # comment on numpy
    "flask>=3.0.0",
]
"#;
        let mut doc: DocumentMut = toml.parse().unwrap();
        let deps = doc["project"]["dependencies"]
            .as_array_mut()
            .expect("dependencies array");

        let name = PackageName::from_str("numpy").unwrap();
        remove_dependency(&name, deps);

        assert_snapshot!(
            doc.to_string(),
            @r#"
[project]
dependencies = [
    "requests>=2.32.5",
    # comment on numpy
    "flask>=3.0.0",
]
"#
        );
    }

    #[test]
    fn remove_first_item_with_trailing_comment_preserves_leading_comments() {
        let toml = r#"
[project]
dependencies = [
    # should be in alphabetical order
    "basedmypy[faster-cache]>=2.8.1", # this is a comment
    "basedpyright>=1.18.2,<2.0.0",
]
"#;
        let mut doc: DocumentMut = toml.parse().unwrap();
        let deps = doc["project"]["dependencies"]
            .as_array_mut()
            .expect("dependencies array");

        let name = PackageName::from_str("basedmypy").unwrap();
        remove_dependency(&name, deps);

        assert_snapshot!(
            doc.to_string(),
            @r#"
[project]
dependencies = [
    # should be in alphabetical order
    # this is a comment
    "basedpyright>=1.18.2,<2.0.0",
]
"#
        );
    }

    #[test]
    fn remove_multiple_adjacent_matches_preserves_comment_order() {
        let toml = r#"
[project]
dependencies = [
    "iniconfig>=2.0.0", # comment on iniconfig
    "typing-extensions>=4.0.0 ; python_version < '3.11'", # comment on first typing-extensions
    "typing-extensions>=4.0.0 ; python_version >= '3.11'",
    "sniffio>=1.3.0",
]
"#;
        let mut doc: DocumentMut = toml.parse().unwrap();
        let deps = doc["project"]["dependencies"]
            .as_array_mut()
            .expect("dependencies array");

        let name = PackageName::from_str("typing-extensions").unwrap();
        remove_dependency(&name, deps);

        assert_snapshot!(
            doc.to_string(),
            @r#"
[project]
dependencies = [
    "iniconfig>=2.0.0", # comment on iniconfig
    # comment on first typing-extensions
    "sniffio>=1.3.0",
]
"#
        );
    }

    #[test]
    fn add_index_syncs_format_on_url_update() {
        let toml = r#"
[[tool.uv.index]]
name = "index"
url = "https://example.com/flat/"
format = "flat"
"#;

        let mut doc = PyProjectTomlMut::from_toml(toml, DependencyTarget::PyProjectToml).unwrap();

        // The URL spelling changes, but the canonical URL does not, so format should be preserved.
        let equivalent_index = Index::from_str("index=https://example.com/flat").unwrap();
        doc.add_index(&equivalent_index, Path::new(".")).unwrap();

        assert_snapshot!(doc.to_string(), @r#"

[[tool.uv.index]]
name = "index"
url = "https://example.com/flat"
format = "flat"
"#);

        let new_index = Index::from_str("index=https://pypi.org/simple").unwrap();
        doc.add_index(&new_index, Path::new(".")).unwrap();

        assert_snapshot!(doc.to_string(), @r#"

[[tool.uv.index]]
name = "index"
url = "https://pypi.org/simple"
"#);
    }

    #[cfg(windows)]
    #[test]
    fn add_index_preserves_format_when_windows_path_unchanged() -> Result<()> {
        let toml = r#"
[[tool.uv.index]]
name = "index"
url = 'C:\links'
format = "flat"
"#;

        let mut doc = PyProjectTomlMut::from_toml(toml, DependencyTarget::PyProjectToml)?;

        let new_index = Index::from_str(r"index=C:\links")?;
        doc.add_index(&new_index, &std::env::current_dir()?)?;

        let index = doc.doc["tool"]["uv"]["index"]
            .as_array_of_tables()
            .and_then(|indexes| indexes.get(0))
            .expect("index table");
        assert_eq!(
            index.get("url").and_then(|item| item.as_str()),
            Some("C:/links")
        );
        assert_eq!(
            index.get("format").and_then(|item| item.as_str()),
            Some("flat")
        );

        Ok(())
    }
}
