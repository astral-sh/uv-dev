//! Typed representation of the vendored Packse scenario TOML files.
//!
//! The nested TOML tables map directly onto [`Scenario::packages`]:
//! `[packages.<name>.versions.<version>]` becomes a [`PackageName`] key, then a [`Version`] key.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer};

use uv_configuration::TargetTriple;
use uv_distribution_filename::WheelFilename;
use uv_normalize::{ExtraName, PackageName};
use uv_pep440::{Version, VersionSpecifiers};
use uv_pep508::{MarkerTree, Requirement};
use uv_python::PythonVersion;

/// A complete packse scenario definition.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// The scenario name (e.g., `"fork-basic"`).
    pub name: String,

    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,

    /// Packages keyed by the TOML segment in `[packages.<name>]`.
    #[serde(default)]
    pub packages: BTreeMap<PackageName, Package>,

    /// The root (entrypoint) requirements.
    pub root: RootPackage,

    /// What we expect the resolver to produce.
    pub expected: Expected,

    /// Metadata about the Python environment.
    #[serde(default)]
    pub environment: Environment,

    /// Additional resolver options.
    #[serde(default)]
    pub resolver_options: ResolverOptions,

    /// Options for generating tests from this scenario.
    #[serde(default)]
    pub testgen: TestGeneration,
}

impl Scenario {
    /// Parse a single scenario from a TOML file path.
    pub fn from_path(path: &Path) -> Result<Self> {
        let contents = fs_err::read_to_string(path)
            .with_context(|| format!("failed to read scenario file `{}`", path.display()))?;
        toml::from_str(&contents)
            .with_context(|| format!("failed to parse scenario file `{}`", path.display()))
    }

    /// Construct an otherwise-empty scenario for indexes that should only expose vendored files.
    pub fn empty() -> Self {
        Self {
            name: String::new(),
            description: None,
            packages: BTreeMap::new(),
            root: RootPackage {
                requires_python: None,
                requires: Vec::new(),
            },
            expected: Expected {
                satisfiable: true,
                packages: BTreeMap::new(),
                explanation: None,
            },
            environment: Environment::default(),
            resolver_options: ResolverOptions::default(),
            testgen: TestGeneration::default(),
        }
    }
}

/// A package with one or more versions.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub versions: BTreeMap<Version, PackageMetadata>,
}

/// Metadata for a single version of a package.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PackageMetadata {
    /// The `Requires-Python` specifier. Defaults to `">=3.12"`; `false` omits it.
    #[serde(
        default = "default_requires_python",
        deserialize_with = "deserialize_requires_python"
    )]
    pub requires_python: Option<VersionSpecifiers>,

    /// Dependency requirements.
    #[serde(default)]
    pub requires: Vec<Requirement>,

    /// Build requirements for the generated source distribution.
    #[serde(default)]
    pub build_requires: Vec<Requirement>,

    /// Literal `Requires-Dist` lines for tests of invalid or unusual metadata.
    #[serde(default)]
    pub raw_requires_dist: Vec<String>,

    /// Extra names mapped to their optional dependency requirements.
    #[serde(default)]
    pub extras: BTreeMap<ExtraName, Vec<Requirement>>,

    /// Console script names and their Python entry points.
    #[serde(default)]
    pub scripts: BTreeMap<String, String>,

    /// Import package name, when different from the normalized distribution name.
    #[serde(default)]
    pub module_name: Option<String>,

    /// Contents of the generated package's `__init__.py`.
    #[serde(default)]
    pub init_py: Option<String>,

    /// Whether to produce a source distribution, and optionally its metadata.
    #[serde(
        default = "default_artifact",
        deserialize_with = "deserialize_artifact"
    )]
    pub sdist: Option<ArtifactMetadata>,

    /// Build backend included in a generated source distribution.
    #[serde(default)]
    pub sdist_backend: SdistBackend,

    /// Project directory beneath the source archive's top-level directory.
    #[serde(default)]
    pub sdist_subdirectory: Option<String>,

    /// Whether to produce wheels, and optionally their shared metadata.
    #[serde(
        default = "default_artifact",
        deserialize_with = "deserialize_artifact"
    )]
    pub wheel: Option<ArtifactMetadata>,

    /// Whether this version is yanked, including an optional reason.
    #[serde(default)]
    pub yanked: Yanked,

    /// Upload time shared by artifacts without a more specific upload time.
    #[serde(default)]
    pub upload_time: Option<String>,

    /// Specific wheel tags to produce (e.g., `["cp312-abi3-win_amd64"]`).
    /// An empty list means produce only the default `py3-none-any` wheel.
    #[serde(default)]
    pub wheel_tags: Vec<WheelTag>,
}

