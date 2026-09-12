//! Marker tree operations that use Algebraic Decision Diagrams (ADD).
//!
//! An ADD contains decision nodes and two terminal nodes, `true` and `false`. Each decision node
//! represents a marker variable. An edge to a child assigns a value to that variable. Depending on
//! the variable, an edge contains a binary value or a disjoint set of ranges. This differs from a
//! traditional Binary Decision Diagram.
//!
//! For example, the marker `python_version > '3.7' and os_name == 'Linux'` creates the following
//! marker tree:
//!
//! ```text
//! python_version:
//!   (> '3.7')  -> os_name:
//!                   (> 'Linux')  -> FALSE
//!                   (== 'Linux') -> TRUE
//!                   (< 'Linux')  -> FALSE
//!   (<= '3.7') -> FALSE
//! ```
//!
//! Marker trees use Reduced Ordered ADDs. An ADD is ordered when variables appear in the same
//! order on every path from the root. An ADD is reduced when:
//! - It merges isomorphic nodes.
//! - It removes nodes with isomorphic children.
//!
//! These rules make marker trees canonical for each marker function and variable order. The
//! variable order is fixed at compile time, so construction normalizes equivalent marker trees.
//! This identifies always-true and unsatisfiable marker trees. The resolver uses this information
//! when it forks.
//!
//! ADD operations, such as conjunction and negation, take polynomial time. Universal resolution
//! uses these operations to combine marker trees. Because ADDs solve the SAT problem, constructing
//! an arbitrary ADD can take exponential time in the worst case. In practice, marker trees have
//! few variables, and user-provided marker trees are usually simple.
//!
//! Complemented edges let a marker tree and its complement share one internal node. Negating a
//! marker tree therefore takes constant time. The implementation needs only one operation for `AND`
//! and `OR`; it derives the other from its De Morgan complement.
//!
//! The global [`Interner`] creates and manages ADDs. A [`NodeId`] references a [`Node`] in the
//! interner or a terminal `true` or `false` node. This reference can be complemented. Interning
//! merges isomorphic nodes across all marker trees.

use std::cmp::Ordering;
use std::fmt;
use std::ops::Bound;
use std::sync::{LazyLock, Mutex, MutexGuard};

use arcstr::ArcStr;
use itertools::{Either, Itertools};
use rustc_hash::FxHashMap;
use version_ranges::Ranges;

use uv_pep440::{Operator, Version, VersionSpecifier, release_specifier_to_range};

use crate::marker::MarkerValueExtra;
use crate::marker::lowering::{
    CanonicalMarkerListPair, CanonicalMarkerValueExtra, CanonicalMarkerValueString,
    CanonicalMarkerValueVersion,
};
use crate::marker::tree::ContainerOperator;
use crate::{
    ExtraOperator, MarkerExpression, MarkerOperator, MarkerValueString, MarkerValueVersion,
};

/// The global node interner.
pub(crate) static INTERNER: LazyLock<Interner> = LazyLock::new(Interner::default);

/// An interner for decision nodes.
///
/// Interning merges isomorphic decision nodes and makes node comparisons inexpensive.
#[derive(Default)]
pub(crate) struct Interner {
    pub(crate) shared: InternerShared,
    state: Mutex<InternerState>,
}

/// The shared [`Interner`] state that does not require a lock.
#[derive(Default)]
pub(crate) struct InternerShared {
    /// A list of unique [`Node`]s.
    nodes: boxcar::Vec<Node>,
}

/// The mutable [`Interner`] state, stored behind a lock.
#[derive(Default)]
struct InternerState {
    /// Maps each [`Node`] to a unique [`NodeId`] index in [`InternerShared`].
    unique: FxHashMap<Node, NodeId>,

    /// A cache for `AND` operations between two nodes.
    /// The `OR` operation uses `AND`.
    cache: FxHashMap<(NodeId, NodeId), NodeId>,

    /// The [`NodeId`] for the disjunction of known, mutually incompatible markers.
    exclusions: Option<NodeId>,
}

impl InternerShared {
    /// Returns the node for the given [`NodeId`].
    pub(crate) fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }
}

impl Interner {
    /// Locks the interner state and returns a guard for marker operations.
    pub(crate) fn lock(&self) -> InternerGuard<'_> {
        InternerGuard {
            state: self.state.lock().unwrap(),
            shared: &self.shared,
        }
    }
}

/// A lock of [`InternerState`].
pub(crate) struct InternerGuard<'a> {
    state: MutexGuard<'a, InternerState>,
    shared: &'a InternerShared,
}

