use std::collections::BTreeSet;

use uv_configuration::NormalizedRequirements;
use uv_distribution_types::{Requirement, StaticMetadata};
use uv_normalize::{ExtraName, PackageName};
use uv_pep440::{Version, release_specifiers_to_ranges};
use version_ranges::Ranges;

/// Compare payload semantics without changing declaration precedence or lookup cardinality.
pub(super) fn same_static_metadata(left: &[StaticMetadata], right: &[StaticMetadata]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| MetadataView::from(left) == MetadataView::from(right))
}

#[derive(PartialEq, Eq)]
struct MetadataView<'a> {
    name: &'a PackageName,
    version: Option<&'a Version>,
    requires_dist: NormalizedRequirements,
    requires_python: Ranges<Version>,
    provides_extra: BTreeSet<&'a ExtraName>,
}

impl<'a> From<&'a StaticMetadata> for MetadataView<'a> {
    fn from(metadata: &'a StaticMetadata) -> Self {
        let StaticMetadata {
            name,
            version,
            requires_dist,
            requires_python,
            provides_extra,
        } = metadata;
        Self {
            name,
            version: version.as_ref(),
            requires_dist: NormalizedRequirements::from(
                requires_dist
                    .iter()
                    .cloned()
                    .map(Requirement::from)
                    .collect::<Vec<_>>(),
            ),
            requires_python: requires_python
                .as_ref()
                .map_or_else(Ranges::full, |specifiers| {
                    release_specifiers_to_ranges(specifiers.clone())
                }),
            provides_extra: provides_extra.iter().collect(),
        }
    }
}

/// Canonicalize one payload for preview output, leaving the outer declaration list intact.
pub(super) fn normalize_static_metadata(metadata: StaticMetadata) -> StaticMetadata {
    let StaticMetadata {
        name,
        version,
        requires_dist,
        requires_python,
        provides_extra,
    } = metadata;
    StaticMetadata {
        name,
        version,
        requires_dist: NormalizedRequirements::from(
            requires_dist
                .into_vec()
                .into_iter()
                .map(Requirement::from)
                .collect::<Vec<_>>(),
        )
        .into_iter()
        .map(Into::into)
        .collect(),
        requires_python,
        provides_extra: provides_extra
            .into_vec()
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use uv_distribution_types::StaticMetadata;

    use super::{normalize_static_metadata, same_static_metadata};

    fn metadata(
        requires_dist: &[&str],
        requires_python: Option<&str>,
        extras: &[&str],
    ) -> StaticMetadata {
        serde_json::from_value(serde_json::json!({
            "name": "parent",
            "version": "1.0",
            "requires-dist": requires_dist,
            "requires-python": requires_python,
            "provides-extra": extras,
        }))
        .expect("static metadata fixture")
    }

    #[test]
    fn payload_equivalence_retains_raw_declarations() {
        let left = metadata(
            &["b[two,one]", "a>=1,>=2", "a<4", "a<4"],
            Some(">=3.9,>=3.10"),
            &["gpu", "cpu", "gpu"],
        );
        let right = metadata(
            &["a>=2,<4", "b[one,two,one]"],
            Some(">=3.10"),
            &["cpu", "gpu"],
        );
        let original = serde_json::to_string(&left).expect("serialize metadata");
        assert!(same_static_metadata(std::slice::from_ref(&left), &[right]));
        assert_eq!(
            serde_json::to_string(&left).expect("serialize metadata"),
            original
        );

        let normalized = normalize_static_metadata(left);
        assert_eq!(normalized.requires_dist.len(), 2);
        assert_eq!(normalized.provides_extra.len(), 2);
        assert!(same_static_metadata(
            std::slice::from_ref(&normalized),
            &[normalize_static_metadata(normalized.clone())],
        ));
    }

    #[test]
    fn payload_comparison_retains_resolution_policies() {
        for (left, right) in [
            ("a>=1rc1,>=1", "a>=1"),
            ("a==1", "a==1,>=0"),
            ("a!=1.*", "a!=1.0.*"),
            (
                "a @ https://example.invalid/a-1.0.tar.gz",
                "a @ https://other.invalid/a-1.0.tar.gz",
            ),
            (
                "a @ https://example.invalid/a-1.0.tar.gz#sha256=abc",
                "a @ https://example.invalid/a-1.0.tar.gz#sha256=def",
            ),
        ] {
            assert!(
                !same_static_metadata(
                    &[metadata(&[left], None, &[])],
                    &[metadata(&[right], None, &[])],
                ),
                "{left} versus {right}"
            );
        }
        assert!(!same_static_metadata(
            &[metadata(&["a; python_version < '0'"], None, &[])],
            &[metadata(&[], None, &[])],
        ));
        for (left, right) in [
            (">=3.9", ">=3.9,!=3.10"),
            (">=3.9,!=3.10.*", ">=3.9,!=3.10.0.*"),
        ] {
            assert!(!same_static_metadata(
                &[metadata(&[], Some(left), &[])],
                &[metadata(&[], Some(right), &[])],
            ));
        }
        assert!(same_static_metadata(
            &[metadata(&[], None, &[])],
            &[metadata(&[], Some(""), &[])],
        ));
    }

    #[test]
    fn payload_comparison_retains_outer_selection() {
        let first = metadata(&["a"], None, &[]);
        let second = metadata(&["b"], None, &[]);
        assert!(!same_static_metadata(
            &[first.clone(), second.clone()],
            &[second, first.clone()],
        ));
        assert!(!same_static_metadata(
            &[first.clone(), first.clone()],
            std::slice::from_ref(&first),
        ));
        let mut fallback = first.clone();
        fallback.version = None;
        assert!(!same_static_metadata(&[first], &[fallback]));
    }
}