fn deserialize_requires_python<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<VersionSpecifiers>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Value {
        Specifiers(VersionSpecifiers),
        Enabled(bool),
    }

    match Value::deserialize(deserializer)? {
        Value::Specifiers(specifiers) => Ok(Some(specifiers)),
        Value::Enabled(false) => Ok(None),
        Value::Enabled(true) => Err(serde::de::Error::custom(
            "requires_python must be a version specifier or false",
        )),
    }
}

/// Backend used by a generated source distribution.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum SdistBackend {
    /// A self-contained PEP 517 backend requiring no packages from an index.
    InTree,
    /// Hatchling, for coverage that requires a separate build backend.
    #[default]
    Hatchling,
    /// A legacy setuptools project without `pyproject.toml` or static metadata.
    LegacySetuptools,
}

/// Yanked release metadata exposed by the Simple API.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(from = "YankedValue")]
pub enum Yanked {
    #[default]
    No,
    Yes,
    Reason(String),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum YankedValue {
    Enabled(bool),
    Reason(String),
}

impl From<YankedValue> for Yanked {
    fn from(value: YankedValue) -> Self {
        match value {
            YankedValue::Enabled(false) => Self::No,
            YankedValue::Enabled(true) => Self::Yes,
            YankedValue::Reason(reason) => Self::Reason(reason),
        }
    }
}

impl Yanked {
    pub fn is_yanked(&self) -> bool {
        !matches!(self, Self::No)
    }

    pub(super) fn simple_api_value(&self) -> Option<serde_json::Value> {
        match self {
            Self::No => None,
            Self::Yes => Some(serde_json::Value::Bool(true)),
            Self::Reason(reason) => Some(serde_json::Value::String(reason.clone())),
        }
    }
}

fn deserialize_artifact<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<ArtifactMetadata>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Debug, Deserialize)]
    #[serde(untagged)]
    enum Helper {
        Bool(bool),
        Metadata(ArtifactMetadata),
    }

    match Helper::deserialize(deserializer)? {
        Helper::Bool(false) => Ok(None),
        Helper::Bool(true) => Ok(Some(ArtifactMetadata::default())),
        Helper::Metadata(metadata) => Ok(Some(metadata)),
    }
}

/// Metadata advertised for an artifact by the Packse Simple API.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactMetadata {
    pub upload_time: Option<String>,
}

/// A validated three-component compatibility tag for generated wheels.
#[derive(Clone, Debug)]
pub struct WheelTag(String);

impl WheelTag {
    /// Return the compatibility tag as it should appear in a wheel filename.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for WheelTag {
    type Err = String;

    fn from_str(tag: &str) -> Result<Self, Self::Err> {
        if tag.split('-').count() != 3 {
            return Err(format!(
                "wheel tag `{tag}` must have exactly three components"
            ));
        }
        WheelFilename::from_str(&format!("package-0-{tag}.whl"))
            .map_err(|error| format!("wheel tag `{tag}` is invalid: {error}"))?;
        Ok(Self(tag.to_string()))
    }
}

impl<'de> Deserialize<'de> for WheelTag {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let tag = String::deserialize(deserializer)?;
        Self::from_str(&tag).map_err(serde::de::Error::custom)
    }
}

/// The root/entrypoint package.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootPackage {
    /// `Requires-Python` for the root.
    #[serde(default = "default_requires_python")]
    pub requires_python: Option<VersionSpecifiers>,

    /// Top-level requirements.
    #[serde(default)]
    pub requires: Vec<Requirement>,
}

/// Expected resolution outcome.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expected {
    /// Whether the scenario is satisfiable.
    pub satisfiable: bool,

    /// Expected installed package names mapped to resolved versions.
    #[serde(default)]
    pub packages: BTreeMap<PackageName, Version>,

    /// Optional explanation.
    #[serde(default)]
    pub explanation: Option<String>,
}

