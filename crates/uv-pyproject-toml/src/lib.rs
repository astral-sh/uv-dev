//! Types for deserializing the standardized fields in `pyproject.toml`.
//!
//! This crate is limited to the data model defined by the Python packaging standards. It does not
//! read from the filesystem, interpret tool-specific configuration, or apply build-backend policy.
//! All public types implement [`Deserialize`], and the generic wire types let callers retain raw
//! strings or specialized map types. Consumers can therefore use their existing TOML parser,
//! diagnostics, and eager or deferred validation strategy.
//!
//! The data model is derived from the MIT-licensed
//! [`pyproject-toml`](https://github.com/PyO3/pyproject-toml-rs) crate.

use std::collections::BTreeMap;
use std::hash::{BuildHasher, Hash};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use indexmap::IndexMap;
use serde::de::{IntoDeserializer, MapAccess};
use serde::{Deserialize, Deserializer, Serialize};
use uv_normalize::{ExtraName, GroupName, PackageName};
use uv_pep440::{Version, VersionSpecifiers};
use uv_pep508::Requirement;

/// The `[build-system]` section of a `pyproject.toml`, as specified in PEP 517.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub struct BuildSystem<Requirement = uv_pep508::Requirement, BackendPath = Vec<String>> {
    /// The PEP 508 dependencies required to execute the build system.
    pub requires: Vec<Requirement>,
    /// The Python object used to perform the build.
    pub build_backend: Option<String>,
    /// The directories containing an in-tree build backend.
    pub backend_path: Option<BackendPath>,
}

/// The resolution-related fields of a PEP 621 project table.
///
/// The field types are parameters so consumers can preserve their own parsing and validation
/// policy without using `serde(flatten)`. For example, a metadata reader can retain requirement
/// strings for contextual lowering, while another consumer can deserialize them eagerly.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub struct ProjectWire<
    Name = PackageName,
    ProjectVersion = Option<Version>,
    RequiresPython = Option<VersionSpecifiers>,
    Dependencies = Option<Vec<uv_pep508::Requirement>>,
    OptionalDependencyGroups = Option<OptionalDependencies<uv_pep508::Requirement>>,
    GuiScripts = Option<Ignored>,
    Scripts = Option<Ignored>,
> {
    /// The project name.
    pub name: Name,
    /// The project version.
    pub version: ProjectVersion,
    /// Fields provided dynamically by the build backend.
    pub dynamic: Option<Vec<String>>,
    /// The Python versions supported by the project.
    pub requires_python: RequiresPython,
    /// Runtime dependencies.
    pub dependencies: Dependencies,
    /// Dependencies grouped by optional feature.
    pub optional_dependencies: OptionalDependencyGroups,
    /// Whether the project defines GUI scripts.
    pub gui_scripts: GuiScripts,
    /// Whether the project defines console scripts.
    pub scripts: Scripts,
}

/// A value that is intentionally ignored during deserialization.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ignored;

impl<'de> Deserialize<'de> for Ignored {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        serde::de::IgnoredAny::deserialize(deserializer).map(|_| Self)
    }
}

/// The standardized sections of a `pyproject.toml`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub struct PyProjectToml {
    /// Build-system metadata defined by PEP 517.
    pub build_system: Option<BuildSystem>,
    /// Project metadata defined by PEP 621.
    pub project: Option<Project>,
    /// Dependency groups defined by PEP 735.
    pub dependency_groups: Option<DependencyGroups>,
}

impl PyProjectToml {
    /// Parse a `pyproject.toml` document.
    pub fn from_toml(contents: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(contents)
    }
}

impl FromStr for PyProjectToml {
    type Err = toml::de::Error;

    fn from_str(contents: &str) -> Result<Self, Self::Err> {
        Self::from_toml(contents)
    }
}

