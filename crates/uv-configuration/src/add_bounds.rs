use std::fmt::{Display, Formatter};
use std::{fmt, iter};

use serde::{Deserialize, Serialize};

use uv_pep440::{Version, VersionSpecifier, VersionSpecifiers};

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
    /// Return the version specifiers for this bound policy and a resolved version.
    pub fn specifiers(self, version: Version) -> VersionSpecifiers {
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use uv_pep440::Version;

    use super::AddBoundsKind;

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
}