/// Python environment metadata.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    /// Active Python version.
    #[serde(default = "default_python")]
    pub python: PythonVersion,

    /// Additional Python versions available on the system.
    #[serde(default)]
    pub additional_python: Vec<PythonVersion>,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            python: default_python(),
            additional_python: Vec::new(),
        }
    }
}

/// Additional resolver options.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolverOptions {
    /// Version selection strategy.
    #[serde(default)]
    pub resolution: Option<Resolution>,

    /// Python version override for resolution.
    #[serde(default)]
    pub python: Option<PythonVersion>,

    /// Enable pre-release selection.
    #[serde(default)]
    pub prereleases: bool,

    /// Packages that must use pre-built wheels (no building from source).
    #[serde(default)]
    pub no_build: Vec<PackageName>,

    /// Packages that must NOT use pre-built wheels (must build from source).
    #[serde(default)]
    pub no_binary: Vec<PackageName>,

    /// Universal (multi-platform) resolution mode.
    #[serde(default)]
    pub universal: bool,

    /// Python platform to resolve for.
    #[serde(default)]
    pub python_platform: Option<TargetTriple>,

    /// Required environments (platform markers).
    #[serde(default)]
    pub required_environments: Vec<MarkerTree>,
}

/// Options for generating tests from a scenario.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestGeneration {
    /// Disable test generation for this scenario.
    #[serde(default)]
    pub disable: bool,

    /// Select the generated test command for this scenario.
    #[serde(default)]
    pub kind: Option<ScenarioTest>,
}

/// The command template used to generate a scenario test.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ScenarioTest {
    Install,
    Compile,
    Lock,
}

/// The version selection strategy used by the resolver.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Resolution {
    Highest,
    Lowest,
    LowestDirect,
}

impl fmt::Display for Resolution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Highest => formatter.write_str("highest"),
            Self::Lowest => formatter.write_str("lowest"),
            Self::LowestDirect => formatter.write_str("lowest-direct"),
        }
    }
}

#[expect(clippy::unnecessary_wraps)] // Must return `Option` for serde `default`
fn default_requires_python() -> Option<VersionSpecifiers> {
    Some(VersionSpecifiers::from_str(">=3.12").expect("default requires-python should be valid"))
}

#[expect(clippy::unnecessary_wraps)] // Must return `Option` for serde `default`
fn default_artifact() -> Option<ArtifactMetadata> {
    Some(ArtifactMetadata::default())
}

fn default_python() -> PythonVersion {
    PythonVersion::from_str("3.12").expect("default Python version should be valid")
}

#[cfg(test)]
mod tests {
    use walkdir::WalkDir;

    use super::*;