impl InternerGuard<'_> {
    /// Creates a decision node with the given variable and children.
    fn create_node(&mut self, var: Variable, children: Edges) -> NodeId {
        let mut node = Node { var, children };
        let mut first = node.children.nodes().next().unwrap();

        // Complementing the root and every child edge produces an equivalent node. Never
        // complement the first child edge, so each marker keeps one canonical representation.
        let mut flipped = false;
        if first.is_complement() {
            node = node.not();
            first = first.not();
            flipped = true;
        }

        // Reduction: If every child references the same node, return that node without the parent.
        if node.children.nodes().all(|node| node == first) {
            return if flipped { first.not() } else { first };
        }

        // Insert the node.
        // Probing before inserting keeps the clone off the common path where an isomorphic node
        // has already been interned. Cloning a [`Node`] copies every outgoing edge range.
        let id = if let Some(&id) = self.state.unique.get(&node) {
            id
        } else {
            let id = NodeId::new(self.shared.nodes.push(node.clone()), false);
            self.state.unique.insert(node, id);
            id
        };

        if flipped { id.not() } else { id }
    }

    /// Returns a decision node for a single marker expression.
    pub(crate) fn expression(&mut self, expr: MarkerExpression) -> NodeId {
        let (var, children) = match expr {
            // A version-key variable. Its edges contain disjoint version ranges.
            MarkerExpression::Version { key, specifier } => match key {
                MarkerValueVersion::ImplementationVersion => (
                    Variable::Version(CanonicalMarkerValueVersion::ImplementationVersion),
                    Edges::from_specifier(specifier),
                ),
                MarkerValueVersion::PythonFullVersion => (
                    Variable::Version(CanonicalMarkerValueVersion::PythonFullVersion),
                    Edges::from_specifier(specifier),
                ),
                // Normalize `python_version` markers to `python_full_version` nodes.
                MarkerValueVersion::PythonVersion => {
                    match python_version_to_full_version(specifier.only_release()) {
                        Ok(specifier) => (
                            Variable::Version(CanonicalMarkerValueVersion::PythonFullVersion),
                            Edges::from_specifier(specifier),
                        ),
                        Err(node) => return node,
                    }
                }
            },
            // A version-key variable. Its edges contain disjoint version ranges.
            MarkerExpression::VersionIn {
                key,
                versions,
                operator,
            } => match key {
                MarkerValueVersion::ImplementationVersion => (
                    Variable::Version(CanonicalMarkerValueVersion::ImplementationVersion),
                    Edges::from_versions(versions, operator),
                ),
                MarkerValueVersion::PythonFullVersion => (
                    Variable::Version(CanonicalMarkerValueVersion::PythonFullVersion),
                    Edges::from_versions(versions, operator),
                ),
                // Normalize `python_version` markers to `python_full_version` nodes.
                MarkerValueVersion::PythonVersion => {
                    match Edges::from_python_versions(versions, operator) {
                        Ok(edges) => (
                            Variable::Version(CanonicalMarkerValueVersion::PythonFullVersion),
                            edges,
                        ),
                        Err(node) => return node,
                    }
                }
            },
            // The `in` and `contains` operators do not select one value and can overlap. For
            // example, `'nux' in os_name` and `os_name == 'Linux'` can both be `true` in the same
            // marker environment. One variable cannot represent both expressions. Represent these
            // operators and their negations as separate variables outside the key's value range.
            //
            // The `in` operator can prevent simplification to a constant `true` or `false`. For
            // example, `os_name == 'Windows' and os_name in 'Linux'` is not obviously unsatisfiable.
            MarkerExpression::String {
                key,
                operator: MarkerOperator::In,
                value,
            } => (
                Variable::In {
                    key: key.into(),
                    value,
                },
                Edges::from_bool(true),
            ),
            MarkerExpression::String {
                key,
                operator: MarkerOperator::NotIn,
                value,
            } => (
                Variable::In {
                    key: key.into(),
                    value,
                },
                Edges::from_bool(false),
            ),
            MarkerExpression::String {
                key,
                operator: MarkerOperator::Contains,
                value,
            } => (
                Variable::Contains {
                    key: key.into(),
                    value,
                },
                Edges::from_bool(true),
            ),
            MarkerExpression::String {
                key,
                operator: MarkerOperator::NotContains,
                value,
            } => (
                Variable::Contains {
                    key: key.into(),
                    value,
                },
                Edges::from_bool(false),
            ),
            // A string-key variable. Its edges contain disjoint string ranges.
            MarkerExpression::String {
                key,
                operator,
                value,
            } => {
                // Normalize `platform_system` markers to `sys_platform` nodes.
                //
                // The `platform` module is "primarily intended for diagnostic information to be
                // read by humans."
                //
                // Normalize only when both values are exactly equivalent. For example, normalize
                // `platform_system == 'Windows'` to `sys_platform == 'win32'`. Do not normalize
                // `platform_system == 'FreeBSD'` to `sys_platform == 'freebsd'`, because FreeBSD
                // usually includes a major version in its `sys.platform` output.
                //
                // Record known incompatible values that cannot be normalized in `exclusions`.
                //
                // See: https://discuss.python.org/t/clarify-usage-of-platform-system/70900
                let (key, value) = match (key, value.as_ref()) {
                    (MarkerValueString::PlatformSystem, "Windows") => (
                        CanonicalMarkerValueString::SysPlatform,
                        arcstr::literal!("win32"),
                    ),
                    (MarkerValueString::PlatformSystem, "Darwin") => (
                        CanonicalMarkerValueString::SysPlatform,
                        arcstr::literal!("darwin"),
                    ),
                    (MarkerValueString::PlatformSystem, "Linux") => (
                        CanonicalMarkerValueString::SysPlatform,
                        arcstr::literal!("linux"),
                    ),
                    (MarkerValueString::PlatformSystem, "AIX") => (
                        CanonicalMarkerValueString::SysPlatform,
                        arcstr::literal!("aix"),
                    ),
                    (MarkerValueString::PlatformSystem, "Emscripten") => (
                        CanonicalMarkerValueString::SysPlatform,
                        arcstr::literal!("emscripten"),
                    ),
                    // See: https://peps.python.org/pep-0738/#sys
                    (MarkerValueString::PlatformSystem, "Android") => (
                        CanonicalMarkerValueString::SysPlatform,
                        arcstr::literal!("android"),
                    ),
                    _ => (key.into(), value),
                };
                (
                    Variable::String(key),
                    Edges::from_string(key, operator, value),
                )
            }
            MarkerExpression::List { pair, operator } => (
                Variable::List(pair),
                Edges::from_bool(operator == ContainerOperator::In),
            ),
            // A variable that records whether a specific extra exists.
            MarkerExpression::Extra {
                name: MarkerValueExtra::Extra(extra),
                operator: ExtraOperator::Equal,
            } => (
                Variable::Extra(CanonicalMarkerValueExtra::Extra(extra)),
                Edges::from_bool(true),
            ),
            MarkerExpression::Extra {
                name: MarkerValueExtra::Extra(extra),
                operator: ExtraOperator::NotEqual,
            } => (
                Variable::Extra(CanonicalMarkerValueExtra::Extra(extra)),
                Edges::from_bool(false),
            ),
            // Invalid `extra` names are always `false`.
            MarkerExpression::Extra {
                name: MarkerValueExtra::Arbitrary(_),
                ..
            } => return NodeId::FALSE,
        };

        self.create_node(var, children)
    }

    /// Returns a decision node representing the disjunction of two nodes.
    fn or(&mut self, xi: NodeId, yi: NodeId) -> NodeId {
        // Use inexpensive negation to implement OR with its De Morgan complement.
        self.and(xi.not(), yi.not()).not()
    }

    /// Returns a decision node representing the disjunction of two nodes known not to have a
    /// trivial disjunction.
    pub(crate) fn or_nontrivial(&mut self, xi: NodeId, yi: NodeId) -> NodeId {
        self.and_nontrivial(xi.not(), yi.not()).not()
    }

    /// Returns a decision node representing the conjunction of two nodes.
    fn and(&mut self, xi: NodeId, yi: NodeId) -> NodeId {
        if let Some(result) = xi.and_trivial(yi) {
            return result;
        }

        self.and_nontrivial(xi, yi)
    }

    /// Returns a decision node representing the conjunction of two nodes known not to have a
    /// trivial conjunction.
    pub(crate) fn and_nontrivial(&mut self, xi: NodeId, yi: NodeId) -> NodeId {
        debug_assert!(
            xi.and_trivial(yi).is_none(),
            "`and_nontrivial` requires a non-trivial conjunction"
        );

        // The operation was memoized.
        if let Some(result) = self.state.cache.get(&(xi, yi)) {
            return *result;
        }

        let (x, y) = (self.shared.node(xi), self.shared.node(yi));

        // Check whether the conjunction _could_ contain a conflict.
        //
        // Check only the top level. These variables have higher priority, so they _must_ appear at
        // the top when present. If they are absent there, they cannot appear in any child.
        let conflicts = x.var.is_conflicting_variable() && y.var.is_conflicting_variable();

        // Apply Shannon expansion to the higher-order variable.
        let (func, children) = match x.var.cmp(&y.var) {
            // X has higher order than Y. Apply Y to every child of X.
            Ordering::Less => {
                let children = x.children.map(xi, |node| self.and(node, yi));
                (x.var.clone(), children)
            }
            // Y has higher order than X. Apply X to every child of Y.
            Ordering::Greater => {
                let children = y.children.map(yi, |node| self.and(node, xi));
                (y.var.clone(), children)
            }
            // X and Y represent the same variable. Merge their children.
            Ordering::Equal => {
                let children = x.children.apply(xi, &y.children, yi, |x, y| self.and(x, y));
                (x.var.clone(), children)
            }
        };

        // Create the output node.
        let node = self.create_node(func, children);

        // If the node includes known incompatibilities, map it to `false`.
        let node = if conflicts {
            let exclusions = self.exclusions();
            if self.disjointness(node, exclusions.not()) {
                NodeId::FALSE
            } else {
                node
            }
        } else {
            node
        };

        // Memoize the result of this operation.
        //
        // Fixed variable ordering can duplicate subgraphs across branches. Memoization keeps ADD
        // operations within polynomial time.
        self.state.cache.insert((xi, yi), node);

        node
    }

    /// Returns `true` if both marker trees cannot apply to the same environment.
    pub(crate) fn is_disjoint_nontrivial(&mut self, xi: NodeId, yi: NodeId) -> bool {
        debug_assert!(
            xi.is_disjoint_trivial(yi).is_none(),
            "`is_disjoint_nontrivial` requires non-trivial disjointness"
        );

        let (x, y) = (self.shared.node(xi), self.shared.node(yi));

        // Check whether the conjunction _could_ contain a conflict.
        //
        // Check only the top level. These variables have higher priority, so they _must_ appear at
        // the top when present. If they are absent there, they cannot appear in any child.
        if x.var.is_conflicting_variable() && y.var.is_conflicting_variable() {
            return self.and(xi, yi).is_false();
        }

        // Apply Shannon expansion to the higher-order variable.
        match x.var.cmp(&y.var) {
            // X has higher order than Y. Y must be disjoint with every child of X.
            Ordering::Less => x
                .children
                .nodes()
                .all(|x| self.disjointness(x.negate(xi), yi)),
            // Y has higher order than X. X must be disjoint with every child of Y.
            Ordering::Greater => y
                .children
                .nodes()
                .all(|y| self.disjointness(y.negate(yi), xi)),
            // X and Y represent the same variable. Their merged edges must be unsatisfiable.
            Ordering::Equal => x.children.is_disjoint(xi, &y.children, yi, self),
        }
    }

    /// Returns `true` if both marker trees cannot apply to the same environment.
    fn disjointness(&mut self, xi: NodeId, yi: NodeId) -> bool {
        // NOTE(charlie): This is equivalent to `is_disjoint`, with the exception that it doesn't
        // perform the mutually-incompatible marker check. If it did, we'd create an infinite loop,
        // since `is_disjoint` calls `and` (when relevant variables are present) which then calls
        // `disjointness`.

        // `false` is disjoint with any marker.
        if xi.is_false() || yi.is_false() {
            return true;
        }
        // `true` is not disjoint with any marker except `false`.
        if xi.is_true() || yi.is_true() {
            return false;
        }
        // `X` and `X` are not disjoint.
        if xi == yi {
            return false;
        }
        // `X` and `not X` are disjoint by definition.
        if xi.not() == yi {
            return true;
        }

        let (x, y) = (self.shared.node(xi), self.shared.node(yi));

        // Apply Shannon expansion to the higher-order variable.
        match x.var.cmp(&y.var) {
            // X has higher order than Y. Y must be disjoint with every child of X.
            Ordering::Less => x
                .children
                .nodes()
                .all(|x| self.disjointness(x.negate(xi), yi)),
            // Y has higher order than X. X must be disjoint with every child of Y.
            Ordering::Greater => y
                .children
                .nodes()
                .all(|y| self.disjointness(y.negate(yi), xi)),
            // X and Y represent the same variable. Their merged edges must be unsatisfiable.
            Ordering::Equal => x.children.is_disjoint(xi, &y.children, yi, self),
        }
    }

    // Restrict the output of selected boolean variables in the tree.
    //
    // If `f` returns `Some`, simplify the tree as if the variable has that boolean value. If `f`
    // returns `None`, leave the variable unchanged.
    pub(crate) fn restrict_by(
        &mut self,
        i: NodeId,
        f: &impl Fn(&Variable) -> Option<bool>,
    ) -> NodeId {
        if matches!(i, NodeId::TRUE | NodeId::FALSE) {
            return i;
        }

        let node = self.shared.node(i);
        if let Edges::Boolean { high, low } = node.children {
            if let Some(value) = f(&node.var) {
                // Restrict this variable to the given output by merging it
                // with the relevant child.
                let node = if value { high } else { low };
                return self.restrict_by(node.negate(i), f);
            }
        }

        // Restrict all nodes recursively.
        let children = node.children.map(i, |node| self.restrict_by(node, f));
        self.create_node(node.var.clone(), children)
    }

    /// Restricts a marker under the assumption that another marker is true.
    ///
    /// The returned marker matches `value` wherever `assumption` is true. Its value outside the
    /// assumption is unspecified. This removes decisions that only restate the assumption.
    pub(crate) fn restrict(&mut self, value: NodeId, assumption: NodeId) -> NodeId {
        let mut cache = FxHashMap::default();
        self.restrict_cached(value, assumption, &mut cache)
    }

    fn restrict_cached(
        &mut self,
        value: NodeId,
        assumption: NodeId,
        cache: &mut FxHashMap<(NodeId, NodeId), NodeId>,
    ) -> NodeId {
        if assumption.is_true() || matches!(value, NodeId::TRUE | NodeId::FALSE) {
            return value;
        }
        if assumption.is_false() {
            return NodeId::FALSE;
        }
        if value == assumption {
            return NodeId::TRUE;
        }
        if value == assumption.not() {
            return NodeId::FALSE;
        }
        if let Some(&result) = cache.get(&(value, assumption)) {
            return result;
        }

        let value_node = self.shared.node(value);
        let assumption_node = self.shared.node(assumption);
        let result = match value_node.var.cmp(&assumption_node.var) {
            Ordering::Less => {
                let children = value_node.children.map(value, |value| {
                    self.restrict_cached(value, assumption, cache)
                });
                self.create_node(value_node.var.clone(), children)
            }
            Ordering::Greater => {
                // The value does not depend on this variable. Existentially quantify it out of the
                // assumption, and continue with the remaining variables.
                let mut quantified_assumption = NodeId::FALSE;
                for child in assumption_node.children.nodes() {
                    quantified_assumption =
                        self.or(quantified_assumption, child.negate(assumption));
                }
                self.restrict_cached(value, quantified_assumption, cache)
            }
            Ordering::Equal => {
                // Split both trees into matching ranges. Replace any ranges that are unreachable
                // under the assumption with the first reachable child, simplifying them out of the
                // resulting marker.
                let mut fallback = None;
                value_node.children.apply(
                    value,
                    &assumption_node.children,
                    assumption,
                    |value, assumption| {
                        if assumption.is_false() {
                            NodeId::FALSE
                        } else {
                            let result = self.restrict_cached(value, assumption, cache);
                            fallback.get_or_insert(result);
                            result
                        }
                    },
                );
                let Some(fallback) = fallback else {
                    return NodeId::FALSE;
                };
                let children = value_node.children.apply(
                    value,
                    &assumption_node.children,
                    assumption,
                    |value, assumption| {
                        if assumption.is_false() {
                            fallback
                        } else {
                            self.restrict_cached(value, assumption, cache)
                        }
                    },
                );
                self.create_node(value_node.var.clone(), children)
            }
        };

        cache.insert((value, assumption), result);
        result
    }

    /// Returns a new tree that contains only non-`extra` nodes.
    ///
    /// Returns an always-true tree if every node is an `extra` node.
    ///
    /// Assumes every `extra` node is true.
    ///
    /// For example, the marker
    /// `((os_name == ... and extra == foo) or (sys_platform == ... and extra != foo))`,
    /// becomes
    /// `os_name == ... or sys_platform == ...`.
    pub(crate) fn without_extras(&mut self, i: NodeId) -> NodeId {
        let mut cache = FxHashMap::default();
        self.without_extras_cached(i, &mut cache)
    }

    fn without_extras_cached(
        &mut self,
        mut i: NodeId,
        cache: &mut FxHashMap<NodeId, NodeId>,
    ) -> NodeId {
        if matches!(i, NodeId::TRUE | NodeId::FALSE) {
            return i;
        }

        if let Some(&cached) = cache.get(&i) {
            return cached;
        }

        let original = i;
        let parent = i;
        let node = self.shared.node(i);
        let result = if matches!(node.var, Variable::Extra(_)) {
            i = NodeId::FALSE;
            for child in node.children.nodes() {
                i = self.or(i, child.negate(parent));
            }
            if i.is_true() {
                NodeId::TRUE
            } else {
                self.without_extras_cached(i, cache)
            }
        } else {
            // Restrict all nodes recursively.
            let children = node
                .children
                .map(i, |node| self.without_extras_cached(node, cache));
            self.create_node(node.var.clone(), children)
        };
        cache.insert(original, result);
        result
    }

    /// Returns a new tree that contains only `extra` nodes.
    ///
    /// Returns an always-true tree if no `extra` nodes exist.
    ///
    /// Assumes every non-`extra` node is true.
    pub(crate) fn only_extras(&mut self, mut i: NodeId) -> NodeId {
        if matches!(i, NodeId::TRUE | NodeId::FALSE) {
            return i;
        }

        let parent = i;
        let node = self.shared.node(i);
        if !matches!(node.var, Variable::Extra(_)) {
            i = NodeId::FALSE;
            for child in node.children.nodes() {
                i = self.or(i, child.negate(parent));
            }
            if i.is_true() {
                return NodeId::TRUE;
            }
            self.only_extras(i)
        } else {
            // Restrict all nodes recursively.
            let children = node.children.map(i, |node| self.only_extras(node));
            self.create_node(node.var.clone(), children)
        }
    }

    /// Simplifies this tree under the *assumption* that the given Python version range is true and
    /// its complement is false.
    ///
    /// For example, with `requires-python = '>=3.8'` and a marker tree of
    /// `python_full_version >= '3.8' and python_full_version <= '3.10'`, this
    /// becomes `python_full_version <= '3.10'`.
    pub(crate) fn simplify_python_versions(
        &mut self,
        i: NodeId,
        py_lower: Bound<&Version>,
        py_upper: Bound<&Version>,
    ) -> NodeId {
        if matches!(i, NodeId::TRUE | NodeId::FALSE)
            || matches!((py_lower, py_upper), (Bound::Unbounded, Bound::Unbounded))
        {
            return i;
        }

        let node = self.shared.node(i);
        // Find a `python_full_version` expression or simplify recursively.
        let Node {
            var: Variable::Version(CanonicalMarkerValueVersion::PythonFullVersion),
            children: Edges::Version { edges },
        } = node
        else {
            // Simplify all nodes recursively.
            let children = node.children.map(i, |node_id| {
                self.simplify_python_versions(node_id, py_lower, py_upper)
            });
            return self.create_node(node.var.clone(), children);
        };
        let py_range = Ranges::from_range_bounds((py_lower.cloned(), py_upper.cloned()));
        if py_range.is_empty() {
            // The bounds cannot match any version, so the marker is always false.
            return NodeId::FALSE;
        }
        let mut new = SmallVec::new();
        for &(ref range, node) in edges {
            let overlap = range.intersection(&py_range);
            if overlap.is_empty() {
                continue;
            }
            new.push((overlap.clone(), node));
        }

        // Every remaining range intersects the Python version bounds. Extend the lower and upper
        // bounds to negative and positive infinity, respectively.
        //
        // The resulting marker applies only when the Python version bounds are already satisfied.
        let &(ref first_range, first_node_id) = new.first().unwrap();
        let first_upper = first_range.bounding_range().unwrap().1;
        let clipped = Ranges::from_range_bounds((Bound::Unbounded, first_upper.cloned()));
        *new.first_mut().unwrap() = (clipped, first_node_id);

        let &(ref last_range, last_node_id) = new.last().unwrap();
        let last_lower = last_range.bounding_range().unwrap().0;
        let clipped = Ranges::from_range_bounds((last_lower.cloned(), Bound::Unbounded));
        *new.last_mut().unwrap() = (clipped, last_node_id);

        self.create_node(node.var.clone(), Edges::Version { edges: new })
            .negate(i)
    }

    /// Adds the given Python version range as a requirement for this marker tree.
    ///
    /// For example, with `requires-python = '>=3.8'` and a marker tree of
    /// `python_full_version <= '3.10'`, the marker becomes
    /// `python_full_version >= '3.8' and python_full_version <= '3.10'`.
    pub(crate) fn complexify_python_versions(
        &mut self,
        i: NodeId,
        py_lower: Bound<&Version>,
        py_upper: Bound<&Version>,
    ) -> NodeId {
        if matches!(i, NodeId::FALSE)
            || matches!((py_lower, py_upper), (Bound::Unbounded, Bound::Unbounded))
        {
            return i;
        }

        let py_range = Ranges::from_range_bounds((py_lower.cloned(), py_upper.cloned()));
        if py_range.is_empty() {
            // The bounds cannot match any version, so the marker is always false.
            return NodeId::FALSE;
        }
        if matches!(i, NodeId::TRUE) {
            let var = Variable::Version(CanonicalMarkerValueVersion::PythonFullVersion);
            let edges = Edges::Version {
                edges: Edges::from_range(&py_range),
            };
            return self.create_node(var, edges).negate(i);
        }

        let node = self.shared.node(i);
        let Node {
            var: Variable::Version(CanonicalMarkerValueVersion::PythonFullVersion),
            children: Edges::Version { edges },
        } = node
        else {
            // Complexify all nodes recursively.
            let children = node.children.map(i, |node_id| {
                self.complexify_python_versions(node_id, py_lower, py_upper)
            });
            return self.create_node(node.var.clone(), children);
        };
        // Remove ranges that do not intersect the Python version bounds. These ranges are always
        // false.
        //
        // For finite bounds, clip the existing edges. Add an always-false range for Python
        // versions outside those bounds.
        let mut new: SmallVec<_> = edges
            .iter()
            .filter(|(range, _)| !py_range.intersection(range).is_empty())
            .cloned()
            .collect();
        // `new` must contain at least one element. The edges cover all values, and `py_range` is
        // non-empty, so at least one edge intersects the range.
        assert!(
            !new.is_empty(),
            "expected at least one non-empty intersection"
        );
        // Map always-false values to this `NodeId`. Negate it when the parent is negated.
        let exclude_node_id = NodeId::FALSE.negate(i);
        if !matches!(py_lower, Bound::Unbounded) {
            let &(ref first_range, first_node_id) = new.first().unwrap();
            let first_upper = first_range.bounding_range().unwrap().1;
            // Extend an always-false first range to negative infinity. Values below the lower
            // bound must be false. This also prevents adjacent ranges from mapping to the same
            // node, which would break the canonical representation.
            if exclude_node_id == first_node_id {
                let clipped = Ranges::from_range_bounds((Bound::Unbounded, first_upper.cloned()));
                *new.first_mut().unwrap() = (clipped, first_node_id);
            } else {
                let clipped = Ranges::from_range_bounds((py_lower.cloned(), first_upper.cloned()));
                *new.first_mut().unwrap() = (clipped, first_node_id);

                let py_range_lower =
                    Ranges::from_range_bounds((py_lower.cloned(), Bound::Unbounded));
                new.insert(0, (py_range_lower.complement(), NodeId::FALSE.negate(i)));
            }
        }
        if !matches!(py_upper, Bound::Unbounded) {
            let &(ref last_range, last_node_id) = new.last().unwrap();
            let last_lower = last_range.bounding_range().unwrap().0;
            // As with the lower bound, keep the representation canonical.
            if exclude_node_id == last_node_id {
                let clipped = Ranges::from_range_bounds((last_lower.cloned(), Bound::Unbounded));
                *new.last_mut().unwrap() = (clipped, last_node_id);
            } else {
                let clipped = Ranges::from_range_bounds((last_lower.cloned(), py_upper.cloned()));
                *new.last_mut().unwrap() = (clipped, last_node_id);

                let py_range_upper =
                    Ranges::from_range_bounds((Bound::Unbounded, py_upper.cloned()));
                new.push((py_range_upper.complement(), exclude_node_id));
            }
        }
        self.create_node(node.var.clone(), Edges::Version { edges: new })
            .negate(i)
    }

    /// The disjunction of known incompatible conditions.
    ///
    /// For example, `sys_platform == 'win32'` and `platform_system == 'Darwin'` cannot both be
    /// true, even though the marker specification and grammar do not _forbid_ this combination.
    ///
    /// This method adds environment assumptions that the PEP 508 specification does not guarantee.
    fn exclusions(&mut self) -> NodeId {
        /// Applies a disjunction operation to two nodes.
        ///
        /// This matches [`InternerGuard::or`] but excludes information outside the marker algebra.
        fn disjunction(
            guard: &mut InternerGuard<'_>,
            cache: &mut FxHashMap<(NodeId, NodeId), NodeId>,
            xi: NodeId,
            yi: NodeId,
        ) -> NodeId {
            // Use inexpensive negation to implement OR with its De Morgan complement.
            conjunction(guard, cache, xi.not(), yi.not()).not()
        }

        /// Applies a conjunction operation to two nodes.
        ///
        /// This matches [`InternerGuard::and`] but excludes information outside the marker algebra.
        fn conjunction(
            guard: &mut InternerGuard<'_>,
            cache: &mut FxHashMap<(NodeId, NodeId), NodeId>,
            xi: NodeId,
            yi: NodeId,
        ) -> NodeId {
            if xi.is_true() {
                return yi;
            }
            if yi.is_true() {
                return xi;
            }
            if xi == yi {
                return xi;
            }
            if xi.is_false() || yi.is_false() {
                return NodeId::FALSE;
            }
            // `X and not X` is `false` by definition.
            if xi.not() == yi {
                return NodeId::FALSE;
            }

            // The operation was memoized.
            if let Some(result) = cache.get(&(xi, yi)) {
                return *result;
            }

            let (x, y) = (guard.shared.node(xi), guard.shared.node(yi));

            // Apply Shannon expansion to the higher-order variable.
            let (func, children) = match x.var.cmp(&y.var) {
                // X has higher order than Y. Apply Y to every child of X.
                Ordering::Less => {
                    let children = x
                        .children
                        .map(xi, |node| conjunction(guard, cache, node, yi));
                    (x.var.clone(), children)
                }
                // Y has higher order than X. Apply X to every child of Y.
                Ordering::Greater => {
                    let children = y
                        .children
                        .map(yi, |node| conjunction(guard, cache, node, xi));
                    (y.var.clone(), children)
                }
                // X and Y represent the same variable. Merge their children.
                Ordering::Equal => {
                    let children = x
                        .children
                        .apply(xi, &y.children, yi, |x, y| conjunction(guard, cache, x, y));
                    (x.var.clone(), children)
                }
            };

            // Create the output node.
            let node = guard.create_node(func, children);

            // Memoize the result of this operation.
            cache.insert((xi, yi), node);

            node
        }

        if let Some(exclusions) = self.state.exclusions {
            return exclusions;
        }
        let mut tree = NodeId::FALSE;
        // These operations omit known-incompatibility checks, so their results must not be reused
        // by regular marker operations.
        let mut cache = FxHashMap::default();

        // Create all nodes in advance.
        let os_name_nt = self.expression(MarkerExpression::String {
            key: MarkerValueString::OsName,
            operator: MarkerOperator::Equal,
            value: arcstr::literal!("nt"),
        });
        let os_name_posix = self.expression(MarkerExpression::String {
            key: MarkerValueString::OsName,
            operator: MarkerOperator::Equal,
            value: arcstr::literal!("posix"),
        });
        let sys_platform_linux = self.expression(MarkerExpression::String {
            key: MarkerValueString::SysPlatform,
            operator: MarkerOperator::Equal,
            value: arcstr::literal!("linux"),
        });
        let sys_platform_darwin = self.expression(MarkerExpression::String {
            key: MarkerValueString::SysPlatform,
            operator: MarkerOperator::Equal,
            value: arcstr::literal!("darwin"),
        });
        let sys_platform_ios = self.expression(MarkerExpression::String {
            key: MarkerValueString::SysPlatform,
            operator: MarkerOperator::Equal,
            value: arcstr::literal!("ios"),
        });
        let sys_platform_win32 = self.expression(MarkerExpression::String {
            key: MarkerValueString::SysPlatform,
            operator: MarkerOperator::Equal,
            value: arcstr::literal!("win32"),
        });
        let platform_system_freebsd = self.expression(MarkerExpression::String {
            key: MarkerValueString::PlatformSystem,
            operator: MarkerOperator::Equal,
            value: arcstr::literal!("FreeBSD"),
        });
        let platform_system_netbsd = self.expression(MarkerExpression::String {
            key: MarkerValueString::PlatformSystem,
            operator: MarkerOperator::Equal,
            value: arcstr::literal!("NetBSD"),
        });
        let platform_system_openbsd = self.expression(MarkerExpression::String {
            key: MarkerValueString::PlatformSystem,
            operator: MarkerOperator::Equal,
            value: arcstr::literal!("OpenBSD"),
        });
        let platform_system_sunos = self.expression(MarkerExpression::String {
            key: MarkerValueString::PlatformSystem,
            operator: MarkerOperator::Equal,
            value: arcstr::literal!("SunOS"),
        });
        let platform_system_ios = self.expression(MarkerExpression::String {
            key: MarkerValueString::PlatformSystem,
            operator: MarkerOperator::Equal,
            value: arcstr::literal!("iOS"),
        });
        let platform_system_ipados = self.expression(MarkerExpression::String {
            key: MarkerValueString::PlatformSystem,
            operator: MarkerOperator::Equal,
            value: arcstr::literal!("iPadOS"),
        });
        let sys_platform_aix = self.expression(MarkerExpression::String {
            key: MarkerValueString::SysPlatform,
            operator: MarkerOperator::Equal,
            value: arcstr::literal!("aix"),
        });
        let sys_platform_android = self.expression(MarkerExpression::String {
            key: MarkerValueString::SysPlatform,
            operator: MarkerOperator::Equal,
            value: arcstr::literal!("android"),
        });
        let sys_platform_emscripten = self.expression(MarkerExpression::String {
            key: MarkerValueString::SysPlatform,
            operator: MarkerOperator::Equal,
            value: arcstr::literal!("emscripten"),
        });
        let sys_platform_cygwin = self.expression(MarkerExpression::String {
            key: MarkerValueString::SysPlatform,
            operator: MarkerOperator::Equal,
            value: arcstr::literal!("cygwin"),
        });
        let sys_platform_wasi = self.expression(MarkerExpression::String {
            key: MarkerValueString::SysPlatform,
            operator: MarkerOperator::Equal,
            value: arcstr::literal!("wasi"),
        });

        // Pairs of `os_name` and `sys_platform` that are known to be incompatible.
        //
        // For example: `os_name == 'nt' and sys_platform == 'darwin'`
        let mut pairs = vec![
            (os_name_nt, sys_platform_linux),
            (os_name_nt, sys_platform_darwin),
            (os_name_nt, sys_platform_ios),
            (os_name_posix, sys_platform_win32),
        ];

        // Pairs of `platform_system` and `sys_platform` that are known to be incompatible.
        //
        // For example: `platform_system == 'FreeBSD' and sys_platform == 'aix'`
        for platform_system in [
            platform_system_freebsd,
            platform_system_netbsd,
            platform_system_openbsd,
            platform_system_sunos,
            platform_system_ios,
            platform_system_ipados,
        ] {
            for sys_platform in [
                sys_platform_aix,
                sys_platform_android,
                sys_platform_emscripten,
                sys_platform_ios,
                sys_platform_linux,
                sys_platform_darwin,
                sys_platform_win32,
                sys_platform_cygwin,
                sys_platform_wasi,
            ] {
                // Some of these pairs are compatible.
                if sys_platform == sys_platform_ios
                    && (platform_system == platform_system_ios
                        || platform_system == platform_system_ipados)
                {
                    continue;
                }
                pairs.push((platform_system, sys_platform));
            }
        }

        for (a, b) in pairs {
            let a_and_b = conjunction(self, &mut cache, a, b);
            tree = disjunction(self, &mut cache, tree, a_and_b);
        }

        self.state.exclusions = Some(tree);
        tree
    }
}

