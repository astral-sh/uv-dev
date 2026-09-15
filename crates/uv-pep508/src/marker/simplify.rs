use std::fmt;
use std::ops::Bound;

use arcstr::ArcStr;
use indexmap::IndexMap;
use itertools::Itertools;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use version_ranges::Ranges;

use uv_pep440::{Version, VersionSpecifier};

use crate::marker::tree::ContainerOperator;
use crate::{ExtraOperator, MarkerExpression, MarkerOperator, MarkerTree, MarkerTreeKind};

/// Returns a simplified DNF expression for a given marker tree.
///
/// Marker trees are represented as decision diagrams that cannot be directly serialized to.
/// a boolean expression. Instead, you must traverse and collect all possible solutions to the
/// diagram, which can be used to create a DNF expression, or all non-solutions to the diagram,
/// which can be used to create a CNF expression.
///
/// We choose DNF as it is easier to simplify for user-facing output.
pub(crate) fn to_dnf(tree: MarkerTree) -> Vec<Vec<MarkerExpression>> {
    let mut dnf = Vec::new();
    collect_dnf(tree, &mut dnf, &mut Vec::new());
    simplify(&mut dnf);
    sort(&mut dnf);
    dnf
}

/// Walk a [`MarkerTree`] recursively and construct a DNF expression.
///
/// A decision diagram can be converted to DNF form by performing a depth-first traversal of
/// the tree and collecting all paths to a `true` terminal node.
///
/// `path` is the list of marker expressions traversed on the current path.
fn collect_dnf(
    tree: MarkerTree,
    dnf: &mut Vec<Vec<MarkerExpression>>,
    path: &mut Vec<MarkerExpression>,
) {
    match tree.kind() {
        // Reached a `false` node, meaning the conjunction is irrelevant for DNF.
        MarkerTreeKind::False => {}
        // Reached a solution, store the conjunction.
        MarkerTreeKind::True => {
            if !path.is_empty() {
                dnf.push(path.clone());
            }
        }
        MarkerTreeKind::Version(marker) => {
            for (tree, range) in collect_edges(marker.edges()) {
                // Detect whether the range for this edge can be simplified as an inequality.
                if let Some(excluded) = range_inequality(&range) {
                    let current = path.len();
                    for version in excluded {
                        path.push(MarkerExpression::Version {
                            key: marker.key().into(),
                            specifier: VersionSpecifier::not_equals_version(version.clone()),
                        });
                    }

                    collect_dnf(tree, dnf, path);
                    path.truncate(current);
                    continue;
                }

                // Detect whether the range for this edge can be simplified as a star specifier.
                if let Some(specifier) = star_range_specifier(&range) {
                    path.push(MarkerExpression::Version {
                        key: marker.key().into(),
                        specifier,
                    });

                    collect_dnf(tree, dnf, path);
                    path.pop();
                    continue;
                }

                for bounds in range.iter() {
                    let current = path.len();
                    for specifier in VersionSpecifier::from_release_only_bounds(bounds) {
                        path.push(MarkerExpression::Version {
                            key: marker.key().into(),
                            specifier,
                        });
                    }

                    collect_dnf(tree, dnf, path);
                    path.truncate(current);
                }
            }
        }
        MarkerTreeKind::String(marker) => {
            for (tree, range) in collect_edges(marker.children()) {
                // Detect whether the range for this edge can be simplified as an inequality.
                if let Some(excluded) = range_inequality(&range) {
                    let current = path.len();
                    for value in excluded {
                        path.push(MarkerExpression::String {
                            key: marker.key().into(),
                            operator: MarkerOperator::NotEqual,
                            value: value.clone(),
                        });
                    }

                    collect_dnf(tree, dnf, path);
                    path.truncate(current);
                    continue;
                }

                for bounds in range.iter() {
                    let current = path.len();
                    for (operator, value) in MarkerOperator::from_bounds(bounds) {
                        path.push(MarkerExpression::String {
                            key: marker.key().into(),
                            operator,
                            value: value.clone(),
                        });
                    }

                    collect_dnf(tree, dnf, path);
                    path.truncate(current);
                }
            }
        }
        MarkerTreeKind::In(marker) => {
            for (value, tree) in marker.children() {
                let operator = if value {
                    MarkerOperator::In
                } else {
                    MarkerOperator::NotIn
                };

                let expr = MarkerExpression::String {
                    key: marker.key().into(),
                    value: ArcStr::from(marker.value()),
                    operator,
                };

                path.push(expr);
                collect_dnf(tree, dnf, path);
                path.pop();
            }
        }
        MarkerTreeKind::Contains(marker) => {
            for (value, tree) in marker.children() {
                let operator = if value {
                    MarkerOperator::Contains
                } else {
                    MarkerOperator::NotContains
                };

                let expr = MarkerExpression::String {
                    key: marker.key().into(),
                    value: ArcStr::from(marker.value()),
                    operator,
                };

                path.push(expr);
                collect_dnf(tree, dnf, path);
                path.pop();
            }
        }
        MarkerTreeKind::List(marker) => {
            for (is_high, tree) in marker.children() {
                let expr = MarkerExpression::List {
                    pair: marker.pair().clone(),
                    operator: if is_high {
                        ContainerOperator::In
                    } else {
                        ContainerOperator::NotIn
                    },
                };

                path.push(expr);
                collect_dnf(tree, dnf, path);
                path.pop();
            }
        }
        MarkerTreeKind::Extra(marker) => {
            for (value, tree) in marker.children() {
                let operator = if value {
                    ExtraOperator::Equal
                } else {
                    ExtraOperator::NotEqual
                };

                let expr = MarkerExpression::Extra {
                    name: marker.name().clone().into(),
                    operator,
                };

                path.push(expr);
                collect_dnf(tree, dnf, path);
                path.pop();
            }
        }
    }
}