/// The `[project]` section of a `pyproject.toml`, as specified in PEP 621.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub struct Project {
    /// The normalized project name.
    pub name: PackageName,
    /// The project version.
    pub version: Option<Version>,
    /// A one-line summary of the project.
    pub description: Option<String>,
    /// The full project description.
    pub readme: Option<Readme>,
    /// The Python versions supported by the project.
    pub requires_python: Option<VersionSpecifiers>,
    /// The license under which the project is distributed.
    pub license: Option<License>,
    /// The paths to license files included in the distribution.
    pub license_files: Option<Vec<String>>,
    /// The people or organizations considered authors of the project.
    pub authors: Option<Vec<Contact>>,
    /// The people or organizations maintaining the project.
    pub maintainers: Option<Vec<Contact>>,
    /// Search keywords for the project.
    pub keywords: Option<Vec<String>>,
    /// Trove classifiers for the project.
    pub classifiers: Option<Vec<String>>,
    /// Project-related URLs.
    pub urls: Option<IndexMap<String, String>>,
    /// Arbitrary entry-point groups.
    pub entry_points: Option<IndexMap<String, IndexMap<String, String>>>,
    /// Console-script entry points.
    pub scripts: Option<IndexMap<String, String>>,
    /// GUI-script entry points.
    pub gui_scripts: Option<IndexMap<String, String>>,
    /// Runtime dependencies.
    pub dependencies: Option<Vec<Requirement>>,
    /// Dependencies grouped by optional feature.
    pub optional_dependencies: Option<OptionalDependencies<Requirement>>,
    /// Import names exclusively provided by the project, as defined by PEP 794.
    pub import_names: Option<Vec<String>>,
    /// Import namespaces provided by the project, as defined by PEP 794.
    pub import_namespaces: Option<Vec<String>>,
    /// Fields provided dynamically by the build backend.
    pub dynamic: Option<Vec<String>>,
}

/// A map implementation used by standardized `pyproject.toml` tables.
///
/// This trait supports both insertion-ordered maps for external tooling and sorted maps for uv's
/// internal representations. It is an implementation detail of the generic table wrappers.
#[doc(hidden)]
pub trait TableMap<K, V>: Default {
    /// An iterator over key-value pairs.
    type Iter<'a>: Iterator<Item = (&'a K, &'a V)>
    where
        Self: 'a,
        K: 'a,
        V: 'a;

    /// An iterator over keys.
    type Keys<'a>: Iterator<Item = &'a K>
    where
        Self: 'a,
        K: 'a,
        V: 'a;

    /// Insert a value, returning the previous value for the key, if present.
    fn insert(&mut self, key: K, value: V) -> Option<V>;

    /// Return the value for a key.
    fn get(&self, key: &K) -> Option<&V>;

    /// Return whether the map contains a key.
    fn contains_key(&self, key: &K) -> bool;

    /// Iterate over key-value pairs.
    fn iter(&self) -> Self::Iter<'_>;

    /// Iterate over keys.
    fn keys(&self) -> Self::Keys<'_>;
}

impl<K, V> TableMap<K, V> for BTreeMap<K, V>
where
    K: Ord,
{
    type Iter<'a>
        = std::collections::btree_map::Iter<'a, K, V>
    where
        K: 'a,
        V: 'a;
    type Keys<'a>
        = std::collections::btree_map::Keys<'a, K, V>
    where
        K: 'a,
        V: 'a;

    fn insert(&mut self, key: K, value: V) -> Option<V> {
        Self::insert(self, key, value)
    }

    fn get(&self, key: &K) -> Option<&V> {
        Self::get(self, key)
    }

    fn contains_key(&self, key: &K) -> bool {
        Self::contains_key(self, key)
    }

    fn iter(&self) -> Self::Iter<'_> {
        Self::iter(self)
    }

    fn keys(&self) -> Self::Keys<'_> {
        Self::keys(self)
    }
}

impl<K, V, S> TableMap<K, V> for IndexMap<K, V, S>
where
    K: Eq + Hash,
    S: BuildHasher + Default,
{
    type Iter<'a>
        = indexmap::map::Iter<'a, K, V>
    where
        K: 'a,
        V: 'a,
        S: 'a;
    type Keys<'a>
        = indexmap::map::Keys<'a, K, V>
    where
        K: 'a,
        V: 'a,
        S: 'a;

    fn insert(&mut self, key: K, value: V) -> Option<V> {
        Self::insert(self, key, value)
    }

    fn get(&self, key: &K) -> Option<&V> {
        Self::get(self, key)
    }

    fn contains_key(&self, key: &K) -> bool {
        Self::contains_key(self, key)
    }

    fn iter(&self) -> Self::Iter<'_> {
        Self::iter(self)
    }

    fn keys(&self) -> Self::Keys<'_> {
        Self::keys(self)
    }
}

/// Dependencies grouped by normalized optional-feature name.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct OptionalDependencies<
    Requirement = uv_pep508::Requirement,
    Map = IndexMap<ExtraName, Vec<Requirement>>,
