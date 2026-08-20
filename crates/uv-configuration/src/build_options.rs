use uv_normalize::PackageName;
pub use uv_pypi_types::BuildKind;

use crate::{BuildPolicy, BuildPolicyPackage, PackageNameSpecifier, PackageNameSpecifiers};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BuildOutput {
    /// Send the build backend output to `stderr`.
    Stderr,
    /// Send the build backend output to `tracing`.
    Debug,
    /// Do not display the build backend output.
    Quiet,
}

#[derive(Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct BuildOptions {
    no_binary: NoBinary,
    no_build: NoBuild,
    /// Whether an unnamed editable is covered by an explicit global no-build restriction.
    ///
    /// Pip's `--only-binary :all:` and `--no-build` both map to [`NoBuild::All`], but only the
    /// latter applies to editable requirements. This runtime-only override preserves that input
    /// distinction without changing the persisted build-options format.
    #[serde(skip)]
    no_build_unnamed_editable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    build_policy: Option<BuildPolicy>,
    #[serde(default, skip_serializing_if = "BuildPolicyPackage::is_empty")]
    build_policy_package: BuildPolicyPackage,
}

/// Custom `Debug` to hide runtime-only provenance from `--show-settings` output.
#[expect(clippy::missing_fields_in_debug)]
impl std::fmt::Debug for BuildOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuildOptions")
            .field("no_binary", &self.no_binary)
            .field("no_build", &self.no_build)
            .field("build_policy", &self.build_policy)
            .field("build_policy_package", &self.build_policy_package)
            .finish()
    }
}

impl BuildOptions {
    pub fn new(no_binary: NoBinary, no_build: NoBuild) -> Self {
        Self {
            no_binary,
            no_build,
            no_build_unnamed_editable: None,
            build_policy: None,
            build_policy_package: BuildPolicyPackage::default(),
        }
    }

    #[must_use]
    pub fn combine(self, no_binary: NoBinary, no_build: NoBuild) -> Self {
        Self {
            no_binary: self.no_binary.combine(no_binary),
            no_build: self.no_build.combine(no_build),
            no_build_unnamed_editable: self.no_build_unnamed_editable,
            build_policy: self.build_policy,
            build_policy_package: self.build_policy_package,
        }
    }

