use std::collections::BTreeMap;
use std::fmt::{self, Display};
use std::ops::{Deref, DerefMut};
use std::str::FromStr;

use uv_normalize::PackageName;

/// Whether source distributions may be used for a package.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum BuildPolicy {
    /// Allow wheels and source distributions.
    #[default]
    Allow,

    /// Keep source distributions only when the selected version lacks sufficient wheel coverage.
    /// Package versions are selected normally. uv does not select an older version just because
    /// that version has a wheel. For platform-specific resolutions, the selected version must have
    /// a compatible wheel. For universal resolutions, the wheels must cover all applicable
    /// environment markers.
    /// uv may still build source distributions to obtain metadata.
    IfNecessary,

    /// Require wheels, even when doing so changes the selected version.
    /// As with `--no-build`, uv may reuse cached wheels built from source. Editable requirements
    /// may still be built, and their build backends may run arbitrary Python code.
    Disallow,

    /// Require source distributions instead of pre-built wheels.
    /// As with `--no-binary`, uv may still use pre-built wheels to read package metadata and reuse
    /// cached wheels built from source.
    Force,
}

impl Display for BuildPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Allow => "allow",
            Self::IfNecessary => "if-necessary",
            Self::Disallow => "disallow",
            Self::Force => "force",
        })
    }
}

impl FromStr for BuildPolicy {
    type Err = BuildPolicyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "allow" => Ok(Self::Allow),
            "if-necessary" => Ok(Self::IfNecessary),
            "disallow" => Ok(Self::Disallow),
            "force" => Ok(Self::Force),
            _ => Err(BuildPolicyError::Policy(value.to_owned())),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BuildPolicyError {
    #[error("unknown build policy `{0}`; expected `allow`, `if-necessary`, `disallow`, or `force`")]
    Policy(String),
    #[error("invalid `build-policy-package` value `{0}`: expected `PACKAGE=POLICY`")]
    Format(String),
    #[error(transparent)]
    Package(#[from] uv_normalize::InvalidNameError),
}

/// A package-specific source-build policy from the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPolicyPackageEntry {
    package: PackageName,
    policy: BuildPolicy,
}

impl FromStr for BuildPolicyPackageEntry {
    type Err = BuildPolicyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some((package, policy)) = value.split_once('=') else {
            return Err(BuildPolicyError::Format(value.to_owned()));
        };
        Ok(Self {
            package: package.parse()?,
            policy: policy.parse()?,
        })
    }
}

/// Source-build policies that apply to individual packages.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct BuildPolicyPackage(BTreeMap<PackageName, BuildPolicy>);

impl Deref for BuildPolicyPackage {
    type Target = BTreeMap<PackageName, BuildPolicy>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for BuildPolicyPackage {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl FromIterator<BuildPolicyPackageEntry> for BuildPolicyPackage {
    fn from_iter<T: IntoIterator<Item = BuildPolicyPackageEntry>>(iter: T) -> Self {
        Self(
            iter.into_iter()
                .map(|entry| (entry.package, entry.policy))
                .collect(),
        )
    }
}

impl IntoIterator for BuildPolicyPackage {
    type Item = (PackageName, BuildPolicy);
    type IntoIter = std::collections::btree_map::IntoIter<PackageName, BuildPolicy>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// Resolved global and package-specific build policies.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct BuildPolicies {
    default: Option<BuildPolicy>,
    packages: BTreeMap<PackageName, BuildPolicy>,
}

impl BuildPolicies {
    pub fn new(default: Option<BuildPolicy>, packages: BuildPolicyPackage) -> Self {
        Self {
            default,
            packages: packages.0,
        }
    }

    pub fn global(&self) -> Option<BuildPolicy> {
        self.default
    }

    pub fn packages(&self) -> &BTreeMap<PackageName, BuildPolicy> {
        &self.packages
    }

    /// Return the explicit package policy, falling back to the global policy.
    pub fn get(&self, package: &PackageName) -> Option<BuildPolicy> {
        self.packages.get(package).copied().or(self.default)
    }

    /// Whether builds can be rejected before a package name is known.
    pub(crate) fn denies_all(&self) -> bool {
        self.default == Some(BuildPolicy::Disallow)
            && self
                .packages
                .values()
                .all(|policy| *policy == BuildPolicy::Disallow)
    }

    pub fn is_empty(&self) -> bool {
        self.default.is_none() && self.packages.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_overrides_global() -> Result<(), BuildPolicyError> {
        let policies = BuildPolicies::new(
            Some(BuildPolicy::Disallow),
            ["numpy=force".parse()?, "numpy=allow".parse()?]
                .into_iter()
                .collect(),
        );
        assert_eq!(policies.get(&"numpy".parse()?), Some(BuildPolicy::Allow));
        assert_eq!(policies.get(&"other".parse()?), Some(BuildPolicy::Disallow));
        Ok(())
    }

    #[test]
    fn parse_policy() -> Result<(), BuildPolicyError> {
        assert_eq!(
            "if-necessary".parse::<BuildPolicy>()?,
            BuildPolicy::IfNecessary
        );
        assert_eq!(
            "Num_Py=disallow".parse::<BuildPolicyPackageEntry>()?,
            BuildPolicyPackageEntry {
                package: "num-py".parse()?,
                policy: BuildPolicy::Disallow
            }
        );
        assert!("numpy=allow".parse::<BuildPolicy>().is_err());
        assert!("allow".parse::<BuildPolicyPackageEntry>().is_err());
        assert!("=disallow".parse::<BuildPolicyPackageEntry>().is_err());
        assert!(
            "numpy=disallow=force"
                .parse::<BuildPolicyPackageEntry>()
                .is_err()
        );
        Ok(())
    }
}
