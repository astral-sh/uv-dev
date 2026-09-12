use std::ops::Bound;

use uv_distribution_types::{RequiresPython, RequiresPythonRange};
use uv_pep440::{LowerBound, UpperBound, Version, VersionSpecifiers};
use uv_pep508::MarkerTree;

fn version(value: &str) -> Version {
    value.parse().expect("valid Python version")
}

fn marker(value: &str) -> MarkerTree {
    value.parse().expect("valid marker expression")
}

fn with_bounds(lower: Bound<Version>, upper: Bound<Version>) -> RequiresPython {
    let range = RequiresPythonRange::new(LowerBound::new(lower), UpperBound::new(upper));
    let unbounded = RequiresPython::from_specifiers(VersionSpecifiers::empty());
    let requirement = unbounded.narrow(&range).unwrap_or(unbounded);
    assert_eq!(requirement.range(), &range);
    requirement
}

#[test]
fn marker_bound_kind_matrix() {
    let lower_bounds = [
        (Bound::Unbounded, None),
        (
            Bound::Included(Version::new([3, 9])),
            Some("python_full_version >= '3.9'"),
        ),
        (
            Bound::Excluded(Version::new([3, 9])),
            Some("python_full_version > '3.9'"),
        ),
    ];
    let upper_bounds = [
        (Bound::Unbounded, None),
        (
            Bound::Included(Version::new([3, 14])),
            Some("python_full_version <= '3.14'"),
        ),
        (
            Bound::Excluded(Version::new([3, 14])),
            Some("python_full_version < '3.14'"),
        ),
    ];

    for (lower, lower_expression) in lower_bounds {
        for (upper, upper_expression) in &upper_bounds {
            let expected = match (lower_expression, *upper_expression) {
                (None, None) => MarkerTree::TRUE,
                (Some(expression), None) | (None, Some(expression)) => marker(expression),
                (Some(lower), Some(upper)) => marker(&format!("{lower} and {upper}")),
            };
            assert_eq!(
                with_bounds(lower.clone(), upper.clone()).to_marker_tree(),
                expected,
                "lower: {lower:?}; upper: {upper:?}",
            );
        }
    }
}

#[test]
fn marker_equal_and_crossed_endpoints() {
    let equal = Version::new([3, 11]);
    for lower in [
        Bound::Included(equal.clone()),
        Bound::Excluded(equal.clone()),
    ] {
        for upper in [
            Bound::Included(equal.clone()),
            Bound::Excluded(equal.clone()),
        ] {
            let expected =
                if matches!(lower, Bound::Included(_)) && matches!(upper, Bound::Included(_)) {
                    marker("python_full_version == '3.11'")
                } else {
                    MarkerTree::FALSE
                };
            assert_eq!(
                with_bounds(lower.clone(), upper.clone()).to_marker_tree(),
                expected,
                "lower: {lower:?}; upper: {upper:?}",
            );
        }
    }

    assert_eq!(
        with_bounds(
            Bound::Included(Version::new([3, 14])),
            Bound::Included(Version::new([3, 9])),
        )
        .to_marker_tree(),
        MarkerTree::FALSE,
    );
}

#[test]
fn marker_uses_the_current_bounding_range() {
    let original = RequiresPython::from_specifiers(
        ">=3.9, !=3.11.*, <3.14"
            .parse()
            .expect("valid Python version specifiers"),
    );
    assert!(!original.contains(&Version::new([3, 11])));
    assert_eq!(
        original.to_marker_tree(),
        marker("python_full_version >= '3.9' and python_full_version < '3.14'"),
    );

    let narrowed = original
        .narrow(&RequiresPythonRange::new(
            LowerBound::new(Bound::Included(Version::new([3, 10]))),
            UpperBound::new(Bound::Excluded(Version::new([3, 12]))),
        ))
        .expect("strictly narrower Python range");
    assert_eq!(
        narrowed.to_marker_tree(),
        marker("python_full_version >= '3.10' and python_full_version < '3.12'"),
    );
}

#[test]
fn marker_uses_release_only_versions() {
    let requirement = RequiresPython::greater_than_equal_version(&version("3.13.0rc1+vendor.2"));
    assert_eq!(
        requirement.to_marker_tree(),
        marker("python_full_version >= '3.13'"),
    );

    for (specifiers, expected) in [
        (
            ">=3.13rc1, <3.14rc2",
            "python_full_version >= '3.13' and python_full_version < '3.14'",
        ),
        ("==3.13rc1+vendor.2", "python_full_version == '3.13'"),
    ] {
        let requirement =
            RequiresPython::from_specifiers(specifiers.parse().expect("valid version specifiers"));
        assert_eq!(requirement.to_marker_tree(), marker(expected));
    }

    assert_eq!(
        with_bounds(
            Bound::Included(version("3.9.0rc1+vendor")),
            Bound::Excluded(version("3.14.0rc2+vendor")),
        )
        .to_marker_tree(),
        marker("python_full_version >= '3.9' and python_full_version < '3.14'"),
    );
}
