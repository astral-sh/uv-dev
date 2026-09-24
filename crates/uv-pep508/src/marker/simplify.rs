use std::alloc::{Allocator, Global};
use std::cell::Cell;
use std::fmt;
use std::ops::Bound;

use arcstr::ArcStr;
use bumpalo::Bump;
use indexmap::IndexMap;
use itertools::{Either, Itertools};
use rustc_hash::FxBuildHasher;
use version_ranges::Ranges;

use uv_pep440::{Version, VersionSpecifier};

use crate::marker::tree::ContainerOperator;
use crate::{ExtraOperator, MarkerExpression, MarkerOperator, MarkerTree, MarkerTreeKind};

thread_local! {
    static DNF_ARENA: Cell<Option<Bump>> = const { Cell::new(None) };
}

/// Use temporary DNF clauses, then release their storage together.
pub(crate) fn with_dnf<R>(
    tree: MarkerTree,
    use_dnf: impl FnOnce(&[Vec<MarkerExpression, &Bump>]) -> R,
) -> R {
    // Removing the arena lets nested formatting and thread-local destructors use their own.
    let mut arena = DNF_ARENA
        .try_with(Cell::take)
        .ok()
        .flatten()
        .unwrap_or_default();
    let result = use_dnf(&to_dnf_in(tree, &arena));
    // Reuse ordinary clause storage without retaining unusually large diagrams.
    if arena.allocated_bytes() <= 64 * 1024 {
        arena.reset();
        let _ = DNF_ARENA.try_with(|slot| slot.set(Some(arena)));
    }
    result
}

/// Returns a simplified DNF expression for a given marker tree.
///
/// Marker trees are represented as decision diagrams that cannot be directly serialized to.
/// a boolean expression. Instead, you must traverse and collect all possible solutions to the
/// diagram, which can be used to create a DNF expression, or all non-solutions to the diagram,
/// which can be used to create a CNF expression.
///
/// We choose DNF as it is easier to simplify for user-facing output.
pub(crate) fn to_dnf(tree: MarkerTree) -> Vec<Vec<MarkerExpression>> {
    to_dnf_in(tree, Global)
}

/// Returns a simplified DNF expression with clause storage allocated by `allocator`.
fn to_dnf_in<A: Allocator + Copy>(
    tree: MarkerTree,
    allocator: A,
) -> Vec<Vec<MarkerExpression, A>, A> {
    let mut dnf = Vec::new_in(allocator);
    collect_dnf(tree, &mut dnf, &mut Vec::new_in(allocator));
    simplify(&mut dnf, allocator);
    sort(&mut dnf);
    dnf
}

