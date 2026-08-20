use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Display};
use std::ops::{Deref, DerefMut};
use std::str::FromStr;

use uv_normalize::PackageName;

use crate::{NoBinary, NoBuild};

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

/// Resolved global and package-specific build policies, including legacy restrictions.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(from = "BuildPolicyInputs", into = "BuildPolicyInputs")]
pub struct BuildPolicies {
    default: Option<BuildPolicy>,
    packages: BTreeMap<PackageName, Option<BuildPolicy>>,
    inputs: BuildPolicyInputs,
}

impl BuildPolicies {
    pub fn new(
        no_binary: NoBinary,
        no_build: NoBuild,
        global: Option<BuildPolicy>,
        packages: BuildPolicyPackage,
    ) -> Self {
        BuildPolicyInputs {
            no_binary,
            no_build,
            policy: ConfiguredBuildPolicies {
                default: global,
                packages: packages.0,
            },
        }
        .into()
    }

    /// Combine requirements-file restrictions with the original build-policy inputs.
    #[must_use]
    pub fn combine(self, no_binary: NoBinary, no_build: NoBuild) -> Self {
        BuildPolicyInputs {
            no_binary: self.inputs.no_binary.combine(no_binary),
            no_build: self.inputs.no_build.combine(no_build),
            policy: self.inputs.policy,
        }
        .into()
    }

    /// Return equivalent policies with sorted, deduplicated legacy package restrictions.
    ///
    /// This provides stable input provenance when persisting or comparing policies.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        if let NoBinary::Packages(packages) = &mut self.inputs.no_binary {
            packages.sort_unstable();
            packages.dedup();
            if packages.is_empty() {
                self.inputs.no_binary = NoBinary::None;
            }
        }
        if let NoBuild::Packages(packages) = &mut self.inputs.no_build {
            packages.sort_unstable();
            packages.dedup();
            if packages.is_empty() {
                self.inputs.no_build = NoBuild::None;
            }
        }
        self
    }

    /// Return the effective build policy for a package.
    ///
    /// `None` means that contradictory legacy restrictions forbid both wheels and source
    /// distributions. Legacy restrictions take precedence over the configured policy.
    pub fn effective_policy(&self, package: &PackageName) -> Option<BuildPolicy> {
        self.packages.get(package).copied().unwrap_or(self.default)
    }

    /// Whether any package may need its policy resolved from wheel coverage.
    pub fn has_if_necessary(&self) -> bool {
        self.default == Some(BuildPolicy::IfNecessary)
            || self
                .packages
                .values()
                .any(|policy| *policy == Some(BuildPolicy::IfNecessary))
    }

    pub fn no_binary_package(&self, package: &PackageName) -> bool {
        matches!(
            self.effective_policy(package),
            None | Some(BuildPolicy::Force)
        )
    }

    pub fn no_build_package(&self, package: &PackageName) -> bool {
        matches!(
            self.effective_policy(package),
            None | Some(BuildPolicy::Disallow)
        )
    }

    pub fn no_build_requirement(&self, package: Option<&PackageName>) -> bool {
        match package {
            Some(package) => self.no_build_package(package),
            None => {
                // Preserve the stronger legacy behavior for unnamed sources. Package-specific
                // exceptions apply after the package name is known.
                matches!(self.inputs.no_build, NoBuild::All)
                    || (matches!(self.default, None | Some(BuildPolicy::Disallow))
                        && self
                            .packages
                            .values()
                            .all(|policy| matches!(policy, None | Some(BuildPolicy::Disallow))))
            }
        }
    }

    /// Resolve a package's conditional policy after determining its wheel coverage.
    ///
    /// Concrete policies and legacy restrictions are left unchanged. The retained inputs are also
    /// updated, so serialization and requirements-file merging preserve the resolved policy.
    pub fn resolve_if_necessary(&mut self, package: PackageName, has_wheel: bool) {
        if self.effective_policy(&package) == Some(BuildPolicy::IfNecessary) {
            let policy = if has_wheel {
                BuildPolicy::Disallow
            } else {
                BuildPolicy::Allow
            };
            self.inputs.policy.packages.insert(package.clone(), policy);
            if Some(policy) == self.default {
                self.packages.remove(&package);
            } else {
                self.packages.insert(package, Some(policy));
            }
        }
    }

    /// Whether a global or package-specific build policy was explicitly configured.
    pub fn is_configured(&self) -> bool {
        !self.inputs.policy.is_empty()
    }

    /// Return the configured global policy, before applying legacy restrictions.
    pub fn configured_global(&self) -> Option<BuildPolicy> {
        self.inputs.policy.default
    }

    /// Return a configured package policy, falling back to the configured global policy.
    pub fn configured_policy(&self, package: &PackageName) -> Option<BuildPolicy> {
        self.inputs
            .policy
            .packages
            .get(package)
            .copied()
            .or(self.inputs.policy.default)
    }

    /// Return the configured package policies, before applying legacy restrictions.
    pub fn configured_packages(&self) -> &BTreeMap<PackageName, BuildPolicy> {
        &self.inputs.policy.packages
    }

    /// Return the original [`NoBuild`] restriction.
    pub fn no_build(&self) -> &NoBuild {
        &self.inputs.no_build
    }

    /// Return the original [`NoBinary`] restriction.
    pub fn no_binary(&self) -> &NoBinary {
        &self.inputs.no_binary
    }
}