>(Map, #[serde(skip)] std::marker::PhantomData<Requirement>);

impl<Requirement, Map> OptionalDependencies<Requirement, Map> {
    /// Consume the wrapper and return the underlying map.
    pub fn into_inner(self) -> Map {
        self.0
    }
}

impl<Requirement, Map> OptionalDependencies<Requirement, Map>
where
    Map: TableMap<ExtraName, Vec<Requirement>>,
{
    /// Return an iterator over optional dependency groups.
    pub fn iter(&self) -> Map::Iter<'_> {
        self.0.iter()
    }

    /// Return the requirements for an optional dependency group.
    pub fn get(&self, extra: &ExtraName) -> Option<&Vec<Requirement>> {
        self.0.get(extra)
    }

    /// Return whether an optional dependency group exists.
    pub fn contains_key(&self, extra: &ExtraName) -> bool {
        self.0.contains_key(extra)
    }

    /// Return an iterator over optional dependency group names.
    pub fn keys(&self) -> Map::Keys<'_> {
        self.0.keys()
    }
}

impl<Requirement, Map> Default for OptionalDependencies<Requirement, Map>
where
    Map: Default,
{
    fn default() -> Self {
        Self(Map::default(), std::marker::PhantomData)
    }
}

impl<Requirement, Map> std::ops::Deref for OptionalDependencies<Requirement, Map> {
    type Target = Map;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a, Requirement: 'a, Map> IntoIterator for &'a OptionalDependencies<Requirement, Map>
where
    Map: TableMap<ExtraName, Vec<Requirement>>,
{
    type Item = (&'a ExtraName, &'a Vec<Requirement>);
    type IntoIter = Map::Iter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'de, Requirement, Map> Deserialize<'de> for OptionalDependencies<Requirement, Map>
where
    Requirement: Deserialize<'de>,
    Map: TableMap<ExtraName, Vec<Requirement>>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct OptionalDependenciesVisitor<Requirement, Map>(
            std::marker::PhantomData<(Requirement, Map)>,
        );

        impl<'de, Requirement, Map> serde::de::Visitor<'de>
            for OptionalDependenciesVisitor<Requirement, Map>
        where
            Requirement: Deserialize<'de>,
            Map: TableMap<ExtraName, Vec<Requirement>>,
        {
            type Value = OptionalDependencies<Requirement, Map>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a table with unique normalized extra names")
            }

            fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut dependencies = Map::default();
                while let Some((name, requirements)) =
                    access.next_entry::<ExtraName, Vec<Requirement>>()?
                {
                    if dependencies.insert(name.clone(), requirements).is_some() {
                        return Err(serde::de::Error::custom(format!(
                            "duplicate normalized extra name `{name}`"
                        )));
                    }
                }
                Ok(OptionalDependencies(dependencies, std::marker::PhantomData))
            }
        }

        deserializer.deserialize_map(OptionalDependenciesVisitor(std::marker::PhantomData))
    }
}

/// The `project.readme` field.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged, rename_all_fields = "kebab-case")]
pub enum Readme {
    /// A path to the project readme.
    String(PathBuf),
    /// A path to the project readme with explicit metadata.
    File {
        /// The path to the readme.
        file: PathBuf,
        /// The readme's content type.
        content_type: String,
        /// The readme's character encoding.
        charset: Option<String>,
    },
    /// An inline project readme.
    Text {
        /// The readme contents.
        text: String,
        /// The readme's content type.
        content_type: String,
        /// The readme's character encoding.
        charset: Option<String>,
    },
}

impl Readme {
    /// Return the path to the readme, if it is stored in a file.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::String(path) => Some(path),
            Self::File { file, .. } => Some(file),
            Self::Text { .. } => None,
        }
    }
}

/// The `project.license` field.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum License {
    /// An SPDX license expression, as defined by PEP 639.
    Spdx(String),
    /// Inline license text.
    Text {
        /// The full license text.
        text: String,
    },
    /// A path to a file containing the license text.
    File {
        /// The path to the license file.
        file: String,
    },
}

impl License {
    /// Return the path to the license file, if one is specified.
    pub fn file(&self) -> Option<&str> {
        if let Self::File { file } = self {
            Some(file)
        } else {
            None
        }
    }
}