/// Simplifies a DNF expression.
///
/// A decision diagram is canonical, but only for a given variable order. Depending on the
/// pre-defined order, the DNF expression produced by a decision tree can still be further
/// simplified.
///
/// For example, the decision diagram for the expression `A or B` will be represented as
/// `A or (not A and B)` or `B or (not B and A)`, depending on the variable order. In both
/// cases, the negation in the second clause is redundant.
///
/// Completely simplifying a DNF expression is NP-hard and amounts to the set cover problem.
/// Additionally, marker expressions can contain complex expressions involving version ranges
/// that are not trivial to simplify. Instead, we choose to simplify at the boolean variable
/// level without any truth table expansion. Combined with the normalization applied by decision
/// trees, this seems to be sufficient in practice.
///
/// Note: This function has quadratic time complexity. However, it is not applied on every marker
/// operation, only to user facing output, which are typically very simple.
fn simplify(dnf: &mut Vec<Vec<MarkerExpression>>) {
    const CLAUSE_MEMBERSHIP_INDEX_THRESHOLD: usize = 32;

    if dnf.len() < 2 {
        return;
    }
    if dnf
        .iter()
        .all(|clause| clause.len() < CLAUSE_MEMBERSHIP_INDEX_THRESHOLD)
    {
        simplify_small(dnf);
        return;
    }

    for i in 0..dnf.len() {
        let clause = &dnf[i];

        let positions = (clause.len() >= CLAUSE_MEMBERSHIP_INDEX_THRESHOLD).then(|| {
            let mut positions = FxHashMap::default();
            positions.reserve(clause.len());
            for (position, term) in clause.iter().enumerate() {
                positions.entry(term).or_insert(position);
            }
            positions
        });

        // Find redundant terms in this clause.
        let mut redundant_terms = Vec::new();
        let mut removed_terms = positions.as_ref().map(|_| vec![false; clause.len()]);
        'term: for (skipped, skipped_term) in clause.iter().enumerate() {
            for (j, other_clause) in dnf.iter().enumerate() {
                if i == j {
                    continue;
                }

                // Let X be this clause with a given term A set to it's negation.
                // If there exists another clause that is a subset of X, the term A is
                // redundant in this clause.
                //
                // For example, `A or (not A and B)` can be simplified to `A or B`,
                // eliminating the `not A` term.
                if other_clause.iter().all(|term| {
                    // For the term to be redundant in this clause, the other clause can
                    // contain the negation of the term but not the term itself.
                    if term == skipped_term {
                        return false;
                    }
                    if is_negation(term, skipped_term) {
                        return true;
                    }

                    positions
                        .as_ref()
                        .map_or_else(
                            || clause.iter().position(|candidate| candidate == term),
                            |positions| positions.get(term).copied(),
                        )
                        // If the term was already removed from this one, we cannot
                        // depend on it for further simplification.
                        .is_some_and(|position| {
                            removed_terms.as_ref().map_or_else(
                                || !redundant_terms.contains(&position),
                                |removed_terms| !removed_terms[position],
                            )
                        })
                }) {
                    redundant_terms.push(skipped);
                    if let Some(removed_terms) = &mut removed_terms {
                        removed_terms[skipped] = true;
                    }
                    continue 'term;
                }
            }
        }

        // Eliminate any redundant terms.
        for position in redundant_terms.into_iter().rev() {
            dnf[i].remove(position);
        }
    }

    // Once we have eliminated redundant terms, there may also be redundant clauses.
    // For example, `(A and B) or (not A and B)` would have been simplified above to
    // `(A and B) or B` and can now be further simplified to just `B`.
    let mut redundant_clauses = Vec::new();
    let mut removed_clauses = dnf
        .iter()
        .any(|clause| clause.len() >= CLAUSE_MEMBERSHIP_INDEX_THRESHOLD)
        .then(|| vec![false; dnf.len()]);
    'clause: for i in 0..dnf.len() {
        let clause = &dnf[i];
        let terms = (clause.len() >= CLAUSE_MEMBERSHIP_INDEX_THRESHOLD)
            .then(|| clause.iter().collect::<FxHashSet<_>>());

        for (j, other_clause) in dnf.iter().enumerate() {
            // Ignore clauses that are going to be eliminated.
            if i == j
                || removed_clauses.as_ref().map_or_else(
                    || redundant_clauses.contains(&j),
                    |removed_clauses| removed_clauses[j],
                )
            {
                continue;
            }

            // There is another clause that is a subset of this one, thus this clause is redundant.
            if other_clause.iter().all(|term| {
                terms
                    .as_ref()
                    .map_or_else(|| clause.contains(term), |terms| terms.contains(term))
            }) {
                redundant_clauses.push(i);
                if let Some(removed_clauses) = &mut removed_clauses {
                    removed_clauses[i] = true;
                }
                continue 'clause;
            }
        }
    }

    // Eliminate any redundant clauses.
    for position in redundant_clauses.into_iter().rev() {
        dnf.remove(position);
    }
}

