//! Rewrite dependency constraints for project upgrades.

use std::collections::BTreeSet;

use uv_pep440::{
    Operator, Version, VersionSpecifier, VersionSpecifierBuildError, VersionSpecifiers,
};

/// A failure to express an upgraded set of version constraints.
#[derive(Debug, thiserror::Error)]
pub enum ProposeSpecifiersError {
    #[error("The upgraded requirement does not admit every resolved version")]
    Unrepresentable { specifiers: VersionSpecifiers },
    #[error(transparent)]
    InvalidSpecifier(#[from] VersionSpecifierBuildError),
    #[error("Cannot rewrite a version constraint without a release segment")]
    MissingRelease,
    #[error(
        "Cannot expand version `{version}` at release segment {segment_index} (`{segment}`) beyond its maximum value"
    )]
    ReleaseOverflow {
        version: Version,
        segment_index: usize,
        segment: u64,
    },
}

/// Propose version specifiers that admit every resolved version.
///
/// Return `None` if no update is needed. Otherwise, rewrite only blocking specifiers and return
/// them in `Some`. When multiple versions are resolved, choose bounds that admit all of them, or
/// return an error if that is impossible.
///
/// For example, resolving `foo>=1,<2` to `2.4` produces `>=1, <3`.
pub fn propose_specifiers(
    specifiers: &VersionSpecifiers,
    resolved_versions: &BTreeSet<Version>,
) -> Result<Option<VersionSpecifiers>, ProposeSpecifiersError> {
    if resolved_versions.is_empty() {
        return Ok(None);
    }

    if resolved_versions
        .iter()
        .all(|version| specifiers.contains(version))
    {
        return Ok(None);
    }
    let specifiers = specifiers
        .iter()
        .cloned()
        .map(|specifier| rewrite_specifier(specifier, resolved_versions))
        .collect::<Result<VersionSpecifiers, ProposeSpecifiersError>>()?;
    if !resolved_versions
        .iter()
        .all(|version| specifiers.contains(version))
    {
        return Err(ProposeSpecifiersError::Unrepresentable { specifiers });
    }
    Ok(Some(specifiers))
}

/// Attempt to rewrite a [`VersionSpecifier`] to admit all resolved versions while preserving its
/// operator.
fn rewrite_specifier(
    specifier: VersionSpecifier,
    resolved_versions: &BTreeSet<Version>,
) -> Result<VersionSpecifier, ProposeSpecifiersError> {
    if resolved_versions
        .iter()
        .all(|version| specifier.contains(version))
    {
        return Ok(specifier);
    }
    let (Some(lowest_resolved_version), Some(highest_resolved_version)) =
        (resolved_versions.first(), resolved_versions.last())
    else {
        return Ok(specifier);
    };

    Ok(match specifier.operator() {
        Operator::GreaterThan
        | Operator::GreaterThanEqual
        | Operator::NotEqual
        | Operator::NotEqualStar => specifier,
        Operator::TildeEqual => VersionSpecifier::from_version(
            Operator::TildeEqual,
            compatible_version_at_precision(
                lowest_resolved_version,
                specifier.version().release().len(),
            )?,
        )?,
        Operator::Equal => VersionSpecifier::equals_version(lowest_resolved_version.clone()),
        Operator::EqualStar => VersionSpecifier::equals_star_version(
            lowest_resolved_version
                .only_release_at_precision(specifier.version().release().len())
                .ok_or(ProposeSpecifiersError::MissingRelease)?,
        ),
        Operator::ExactEqual => {
            VersionSpecifier::from_version(Operator::ExactEqual, lowest_resolved_version.clone())?
        }
        Operator::LessThan => VersionSpecifier::less_than_version(increment_version_at_precision(
            highest_resolved_version,
            specifier.version().release().len(),
        )?),
        Operator::LessThanEqual => VersionSpecifier::from_version(
            Operator::LessThanEqual,
            highest_resolved_version.clone().without_local(),
        )?,
    })
}