/// An entry in `project.authors` or `project.maintainers`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(
    untagged,
    deny_unknown_fields,
    expecting = "a table with 'name' and/or 'email' keys"
)]
pub enum Contact {
    /// A contact with a name and email address.
    NameEmail { name: String, email: String },
    /// A contact with only a name.
    Name { name: String },
    /// A contact with only an email address.
    Email { email: String },
}

impl Contact {
    /// Return the contact name, if one is provided.
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::NameEmail { name, .. } | Self::Name { name } => Some(name),
            Self::Email { .. } => None,
        }
    }

    /// Return the contact email address, if one is provided.
    pub fn email(&self) -> Option<&str> {
        match self {
            Self::NameEmail { email, .. } | Self::Email { email } => Some(email),
            Self::Name { .. } => None,
        }
    }
}

/// The `[dependency-groups]` section of a `pyproject.toml`, as specified in PEP 735.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct DependencyGroups<
    Requirement = uv_pep508::Requirement,
    Object = UnsupportedDependencyGroupObject,
    Map = IndexMap<GroupName, Vec<DependencyGroupSpecifier<Requirement, Object>>>,
>(
    Map,
    #[serde(skip)] std::marker::PhantomData<(Requirement, Object)>,
);

impl<Requirement, Object, Map> DependencyGroups<Requirement, Object, Map> {
    /// Consume the wrapper and return the underlying map.
    pub fn into_inner(self) -> Map {
        self.0
    }
}

impl<Requirement, Object, Map> DependencyGroups<Requirement, Object, Map>
where
    Map: TableMap<GroupName, Vec<DependencyGroupSpecifier<Requirement, Object>>>,
{
    /// Return an iterator over the dependency groups.
    pub fn iter(&self) -> Map::Iter<'_> {
        self.0.iter()
    }

    /// Return the specifiers for a dependency group.
    pub fn get(
        &self,
        group: &GroupName,
    ) -> Option<&Vec<DependencyGroupSpecifier<Requirement, Object>>> {
        self.0.get(group)
    }

    /// Return whether a dependency group exists.
    pub fn contains_key(&self, group: &GroupName) -> bool {
        self.0.contains_key(group)
    }

    /// Return an iterator over dependency group names.
    pub fn keys(&self) -> Map::Keys<'_> {
        self.0.keys()
    }
}

impl<Requirement, Object, Map> Default for DependencyGroups<Requirement, Object, Map>
where
    Map: Default,
{
    fn default() -> Self {
        Self(Map::default(), std::marker::PhantomData)
    }
}

impl<Requirement, Object, Map> std::ops::Deref for DependencyGroups<Requirement, Object, Map> {
    type Target = Map;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'de, Requirement, Object, Map> Deserialize<'de> for DependencyGroups<Requirement, Object, Map>
where
    Requirement: Deserialize<'de>,
    Object: Deserialize<'de>,
    Map: TableMap<GroupName, Vec<DependencyGroupSpecifier<Requirement, Object>>>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct DependencyGroupsVisitor<Requirement, Object, Map>(
            std::marker::PhantomData<(Requirement, Object, Map)>,
        );

        impl<'de, Requirement, Object, Map> serde::de::Visitor<'de>
            for DependencyGroupsVisitor<Requirement, Object, Map>
        where
            Requirement: Deserialize<'de>,
            Object: Deserialize<'de>,
            Map: TableMap<GroupName, Vec<DependencyGroupSpecifier<Requirement, Object>>>,
        {
            type Value = DependencyGroups<Requirement, Object, Map>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a table with unique normalized dependency group names")
            }

            fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut groups = Map::default();
                while let Some((name, specifiers)) = access
                    .next_entry::<GroupName, Vec<DependencyGroupSpecifier<Requirement, Object>>>()?
                {
                    if groups.insert(name.clone(), specifiers).is_some() {
                        return Err(serde::de::Error::custom(format!(
                            "duplicate normalized dependency group name `{name}`"
                        )));
                    }
                }
                Ok(DependencyGroups(groups, std::marker::PhantomData))
            }
        }

        deserializer.deserialize_map(DependencyGroupsVisitor(std::marker::PhantomData))
    }
}

impl<'a, Requirement: 'a, Object: 'a, Map> IntoIterator
    for &'a DependencyGroups<Requirement, Object, Map>