impl Default for BuildPolicies {
    fn default() -> Self {
        BuildPolicyInputs::default().into()
    }
}

/// Original inputs retained for requirements-file merging, diagnostics, and serialization.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct BuildPolicyInputs {
    no_binary: NoBinary,
    no_build: NoBuild,
    #[serde(default, skip_serializing_if = "ConfiguredBuildPolicies::is_empty")]
    policy: ConfiguredBuildPolicies,
}

impl BuildPolicyInputs {
    fn no_binary_package(&self, package: &PackageName) -> bool {
        match &self.no_binary {
            NoBinary::None => false,
            NoBinary::All => match &self.no_build {
                NoBuild::Packages(packages) => !packages.contains(package),
                _ => true,
            },
            NoBinary::Packages(packages) => packages.contains(package),
        }
    }

    fn no_build_package(&self, package: &PackageName) -> bool {
        match &self.no_build {
            NoBuild::None => false,
            NoBuild::All => match &self.no_binary {
                NoBinary::Packages(packages) => !packages.contains(package),
                _ => true,
            },
            NoBuild::Packages(packages) => packages.contains(package),
        }
    }
}

impl From<BuildPolicyInputs> for BuildPolicies {
    fn from(inputs: BuildPolicyInputs) -> Self {
        fn resolve(no_binary: bool, no_build: bool, policy: BuildPolicy) -> Option<BuildPolicy> {
            match (no_binary, no_build) {
                (true, true) => None,
                (true, false) => Some(BuildPolicy::Force),
                (false, true) => Some(BuildPolicy::Disallow),
                (false, false) => Some(policy),
            }
        }

        let default = resolve(
            matches!(inputs.no_binary, NoBinary::All),
            matches!(inputs.no_build, NoBuild::All),
            inputs.policy.default.unwrap_or_default(),
        );
        let mut names = inputs
            .policy
            .packages
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        if let NoBinary::Packages(packages) = &inputs.no_binary {
            names.extend(packages.iter().cloned());
        }
        if let NoBuild::Packages(packages) = &inputs.no_build {
            names.extend(packages.iter().cloned());
        }
        let packages = names
            .into_iter()
            .filter_map(|package| {
                let policy = resolve(
                    inputs.no_binary_package(&package),
                    inputs.no_build_package(&package),
                    inputs.policy.get(&package),
                );
                (policy != default).then_some((package, policy))
            })
            .collect();
        Self {
            default,
            packages,
            inputs,
        }
    }
}