/// Simplify the short marker clauses that dominate normal dependency metadata without indexing.
fn simplify_small(dnf: &mut Vec<Vec<MarkerExpression>>) {
    for i in 0..dnf.len() {
        let clause = &dnf[i];
        let mut redundant_terms = Vec::new();
        'term: for (skipped, skipped_term) in clause.iter().enumerate() {
            for (j, other_clause) in dnf.iter().enumerate() {
                if i == j {
                    continue;
                }
                if other_clause.iter().all(|term| {
                    if term == skipped_term {
                        return false;
                    }
                    if is_negation(term, skipped_term) {
                        return true;
                    }
                    clause
                        .iter()
                        .position(|candidate| candidate == term)
                        .is_some_and(|position| !redundant_terms.contains(&position))
                }) {
                    redundant_terms.push(skipped);
                    continue 'term;
                }
            }
        }
        redundant_terms.sort_by(|left, right| right.cmp(left));
        for position in redundant_terms {
            dnf[i].remove(position);
        }
    }

    let mut redundant_clauses = Vec::new();
    'clause: for i in 0..dnf.len() {
        let clause = &dnf[i];
        for (j, other_clause) in dnf.iter().enumerate() {
            if i == j || redundant_clauses.contains(&j) {
                continue;
            }
            if other_clause.iter().all(|term| clause.contains(term)) {
                redundant_clauses.push(i);
                continue 'clause;
            }
        }
    }
    for position in redundant_clauses.into_iter().rev() {
        dnf.remove(position);
    }
}

/// Sort the clauses in a DNF expression, for backwards compatibility. The goal is to avoid
/// unnecessary churn in the display output of the marker expressions, e.g., when modifying the
/// internal representations used in the marker algebra.
fn sort(dnf: &mut [Vec<MarkerExpression>]) {
    // Sort each clause.
    for clause in dnf.iter_mut() {
        clause.sort_by_key(MarkerExpression::kind);
    }
    // Sort the clauses.
    dnf.sort_by(|a, b| {
        a.iter()
            .map(MarkerExpression::kind)
            .cmp(b.iter().map(MarkerExpression::kind))
    });
}

