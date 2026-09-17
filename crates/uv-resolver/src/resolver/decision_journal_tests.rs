use pubgrub::{Id, State};
use tokio::sync::mpsc;

use uv_distribution_types::{
    IncompatibleDist, IncompatibleSource, IncompatibleWheel, IndexCapabilities,
    PythonRequirementKind, RequiresPython,
};
use uv_pep440::{MIN_VERSION, Version, VersionSpecifiers};
use uv_pep508::{MarkerEnvironment, MarkerEnvironmentBuilder, MarkerTree};
use uv_resolver_types::PackageNodeKind;
use uv_types::EmptyInstalledPackages;

use crate::dependency_provider::UvDependencyProvider;
use crate::pubgrub::{
    DependencySource, PubGrubDependency, PubGrubPackage, PubGrubPackageInner, Range,
};
use crate::python_requirement::PythonRequirement;

use super::batch_prefetch::BatchPrefetcher;
use super::requests::MetadataRequests;
use super::{ForkState, InMemoryIndex, ResolverEnvironment, UnavailableVersion};

type Decision = (Id<PubGrubPackage>, Version);

fn package(name: &str) -> PubGrubPackage {
    PubGrubPackage::from_package(
        name.parse().expect("valid package name"),
        PackageNodeKind::Base,
        MarkerTree::TRUE,
    )
}

fn dependency(package: &PubGrubPackage, version: Range<Version>) -> PubGrubDependency {
    PubGrubDependency {
        package: package.clone(),
        version,
        parent: None,
        source: DependencySource::Unspecified,
    }
}

fn initialized_state(dependencies: Vec<PubGrubDependency>) -> (ForkState, InMemoryIndex) {
    let marker_env = MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
        implementation_name: "cpython",
        implementation_version: "3.12.0",
        os_name: "posix",
        platform_machine: "x86_64",
        platform_python_implementation: "CPython",
        platform_release: "",
        platform_system: "Linux",
        platform_version: "",
        python_full_version: "3.12.0",
        python_version: "3.12",
        sys_platform: "linux",
    })
    .expect("valid marker environment");
    let python_requirement = PythonRequirement::from_marker_environment(
        &marker_env,
        RequiresPython::greater_than_equal_version(&Version::new([3, 12])),
    );
    let index = InMemoryIndex::default();
    let (sender, _receiver) = mpsc::channel(1);
    let requests = MetadataRequests::new(index.clone(), sender, None);
    let pubgrub = State::<UvDependencyProvider>::init(
        PubGrubPackage::from(PubGrubPackageInner::Root(None)),
        MIN_VERSION.clone(),
    );
    let mut state = ForkState::new(
        pubgrub,
        ResolverEnvironment::universal(Vec::new()),
        python_requirement,
        BatchPrefetcher::new(IndexCapabilities::default(), requests),
    );
    assert!(state.decision_journal.is_none());
    state.decision_journal = Some(Vec::new());

    let root = state.pubgrub.root_package;
    assert!(state.pubgrub.unit_propagation(root).unwrap().is_empty());
    state.add_package_version_dependencies(
        root,
        &MIN_VERSION,
        dependencies,
        &index,
        &EmptyInstalledPackages,
    );
    assert!(state.pubgrub.unit_propagation(root).unwrap().is_empty());
    (state, index)
}

fn assert_journal(state: &ForkState, prefix: &[Decision], expected: &[Decision]) {
    assert_eq!(state.decision_journal.as_deref(), Some(expected));
    let mut full = prefix.to_vec();
    full.extend_from_slice(expected);
    assert_eq!(
        state
            .pubgrub
            .partial_solution
            .extract_solution()
            .collect::<Vec<_>>(),
        full,
    );
}