where
    Map: TableMap<GroupName, Vec<DependencyGroupSpecifier<Requirement, Object>>>,
{
    type Item = (
        &'a GroupName,
        &'a Vec<DependencyGroupSpecifier<Requirement, Object>>,
    );
    type IntoIter = Map::Iter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// An item in a dependency group.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum DependencyGroupSpecifier<
    Requirement = uv_pep508::Requirement,
    Object = UnsupportedDependencyGroupObject,
> {
    /// A PEP 508 requirement.
    Requirement(Requirement),
    /// A reference to another dependency group.
    IncludeGroup {
        /// The dependency group to include.
        #[serde(rename = "include-group")]
        include_group: GroupName,
    },
    /// An unrecognized dependency object retained for caller-specific validation.
    Object(Object),
}

impl<'de, Requirement, Object> Deserialize<'de> for DependencyGroupSpecifier<Requirement, Object>
where
    Requirement: Deserialize<'de>,
    Object: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct DependencyGroupSpecifierVisitor<Requirement, Object>(
            std::marker::PhantomData<(Requirement, Object)>,
        );

        impl<'de, Requirement, Object> serde::de::Visitor<'de>
            for DependencyGroupSpecifierVisitor<Requirement, Object>
        where
            Requirement: Deserialize<'de>,
            Object: Deserialize<'de>,
        {
            type Value = DependencyGroupSpecifier<Requirement, Object>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a requirement string or a dependency-group table")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Requirement::deserialize(value.into_deserializer()).map(Self::Value::Requirement)
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut values = BTreeMap::<String, String>::new();
                while let Some((key, value)) = map.next_entry()? {
                    values.insert(key, value);
                }

                if values.is_empty() {
                    return Err(serde::de::Error::custom("missing field `include-group`"));
                }

                if values.len() == 1
                    && let Some(include_group) = values
                        .get("include-group")
                        .map(String::as_str)
                        .map(GroupName::from_str)
                        .transpose()
                        .map_err(serde::de::Error::custom)?
                {
                    return Ok(Self::Value::IncludeGroup { include_group });
                }

                Object::deserialize(values.into_deserializer()).map(Self::Value::Object)
            }
        }

        deserializer.deserialize_any(DependencyGroupSpecifierVisitor(std::marker::PhantomData))
    }
}

/// A dependency object is not supported by the concrete standards model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnsupportedDependencyGroupObject {}

impl Serialize for UnsupportedDependencyGroupObject {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match *self {}
    }
}