/// A unique variable for a decision node.
///
/// This `enum` defines the variable order for all ADDs. A poor order can increase ADD size
/// exponentially. Computing an optimal order dynamically is NP-complete.
///
/// The effect of this order on common marker trees may need investigation. Most marker trees are
/// small, so the effect may be limited.
#[derive(PartialOrd, Ord, PartialEq, Eq, Hash, Clone, Debug)]
pub(crate) enum Variable {
    /// A string marker, such as `os_name`.
    String(CanonicalMarkerValueString),
    /// A version marker, such as `python_version`.
    ///
    /// This highest-order variable usually contains the most complex ranges. Its position lets
    /// the tree merge ranges at the top level.
    Version(CanonicalMarkerValueVersion),
    /// A `<key> in <value>` expression for a specific string marker and value.
    In {
        key: CanonicalMarkerValueString,
        value: ArcStr,
    },
    /// A `<value> in <key>` expression for a specific string marker and value.
    Contains {
        key: CanonicalMarkerValueString,
        value: ArcStr,
    },
    /// Whether a specific extra exists.
    ///
    /// Keep extras at the leaves so simplification can remove them without rebuilding the tree.
    Extra(CanonicalMarkerValueExtra),
    /// A `<value> in <key>` or `<value> not in <key>` expression where the key is a list.
    ///
    /// Keep extras and groups at the leaves so simplification can remove them without rebuilding
    /// the tree.
    List(CanonicalMarkerListPair),
}