/// Walk a [`MarkerTree`] recursively and construct a DNF expression.
///
/// A decision diagram can be converted to DNF form by performing a depth-first traversal of
/// the tree and collecting all paths to a `true` terminal node.
///
/// `path` is the list of marker expressions traversed on the current path.
fn collect_dnf<A: Allocator + Copy>(
    tree: MarkerTree,
    dnf: &mut Vec<Vec<MarkerExpression, A>, A>,
    path: &mut Vec<MarkerExpression, A>,
) {
    match tree.kind() {
        // Reached a `false` node, meaning the conjunction is irrelevant for DNF.
        MarkerTreeKind::False => {}
        // Reached a solution, store the conjunction.
        MarkerTreeKind::True => {
            if !path.is_empty() {
                let mut clause = Vec::with_capacity_in(path.len(), *path.allocator());
                clause.extend_from_slice(path);
                dnf.push(clause);
            }
        }
        MarkerTreeKind::Version(marker) => {
            for (tree, range) in collect_edges_in(
                marker.edges().filter(|(_, tree)| !tree.is_false()),
                *path.allocator(),
            ) {
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
        MarkerTreeKind::VersionString(marker) => {
            for (tree, range) in collect_edges_in(
                marker.edges().filter(|(_, tree)| !tree.is_false()),
                *path.allocator(),
            ) {
                for (lower, upper) in range.iter() {
                    let current = path.len();
                    let lower = lower.map(|version| ArcStr::from(version.to_string()));
                    let upper = upper.map(|version| ArcStr::from(version.to_string()));
                    for (operator, value) in
                        MarkerOperator::from_bounds((lower.as_ref(), upper.as_ref()))
                    {
                        path.push(MarkerExpression::String {
                            key: marker.key().into(),
                            operator,
                            value,
                        });
                    }
                    collect_dnf(tree, dnf, path);
                    path.truncate(current);
                }
            }
        }
        MarkerTreeKind::String(marker) => {
            for (tree, range) in collect_edges_in(
                marker.children().filter(|(_, tree)| !tree.is_false()),
                *path.allocator(),
            ) {
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
                if tree.is_false() {
                    continue;
                }
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
                if tree.is_false() {
                    continue;
                }
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
                if tree.is_false() {
                    continue;
                }
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
                if tree.is_false() {
                    continue;
                }
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
fn simplify<A: Allocator + Copy>(dnf: &mut Vec<Vec<MarkerExpression, A>, A>, allocator: A) {
    let mut redundant = Vec::new_in(allocator);
    for i in 0..dnf.len() {
        let clause = &dnf[i];

        // Find redundant terms in this clause.
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

                    // TODO(ibraheem): if we intern variables we could reduce this
                    // from a linear search to an integer `HashSet` lookup
                    clause
                        .iter()
                        .position(|x| x == term)
                        // If the term was already removed from this one, we cannot
                        // depend on it for further simplification.
                        .is_some_and(|i| !redundant.contains(&i))
                }) {
                    redundant.push(skipped);
                    continue 'term;
                }
            }
        }

        // Indices are collected in ascending order. Remove them from the end,
        // retaining the scratch allocation for the following clauses.
        for term in redundant.drain(..).rev() {
            dnf[i].remove(term);
        }
    }

    // Once we have eliminated redundant terms, there may also be redundant clauses.
    // For example, `(A and B) or (not A and B)` would have been simplified above to
    // `(A and B) or B` and can now be further simplified to just `B`.
    'clause: for i in 0..dnf.len() {
        let clause = &dnf[i];

        for (j, other_clause) in dnf.iter().enumerate() {
            // Ignore clauses that are going to be eliminated.
            if i == j || redundant.contains(&j) {
                continue;
            }

            // There is another clause that is a subset of this one, thus this clause is redundant.
            if other_clause.iter().all(|term| {
                // TODO(ibraheem): if we intern variables we could reduce this
                // from a linear search to an integer `HashSet` lookup
                clause.contains(term)
            }) {
                redundant.push(i);
                continue 'clause;
            }
        }
    }

    // Eliminate any redundant clauses.
    for i in redundant.into_iter().rev() {
        dnf.remove(i);
    }
}

/// Sort the clauses in a DNF expression, for backwards compatibility. The goal is to avoid
/// unnecessary churn in the display output of the marker expressions, e.g., when modifying the
/// internal representations used in the marker algebra.
fn sort<A: Allocator>(dnf: &mut [Vec<MarkerExpression, A>]) {
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
) -> impl Iterator<Item = (MarkerTree, Ranges<T>)>
where
    T: Ord + Clone + 'a,
{
    collect_edges_in(map, Global)
}

fn collect_edges_in<'a, T, A>(
    map: impl Iterator<Item = (&'a Ranges<T>, MarkerTree)>,
    allocator: A,
) -> impl Iterator<Item = (MarkerTree, Ranges<T>)>
where
    T: Ord + Clone + 'a,
    A: Allocator,
{
    // Small nodes can reuse the traversal's storage without building a hash table.
    // Larger nodes retain hashed grouping to bound the duplicate lookup cost.
    let maximum_len = map.size_hint().1.unwrap_or(usize::MAX);
    if maximum_len <= 5 {
        let mut paths: Vec<(MarkerTree, Ranges<T>), A> =
            Vec::with_capacity_in(maximum_len, allocator);
        for (range, tree) in map {
            let (start, end) = range.bounding_range().unwrap();
            let range = Ranges::from_range_bounds((start.cloned(), end.cloned()));
            if let Some((_, union)) = paths.iter_mut().find(|(existing, _)| *existing == tree) {
                *union = union.union(&range);
            } else {
                paths.push((tree, range));
            }
        }
        return Either::Left(paths.into_iter());
    }

    let mut paths: IndexMap<_, Ranges<_>, FxBuildHasher> = IndexMap::default();
    for (range, tree) in map {
        // OK because all ranges are guaranteed to be non-empty.
        let (start, end) = range.bounding_range().unwrap();
        // Combine the ranges.
        let range = Ranges::from_range_bounds((start.cloned(), end.cloned()));
        paths
            .entry(tree)
            .and_modify(|union| *union = union.union(&range))
            .or_insert(range);
    }

    Either::Right(paths.into_iter())
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
    use std::thread;

    use bumpalo::Bump;
    use version_ranges::Ranges;

    use super::{collect_edges_in, to_dnf, to_dnf_in, with_dnf};
    use crate::MarkerTree;

    #[test]
    fn dnf_simplifies_successive_clauses() {
        let marker: MarkerTree = "extra == 'a' or extra == 'b' or extra == 'c' or extra == 'd'"
            .parse()
            .unwrap();
        let dnf = marker.to_dnf();
        assert_eq!(dnf.len(), 4);
        assert!(dnf.iter().all(|clause| clause.len() == 1));
        assert_eq!(
            marker.try_to_string().unwrap(),
            "extra == 'a' or extra == 'b' or extra == 'c' or extra == 'd'"
        );
    }

    #[test]
    fn arena_edge_groups_preserve_order_and_gaps() {
        let arena = Bump::new();
        let edges: [(Ranges<i32>, MarkerTree); 6] = [
            (Ranges::singleton(1), MarkerTree::FALSE),
            (Ranges::singleton(2), MarkerTree::TRUE),
            (Ranges::singleton(3), MarkerTree::FALSE),
            (Ranges::singleton(4), MarkerTree::TRUE),
            (Ranges::singleton(5), MarkerTree::FALSE),
            (Ranges::singleton(6), MarkerTree::TRUE),
        ];
        for length in [4, 6] {
            let groups: Vec<_> = collect_edges_in(
                edges[..length].iter().map(|(range, tree)| (range, *tree)),
                &arena,
            )
            .collect();
            let mut false_range = Ranges::singleton(1).union(&Ranges::singleton(3));
            let mut true_range = Ranges::singleton(2).union(&Ranges::singleton(4));
            if length == 6 {
                false_range = false_range.union(&Ranges::singleton(5));
                true_range = true_range.union(&Ranges::singleton(6));
            }
            assert_eq!(
                groups,
                [
                    (MarkerTree::FALSE, false_range),
                    (MarkerTree::TRUE, true_range)
                ]
            );
        }
    }

    #[test]
    fn arena_dnf_matches_owned_clauses() {
        let mut arena = Bump::new();
        for expression in [
            "python_version >= '3.10'",
            "python_version != '3.10.*' and sys_platform != 'win32'",
            "extra == 'test' or (os_name == 'posix' and python_version < '3.12')",
            "(sys_platform == 'win32' and python_version < '3.11') or (sys_platform == 'linux' and python_version >= '3.9')",
        ] {
            let tree = expression.parse::<MarkerTree>().unwrap();
            let expected = to_dnf(tree);
            {
                let actual = to_dnf_in(tree, &arena);
                assert_eq!(
                    actual.iter().map(|clause| &clause[..]).collect::<Vec<_>>(),
                    expected.iter().map(Vec::as_slice).collect::<Vec<_>>(),
                    "{expression}"
                );
            }
            arena.reset();
        }
    }

    #[test]
    fn nested_dnf_keeps_outer_clauses_alive() {
        let outer = "sys_platform == 'win32' or python_version < '3.12'"
            .parse::<MarkerTree>()
            .unwrap();
        let inner = "os_name == 'posix'".parse::<MarkerTree>().unwrap();
        let expected = to_dnf(outer);
        with_dnf(outer, |outer| {
            with_dnf(inner, |inner| assert_eq!(inner.len(), 1));
            assert_eq!(
                outer.iter().map(|clause| &clause[..]).collect::<Vec<_>>(),
                expected.iter().map(Vec::as_slice).collect::<Vec<_>>()
            );
        });
    }

    #[test]
    fn format_during_thread_local_destruction() {
        struct FormatOnDrop;

        impl Drop for FormatOnDrop {
            fn drop(&mut self) {
                let marker = "sys_platform == 'win32'".parse::<MarkerTree>().unwrap();
                assert_eq!(
                    marker.contents().unwrap().to_string(),
                    "sys_platform == 'win32'"
                );
            }
        }

        thread_local! {
            static FORMAT_ON_DROP: FormatOnDrop = const { FormatOnDrop };
        }

        thread::spawn(|| {
            FORMAT_ON_DROP.with(|_| {});
            let marker = "os_name == 'posix'".parse::<MarkerTree>().unwrap();
            assert_eq!(marker.contents().unwrap().to_string(), "os_name == 'posix'");
        })
        .join()
        .unwrap();
    }
}