impl From<BuildPolicies> for BuildPolicyInputs {
    fn from(policies: BuildPolicies) -> Self {
        policies.inputs
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct ConfiguredBuildPolicies {
    default: Option<BuildPolicy>,
    packages: BTreeMap<PackageName, BuildPolicy>,
}

impl ConfiguredBuildPolicies {
    fn get(&self, package: &PackageName) -> BuildPolicy {
        self.packages
            .get(package)
            .copied()
            .or(self.default)
            .unwrap_or_default()
    }

    fn is_empty(&self) -> bool {
        self.default.is_none() && self.packages.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;

    use super::*;

    #[test]
    fn package_overrides_global() -> Result<(), BuildPolicyError> {
        let policies = BuildPolicies::new(
            NoBinary::None,
            NoBuild::None,
            Some(BuildPolicy::Disallow),
            ["numpy=force".parse()?, "numpy=allow".parse()?]
                .into_iter()
                .collect(),
        );
        assert_eq!(
            policies.effective_policy(&"numpy".parse()?),
            Some(BuildPolicy::Allow)
        );
        assert_eq!(
            policies.effective_policy(&"other".parse()?),
            Some(BuildPolicy::Disallow)
        );
        Ok(())
    }

    #[test]
    fn build_policy_unnamed_legacy_exceptions() -> Result<(), Error> {
        let package = PackageName::from_str("example")?;
        let other = PackageName::from_str("other")?;
        let policies = BuildPolicies::new(
            NoBinary::None,
            NoBuild::None,
            Some(BuildPolicy::Disallow),
            BuildPolicyPackage::default(),
        );
        assert!(policies.no_build_requirement(None));

        let policies = policies.combine(NoBinary::Packages(vec![package.clone()]), NoBuild::None);
        assert!(!policies.no_build_requirement(None));
        assert!(!policies.no_build_requirement(Some(&package)));
        assert!(policies.no_build_requirement(Some(&other)));

        let policies = policies.combine(NoBinary::None, NoBuild::Packages(vec![package.clone()]));
        assert!(policies.no_build_requirement(None));
        assert!(policies.no_build_requirement(Some(&package)));

        // Preserve the existing behavior of the explicit global no-build restriction.
        let policies = BuildPolicies::new(
            NoBinary::Packages(vec![package.clone()]),
            NoBuild::All,
            Some(BuildPolicy::Disallow),
            BuildPolicyPackage::default(),
        );
        assert!(policies.no_build_requirement(None));
        assert!(!policies.no_build_requirement(Some(&package)));

        // An overridden configured exception does not permit any source builds.
        let policies = BuildPolicies::new(
            NoBinary::None,
            NoBuild::Packages(vec![package.clone()]),
            Some(BuildPolicy::Disallow),
            ["example=allow".parse()?].into_iter().collect(),
        );
        assert!(policies.no_build_requirement(None));
        assert!(policies.no_build_requirement(Some(&package)));
        Ok(())
    }

    #[test]
    fn normalized_build_policy_options() -> Result<(), Error> {
        let alpha = PackageName::from_str("alpha")?;
        let beta = PackageName::from_str("beta")?;
        let packages = ["example=allow".parse()?]
            .into_iter()
            .collect::<BuildPolicyPackage>();
        let policies = BuildPolicies::new(
            NoBinary::Packages(vec![beta.clone(), alpha.clone(), beta.clone()]),
            NoBuild::Packages(vec![beta.clone(), beta.clone(), alpha.clone()]),
            Some(BuildPolicy::IfNecessary),
            packages.clone(),
        )
        .normalized();
        assert_eq!(
            policies,
            BuildPolicies::new(
                NoBinary::Packages(vec![alpha.clone(), beta.clone()]),
                NoBuild::Packages(vec![alpha, beta]),
                Some(BuildPolicy::IfNecessary),
                packages,
            )
        );
        assert_eq!(policies.clone().normalized(), policies);
        assert_eq!(
            BuildPolicies::new(
                NoBinary::Packages(vec![]),
                NoBuild::Packages(vec![]),
                None,
                BuildPolicyPackage::default(),
            )
            .normalized(),
            BuildPolicies::default()
        );
        Ok(())
    }

    #[test]
    fn effective_build_policy_legacy_restrictions() -> Result<(), Error> {
        let package = PackageName::from_str("example")?;
        let other = PackageName::from_str("other")?;
        for (no_binary, no_build, expected, expected_other) in [
            (
                NoBinary::None,
                NoBuild::None,
                Some(BuildPolicy::IfNecessary),
                Some(BuildPolicy::IfNecessary),
            ),
            (
                NoBinary::All,
                NoBuild::None,
                Some(BuildPolicy::Force),
                Some(BuildPolicy::Force),
            ),
            (
                NoBinary::Packages(vec![package.clone()]),
                NoBuild::None,
                Some(BuildPolicy::Force),
                Some(BuildPolicy::IfNecessary),
            ),
            (
                NoBinary::None,
                NoBuild::All,
                Some(BuildPolicy::Disallow),
                Some(BuildPolicy::Disallow),
            ),
            (
                NoBinary::None,
                NoBuild::Packages(vec![package.clone()]),
                Some(BuildPolicy::Disallow),
                Some(BuildPolicy::IfNecessary),
            ),
            (NoBinary::All, NoBuild::All, None, None),
            (
                NoBinary::All,
                NoBuild::Packages(vec![package.clone()]),
                Some(BuildPolicy::Disallow),
                Some(BuildPolicy::Force),
            ),
            (
                NoBinary::Packages(vec![package.clone()]),
                NoBuild::All,
                Some(BuildPolicy::Force),
                Some(BuildPolicy::Disallow),
            ),
            (
                NoBinary::Packages(vec![package.clone()]),
                NoBuild::Packages(vec![package.clone()]),
                None,
                Some(BuildPolicy::IfNecessary),
            ),
        ] {
            let policies = BuildPolicies::new(
                no_binary,
                no_build,
                Some(BuildPolicy::IfNecessary),
                BuildPolicyPackage::default(),
            );
            assert_eq!(policies.effective_policy(&package), expected);
            assert_eq!(policies.effective_policy(&other), expected_other);
            assert_eq!(
                policies.no_binary_package(&package),
                matches!(expected, None | Some(BuildPolicy::Force))
            );
            assert_eq!(
                policies.no_build_package(&package),
                matches!(expected, None | Some(BuildPolicy::Disallow))
            );
        }
        assert_eq!(
            BuildPolicies::default().effective_policy(&package),
            Some(BuildPolicy::Allow)
        );
        Ok(())
    }

    #[test]
    fn resolve_conditional_build_policy() -> Result<(), Error> {
        let covered = PackageName::from_str("covered")?;
        let uncovered = PackageName::from_str("uncovered")?;
        let forced = PackageName::from_str("forced")?;
        let disallowed = PackageName::from_str("disallowed")?;
        let conflicting = PackageName::from_str("conflicting")?;
        let explicit = PackageName::from_str("explicit")?;
        let mut policies = BuildPolicies::new(
            NoBinary::Packages(vec![forced.clone(), conflicting.clone()]),
            NoBuild::Packages(vec![disallowed.clone(), conflicting.clone()]),
            Some(BuildPolicy::IfNecessary),
            ["explicit=force".parse()?].into_iter().collect(),
        );
        policies.resolve_if_necessary(covered.clone(), true);
        policies.resolve_if_necessary(uncovered.clone(), false);
        policies.resolve_if_necessary(forced.clone(), true);
        policies.resolve_if_necessary(disallowed.clone(), false);
        policies.resolve_if_necessary(conflicting.clone(), true);
        policies.resolve_if_necessary(explicit.clone(), true);

        assert_eq!(
            policies.effective_policy(&covered),
            Some(BuildPolicy::Disallow)
        );
        assert_eq!(
            policies.effective_policy(&uncovered),
            Some(BuildPolicy::Allow)
        );
        assert_eq!(policies.effective_policy(&forced), Some(BuildPolicy::Force));
        assert_eq!(
            policies.effective_policy(&disallowed),
            Some(BuildPolicy::Disallow)
        );
        assert_eq!(policies.effective_policy(&conflicting), None);
        assert_eq!(
            policies.effective_policy(&explicit),
            Some(BuildPolicy::Force)
        );
        assert_eq!(policies.configured_global(), Some(BuildPolicy::IfNecessary));
        for package in [&forced, &disallowed, &conflicting] {
            assert!(!policies.configured_packages().contains_key(package));
        }
        assert_eq!(
            policies.clone().combine(NoBinary::None, NoBuild::None),
            policies
        );
        Ok(())
    }

    #[test]
    fn combine_build_policy_preserves_legacy_provenance() -> Result<(), Error> {
        let package = PackageName::from_str("example")?;
        let configured = BuildPolicies::new(
            NoBinary::None,
            NoBuild::None,
            None,
            ["example=force".parse()?].into_iter().collect(),
        );
        let legacy = BuildPolicies::new(
            NoBinary::Packages(vec![package.clone()]),
            NoBuild::None,
            None,
            BuildPolicyPackage::default(),
        );
        assert_eq!(
            configured.effective_policy(&package),
            legacy.effective_policy(&package)
        );
        assert_eq!(
            configured
                .combine(NoBinary::None, NoBuild::All)
                .effective_policy(&package),
            Some(BuildPolicy::Disallow)
        );
        assert_eq!(
            legacy
                .combine(NoBinary::None, NoBuild::All)
                .effective_policy(&package),
            Some(BuildPolicy::Force)
        );
        Ok(())
    }

    #[test]
    fn build_policy_serialization_preserves_inputs() -> Result<(), Error> {
        let value = serde_json::json!({
            "no-binary": { "packages": ["example"] },
            "no-build": "all",
            "policy": {
                "default": "if-necessary",
                "packages": { "example": "allow" }
            }
        });
        let policies = serde_json::from_value::<BuildPolicies>(value.clone())?;
        assert_eq!(
            policies.effective_policy(&"example".parse()?),
            Some(BuildPolicy::Force)
        );
        assert!(policies.no_build_requirement(None));
        assert_eq!(serde_json::to_value(&policies)?, value);
        assert_eq!(
            serde_json::to_value(BuildPolicies::default())?,
            serde_json::json!({"no-binary": "none", "no-build": "none"})
        );
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