impl Variable {
    /// Returns `true` if the variable occurs in _at least_ one known conflicting marker pair.
    ///
    /// For example, `sys_platform == 'win32'` and `platform_system == 'Darwin'` cannot both be true.
    fn is_conflicting_variable(&self) -> bool {
        let Self::String(marker) = self else {
            return false;
        };
        marker.is_conflicting()
    }
}

/// A decision node in an Algebraic Decision Diagram.
#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub(crate) struct Node {
    /// The variable this node represents.
    pub(crate) var: Variable,
    /// The child edges for the possible outputs of this variable.
    pub(crate) children: Edges,
}

impl Node {
    /// Returns the complement of this node and flips every child ID.
    fn not(self) -> Self {
        Self {
            var: self.var,
            children: self.children.not(),
        }
    }
}

/// An ID that references a decision node in the [`Interner`].
///
/// The lowest bit represents complemented edges.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct NodeId(usize);

impl NodeId {
    // The terminal node representing `true`, or a trivially `true` node.
    pub(crate) const TRUE: Self = Self(0);

    // The terminal node representing `false`, or an unsatisfiable node.
    pub(crate) const FALSE: Self = Self(1);

    /// Creates a [`NodeId`] for the given index and optional complement.
    fn new(index: usize, complement: bool) -> Self {
        // Ensure the index does not interfere with the lowest complement bit.
        let index = (index + 1) << 1;
        Self(index | usize::from(complement))
    }

