use uv_pep440::{LowerBound, UpperBound};
use uv_pep508::MarkerTree;

use uv_distribution_types::{RequiresPythonRange, python_version_ranges};

/// Returns the bounding Python versions that can satisfy the [`MarkerTree`], if it's constrained.
pub(crate) fn requires_python(tree: MarkerTree) -> Option<RequiresPythonRange> {
    let range = python_version_ranges(tree)?;
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
    use uv_pep440::{UpperBound, Version};

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
