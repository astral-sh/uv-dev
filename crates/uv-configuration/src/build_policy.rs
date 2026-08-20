#[cfg(feature = "schemars")]
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::{self, Display};
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
    Fallback,
    /// Require wheels, even when doing so changes the selected version.
    Deny,
    /// Require source distributions instead of pre-built wheels.
    Force,
}

impl Display for BuildPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Allow => "allow",
            Self::Fallback => "fallback",
            Self::Deny => "deny",
            Self::Force => "force",
        })
    }
}

impl FromStr for BuildPolicy {
    type Err = BuildPolicyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "allow" => Ok(Self::Allow),
            "fallback" => Ok(Self::Fallback),
            "deny" => Ok(Self::Deny),
            "force" => Ok(Self::Force),
            _ => Err(BuildPolicyError::Policy(value.to_owned())),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BuildPolicyError {
    #[error("unknown build policy `{0}`; expected `allow`, `fallback`, `deny`, or `force`")]
    Policy(String),
    #[error(transparent)]
    Package(#[from] uv_normalize::InvalidNameError),
}

/// A global build policy or a `package=policy` override.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(try_from = "String")]
pub enum BuildPolicySpecifier {
    Global(BuildPolicy),
    Package(PackageName, BuildPolicy),
}

impl FromStr for BuildPolicySpecifier {
    type Err = BuildPolicyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if let Some((package, policy)) = value.split_once('=') {
            Ok(Self::Package(package.parse()?, policy.parse()?))
        } else {
            Ok(Self::Global(value.parse()?))
        }
    }
}

impl TryFrom<String> for BuildPolicySpecifier {
    type Error = BuildPolicyError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for BuildPolicySpecifier {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("BuildPolicySpecifier")
    }

    fn json_schema(_generator: &mut schemars::generate::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": r"^(([a-zA-Z0-9]|[a-zA-Z0-9][a-zA-Z0-9._-]*[a-zA-Z0-9])=)?(allow|fallback|deny|force)$",
            "description": "A build policy, optionally prefixed with a package name and `=`.",
        })
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
    /// Resolve entries in precedence order, with higher-precedence entries first.
    pub fn from_specifiers(specifiers: impl IntoIterator<Item = BuildPolicySpecifier>) -> Self {
        let mut policies = Self::default();
        for specifier in specifiers {
            match specifier {
                BuildPolicySpecifier::Global(policy) => {
                    policies.default.get_or_insert(policy);
                }
                BuildPolicySpecifier::Package(package, policy) => {
                    policies.packages.entry(package).or_insert(policy);
                }
            }
        }
        policies
    }

    /// Return the explicit package policy, falling back to the global policy.
    pub fn get(&self, package: &PackageName) -> Option<BuildPolicy> {
        self.packages.get(package).copied().or(self.default)
    }

    /// Whether builds can be rejected before a package name is known.
    pub fn denies_all(&self) -> bool {
        self.default == Some(BuildPolicy::Deny)
            && self
                .packages
                .values()
                .all(|policy| *policy == BuildPolicy::Deny)
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
        let policies = BuildPolicies::from_specifiers([
            "numpy=allow".parse()?,
            "deny".parse()?,
            "numpy=force".parse()?,
            "fallback".parse::<BuildPolicySpecifier>()?,
        ]);
        assert_eq!(policies.get(&"numpy".parse()?), Some(BuildPolicy::Allow));
        assert_eq!(policies.get(&"other".parse()?), Some(BuildPolicy::Deny));
        Ok(())
    }

    #[test]
    fn parse_policy() -> Result<(), BuildPolicyError> {
        assert_eq!(
            "fallback".parse::<BuildPolicySpecifier>()?,
            BuildPolicySpecifier::Global(BuildPolicy::Fallback)
        );
        assert_eq!(
            "Num_Py=deny".parse::<BuildPolicySpecifier>()?,
            BuildPolicySpecifier::Package("num-py".parse()?, BuildPolicy::Deny)
        );
        assert!("unknown".parse::<BuildPolicySpecifier>().is_err());
        assert!("=deny".parse::<BuildPolicySpecifier>().is_err());
        assert!("numpy=deny=force".parse::<BuildPolicySpecifier>().is_err());
        Ok(())
    }
}