    /// Returns the index of this ID, ignoring the complemented edge.
    fn index(self) -> usize {
        // Ignore the lowest bit and return a zero-based index.
        (self.0 >> 1) - 1
    }

    /// Returns `true` if this ID represents a complemented edge.
    fn is_complement(self) -> bool {
        // Whether the lowest bit is set.
        (self.0 & 1) == 1
    }

    /// Returns the complement of this node.
    pub(crate) fn not(self) -> Self {
        // Toggle the lowest bit.
        Self(self.0 ^ 1)
    }

    /// Returns the complement of this node if its parent is complemented.
    ///
    /// Restores the complemented state of child nodes when traversing the tree.
    pub(crate) fn negate(self, parent: Self) -> Self {
        if parent.is_complement() {
            self.not()
        } else {
            self
        }
    }

    /// Returns `true` if this node represents an unsatisfiable node.
    pub(crate) fn is_false(self) -> bool {
        self == Self::FALSE
    }

    /// Returns `true` if this node represents a trivially `true` node.
    pub(crate) fn is_true(self) -> bool {
        self == Self::TRUE
    }

    /// Returns the conjunction if it can be determined without inspecting the interner.
    pub(crate) fn and_trivial(self, other: Self) -> Option<Self> {
        if self.is_true() {
            return Some(other);
        }
        if other.is_true() {
            return Some(self);
        }
        if self == other {
            return Some(self);
        }
        if self.is_false() || other.is_false() {
            return Some(Self::FALSE);
        }
        // `X and not X` is `false` by definition.
        if self.not() == other {
            return Some(Self::FALSE);
        }
        None
    }