/// Merge any edges that lead to identical subtrees into a single range.
pub(crate) fn collect_edges<'a, T>(
    map: impl ExactSizeIterator<Item = (&'a Ranges<T>, MarkerTree)>,
) -> IndexMap<MarkerTree, Ranges<T>, FxBuildHasher>
where
    T: Ord + Clone + 'a,
{
    let mut paths: IndexMap<_, Ranges<_>, FxBuildHasher> = IndexMap::default();
    for (range, tree) in map {
        // OK because all ranges are guaranteed to be non-empty.
        let (start, end) = range.bounding_range().unwrap();
        // Combine the ranges.
        let range = Ranges::from_range_bounds((start.cloned(), end.cloned()));
        paths
            .entry(tree)
            .and_modify(|union| *union = union.union(&range))
            .or_insert_with(|| range.clone());
    }

    paths
}

/// Returns `Some` if the expression can be simplified as an inequality consisting
/// of the given values.
///
/// For example, `os_name < 'Linux' or os_name > 'Linux'` can be simplified to
/// `os_name != 'Linux'`.
fn range_inequality<T>(range: &Ranges<T>) -> Option<Vec<&T>>
where
    T: Ord + Clone + fmt::Debug,
{
    if range.is_empty() || range.bounding_range() != Some((Bound::Unbounded, Bound::Unbounded)) {
        return None;
    }

    let mut excluded = Vec::new();
    for ((_, end), (start, _)) in range.iter().tuple_windows() {
        match (end, start) {
            (Bound::Excluded(v1), Bound::Excluded(v2)) if v1 == v2 => excluded.push(v1),
            _ => return None,
        }
    }

    Some(excluded)
}

/// Returns `Some` if the version range can be simplified as a star specifier.
///
/// Only for the two bounds case not covered by [`VersionSpecifier::from_release_only_bounds`].
///
/// For negative ranges like `python_full_version < '3.8' or python_full_version >= '3.9'`,
/// returns `!= '3.8.*'`.
fn star_range_specifier(range: &Ranges<Version>) -> Option<VersionSpecifier> {
    if range.iter().count() != 2 {
        return None;
    }
    // Check for negative star range: two segments [(Unbounded, Excluded(v1)), (Included(v2), Unbounded)]
    let (b1, b2) = range.iter().collect_tuple()?;
    if let ((Bound::Unbounded, Bound::Excluded(v1)), (Bound::Included(v2), Bound::Unbounded)) =
        (b1, b2)
    {
        match *v1.only_release_trimmed().release() {
            [major] if *v2.release() == [major, 1] => {
                Some(VersionSpecifier::not_equals_star_version(Version::new([
                    major, 0,
                ])))
            }
            [major, minor] if *v2.release() == [major, minor + 1] => {
                Some(VersionSpecifier::not_equals_star_version(v1.clone()))
            }
            _ => None,
        }
    } else {
        None
    }
}