impl<'de> Deserialize<'de> for UnsupportedDependencyGroupObject {
    fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Err(serde::de::Error::custom(
            "expected a requirement string or an `include-group` table",
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::str::FromStr;

    use serde::Deserialize;
    use uv_normalize::{ExtraName, GroupName, PackageName};
    use uv_pep440::{Version, VersionSpecifiers};
    use uv_pep508::Requirement;

    use super::{
        DependencyGroupSpecifier, DependencyGroups, Ignored, License, OptionalDependencies,
        ProjectWire, PyProjectToml, Readme,
    };

    #[test]
    fn parse_pyproject_toml() {
        let pyproject_toml = PyProjectToml::from_toml(
            r#"
            [build-system]
            requires = ["uv_build>=0.8.0,<0.9.0"]
            build-backend = "uv_build"

            [project]
            name = "spam"
            version = "2020.0.0"
            description = "Lovely Spam! Wonderful Spam!"
            readme = "README.rst"
            requires-python = ">=3.8"
            license = "MIT OR BSD-3-Clause"
            license-files = ["LICENSE*"]
            authors = [{ name = "Tzu-Ping Chung" }]
            dependencies = ["httpx", "django>2.1; os_name != 'nt'"]

            [project.optional-dependencies]
            test = ["pytest", "pytest-cov[all]"]

            [project.scripts]
            spam-cli = "spam:main_cli"

            [dependency-groups]
            dev = ["ruff", { include-group = "test" }]
            test = ["pytest"]
            "#,
        )
        .unwrap();

        let build_system = pyproject_toml.build_system.as_ref().unwrap();
        assert_eq!(
            build_system.requires,
            [Requirement::from_str("uv_build>=0.8.0,<0.9.0").unwrap()]
        );
        assert_eq!(build_system.build_backend.as_deref(), Some("uv_build"));

        let project = pyproject_toml.project.as_ref().unwrap();
        assert_eq!(project.name, PackageName::from_str("spam").unwrap());
        assert_eq!(
            project.version,
            Some(Version::from_str("2020.0.0").unwrap())
        );
        assert_eq!(
            project.requires_python,
            Some(VersionSpecifiers::from_str(">=3.8").unwrap())
        );
        assert_eq!(project.readme, Some(Readme::String("README.rst".into())));
        assert_eq!(
            project.license,
            Some(License::Spdx("MIT OR BSD-3-Clause".to_string()))
        );
        assert_eq!(
            project.optional_dependencies.as_ref().unwrap()[&ExtraName::from_str("test").unwrap()],
            [
                Requirement::from_str("pytest").unwrap(),
                Requirement::from_str("pytest-cov[all]").unwrap(),
            ]
        );

        let groups = pyproject_toml.dependency_groups.as_ref().unwrap();
        assert_eq!(
            groups
                .iter()
                .find(|(name, _)| *name == &GroupName::from_str("dev").unwrap())
                .map(|(_, specifiers)| specifiers.as_slice()),
            Some(
                [
                    DependencyGroupSpecifier::Requirement(Requirement::from_str("ruff").unwrap()),
                    DependencyGroupSpecifier::IncludeGroup {
                        include_group: GroupName::from_str("test").unwrap(),
                    },
                ]
                .as_slice()
            )
        );
    }

    #[test]
    fn ignores_tool_specific_configuration() {
        let pyproject_toml = PyProjectToml::from_toml(
            r"
            [tool.uv]
            managed = false
            ",
        )
        .unwrap();

        assert_eq!(pyproject_toml, PyProjectToml::default());
    }

    #[test]
    fn rejects_invalid_contact() {
        let error = PyProjectToml::from_toml(
            r#"
            [project]
            name = "spam"
            authors = [{ name = "Ferris", email = 1 }]
            "#,
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("a table with 'name' and/or 'email' keys")
        );
    }

    #[test]
    fn rejects_duplicate_normalized_extras() {
        let error = PyProjectToml::from_toml(
            r#"
            [project]
            name = "spam"

            [project.optional-dependencies]
            dev_test = ["pytest"]
            dev-test = ["ruff"]
            "#,
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("duplicate normalized extra name `dev-test`")
        );
    }

    #[test]
    fn supports_raw_uv_representations() {
        type RawOptionalDependencies =
            OptionalDependencies<String, BTreeMap<ExtraName, Vec<String>>>;
        type RawProject = ProjectWire<
            Option<PackageName>,
            Option<Version>,
            Option<String>,
            Option<Vec<String>>,
            Option<RawOptionalDependencies>,
            Option<Ignored>,
            Option<Ignored>,
        >;
        type RawObject = BTreeMap<String, String>;
        type RawSpecifier = DependencyGroupSpecifier<String, RawObject>;
        type RawGroups =
            DependencyGroups<String, RawObject, BTreeMap<GroupName, Vec<RawSpecifier>>>;

        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "kebab-case")]
        struct RawPyProjectToml {
            project: RawProject,
            dependency_groups: RawGroups,
        }

        let pyproject: RawPyProjectToml = toml::from_str(
            r#"
            [project]
            name = "spam"
            requires-python = "not parsed eagerly"
            dependencies = ["not parsed eagerly @@@"]

            [project.optional-dependencies]
            dev = ["also retained as source text @@@"]

            [dependency-groups]
            dev = ["raw requirement @@@", { path = "." }]
            "#,
        )
        .unwrap();

        assert_eq!(
            pyproject.project.requires_python.as_deref(),
            Some("not parsed eagerly")
        );
        assert_eq!(
            pyproject.project.dependencies.as_deref(),
            Some(["not parsed eagerly @@@".to_string()].as_slice())
        );
        assert!(matches!(
            &pyproject.dependency_groups[&GroupName::from_str("dev").unwrap()][1],
            DependencyGroupSpecifier::Object(object) if object["path"] == "."
        ));

        let error = toml::from_str::<RawPyProjectToml>(
            r#"
            [project]
            name = "spam"

            [dependency-groups]
            dev = [{}]
            "#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("missing field `include-group`"));
    }

    #[test]
    fn rejects_dependency_objects_in_concrete_model() {
        let error = PyProjectToml::from_toml(
            r#"
            [dependency-groups]
            dev = [{ path = "." }]
            "#,
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("expected a requirement string or an `include-group` table")
        );
    }

    #[test]
    fn rejects_duplicate_normalized_groups() {
        let error = PyProjectToml::from_toml(
            r#"
            [dependency-groups]
            dev_test = ["pytest"]
            dev-test = ["ruff"]
            "#,
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("duplicate normalized dependency group name `dev-test`")
        );
    }
}