    /// Returns the disjunction if it can be determined without inspecting the interner.
    pub(crate) fn or_trivial(self, other: Self) -> Option<Self> {
        self.not().and_trivial(other.not()).map(Self::not)
    }

    /// Returns whether the nodes are disjoint if that can be determined without the interner.
    pub(crate) fn is_disjoint_trivial(self, other: Self) -> Option<bool> {
        self.and_trivial(other).map(Self::is_false)
    }
}

/// A [`SmallVec`] that holds two constant edges and the ranges between them.
type SmallVec<T> = smallvec::SmallVec<[T; 5]>;

/// The edges of a decision node.
#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub(crate) enum Edges {
    // The edges of a version variable, representing a disjoint set of ranges that cover
    // the output space.
    //
    // Invariant: All ranges are simple, meaning they can be represented by a bounded
    // interval without gaps. Additionally, there are at least two edges in the set.
    Version {
        edges: SmallVec<(Ranges<Version>, NodeId)>,
    },
    // The edges of a string variable, representing a disjoint set of ranges that cover
    // the output space.
    //
    // Invariant: All ranges are simple, meaning they can be represented by a bounded
    // interval without gaps. Additionally, there are at least two edges in the set.
    String {
        edges: SmallVec<(Ranges<ArcStr>, NodeId)>,
    },
    // The edges of a boolean variable, representing the values `true` (the `high` child)
    // and `false` (the `low` child).
    Boolean {
        high: NodeId,
        low: NodeId,
    },
}

impl Edges {
    /// Returns the [`Edges`] for a boolean variable.
    fn from_bool(complemented: bool) -> Self {
        if complemented {
            Self::Boolean {
                high: NodeId::TRUE,
                low: NodeId::FALSE,
            }
        } else {
            Self::Boolean {
                high: NodeId::FALSE,
                low: NodeId::TRUE,
            }
        }
    }

    /// Returns the [`Edges`] for a string expression.
    ///
    /// Panics for `In` and `Contains`, which require separate boolean variables.
    fn from_string(
        key: CanonicalMarkerValueString,
        operator: MarkerOperator,
        value: ArcStr,
    ) -> Self {
        let range: Ranges<ArcStr> = match (key, operator) {
            // `platform_release` and `platform_version` are `Version | String` fields. Preserve
            // their existing lexicographic behavior here; their version-aware semantics are
            // outside the pure string field behavior handled by this change.
            (
                CanonicalMarkerValueString::PlatformRelease
                | CanonicalMarkerValueString::PlatformVersion,
                MarkerOperator::GreaterThan,
            ) => Ranges::strictly_higher_than(value),
            (
                CanonicalMarkerValueString::PlatformRelease
                | CanonicalMarkerValueString::PlatformVersion,
                MarkerOperator::GreaterEqual,
            ) => Ranges::higher_than(value),
            (
                CanonicalMarkerValueString::PlatformRelease
                | CanonicalMarkerValueString::PlatformVersion,
                MarkerOperator::LessThan,
            ) => Ranges::strictly_lower_than(value),
            (
                CanonicalMarkerValueString::PlatformRelease
                | CanonicalMarkerValueString::PlatformVersion,
                MarkerOperator::LessEqual,
            ) => Ranges::lower_than(value),
            (_, MarkerOperator::Equal) => Ranges::singleton(value),
            (_, MarkerOperator::NotEqual) => Ranges::singleton(value).complement(),
            // The marker specification defines strict ordering comparisons for string-valued
            // fields as always false, while inclusive ordering comparisons are equivalent to
            // equality.
            (_, MarkerOperator::GreaterThan | MarkerOperator::LessThan) => Ranges::empty(),
            (_, MarkerOperator::GreaterEqual | MarkerOperator::LessEqual) => {
                Ranges::singleton(value)
            }
            (_, MarkerOperator::TildeEqual) => {
                unreachable!("string comparisons with ~= are ignored")
            }
            _ => unreachable!("`in` and `contains` are treated as boolean variables"),
        };

        Self::String {
            edges: Self::from_range(&range),
        }
    }

    /// Returns the [`Edges`] for a version specifier.
    fn from_specifier(specifier: VersionSpecifier) -> Self {
        let specifier = release_specifier_to_range(specifier.only_release(), true);
        Self::Version {
            edges: Self::from_range(&specifier),
        }
    }

    /// Returns [`Edges`] that mark values in the given range as `true`.
    ///
    /// Accepts only a `PythonVersion` key and normalizes it to `PythonFullVersion`.
    fn from_python_versions(
        versions: Vec<Version>,
        operator: ContainerOperator,
    ) -> Result<Self, NodeId> {
        let mut range: Ranges<Version> = versions
            .into_iter()
            .map(|version| {
                let specifier = VersionSpecifier::equals_version(version.only_release());
                let specifier = python_version_to_full_version(specifier)?;
                Ok(release_specifier_to_range(specifier, true))
            })
            .flatten_ok()
            .collect::<Result<Ranges<_>, NodeId>>()?;

        if operator == ContainerOperator::NotIn {
            range = range.complement();
        }

        Ok(Self::Version {
            edges: Self::from_range(&range),
        })
    }

    /// Returns [`Edges`] that mark values in the given range as `true`.
    fn from_versions(versions: Vec<Version>, operator: ContainerOperator) -> Self {
        let mut range: Ranges<Version> = versions
            .into_iter()
            .map(|version| (Bound::Included(version.clone()), Bound::Included(version)))
            .collect();

        if operator == ContainerOperator::NotIn {
            range = range.complement();
        }

        Self::Version {
            edges: Self::from_range(&range),
        }
    }

    /// Returns [`Edges`] that mark values in the given range as `true`.
    fn from_range<T>(range: &Ranges<T>) -> SmallVec<(Ranges<T>, NodeId)>
    where
        T: Ord + Clone,
    {
        let mut edges = SmallVec::new();

        // Add the `true` edges.
        for (start, end) in range.iter() {
            let range = Ranges::from_range_bounds((start.cloned(), end.cloned()));
            edges.push((range, NodeId::TRUE));
        }

        // Add the `false` edges.
        for (start, end) in range.complement().iter() {
            let range = Ranges::from_range_bounds((start.cloned(), end.cloned()));
            edges.push((range, NodeId::FALSE));
        }

        // Sort the ranges.
        //
        // The ranges are disjoint, so equality is not possible.
        edges.sort_by(|(range1, _), (range2, _)| compare_disjoint_range_start(range1, range2));
        edges
    }

    /// Merges two [`Edges`] and applies an operation, such as `AND` or `OR`, to intersecting edges.
    ///
    /// For example, given two nodes corresponding to the same boolean variable:
    /// ```text
    /// left  (extra == 'foo'): { true: A, false: B }
    /// right (extra == 'foo'): { true: C, false: D }
    /// ```
    ///
    /// Apply the operation to matching edges to merge them into one node.
    /// ```text
    /// (extra == 'foo'): { true: (A and C), false: (B and D) }
    /// ```
    /// Non-boolean variables require additional handling. See `apply_ranges` for details.
    ///
    /// Both inputs must use the same [`Edges`] variant.
    fn apply(
        &self,
        parent: NodeId,
        right_edges: &Self,
        right_parent: NodeId,
        mut apply: impl FnMut(NodeId, NodeId) -> NodeId,
    ) -> Self {
        match (self, right_edges) {
            // Split and merge overlapping ranges for version or string variables.
            (Self::Version { edges }, Self::Version { edges: right_edges }) => Self::Version {
                edges: Self::apply_ranges(edges, parent, right_edges, right_parent, apply),
            },
            (Self::String { edges }, Self::String { edges: right_edges }) => Self::String {
                edges: Self::apply_ranges(edges, parent, right_edges, right_parent, apply),
            },
            // Merge the low and high edges for boolean variables.
            (
                Self::Boolean { high, low },
                Self::Boolean {
                    high: right_high,
                    low: right_low,
                },
            ) => Self::Boolean {
                high: apply(high.negate(parent), right_high.negate(right_parent)),
                low: apply(low.negate(parent), right_low.negate(right_parent)),
            },
            _ => unreachable!("cannot merge two `Edges` of different types"),
        }
    }