/// Returns `true` if the LHS is the negation of the RHS, or vice versa.
fn is_negation(left: &MarkerExpression, right: &MarkerExpression) -> bool {
    match left {
        MarkerExpression::Version { key, specifier } => {
            let MarkerExpression::Version {
                key: key2,
                specifier: specifier2,
            } = right
            else {
                return false;
            };

            key == key2
                && specifier.version() == specifier2.version()
                && specifier
                    .operator()
                    .negate()
                    .is_some_and(|negated| negated == *specifier2.operator())
        }
        MarkerExpression::VersionIn {
            key,
            versions,
            operator,
        } => {
            let MarkerExpression::VersionIn {
                key: key2,
                versions: versions2,
                operator: operator2,
            } = right
            else {
                return false;
            };

            key == key2 && versions == versions2 && operator != operator2
        }
        MarkerExpression::String {
            key,
            operator,
            value,
        } => {
            let MarkerExpression::String {
                key: key2,
                operator: operator2,
                value: value2,
            } = right
            else {
                return false;
            };

            key == key2
                && value == value2
                && operator
                    .negate()
                    .is_some_and(|negated| negated == *operator2)
        }
        MarkerExpression::Extra { operator, name } => {
            let MarkerExpression::Extra {
                name: name2,
                operator: operator2,
            } = right
            else {
                return false;
            };

            name == name2 && operator.negate() == *operator2
        }
        MarkerExpression::List { pair, operator } => {
            let MarkerExpression::List {
                pair: pair2,
                operator: operator2,
            } = right
            else {
                return false;
            };

            pair == pair2 && operator != operator2
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use uv_normalize::ExtraName;

    use super::*;
    use crate::MarkerValueExtra;

    fn extra(name: &str, operator: ExtraOperator) -> MarkerExpression {
        MarkerExpression::Extra {
            name: MarkerValueExtra::Extra(ExtraName::from_str(name).expect("valid extra name")),
            operator,
        }
    }

    #[test]
    fn indexed_clause_membership_matches_linear_simplification() {
        for (clauses, terms) in [(4, 7), (8, 16), (12, 32)] {
            let disjoint = (0..clauses)
                .map(|clause| {
                    (0..terms)
                        .map(|term| {
                            extra(
                                &format!("clause-{clause}-term-{term}"),
                                ExtraOperator::Equal,
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let mut indexed = disjoint.clone();
            let mut linear = disjoint;
            simplify(&mut indexed);
            simplify_small(&mut linear);
            assert_eq!(indexed, linear);

            let common = (0..terms)
                .map(|term| extra(&format!("common-{term}"), ExtraOperator::Equal))
                .collect::<Vec<_>>();
            let redundant = (0..clauses)
                .flat_map(|clause| {
                    let mut positive = common.clone();
                    positive.push(extra(&format!("selector-{clause}"), ExtraOperator::Equal));
                    let mut negative = common.clone();
                    negative.push(extra(
                        &format!("selector-{clause}"),
                        ExtraOperator::NotEqual,
                    ));
                    [positive, negative]
                })
                .collect::<Vec<_>>();
            let mut indexed = redundant.clone();
            let mut linear = redundant;
            simplify(&mut indexed);
            simplify_small(&mut linear);
            assert_eq!(indexed, linear);
        }
    }

    #[test]
    fn indexed_clause_membership_uses_the_first_duplicate_term() {
        let fillers = (0..30)
            .map(|term| extra(&format!("filler-{term}"), ExtraOperator::Equal))
            .collect::<Vec<_>>();
        let a = extra("a", ExtraOperator::Equal);
        let b = extra("b", ExtraOperator::Equal);
        let not_b = extra("b", ExtraOperator::NotEqual);
        let c = extra("c", ExtraOperator::Equal);
        let mut first = vec![a.clone(), not_b.clone()];
        first.extend(fillers.iter().cloned());
        let mut duplicate = vec![b.clone(), a.clone(), b.clone(), c.clone()];
        duplicate.extend(fillers.iter().cloned());
        let mut third = vec![c.clone(), b.clone()];
        third.extend(fillers.iter().cloned());
        let mut indexed = vec![first.clone(), duplicate.clone(), third.clone()];
        let mut linear = vec![first, duplicate, third];

        simplify(&mut indexed);
        simplify_small(&mut linear);
        assert_eq!(indexed, linear);
        assert_eq!(indexed.len(), 3);
        assert_eq!(&indexed[0][..2], [a.clone(), not_b]);
        assert_eq!(&indexed[1][..2], [a, c.clone()]);
        assert_eq!(&indexed[2][..2], [c, b]);
    }

    #[test]
    fn a_single_wide_clause_is_unchanged() {
        let clause = (0..64)
            .map(|term| extra(&format!("term-{term}"), ExtraOperator::Equal))
            .collect::<Vec<_>>();
        let mut dnf = vec![clause.clone()];

        simplify(&mut dnf);
        assert_eq!(dnf, [clause]);
    }

    #[test]
    fn mixed_width_clauses_match_linear_simplification() {
        let a = extra("a", ExtraOperator::Equal);
        let b = extra("b", ExtraOperator::Equal);
        let not_b = extra("b", ExtraOperator::NotEqual);
        let c = extra("c", ExtraOperator::Equal);
        let mut wide = vec![b.clone(), a.clone(), b.clone(), c.clone()];
        wide.extend((0..30).map(|term| extra(&format!("filler-{term}"), ExtraOperator::Equal)));
        let mut indexed = vec![vec![a, not_b], wide, vec![c, b]];
        let mut linear = indexed.clone();

        simplify(&mut indexed);
        simplify_small(&mut linear);
        assert_eq!(indexed, linear);
    }
}