    /// Return equivalent build options with sorted, deduplicated package restrictions.
    ///
    /// This provides a stable representation when persisting or comparing build options.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        if let NoBinary::Packages(packages) = &mut self.no_binary {
            packages.sort_unstable();
            packages.dedup();
            if packages.is_empty() {
                self.no_binary = NoBinary::None;
            }
        }
        if let NoBuild::Packages(packages) = &mut self.no_build {
            packages.sort_unstable();
            packages.dedup();
            if packages.is_empty() {
                self.no_build = NoBuild::None;
            }
        }
        self
    }

    pub fn no_binary_package(&self, package_name: &PackageName) -> bool {
        self.package_restrictions(package_name, false).0
    }

    fn legacy_no_binary_package(&self, package_name: &PackageName) -> bool {
        match &self.no_binary {
            NoBinary::None => false,
            NoBinary::All => match &self.no_build {
                // Allow `all` to be overridden by specific build exclusions
                NoBuild::Packages(packages) => !packages.contains(package_name),
                _ => true,
            },
            NoBinary::Packages(packages) => packages.contains(package_name),
        }
    }

    pub fn no_build_package(&self, package_name: &PackageName) -> bool {
        self.package_restrictions(package_name, false).1
    }

    /// Return whether source distributions are disabled for a package after resolution.
    ///
    /// `has_compatible_wheel` resolves an `if-necessary` policy for the selected version. It does
    /// not affect concrete policies or legacy restrictions.
    pub fn no_build_package_with_compatible_wheel(
        &self,
        package_name: &PackageName,
        has_compatible_wheel: bool,
    ) -> bool {
        self.package_restrictions(package_name, has_compatible_wheel)
            .1
    }

    fn legacy_no_build_package(&self, package_name: &PackageName) -> bool {
        match &self.no_build {
            NoBuild::All => match &self.no_binary {
                // Allow `all` to be overridden by specific binary exclusions
                NoBinary::Packages(packages) => !packages.contains(package_name),
                _ => true,
            },
            NoBuild::None => false,
            NoBuild::Packages(packages) => packages.contains(package_name),
        }
    }

    /// Return the effective wheel and source restrictions for a package.
    fn package_restrictions(
        &self,
        package_name: &PackageName,
        has_compatible_wheel: bool,
    ) -> (bool, bool) {
        let no_binary = self.legacy_no_binary_package(package_name);
        let no_build = self.legacy_no_build_package(package_name);

        // The legacy options take precedence over the configured build policy.
        if no_binary || no_build {
            return (no_binary, no_build);
        }

        match self
            .configured_build_policy(package_name)
            .unwrap_or_default()
        {
            BuildPolicy::Allow => (false, false),
            BuildPolicy::IfNecessary => (false, has_compatible_wheel),
            BuildPolicy::Disallow => (false, true),
            BuildPolicy::Force => (true, false),
        }
    }

    /// Return whether a source build is disabled for a known or unknown package identity.
    ///
    /// Package-specific exceptions are considered only when `package_name` is known before the
    /// build backend is invoked.
    pub fn no_build_requirement(&self, package_name: Option<&PackageName>, editable: bool) -> bool {
        match package_name {
            Some(name) => self.no_build_package(name),
            None if editable => {
                self.no_build_unnamed_editable
                    .unwrap_or(matches!(self.no_build, NoBuild::All))
                    || self.build_policy == Some(BuildPolicy::Disallow)
            }
            None => self.no_build_unnamed(),
        }
    }

    fn no_build_unnamed(&self) -> bool {
        matches!(self.no_build, NoBuild::All) || self.build_policy == Some(BuildPolicy::Disallow)
    }

    #[must_use]
    pub fn with_build_policy(
        mut self,
        build_policy: Option<BuildPolicy>,
        build_policy_package: BuildPolicyPackage,
    ) -> Self {
        self.build_policy = build_policy;
        self.build_policy_package = build_policy_package;
        self
    }

    /// Set whether an explicit global no-build restriction applies to unnamed editables.
    #[must_use]
    pub fn with_no_build_unnamed_editable(mut self, no_build: bool) -> Self {
        self.no_build_unnamed_editable = Some(no_build);
        self
    }

    /// Return the policy configured for a package, falling back to the global policy.
    pub fn configured_build_policy(&self, package_name: &PackageName) -> Option<BuildPolicy> {
        self.build_policy_package
            .get(package_name)
            .copied()
            .or(self.build_policy)
    }

    /// Return the configured global build policy.
    pub fn build_policy(&self) -> Option<BuildPolicy> {
        self.build_policy
    }

    /// Return the configured package-specific build policies.
    pub fn build_policy_package(&self) -> &BuildPolicyPackage {
        &self.build_policy_package
    }

    /// Whether a global or package-specific build policy was configured.
    pub fn has_build_policy(&self) -> bool {
        self.build_policy.is_some() || !self.build_policy_package.is_empty()
    }

    /// Return the [`NoBuild`] strategy to use.
    pub fn no_build(&self) -> &NoBuild {
        &self.no_build
    }

    /// Return the [`NoBinary`] strategy to use.
    pub fn no_binary(&self) -> &NoBinary {
        &self.no_binary
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum NoBinary {
    /// Allow installation of any wheel.
    #[default]
    None,

    /// Do not allow installation from any wheels.
    All,

    /// Do not allow installation from the specific wheels.
    Packages(Vec<PackageName>),
}

impl NoBinary {
    /// Determine the binary installation strategy to use for the given arguments.
    pub fn from_args(no_binary: Option<bool>, no_binary_package: Vec<PackageName>) -> Self {
        match no_binary {
            Some(true) => Self::All,
            Some(false) => Self::None,
            None => {
                if no_binary_package.is_empty() {
                    Self::None
                } else {
                    Self::Packages(no_binary_package)
                }
            }
        }
    }

    /// Determine the binary installation strategy to use for the given arguments from the pip CLI.
    pub fn from_pip_args(no_binary: Vec<PackageNameSpecifier>) -> Self {
        let combined = PackageNameSpecifiers::from_iter(no_binary.into_iter());
        match combined {
            PackageNameSpecifiers::All => Self::All,
            PackageNameSpecifiers::None => Self::None,
            PackageNameSpecifiers::Packages(packages) => Self::Packages(packages),
        }
    }

    /// Determine the binary installation strategy to use for the given argument from the pip CLI.
    pub fn from_pip_arg(no_binary: PackageNameSpecifier) -> Self {
        Self::from_pip_args(vec![no_binary])
    }

    /// Combine a set of [`NoBinary`] values.
    #[must_use]
    pub fn combine(self, other: Self) -> Self {
        match (self, other) {
            // If both are `None`, the result is `None`.
            (Self::None, Self::None) => Self::None,
            // If either is `All`, the result is `All`.
            (Self::All, _) | (_, Self::All) => Self::All,
            // If one is `None`, the result is the other.
            (Self::Packages(a), Self::None) => Self::Packages(a),
            (Self::None, Self::Packages(b)) => Self::Packages(b),
            // If both are `Packages`, the result is the union of the two.
            (Self::Packages(mut a), Self::Packages(b)) => {
                a.extend(b);
                Self::Packages(a)
            }
        }
    }

    /// Extend a [`NoBinary`] value with another.
    pub fn extend(&mut self, other: Self) {
        match (&mut *self, other) {
            // If either is `All`, the result is `All`.
            (Self::All, _) | (_, Self::All) => *self = Self::All,
            // If both are `None`, the result is `None`.
            (Self::None, Self::None) => {
                // Nothing to do.
            }
            // If one is `None`, the result is the other.
            (Self::Packages(_), Self::None) => {
                // Nothing to do.
            }
            (Self::None, Self::Packages(b)) => {
                // Take ownership of `b`.
                *self = Self::Packages(b);
            }
            // If both are `Packages`, the result is the union of the two.
            (Self::Packages(a), Self::Packages(b)) => {
                a.extend(b);
            }
        }
    }
}

impl NoBinary {
    /// Returns `true` if all wheels are allowed.
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum NoBuild {
    /// Allow building wheels from any source distribution.
    #[default]
    None,

    /// Do not allow building wheels from any source distribution.
    All,

    /// Do not allow building wheels from the given package's source distributions.
    Packages(Vec<PackageName>),
}

impl NoBuild {
    /// Determine the build strategy to use for the given arguments.
    pub fn from_args(no_build: Option<bool>, no_build_package: Vec<PackageName>) -> Self {
        match no_build {
            Some(true) => Self::All,
            Some(false) => Self::None,
            None => {
                if no_build_package.is_empty() {
                    Self::None
                } else {
                    Self::Packages(no_build_package)
                }
            }
        }
    }

    /// Determine the build strategy to use for the given arguments from the pip CLI.
    pub fn from_pip_args(only_binary: Vec<PackageNameSpecifier>, no_build: bool) -> Self {
        if no_build {
            Self::All
        } else {
            let combined = PackageNameSpecifiers::from_iter(only_binary.into_iter());
            match combined {
                PackageNameSpecifiers::All => Self::All,
                PackageNameSpecifiers::None => Self::None,
                PackageNameSpecifiers::Packages(packages) => Self::Packages(packages),
            }
        }
    }

    /// Determine the build strategy to use for the given argument from the pip CLI.
    pub fn from_pip_arg(no_build: PackageNameSpecifier) -> Self {
        Self::from_pip_args(vec![no_build], false)
    }

    /// Combine a set of [`NoBuild`] values.
    #[must_use]
    pub fn combine(self, other: Self) -> Self {
        match (self, other) {
            // If both are `None`, the result is `None`.
            (Self::None, Self::None) => Self::None,
            // If either is `All`, the result is `All`.
            (Self::All, _) | (_, Self::All) => Self::All,
            // If one is `None`, the result is the other.
            (Self::Packages(a), Self::None) => Self::Packages(a),
            (Self::None, Self::Packages(b)) => Self::Packages(b),
            // If both are `Packages`, the result is the union of the two.
            (Self::Packages(mut a), Self::Packages(b)) => {
                a.extend(b);
                Self::Packages(a)
            }
        }
    }

    /// Extend a [`NoBuild`] value with another.
    pub fn extend(&mut self, other: Self) {
        match (&mut *self, other) {
            // If either is `All`, the result is `All`.
            (Self::All, _) | (_, Self::All) => *self = Self::All,
            // If both are `None`, the result is `None`.
            (Self::None, Self::None) => {
                // Nothing to do.
            }
            // If one is `None`, the result is the other.
            (Self::Packages(_), Self::None) => {
                // Nothing to do.
            }
            (Self::None, Self::Packages(b)) => {
                // Take ownership of `b`.
                *self = Self::Packages(b);
            }
            // If both are `Packages`, the result is the union of the two.
            (Self::Packages(a), Self::Packages(b)) => {
                a.extend(b);
            }
        }
    }
}

impl NoBuild {
    /// Returns `true` if all builds are allowed.
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum IndexStrategy {
    /// Only use results from the first index that returns a match for a given package name.
    ///
    /// While this differs from pip's behavior, it's the default index strategy as it's the most
    /// secure.
    #[default]
    #[cfg_attr(feature = "clap", clap(alias = "first-match"))]
    FirstIndex,
    /// Search for every package name across all indexes, exhausting the versions from the first
    /// index before moving on to the next.
    ///
    /// In this strategy, we look for every package across all indexes. When resolving, we attempt
    /// to use versions from the indexes in order, such that we exhaust all available versions from
    /// the first index before moving on to the next. Further, if a version is found to be
    /// incompatible in the first index, we do not reconsider that version in subsequent indexes,
    /// even if the secondary index might contain compatible versions (e.g., variants of the same
    /// versions with different ABI tags or Python version constraints).
    ///
    /// See: <https://peps.python.org/pep-0708/>
    #[cfg_attr(feature = "clap", clap(alias = "unsafe-any-match"))]
    #[serde(alias = "unsafe-any-match")]
    UnsafeFirstMatch,
    /// Search for every package name across all indexes, preferring the "best" version found. If a
    /// package version is in multiple indexes, only look at the entry for the first index.
    ///
    /// In this strategy, we look for every package across all indexes. When resolving, we consider
    /// all versions from all indexes, choosing the "best" version found (typically, the highest
    /// compatible version).
    ///
    /// This most closely matches pip's behavior, but exposes the resolver to "dependency confusion"
    /// attacks whereby malicious actors can publish packages to public indexes with the same name
    /// as internal packages, causing the resolver to install the malicious package in lieu of
    /// the intended internal package.
    ///
    /// See: <https://peps.python.org/pep-0708/>
    UnsafeBestMatch,
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use anyhow::Error;

    use super::*;
    use crate::BuildPolicyPackage;

    #[test]
    fn build_policy_overrides() -> Result<(), Error> {
        let package = PackageName::from_str("example")?;
        let other = PackageName::from_str("other")?;
        let options = BuildOptions::default().with_build_policy(
            Some(BuildPolicy::Disallow),
            ["example=allow".parse()?].into_iter().collect(),
        );
        assert!(!options.no_build_package(&package));
        assert!(options.no_build_package(&other));
        assert!(options.no_build_requirement(None, false));

        // Explicit legacy restrictions take precedence over the new policy.
        let options = options.combine(NoBinary::Packages(vec![other.clone()]), NoBuild::None);
        assert!(options.no_binary_package(&other));
        assert!(!options.no_build_package(&other));
        let options = BuildOptions::new(NoBinary::None, NoBuild::All)
            .with_build_policy(Some(BuildPolicy::Force), BuildPolicyPackage::default());
        assert!(options.no_build_package(&package));
        assert!(!options.no_binary_package(&package));

        let options = BuildOptions::default().with_build_policy(
            Some(BuildPolicy::IfNecessary),
            BuildPolicyPackage::default(),
        );
        assert!(!options.no_build_package(&package));
        assert!(!options.no_build_package_with_compatible_wheel(&package, false));
        assert!(options.no_build_package_with_compatible_wheel(&package, true));

        // Legacy restrictions still take precedence when the conditional policy is resolved.
        let options = options.combine(NoBinary::Packages(vec![package.clone()]), NoBuild::None);
        assert!(options.no_binary_package(&package));
        assert!(!options.no_build_package_with_compatible_wheel(&package, true));
        Ok(())
    }

    #[test]
    fn build_policy_unnamed_requirements() -> Result<(), Error> {
        let package = PackageName::from_str("example")?;
        let other = PackageName::from_str("other")?;

        // Package exceptions apply when the package identity is known, but cannot authorize
        // metadata execution for an unnamed source.
        let options = BuildOptions::default().with_build_policy(
            Some(BuildPolicy::Disallow),
            ["example=allow".parse()?].into_iter().collect(),
        );
        assert!(options.no_build_requirement(None, false));
        assert!(options.no_build_requirement(None, true));
        assert!(!options.no_build_requirement(Some(&package), false));
        assert!(options.no_build_requirement(Some(&other), false));

        let options = options.combine(NoBinary::Packages(vec![package.clone()]), NoBuild::None);
        assert!(options.no_build_requirement(None, false));
        assert!(options.no_build_requirement(None, true));
        assert!(!options.no_build_requirement(Some(&package), false));

        // A global source-only restriction cannot authorize backend execution when the package
        // identity is unknown. The build policy remains the pre-execution decision.
        let options = BuildOptions::new(NoBinary::All, NoBuild::None)
            .with_build_policy(Some(BuildPolicy::Disallow), BuildPolicyPackage::default());
        assert!(options.no_build_requirement(None, false));
        assert!(options.no_build_requirement(None, true));
        assert!(!options.no_build_requirement(Some(&package), false));

        // A permissive global policy allows metadata discovery. Restrictions for the discovered
        // package still apply to subsequent named build work.
        let options = BuildOptions::default().with_build_policy(
            Some(BuildPolicy::Allow),
            ["example=disallow".parse()?].into_iter().collect(),
        );
        assert!(!options.no_build_requirement(None, false));
        assert!(!options.no_build_requirement(None, true));
        assert!(options.no_build_requirement(Some(&package), false));
        assert!(!options.no_build_requirement(Some(&other), false));

        // Preserve the existing behavior of the explicit global no-build restriction.
        let options = BuildOptions::new(NoBinary::Packages(vec![package.clone()]), NoBuild::All)
            .with_build_policy(Some(BuildPolicy::Disallow), BuildPolicyPackage::default());
        assert!(options.no_build_requirement(None, false));
        assert!(options.no_build_requirement(None, true));
        assert!(!options.no_build_requirement(Some(&package), false));

        // Pip's `--only-binary :all:` preserves the editable exemption, while `--no-build`
        // rejects the unnamed editable before backend execution.
        let options = BuildOptions::new(NoBinary::None, NoBuild::All);
        assert!(options.no_build_requirement(None, true));
        let options = options.with_no_build_unnamed_editable(false);
        assert!(!options.no_build_requirement(None, true));
        Ok(())
    }

    #[test]
    fn normalized_build_policy_options() -> Result<(), Error> {
        let alpha = PackageName::from_str("alpha")?;
        let beta = PackageName::from_str("beta")?;
        let packages = ["example=allow".parse()?]
            .into_iter()
            .collect::<BuildPolicyPackage>();
        let options = BuildOptions::new(
            NoBinary::Packages(vec![beta.clone(), alpha.clone(), beta.clone()]),
            NoBuild::Packages(vec![beta.clone(), beta.clone(), alpha.clone()]),
        )
        .with_build_policy(Some(BuildPolicy::IfNecessary), packages.clone())
        .normalized();
        assert_eq!(
            options,
            BuildOptions::new(
                NoBinary::Packages(vec![alpha.clone(), beta.clone()]),
                NoBuild::Packages(vec![alpha, beta]),
            )
            .with_build_policy(Some(BuildPolicy::IfNecessary), packages)
        );
        assert_eq!(options.clone().normalized(), options);
        assert_eq!(
            BuildOptions::new(NoBinary::Packages(vec![]), NoBuild::Packages(vec![])).normalized(),
            BuildOptions::default()
        );
        Ok(())
    }

    #[test]
    fn no_build_from_args() -> Result<(), Error> {
        assert_eq!(
            NoBuild::from_pip_args(vec![PackageNameSpecifier::from_str(":all:")?], false),
            NoBuild::All,
        );
        assert_eq!(
            NoBuild::from_pip_args(vec![PackageNameSpecifier::from_str(":all:")?], true),
            NoBuild::All,
        );
        assert_eq!(
            NoBuild::from_pip_args(vec![PackageNameSpecifier::from_str(":none:")?], true),
            NoBuild::All,
        );
        assert_eq!(
            NoBuild::from_pip_args(vec![PackageNameSpecifier::from_str(":none:")?], false),
            NoBuild::None,
        );
        assert_eq!(
            NoBuild::from_pip_args(
                vec![
                    PackageNameSpecifier::from_str("foo")?,
                    PackageNameSpecifier::from_str("bar")?
                ],
                false
            ),
            NoBuild::Packages(vec![
                PackageName::from_str("foo")?,
                PackageName::from_str("bar")?
            ]),
        );
        assert_eq!(
            NoBuild::from_pip_args(
                vec![
                    PackageNameSpecifier::from_str("test")?,
                    PackageNameSpecifier::All
                ],
                false
            ),
            NoBuild::All,
        );
        assert_eq!(
            NoBuild::from_pip_args(
                vec![
                    PackageNameSpecifier::from_str("foo")?,
                    PackageNameSpecifier::from_str(":none:")?,
                    PackageNameSpecifier::from_str("bar")?
                ],
                false
            ),
            NoBuild::Packages(vec![PackageName::from_str("bar")?]),
        );

        Ok(())
    }
}
