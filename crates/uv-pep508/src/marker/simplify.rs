use std::alloc::{Allocator, Global};
use std::fmt;
use std::ops::Bound;

use arcstr::ArcStr;
use hashbrown::HashMap;
use indexmap::IndexMap;
use itertools::{Either, Itertools};
use rustc_hash::FxBuildHasher;
use version_ranges::Ranges;

use uv_allocator::{Arena, with_arena};
use uv_pep440::{Version, VersionSpecifier};

use crate::marker::tree::ContainerOperator;
use crate::{ExtraOperator, MarkerExpression, MarkerOperator, MarkerTree, MarkerTreeKind};

/// Use temporary DNF clauses, then release their storage together.
pub(crate) fn with_dnf<R>(
    tree: MarkerTree,
    use_dnf: impl FnOnce(&[Vec<MarkerExpression, &Arena>]) -> R,
) -> R {
    with_arena(|arena| use_dnf(&to_dnf_in(tree, arena)))
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
    with_arena(|allocator| to_owned_dnf_in(tree, allocator))
}

fn to_owned_dnf_in<A: Allocator + Copy>(
    tree: MarkerTree,
    allocator: A,
) -> Vec<Vec<MarkerExpression>> {
    // Public DNF values outlive the traversal arena. Move only the surviving
    // clauses into ordinary owned vectors after simplification is complete.
    to_dnf_in(tree, allocator)
        .into_iter()
        .map(|clause| clause.into_iter().collect())
        .collect()
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
            for (tree, range) in collect_edges_in(marker.edges(), *path.allocator()) {
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
            for (tree, range) in collect_edges_in(marker.edges(), *path.allocator()) {
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
            for (tree, range) in collect_edges_in(marker.children(), *path.allocator()) {
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
fn simplify<A: Allocator + Copy>(dnf: &mut Vec<Vec<MarkerExpression, A>, A>, allocator: A) {
    if dnf.len() >= 8 && simplify_indexed(dnf, allocator) {
        return;
    }
    simplify_linear(dnf, allocator);
}

/// Intern large DNF expressions so subset checks compare bit sets instead of marker values.
/// The index follows the clause order because each simplification can affect later clauses.
fn simplify_indexed<A: Allocator + Copy, B: Allocator + Copy>(
    dnf: &mut Vec<Vec<MarkerExpression, A>, A>,
    allocator: B,
) -> bool {
    let mut terms = HashMap::with_hasher_in(FxBuildHasher, allocator);
    let mut expressions = Vec::new_in(allocator);
    let mut clauses = Vec::with_capacity_in(dnf.len(), allocator);
    for clause in dnf.iter() {
        let mut indexes = Vec::with_capacity_in(clause.len(), allocator);
        for term in clause {
            let index = *terms.entry(term).or_insert_with(|| {
                let index = expressions.len();
                expressions.push(term);
                index
            });
            indexes.push(index);
        }
        clauses.push(indexes);
    }

    let words = expressions.len().div_ceil(64);
    // A sparse expression can have far more distinct terms than terms per clause.
    // Bound the dense index to 32 MiB and use the linear algorithm beyond that.
    if dnf.len().saturating_mul(words) > 4 * 1024 * 1024 {
        return false;
    }
    let mut negated = Vec::with_capacity_in(expressions.len(), allocator);
    for expression in &expressions {
        negated.push(negate_expression(expression).and_then(|term| terms.get(&term).copied()));
    }
    drop(terms);
    drop(expressions);

    let mut sets = Vec::with_capacity_in(clauses.len(), allocator);
    for clause in &clauses {
        let mut set = Vec::with_capacity_in(words, allocator);
        set.resize(words, 0u64);
        for &term in clause {
            // The linear algorithm tracks the first occurrence of a repeated term.
            // A set cannot represent that distinction.
            if set[term / 64] & (1 << (term % 64)) != 0 {
                return false;
            }
            set[term / 64] |= 1 << (term % 64);
        }
        sets.push(set);
    }

    for i in 0..clauses.len() {
        for position in 0..clauses[i].len() {
            let skipped = clauses[i][position];
            let redundant = sets.iter().enumerate().any(|(j, other)| {
                if i == j || other[skipped / 64] & (1 << (skipped % 64)) != 0 {
                    return false;
                }
                other
                    .iter()
                    .zip(&sets[i])
                    .enumerate()
                    .all(|(word, (&other, &this))| {
                        let mut missing = other & !this;
                        while missing != 0 {
                            let term = word * 64 + missing.trailing_zeros() as usize;
                            if negated[term] != Some(skipped) {
                                return false;
                            }
                            missing &= missing - 1;
                        }
                        true
                    })
            });
            if redundant {
                clauses[i][position] = usize::MAX;
                sets[i][skipped / 64] &= !(1 << (skipped % 64));
            }
        }
    }

    let mut redundant = Vec::with_capacity_in(clauses.len(), allocator);
    redundant.resize(clauses.len(), false);
    for i in 0..clauses.len() {
        redundant[i] = sets.iter().enumerate().any(|(j, other)| {
            i != j
                && !redundant[j]
                && other
                    .iter()
                    .zip(&sets[i])
                    .all(|(&other, &this)| other & !this == 0)
        });
    }

    for (clause, indexes) in dnf.iter_mut().zip(clauses) {
        let mut position = 0;
        clause.retain(|_| {
            let keep = indexes[position] != usize::MAX;
            position += 1;
            keep
        });
    }
    let mut position = 0;
    dnf.retain(|_| {
        let keep = !redundant[position];
        position += 1;
        keep
    });
    true
}

fn simplify_linear<A: Allocator + Copy>(dnf: &mut Vec<Vec<MarkerExpression, A>, A>, allocator: A) {
    for i in 0..dnf.len() {
        let clause = &dnf[i];

        // Find redundant terms in this clause.
        let mut redundant_terms = Vec::new_in(allocator);
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
                        .is_some_and(|i| !redundant_terms.contains(&i))
                }) {
                    redundant_terms.push(skipped);
                    continue 'term;
                }
            }
        }

        // Eliminate any redundant terms.
        redundant_terms.sort_by(|a, b| b.cmp(a));
        for term in redundant_terms {
            dnf[i].remove(term);
        }
    }

    // Once we have eliminated redundant terms, there may also be redundant clauses.
    // For example, `(A and B) or (not A and B)` would have been simplified above to
    // `(A and B) or B` and can now be further simplified to just `B`.
    let mut redundant_clauses = Vec::new_in(allocator);
    'clause: for i in 0..dnf.len() {
        let clause = &dnf[i];

        for (j, other_clause) in dnf.iter().enumerate() {
            // Ignore clauses that are going to be eliminated.
            if i == j || redundant_clauses.contains(&j) {
                continue;
            }

            // There is another clause that is a subset of this one, thus this clause is redundant.
            if other_clause.iter().all(|term| {
                // TODO(ibraheem): if we intern variables we could reduce this
                // from a linear search to an integer `HashSet` lookup
                clause.contains(term)
            }) {
                redundant_clauses.push(i);
                continue 'clause;
            }
        }
    }

    // Eliminate any redundant clauses.
    for i in redundant_clauses.into_iter().rev() {
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
    map: impl ExactSizeIterator<Item = (&'a Ranges<T>, MarkerTree)>,
    allocator: A,
) -> impl Iterator<Item = (MarkerTree, Ranges<T>)>
where
    T: Ord + Clone + 'a,
    A: Allocator,
{
    // Small nodes can reuse the traversal's storage without building a hash table.
    // Larger nodes retain hashed grouping to bound the duplicate lookup cost.
    if map.len() <= 5 {
        let mut paths: Vec<(MarkerTree, Ranges<T>), A> =
            Vec::with_capacity_in(map.len(), allocator);
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

/// Construct the single expression accepted by [`is_negation`] for this left operand.
fn negate_expression(expression: &MarkerExpression) -> Option<MarkerExpression> {
    Some(match expression {
        MarkerExpression::Version { key, specifier } => MarkerExpression::Version {
            key: *key,
            specifier: VersionSpecifier::from_version(
                specifier.operator().negate()?,
                specifier.version().clone(),
            )
            .ok()?,
        },
        MarkerExpression::VersionIn {
            key,
            versions,
            operator,
        } => MarkerExpression::VersionIn {
            key: *key,
            versions: versions.clone(),
            operator: negate_container_operator(*operator),
        },
        MarkerExpression::String {
            key,
            operator,
            value,
        } => MarkerExpression::String {
            key: *key,
            operator: operator.negate()?,
            value: value.clone(),
        },
        MarkerExpression::List { pair, operator } => MarkerExpression::List {
            pair: pair.clone(),
            operator: negate_container_operator(*operator),
        },
        MarkerExpression::Extra { name, operator } => MarkerExpression::Extra {
            name: name.clone(),
            operator: operator.negate(),
        },
    })
}

fn negate_container_operator(operator: ContainerOperator) -> ContainerOperator {
    match operator {
        ContainerOperator::In => ContainerOperator::NotIn,
        ContainerOperator::NotIn => ContainerOperator::In,
    }
}

#[cfg(test)]
mod tests {
    use std::alloc::Global;
    use std::thread;

    use uv_allocator::Arena;
    use uv_pep440::{Operator, Version, VersionSpecifier};
    use version_ranges::Ranges;

    use super::{
        collect_edges_in, is_negation, negate_expression, simplify_indexed, simplify_linear,
        to_dnf, to_dnf_in, with_dnf,
    };
    use crate::{MarkerExpression, MarkerTree, MarkerValueVersion};

    #[test]
    fn indexed_simplification_matches_linear_order() {
        let mut expressions: Vec<_> = [
            "extra == 'a'",
            "extra != 'a'",
            "extra == 'b'",
            "extra != 'b'",
            "python_version == '3.10'",
            "python_version != '3.10'",
            "python_version >= '3.9'",
            "python_version < '3.9'",
            "python_version ~= '3.9'",
            "python_version in '3.9 3.10'",
            "python_version not in '3.9 3.10'",
            "sys_platform == 'linux'",
            "sys_platform != 'linux'",
            "'test' in extras",
            "'test' not in extras",
        ]
        .map(|value| MarkerExpression::from_str(value).unwrap().unwrap())
        .into();
        expressions.push(MarkerExpression::Version {
            key: MarkerValueVersion::PythonVersion,
            specifier: VersionSpecifier::from_version(Operator::ExactEqual, Version::new([3, 10]))
                .unwrap(),
        });
        for left in &expressions {
            for right in &expressions {
                assert_eq!(
                    is_negation(left, right),
                    negate_expression(left).as_ref() == Some(right)
                );
            }
        }
        let arena = Arena::new();
        let mut seed = 17u64;
        let mut indexed_cases = 0;
        for case in 0..2000 {
            let mut next = || {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                (seed >> 32) as usize
            };
            let mut expected = Vec::new();
            for _ in 0..next() % 30 {
                let mut clause = Vec::new();
                for _ in 0..next() % 12 {
                    let term = expressions[next() % expressions.len()].clone();
                    if case % 2 == 0 || !clause.contains(&term) {
                        clause.push(term);
                    }
                }
                expected.push(clause);
            }
            let mut actual = expected.clone();
            simplify_linear(&mut expected, Global);
            if simplify_indexed(&mut actual, &arena) {
                indexed_cases += 1;
            } else {
                simplify_linear(&mut actual, Global);
            }
            assert_eq!(actual, expected, "case {case}");
        }
        assert!(indexed_cases >= 1000);
    }

    #[test]
    fn arena_edge_groups_preserve_order_and_gaps() {
        let arena = Arena::new();
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
        let mut arena = Arena::new();
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