    /// Merges two range maps and applies the operation to every disjoint, intersecting range.
    ///
    /// For example, two nodes might have the following edges:
    /// ```text
    /// left  (python_version): { [0, 3.4): A,   [3.4, 3.4]: B,   (3.4, inf): C }
    /// right (python_version): { [0, 3.6): D,   [3.6, 3.6]: E,   (3.6, inf): F }
    /// ```
    ///
    /// Unlike boolean variables, these variables have no fixed `true` and `false` edges. Split and
    /// merge the overlapping ranges instead:
    /// ```text
    /// python_version: {
    ///     [0, 3.4):   (A and D),
    ///     [3.4, 3.4]: (B and D),
    ///     (3.4, 3.6): (C and D),
    ///     [3.6, 3.6]: (C and E),
    ///     (3.6, inf): (C and F)
    /// }
    /// ```
    ///
    /// Calls to `restrict_versions` can restrict the left and right edges. Drop ranges outside the
    /// domain of either edge. This should not occur in practice because `requires-python` bounds
    /// are global.
    fn apply_ranges<T>(
        left_edges: &SmallVec<(Ranges<T>, NodeId)>,
        left_parent: NodeId,
        right_edges: &SmallVec<(Ranges<T>, NodeId)>,
        right_parent: NodeId,
        mut apply: impl FnMut(NodeId, NodeId) -> NodeId,
    ) -> SmallVec<(Ranges<T>, NodeId)>
    where
        T: Clone + Ord,
    {
        let mut combined = SmallVec::new();
        for (left_range, left_child) in left_edges {
            // Split both maps into disjoint and overlapping ranges. Merge their intersections.
            //
            // Restricted ranges from `restrict_versions` can contain arbitrary gaps, even when
            // sorted. Do not zip the sets together. Use a quadratic search because each variable
            // usually has few ranges.
            for (right_range, right_child) in right_edges {
                let intersection = right_range.intersection(left_range);
                if intersection.is_empty() {
                    // TODO(ibraheem): take advantage of the sorted ranges to `break` early
                    continue;
                }

                // Merge the intersection.
                let node = apply(
                    left_child.negate(left_parent),
                    right_child.negate(right_parent),
                );

                match combined.last_mut() {
                    // Combine ranges if possible.
                    Some((range, prev)) if *prev == node && can_conjoin(range, &intersection) => {
                        *range = range.union(&intersection);
                    }
                    _ => combined.push((intersection, node)),
                }
            }
        }

        combined
    }

    // Returns `true` if two [`Edges`] are disjoint.
    fn is_disjoint(
        &self,
        parent: NodeId,
        right_edges: &Self,
        right_parent: NodeId,
        interner: &mut InternerGuard<'_>,
    ) -> bool {
        match (self, right_edges) {
            // Split and check overlapping ranges for version or string variables.
            (Self::Version { edges }, Self::Version { edges: right_edges }) => {
                Self::is_disjoint_ranges(edges, parent, right_edges, right_parent, interner)
            }
            (Self::String { edges }, Self::String { edges: right_edges }) => {
                Self::is_disjoint_ranges(edges, parent, right_edges, right_parent, interner)
            }
            // Check the low and high edges for boolean variables.
            (
                Self::Boolean { high, low },
                Self::Boolean {
                    high: right_high,
                    low: right_low,
                },
            ) => {
                interner.disjointness(high.negate(parent), right_high.negate(right_parent))
                    && interner.disjointness(low.negate(parent), right_low.negate(right_parent))
            }
            _ => unreachable!("cannot merge two `Edges` of different types"),
        }
    }

    // Returns `true` if all intersecting ranges in two range maps are disjoint.
    fn is_disjoint_ranges<T>(
        left_edges: &SmallVec<(Ranges<T>, NodeId)>,
        left_parent: NodeId,
        right_edges: &SmallVec<(Ranges<T>, NodeId)>,
        right_parent: NodeId,
        interner: &mut InternerGuard<'_>,
    ) -> bool
    where
        T: Clone + Ord,
    {
        // This matches `apply_ranges` but checks only disjointness, not the resulting edges.
        for (left_range, left_child) in left_edges {
            for (right_range, right_child) in right_edges {
                if right_range.is_disjoint(left_range) {
                    continue;
                }

                // Ensure the intersection is disjoint.
                if !interner.disjointness(
                    left_child.negate(left_parent),
                    right_child.negate(right_parent),
                ) {
                    return false;
                }
            }
        }

        true
    }

    // Apply the given function to all direct children of this node.
    fn map(&self, parent: NodeId, mut f: impl FnMut(NodeId) -> NodeId) -> Self {
        match self {
            Self::Version { edges: map } => Self::Version {
                edges: map
                    .iter()
                    .cloned()
                    .map(|(range, node)| (range, f(node.negate(parent))))
                    .collect(),
            },
            Self::String { edges: map } => Self::String {
                edges: map
                    .iter()
                    .cloned()
                    .map(|(range, node)| (range, f(node.negate(parent))))
                    .collect(),
            },
            Self::Boolean { high, low } => Self::Boolean {
                low: f(low.negate(parent)),
                high: f(high.negate(parent)),
            },
        }
    }

    // Returns an iterator over all direct children of this node.
    fn nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        match self {
            Self::Version { edges: map } => {
                Either::Left(Either::Left(map.iter().map(|(_, node)| *node)))
            }
            Self::String { edges: map } => {
                Either::Left(Either::Right(map.iter().map(|(_, node)| *node)))
            }
            Self::Boolean { high, low } => Either::Right([*high, *low].into_iter()),
        }
    }

    // Returns the complement of this [`Edges`].
    fn not(self) -> Self {
        match self {
            Self::Version { edges: map } => Self::Version {
                edges: map
                    .into_iter()
                    .map(|(range, node)| (range, node.not()))
                    .collect(),
            },
            Self::String { edges: map } => Self::String {
                edges: map
                    .into_iter()
                    .map(|(range, node)| (range, node.not()))
                    .collect(),
            },
            Self::Boolean { high, low } => Self::Boolean {
                high: high.not(),
                low: low.not(),
            },
        }
    }
}