/// Project a version to the given precision while preserving its compatible-release suffixes.
fn compatible_version_at_precision(
    version: &Version,
    precision: usize,
) -> Result<Version, ProposeSpecifiersError> {
    let release = version
        .release()
        .iter()
        .copied()
        .chain(std::iter::repeat(0))
        .take(precision)
        .collect::<Vec<_>>();
    if release.is_empty() {
        return Err(ProposeSpecifiersError::MissingRelease);
    }
    Ok(version.clone().with_release(release).without_local())
}

/// Increment the last release segment after projecting a version to the given precision.
fn increment_version_at_precision(
    version: &Version,
    precision: usize,
) -> Result<Version, ProposeSpecifiersError> {
    let projected = version
        .only_release_at_precision(precision)
        .ok_or(ProposeSpecifiersError::MissingRelease)?;
    let mut release = projected.release().to_vec();
    let segment_index = release.len();
    let Some(last) = release.last_mut() else {
        return Err(ProposeSpecifiersError::MissingRelease);
    };
    let segment = *last;
    *last = segment
        .checked_add(1)
        .ok_or_else(|| ProposeSpecifiersError::ReleaseOverflow {
            version: version.clone(),
            segment_index,
            segment,
        })?;
    Ok(projected.with_release(release))
}

/// Remove upper and exact constraints while retaining lower bounds and exclusions.
pub fn relax_specifiers(specifiers: &VersionSpecifiers) -> VersionSpecifiers {
    specifiers
        .iter()
        .filter_map(|specifier| match specifier.operator() {
            Operator::GreaterThan
            | Operator::GreaterThanEqual
            | Operator::NotEqual
            | Operator::NotEqualStar => Some(specifier.clone()),
            Operator::TildeEqual => Some(VersionSpecifier::greater_than_equal_version(
                specifier.version().clone(),
            )),
            Operator::Equal
            | Operator::EqualStar
            | Operator::ExactEqual
            | Operator::LessThan
            | Operator::LessThanEqual => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{increment_version_at_precision, propose_specifiers};
    use std::collections::BTreeSet;
    use std::str::FromStr;
    use uv_pep440::{Version, VersionSpecifiers};

    fn resolved_versions(versions: &[&str]) -> BTreeSet<Version> {
        versions
            .iter()
            .map(|version| Version::from_str(version).expect("valid version"))
            .collect()
    }

    #[test]
    fn propose_specifiers_preserves_satisfied_constraints() {
        for requirement in ["", ">=1.2", "!=2.3"] {
            let requirement = VersionSpecifiers::from_str(requirement).expect("valid requirement");

            let proposed = propose_specifiers(&requirement, &resolved_versions(&["2.4.0"]))
                .expect("specifiers can be proposed");

            assert!(proposed.is_none());
        }
    }

    #[test]
    fn propose_specifiers_returns_none_without_resolved_versions() {
        let requirement = VersionSpecifiers::from_str("<2").expect("valid requirement");

        let proposed =
            propose_specifiers(&requirement, &BTreeSet::new()).expect("specifiers can be proposed");

        assert!(proposed.is_none());
    }

    #[test]
    fn propose_specifiers_expands_exclusive_upper_bounds_at_existing_precision() {
        for (requirement, version, expected) in [
            (">=1.2,<2", "2.4.0", ">=1.2, <3"),
            (">=1.2,<1.3", "1.4.2", ">=1.2, <1.5"),
        ] {
            let requirement = VersionSpecifiers::from_str(requirement).expect("valid requirement");

            let proposed = propose_specifiers(&requirement, &resolved_versions(&[version]))
                .expect("specifiers can be proposed")
                .expect("specifiers need an update");

            assert_eq!(proposed.to_string(), expected);
        }
    }

    #[test]
    fn propose_specifiers_only_rewrites_blocking_specifiers() {
        let requirement = VersionSpecifiers::from_str(">=1,<2,<4").expect("valid requirement");

        let proposed = propose_specifiers(&requirement, &resolved_versions(&["2.4.0"]))
            .expect("specifiers can be proposed")
            .expect("specifiers need an update");

        assert_eq!(proposed.to_string(), ">=1, <3, <4");
    }

    #[test]
    fn propose_specifiers_preserves_operator_style() {
        for (requirement, version, expected) in [
            ("==1.2.3", "2.4.5", "==2.4.5"),
            ("===1.2.3", "2.4.5", "===2.4.5"),
            ("==1.2.*", "2.4.5", "==2.4.*"),
            ("~=1.2", "2.4.5", "~=2.4"),
            ("~=1.2.3", "2.4.5", "~=2.4.5"),
            ("<=1.2.3", "2.4.5", "<=2.4.5"),
        ] {
            let requirement = VersionSpecifiers::from_str(requirement).expect("valid requirement");

            let proposed = propose_specifiers(&requirement, &resolved_versions(&[version]))
                .expect("specifiers can be proposed")
                .expect("specifiers need an update");

            assert_eq!(proposed.to_string(), expected);
        }
    }

    #[test]
    fn propose_specifiers_preserves_compatible_release_suffixes() {
        let requirement = VersionSpecifiers::from_str("~=1.2").expect("valid requirement");

        let proposed = propose_specifiers(
            &requirement,
            &resolved_versions(&["1!2.4rc1.post2.dev3+local"]),
        )
        .expect("specifiers can be proposed")
        .expect("specifiers need an update");

        assert_eq!(proposed.to_string(), "~=1!2.4rc1.post2.dev3");
    }

    #[test]
    fn propose_specifiers_strips_local_version_from_inclusive_upper_bound() {
        let requirement = VersionSpecifiers::from_str("<=1.2.3").expect("valid requirement");

        let proposed = propose_specifiers(&requirement, &resolved_versions(&["2.4.5+local"]))
            .expect("specifiers can be proposed")
            .expect("specifiers need an update");

        assert_eq!(proposed.to_string(), "<=2.4.5");
    }

    #[test]
    fn propose_specifiers_preserves_lower_bounds_and_exclusions() {
        let requirement = VersionSpecifiers::from_str(">=1.2,!=2.3,<2").expect("valid requirement");

        let proposed = propose_specifiers(&requirement, &resolved_versions(&["2.4.0"]))
            .expect("specifiers can be proposed")
            .expect("specifiers need an update");

        assert_eq!(proposed.to_string(), ">=1.2, !=2.3, <3");
    }

    #[test]
    fn propose_specifiers_expands_upper_bound_for_multiple_versions() {
        let requirement = VersionSpecifiers::from_str("<2").expect("valid requirement");

        let proposed = propose_specifiers(&requirement, &resolved_versions(&["1.5.0", "2.4.0"]))
            .expect("upper bound can admit both versions")
            .expect("specifiers need an update");

        assert_eq!(proposed.to_string(), "<3");
    }

    #[test]
    fn propose_specifiers_uses_lowest_compatible_version_for_multiple_versions() {
        let requirement = VersionSpecifiers::from_str("~=1.2").expect("valid requirement");

        let proposed = propose_specifiers(&requirement, &resolved_versions(&["2.4", "2.5"]))
            .expect("compatible release can admit both versions")
            .expect("specifiers need an update");

        assert_eq!(proposed.to_string(), "~=2.4");
    }

    #[test]
    fn increment_version_at_precision_reports_upper_bound_overflow() {
        let version = Version::new([1, 2, u64::MAX]);

        let error = increment_version_at_precision(&version, 3)
            .expect_err("maximum release segment cannot be incremented");

        assert_eq!(
            error.to_string(),
            "Cannot expand version `1.2.18446744073709551615` at release segment 3 (`18446744073709551615`) beyond its maximum value"
        );
    }
}