    #[test]
    fn fixture_scenarios_parse() -> Result<()> {
        for entry in WalkDir::new(crate::packse::scenarios_dir().join("packages")) {
            let entry = entry?;
            if entry.file_type().is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "toml")
            {
                let scenario = Scenario::from_path(entry.path())?;
                assert!(scenario.testgen.disable, "{}", entry.path().display());
            }
        }
        Ok(())
    }

    #[test]
    fn parse_representative_package_metadata() -> Result<()> {
        let metadata: PackageMetadata = toml::from_str(
            r#"
requires_python = false
requires = ["dependency>=2"]
build_requires = ["build-dependency"]
raw_requires_dist = ["deliberately invalid ;"]
module_name = "import_name"
init_py = "answer = 42\n"
sdist_backend = "legacy-setuptools"
sdist_subdirectory = "project"
yanked = "broken release"
upload_time = "2024-01-01T00:00:00Z"

[scripts]
example = "import_name:main"
"#,
        )?;
        assert!(metadata.requires_python.is_none());
        assert_eq!(metadata.requires[0].to_string(), "dependency>=2");
        assert_eq!(metadata.build_requires[0].to_string(), "build-dependency");
        assert_eq!(metadata.raw_requires_dist, ["deliberately invalid ;"]);
        assert_eq!(metadata.module_name.as_deref(), Some("import_name"));
        assert_eq!(metadata.sdist_subdirectory.as_deref(), Some("project"));
        assert_eq!(
            metadata.yanked.simple_api_value(),
            Some(serde_json::json!("broken release"))
        );
        assert_eq!(metadata.scripts["example"], "import_name:main");
        assert_eq!(metadata.sdist_backend, SdistBackend::LegacySetuptools);
        Ok(())
    }

    #[test]
    fn parse_basic_scenario() {
        let toml = r#"
name = "fork-basic"
description = "An extremely basic test."

[resolver_options]
universal = true

[expected]
satisfiable = true

[root]
requires = ["a>=2 ; sys_platform == 'linux'", "a<2 ; sys_platform == 'darwin'"]

[packages.a.versions."1.0.0"]
[packages.a.versions."2.0.0"]
"#;
        let scenario: Scenario = toml::from_str(toml).expect("scenario should parse");
        let package_name = PackageName::from_str("a").expect("valid package name");
        assert_eq!(scenario.name, "fork-basic");
        assert!(scenario.resolver_options.universal);
        assert_eq!(scenario.packages.len(), 1);
        assert_eq!(scenario.packages[&package_name].versions.len(), 2);
    }

    #[test]
    fn parse_extras_scenario() {
        let toml = r#"
name = "all-extras-required"
description = "Multiple optional dependencies."

[root]
requires = ["a[all]"]

[expected]
satisfiable = true

[expected.packages]
a = "1.0.0"
b = "1.0.0"
c = "1.0.0"

[packages.b.versions."1.0.0"]
[packages.c.versions."1.0.0"]

[packages.a.versions."1.0.0".extras]
all = ["a[extra_b]", "a[extra_c]"]
extra_b = ["b"]
extra_c = ["c"]
"#;
        let scenario: Scenario = toml::from_str(toml).expect("scenario should parse");
        let package_name = PackageName::from_str("a").expect("valid package name");
        let version = Version::from_str("1.0.0").expect("valid version");
        let extra_name = ExtraName::from_str("extra_b").expect("valid extra name");
        assert_eq!(scenario.name, "all-extras-required");
        let a_meta = &scenario.packages[&package_name].versions[&version];
        assert_eq!(a_meta.extras.len(), 3);
        assert_eq!(
            a_meta.extras[&extra_name],
            vec![Requirement::from_str("b").expect("valid requirement")]
        );
    }

    #[test]
    fn parse_test_and_resolution() {
        let toml = r#"
name = "lowest-direct"

[root]
requires = ["a"]

[expected]
satisfiable = true

[testgen]
kind = "compile"

[resolver_options]
resolution = "lowest-direct"
"#;
        let scenario: Scenario = toml::from_str(toml).expect("scenario should parse");
        assert_eq!(scenario.testgen.kind, Some(ScenarioTest::Compile));
        assert_eq!(
            scenario.resolver_options.resolution,
            Some(Resolution::LowestDirect)
        );
    }

    #[test]
    fn reject_invalid_requires_python() {
        let toml = r#"
name = "invalid-requires-python"

[root]
requires = []

[expected]
satisfiable = true

[packages.a.versions."1.0.0"]
requires_python = "not a specifier"
"#;

        assert!(toml::from_str::<Scenario>(toml).is_err());
    }

    #[test]
    fn reject_unknown_metadata_field() {
        let toml = r#"
name = "unknown-metadata-field"

[root]
requires = ["a"]

[expected]
satisfiable = true

[packages.a.versions."1.0.0"]
wheels = false
"#;

        assert!(toml::from_str::<Scenario>(toml).is_err());
    }

    #[test]
    fn reject_invalid_wheel_tag() {
        let toml = r#"
name = "invalid-wheel-tag"

[root]
requires = ["a"]

[expected]
satisfiable = true

[packages.a.versions."1.0.0"]
wheel_tags = ["1-py3-none-any"]
"#;

        assert!(toml::from_str::<Scenario>(toml).is_err());
    }

    #[test]
    fn path_is_included_in_parse_errors() {
        let temporary_directory =
            tempfile::tempdir().expect("temporary directory should be created");
        let path = temporary_directory.path().join("invalid.toml");
        fs_err::write(&path, "not valid TOML = [").expect("invalid scenario should be written");

        let error = Scenario::from_path(&path).expect_err("scenario should fail to parse");
        insta::assert_snapshot!(
            error
                .to_string()
                .replace(temporary_directory.path().to_string_lossy().as_ref(), "[TEMP_DIR]")
                .replace('\\', "/"),
            @"failed to parse scenario file `[TEMP_DIR]/invalid.toml`"
        );
    }
}
