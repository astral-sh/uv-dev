use pubgrub::Ranges;
use smallvec::SmallVec;
use std::ops::Bound;

use uv_pep440::{LowerBound, UpperBound, Version};
use uv_pep508::{CanonicalMarkerValueVersion, MarkerTree, MarkerTreeKind};

use uv_distribution_types::RequiresPythonRange;

/// Returns the bounding Python versions that can satisfy the [`MarkerTree`], if it's constrained.
pub(crate) fn requires_python(tree: MarkerTree) -> Option<RequiresPythonRange> {
    /// A small vector of Python version markers.
    type Markers = SmallVec<[Ranges<Version>; 3]>;

    /// Collect the Python version markers from the tree.
    ///
    /// A true terminal reached without a Python version node accepts every version.
    fn collect_python_markers(tree: MarkerTree, markers: &mut Markers) {
        match tree.kind() {
            MarkerTreeKind::True => {
                markers.push(Ranges::full());
            }
            MarkerTreeKind::False => {}
            MarkerTreeKind::Version(marker) => match marker.key() {
                CanonicalMarkerValueVersion::PythonFullVersion => {
                    // In the reduced, ordered decision tree, a non-false child has a true terminal
                    // and cannot test the same variable again.
                    for (range, tree) in marker.edges() {
                        if !tree.is_false() {
                            markers.push(range.clone());
                        }
                    }
                }
                CanonicalMarkerValueVersion::ImplementationVersion => {
                    for (_, tree) in marker.edges() {
                        collect_python_markers(tree, markers);
                    }
                }
            },
            MarkerTreeKind::VersionString(marker) => {
                for (_, tree) in marker.edges() {
                    collect_python_markers(tree, markers);
                }
            }
            MarkerTreeKind::String(marker) => {
                for (_, tree) in marker.children() {
                    collect_python_markers(tree, markers);
                }
            }
            MarkerTreeKind::In(marker) => {
                for (_, tree) in marker.children() {
                    collect_python_markers(tree, markers);
                }
            }
            MarkerTreeKind::Contains(marker) => {
                for (_, tree) in marker.children() {
                    collect_python_markers(tree, markers);
                }
            }
            MarkerTreeKind::Extra(marker) => {
                for (_, tree) in marker.children() {
                    collect_python_markers(tree, markers);
                }
            }
            MarkerTreeKind::List(marker) => {
                for (_, tree) in marker.children() {
                    collect_python_markers(tree, markers);
                }
            }
        }
    }

    if tree.is_true() || tree.is_false() {
        return None;
    }

    let mut markers = Markers::new();
    collect_python_markers(tree, &mut markers);

    // If there are no Python version markers, return `None`.
    if markers.iter().all(|range| {
        let Some((lower, upper)) = range.bounding_range() else {
            return true;
        };
        matches!((lower, upper), (Bound::Unbounded, Bound::Unbounded))
    }) {
        return None;
    }

    // Take the union of the intersections of the Python version markers.
    let range = markers
        .into_iter()
        .fold(Ranges::empty(), |acc: Ranges<Version>, range| {
            acc.union(&range)
        });

    let (lower, upper) = range.bounding_range()?;

    Some(RequiresPythonRange::new(
        LowerBound::new(lower.cloned()),
        UpperBound::new(upper.cloned()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Bound;
    use std::str::FromStr;
    use uv_pep440::UpperBound;

    #[test]
    fn test_requires_python_with_extra_branches() {
        let tree = MarkerTree::from_str(
            "python_full_version >= '3.8' and (extra == 'foo' or extra == 'bar')",
        )
        .unwrap();
        assert_eq!(
            requires_python(tree),
            Some(RequiresPythonRange::new(
                LowerBound::new(Bound::Included(Version::new([3, 8]))),
                UpperBound::default(),
            ))
        );
    }

    #[test]
    fn test_requires_python() {
        // An exact version match.
        let tree = MarkerTree::from_str("python_full_version == '3.8.*'").unwrap();
        let range = requires_python(tree).unwrap();
        assert_eq!(
            *range.lower(),
            LowerBound::new(Bound::Included(Version::from_str("3.8").unwrap()))
        );
        assert_eq!(
            *range.upper(),
            UpperBound::new(Bound::Excluded(Version::from_str("3.9").unwrap()))
        );

        // A version range with exclusive bounds.
        let tree =
            MarkerTree::from_str("python_full_version > '3.8' and python_full_version < '3.9'")
                .unwrap();
        let range = requires_python(tree).unwrap();
        assert_eq!(
            *range.lower(),
            LowerBound::new(Bound::Excluded(Version::from_str("3.8").unwrap()))
        );
        assert_eq!(
            *range.upper(),
            UpperBound::new(Bound::Excluded(Version::from_str("3.9").unwrap()))
        );

        // A version range with inclusive bounds.
        let tree =
            MarkerTree::from_str("python_full_version >= '3.8' and python_full_version <= '3.9'")
                .unwrap();
        let range = requires_python(tree).unwrap();
        assert_eq!(
            *range.lower(),
            LowerBound::new(Bound::Included(Version::from_str("3.8").unwrap()))
        );
        assert_eq!(
            *range.upper(),
            UpperBound::new(Bound::Included(Version::from_str("3.9").unwrap()))
        );

        // A version with a lower bound.
        let tree = MarkerTree::from_str("python_full_version >= '3.8'").unwrap();
        let range = requires_python(tree).unwrap();
        assert_eq!(
            *range.lower(),
            LowerBound::new(Bound::Included(Version::from_str("3.8").unwrap()))
        );
        assert_eq!(*range.upper(), UpperBound::new(Bound::Unbounded));

        // A version with an upper bound.
        let tree = MarkerTree::from_str("python_full_version < '3.9'").unwrap();
        let range = requires_python(tree).unwrap();
        assert_eq!(*range.lower(), LowerBound::new(Bound::Unbounded));
        assert_eq!(
            *range.upper(),
            UpperBound::new(Bound::Excluded(Version::from_str("3.9").unwrap()))
        );

        // A disjunction with a non-Python marker (i.e., an unbounded range).
        let tree =
            MarkerTree::from_str("python_full_version > '3.8' or sys_platform == 'win32'").unwrap();
        let range = requires_python(tree).unwrap();
        assert_eq!(*range.lower(), LowerBound::new(Bound::Unbounded));
        assert_eq!(*range.upper(), UpperBound::new(Bound::Unbounded));

        // A complex mix of conjunctions and disjunctions.
        let tree = MarkerTree::from_str("(python_full_version >= '3.8' and python_full_version < '3.9') or (python_full_version >= '3.10' and python_full_version < '3.11')").unwrap();
        let range = requires_python(tree).unwrap();
        assert_eq!(
            *range.lower(),
            LowerBound::new(Bound::Included(Version::from_str("3.8").unwrap()))
        );
        assert_eq!(
            *range.upper(),
            UpperBound::new(Bound::Excluded(Version::from_str("3.11").unwrap()))
        );

        // An unbounded range across two specifiers.
        let tree =
            MarkerTree::from_str("python_full_version > '3.8' or python_full_version <= '3.8'")
                .unwrap();
        assert_eq!(requires_python(tree), None);
    }
}