/// Returns the equivalent `python_full_version` specifier for a `python_version` specifier.
///
/// Returns `Err` with a constant node if the equivalent comparison is always `true` or `false`.
fn python_version_to_full_version(specifier: VersionSpecifier) -> Result<VersionSpecifier, NodeId> {
    // Trailing zeroes matter only for (not-)equals-star and tilde-equals. After those two cases,
    // use the trimmed release.
    if specifier.operator().is_star() {
        // Input          python_version  python_full_version
        // ==3.*          3.*             3.*
        // ==3.0.*        3.0             3.0.*
        // ==3.0.0.*      3.0             3.0.*
        // ==3.9.*        3.9             3.9.*
        // ==3.9.0.*      3.9             3.9.*
        // ==3.9.0.0.*    3.9             3.9.*
        // ==3.9.1.*      FALSE           FALSE
        // ==3.9.1.0.*    FALSE           FALSE
        // ==3.9.1.0.0.*  FALSE           FALSE
        return match &*specifier.version().release() {
            // `3.*`
            [_major] => Ok(specifier),
            // Ex) `3.9.*`, `3.9.0.*`, or `3.9.0.0.*`
            [major, minor, rest @ ..] if rest.iter().all(|x| *x == 0) => {
                let python_version = Version::new([major, minor]);
                // Unwrap safety: A star operator with two version segments is always valid.
                Ok(VersionSpecifier::from_version(*specifier.operator(), python_version).unwrap())
            }
            // Ex) `3.9.1.*` or `3.9.0.1.*`
            _ => Err(NodeId::FALSE),
        };
    }

    if *specifier.operator() == Operator::TildeEqual {
        // python_version  python_full_version
        // ~=3             (not possible)
        // ~= 3.0          >= 3.0, < 4.0
        // ~= 3.9          >= 3.9, < 4.0
        // ~= 3.9.0        == 3.9.*
        // ~= 3.9.1        FALSE
        // ~= 3.9.0.0      == 3.9.*
        // ~= 3.9.0.1      FALSE
        return match &*specifier.version().release() {
            // Ex) `3.0`, `3.7`
            [_major, _minor] => Ok(specifier),
            // Ex) `3.9`, `3.9.0`, or `3.9.0.0`
            [major, minor, rest @ ..] if rest.iter().all(|x| *x == 0) => {
                let python_version = Version::new([major, minor]);
                Ok(VersionSpecifier::equals_star_version(python_version))
            }
            // Ex) `3.9.1` or `3.9.0.1`
            _ => Err(NodeId::FALSE),
        };
    }

    // Extract the major and minor version segments if the specifier contains exactly
    // those segments, or if it contains a major segment with an implied minor segment of `0`.
    let major_minor = match *specifier.version().only_release_trimmed().release() {
        // Add a trailing `0` for the minor version, which is implied.
        // For example, `python_version == 3` matches `3.0.1`, `3.0.2`, etc.
        [major] => Some((major, 0)),
        [major, minor] => Some((major, minor)),
        // Specifiers including segments beyond the minor version require separate handling.
        _ => None,
    };

    // `python_version` contains only major and minor version segments. For example, `3.7.0` and
    // `3.7.1` both produce the marker value `3.7`. Convert the specifier to `python_full_version`
    // by finding every full version whose truncated value satisfies the original specifier.
    if let Some((major, minor)) = major_minor {
        let version = Version::new([major, minor]);

        Ok(match specifier.operator() {
            // `python_version == 3.7` is equivalent to `python_full_version == 3.7.*`.
            Operator::Equal | Operator::ExactEqual => {
                VersionSpecifier::equals_star_version(version)
            }
            // `python_version != 3.7` is equivalent to `python_full_version != 3.7.*`.
            Operator::NotEqual => VersionSpecifier::not_equals_star_version(version),

            // `python_version > 3.7` is equivalent to `python_full_version >= 3.8`.
            Operator::GreaterThan => {
                VersionSpecifier::greater_than_equal_version(Version::new([major, minor + 1]))
            }
            // `python_version < 3.7` is equivalent to `python_full_version < 3.7`.
            Operator::LessThan => specifier,
            // `python_version >= 3.7` is equivalent to `python_full_version >= 3.7`.
            Operator::GreaterThanEqual => specifier,
            // `python_version <= 3.7` is equivalent to `python_full_version < 3.8`.
            Operator::LessThanEqual => {
                VersionSpecifier::less_than_version(Version::new([major, minor + 1]))
            }

            Operator::EqualStar | Operator::NotEqualStar | Operator::TildeEqual => {
                // Handled above.
                unreachable!()
            }
        })
    } else {
        let [major, minor, ..] = *specifier.version().release() else {
            unreachable!()
        };

        Ok(match specifier.operator() {
            // `python_version` has at most two release segments. Later nonzero segments make
            // equality impossible.
            Operator::Equal | Operator::ExactEqual => {
                return Err(NodeId::FALSE);
            }

            // Inequalities are always `true` for the same reason.
            Operator::NotEqual => return Err(NodeId::TRUE),

            // `python_version {<,<=} 3.7.8` is equivalent to `python_full_version < 3.8`.
            Operator::LessThan | Operator::LessThanEqual => {
                VersionSpecifier::less_than_version(Version::new([major, minor + 1]))
            }

            // `python_version {>,>=} 3.7.8` is equivalent to `python_full_version >= 3.8`.
            Operator::GreaterThan | Operator::GreaterThanEqual => {
                VersionSpecifier::greater_than_equal_version(Version::new([major, minor + 1]))
            }

            Operator::EqualStar | Operator::NotEqualStar | Operator::TildeEqual => {
                // Handled above.
                unreachable!()
            }
        })
    }
}

/// Compares the start of two ranges that are known to be disjoint.
fn compare_disjoint_range_start<T>(range1: &Ranges<T>, range2: &Ranges<T>) -> Ordering
where
    T: Ord,
{
    let (upper1, _) = range1.bounding_range().unwrap();
    let (upper2, _) = range2.bounding_range().unwrap();

    match (upper1, upper2) {
        (Bound::Unbounded, _) => Ordering::Less,
        (_, Bound::Unbounded) => Ordering::Greater,
        (Bound::Included(v1), Bound::Excluded(v2)) if v1 == v2 => Ordering::Less,
        (Bound::Excluded(v1), Bound::Included(v2)) if v1 == v2 => Ordering::Greater,
        // Disjoint ranges cannot have equal lower bounds.
        (Bound::Included(v1) | Bound::Excluded(v1), Bound::Included(v2) | Bound::Excluded(v2)) => {
            v1.cmp(v2)
        }
    }
}

/// Returns `true` if two disjoint ranges can join without a gap.
fn can_conjoin<T>(range1: &Ranges<T>, range2: &Ranges<T>) -> bool
where
    T: Ord + Clone,
{
    let Some((_, end)) = range1.bounding_range() else {
        return false;
    };
    let Some((start, _)) = range2.bounding_range() else {
        return false;
    };

    match (end, start) {
        (Bound::Included(v1), Bound::Excluded(v2)) if v1 == v2 => true,
        (Bound::Excluded(v1), Bound::Included(v2)) if v1 == v2 => true,
        _ => false,
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_false() {
            return write!(f, "false");
        }

        if self.is_true() {
            return write!(f, "true");
        }

        if self.is_complement() {
            write!(f, "{:?}", INTERNER.shared.node(*self).clone().not())
        } else {
            write!(f, "{:?}", INTERNER.shared.node(*self))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{INTERNER, NodeId};
    use crate::MarkerExpression;

    fn expr(s: &str) -> NodeId {
        INTERNER
            .lock()
            .expression(MarkerExpression::from_str(s).unwrap().unwrap())
    }

    #[test]
    fn basic() {
        let m = || INTERNER.lock();
        let extra_foo = expr("extra == 'foo'");
        assert!(!extra_foo.is_false());

        let os_foo = expr("os_name == 'foo'");
        let extra_and_os_foo = m().or(extra_foo, os_foo);
        assert!(!extra_and_os_foo.is_false());
        assert!(!m().and(extra_foo, os_foo).is_false());

        let trivially_true = m().or(extra_and_os_foo, extra_and_os_foo.not());
        assert!(!trivially_true.is_false());
        assert!(trivially_true.is_true());

        let trivially_false = m().and(extra_foo, extra_foo.not());
        assert!(trivially_false.is_false());

        let e = m().or(trivially_false, os_foo);
        assert!(!e.is_false());

        let extra_not_foo = expr("extra != 'foo'");
        assert!(m().and(extra_foo, extra_not_foo).is_false());
        assert!(m().or(extra_foo, extra_not_foo).is_true());

        let os_geq_bar = expr("os_name >= 'bar'");
        assert_eq!(os_geq_bar, expr("os_name == 'bar'"));

        let os_lt_bar = expr("os_name < 'bar'");
        assert!(os_lt_bar.is_false());
        assert!(m().and(os_geq_bar, os_lt_bar).is_false());
        assert_eq!(m().or(os_geq_bar, os_lt_bar), os_geq_bar);

        let os_leq_bar = expr("os_name <= 'bar'");
        assert_eq!(os_leq_bar, os_geq_bar);
        assert_eq!(m().and(os_geq_bar, os_leq_bar), os_geq_bar);
        assert_eq!(m().or(os_geq_bar, os_leq_bar), os_geq_bar);
    }

    #[test]
    fn version() {
        let m = || INTERNER.lock();
        let eq_3 = expr("python_version == '3'");
        let neq_3 = expr("python_version != '3'");
        let geq_3 = expr("python_version >= '3'");
        let leq_3 = expr("python_version <= '3'");

        let eq_2 = expr("python_version == '2'");
        let eq_1 = expr("python_version == '1'");
        assert!(m().and(eq_2, eq_1).is_false());

        assert_eq!(eq_3.not(), neq_3);
        assert_eq!(eq_3, neq_3.not());

        assert!(m().and(eq_3, neq_3).is_false());
        assert!(m().or(eq_3, neq_3).is_true());

        assert_eq!(m().and(eq_3, geq_3), eq_3);
        assert_eq!(m().and(eq_3, leq_3), eq_3);

        assert_eq!(m().and(geq_3, leq_3), eq_3);

        assert!(!m().and(geq_3, leq_3).is_false());
        assert!(m().or(geq_3, leq_3).is_true());
    }

    #[test]
    fn simplify() {
        let m = || INTERNER.lock();
        let x86 = expr("platform_machine == 'x86_64'");
        let not_x86 = expr("platform_machine != 'x86_64'");
        let windows = expr("platform_machine == 'Windows'");

        let a = m().and(x86, windows);
        let b = m().and(not_x86, windows);
        assert_eq!(m().or(a, b), windows);
    }

    /// Do not panic with `u64::MAX` causing an `u64::MAX + 1` overflow.
    #[test]
    fn python_version_marker_u64_max() {
        // The parse error is converted to a warning and the condition is ignored.
        assert_eq!(
            MarkerExpression::from_str("python_version > '3.18446744073709551615'").unwrap(),
            None,
        );
        assert_eq!(
            MarkerExpression::from_str("python_version <= '3.18446744073709551615'").unwrap(),
            None,
        );

        // `u64::MAX - 1` accepted
        assert!(
            MarkerExpression::from_str("python_version > '3.18446744073709551614'")
                .unwrap()
                .is_some()
        );
    }
}