#[test]
fn records_only_committed_dependency_decisions() {
    let a = package("a");
    let b = package("b");
    let one = Version::new([1]);
    let two = Version::new([2]);
    let (mut state, index) = initialized_state(vec![
        dependency(&a, Range::full()),
        dependency(&b, Range::singleton(one.clone())),
    ]);
    let root = state.pubgrub.root_package;
    let a_id = state.pubgrub.package_store.alloc(a);
    let b_id = state.pubgrub.package_store.alloc(b.clone());
    let root_decision = (root, MIN_VERSION.clone());
    assert_journal(&state, &[], std::slice::from_ref(&root_decision));

    state.next = b_id;
    state.add_decision(b_id, one.clone());
    let prefix = vec![root_decision, (b_id, one.clone())];
    assert_journal(&state, &[], &prefix);

    // Before the first backtrack, PubGrub commits a version without checking its dependencies.
    // It is a real decision even when the next propagation would reject it.
    let mut provisional = state.clone();
    provisional.next = a_id;
    provisional.add_package_version_dependencies(
        a_id,
        &two,
        vec![dependency(&b, Range::singleton(two.clone()))],
        &index,
        &EmptyInstalledPackages,
    );
    let mut provisional_decisions = prefix.clone();
    provisional_decisions.push((a_id, two.clone()));
    assert_journal(&provisional, &[], &provisional_decisions);
    assert_journal(&state, &[], &prefix);

    // Reprioritization retracts the decision and invalidates its journal.
    assert!(state.pubgrub.unit_propagation(b_id).unwrap().is_empty());
    state.conflict_tracker.deprioritize.push(b_id);
    state.reprioritize_conflicts();
    assert!(state.decision_journal.is_none());
    let retained: Vec<_> = state.pubgrub.partial_solution.extract_solution().collect();
    assert_eq!(retained.as_slice(), &prefix[..1]);

    // Re-establish the observation baseline, then restore the compatible dependency decision.
    state.decision_journal = Some(Vec::new());
    state.add_decision(b_id, one.clone());
    assert!(state.pubgrub.unit_propagation(b_id).unwrap().is_empty());
    assert_journal(&state, &retained, &[(b_id, one.clone())]);

    // After a backtrack, a dependency conflict rejects the candidate before adding a decision.
    state.next = a_id;
    state.add_package_version_dependencies(
        a_id,
        &two,
        vec![dependency(&b, Range::singleton(two.clone()))],
        &index,
        &EmptyInstalledPackages,
    );
    assert_journal(&state, &retained, &[(b_id, one.clone())]);

    state.add_package_version_dependencies(
        a_id,
        &one,
        vec![dependency(&b, Range::singleton(one.clone()))],
        &index,
        &EmptyInstalledPackages,
    );
    assert_journal(&state, &retained, &[(b_id, one.clone()), (a_id, one)]);
}

#[test]
fn requires_python_unavailability_records_a_decision() {
    let a = package("a");
    let one = Version::new([1]);
    let two = Version::new([2]);
    let (mut state, index) = initialized_state(vec![dependency(&a, Range::full())]);
    let root = (state.pubgrub.root_package, MIN_VERSION.clone());
    let a_id = state.pubgrub.package_store.alloc(a);
    state.next = a_id;

    state.add_unavailable_version(
        two,
        UnavailableVersion::Offline,
        &index,
        &EmptyInstalledPackages,
    );
    assert_journal(&state, &[], std::slice::from_ref(&root));

    let requires_python: VersionSpecifiers = ">=3.13".parse().expect("valid Requires-Python");
    for reason in [
        IncompatibleDist::Source(IncompatibleSource::RequiresPython(
            requires_python.clone(),
            PythonRequirementKind::Target,
        )),
        IncompatibleDist::Wheel(IncompatibleWheel::RequiresPython(
            requires_python,
            PythonRequirementKind::Installed,
        )),
    ] {
        let mut fork = state.clone();
        fork.add_unavailable_version(
            one.clone(),
            UnavailableVersion::IncompatibleDist(reason),
            &index,
            &EmptyInstalledPackages,
        );
        assert_journal(&fork, &[], &[root.clone(), (a_id, one.clone())]);
    }
    assert_journal(&state, &[], &[root]);
}
