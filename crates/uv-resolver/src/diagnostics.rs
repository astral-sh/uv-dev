use std::error::Error;
use std::sync::Arc;

use uv_errors::{Diagnostic, Hinted};

use crate::{LockError, NoSolutionError, PylockTomlError};

/// Resolve resolver-owned presentation data without changing error sources.
pub fn diagnostic_for_error<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
    if let Some(error) = downcast_error::<NoSolutionError>(error) {
        return error.diagnostic();
    }
    if let Some(error) = downcast_error::<LockError>(error) {
        let info = error.own_info()?;
        return Some(
            Diagnostic::default()
                .with_hints(error.own_hints())
                .with_info(info),
        );
    }
    let error = downcast_error::<PylockTomlError>(error)?;
    let info = error.own_info()?;
    Some(
        Diagnostic::default()
            .with_hints(error.own_hints())
            .with_info(info),
    )
}

fn downcast_error<'a, E: Error + 'static>(error: &'a (dyn Error + 'static)) -> Option<&'a E> {
    error
        .downcast_ref::<E>()
        .or_else(|| error.downcast_ref::<Box<E>>().map(AsRef::as_ref))
        .or_else(|| error.downcast_ref::<Arc<E>>().map(AsRef::as_ref))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use pubgrub::External;
    use rustc_hash::FxHashMap;

    use uv_distribution_types::{IndexCapabilities, IndexLocations, RequiresPython};
    use uv_errors::{ErrorOptions, Hints, write_error_chain_with_options};
    use uv_normalize::PackageName;
    use uv_pep440::Version;
    use uv_pep508::{MarkerEnvironment, MarkerEnvironmentBuilder};
    use uv_warnings::anstream;

    use super::*;
    use crate::candidate_selector::CandidateSelector;
    use crate::fork_indexes::ForkIndexes;
    use crate::fork_urls::ForkUrls;
    use crate::pubgrub::{PubGrubPackage, Range};
    use crate::python_requirement::PythonRequirement;
    use crate::resolver::{RootDependencyProof, UnavailablePackage, UnavailableReason};
    use crate::{ErrorTree, InMemoryIndex, Manifest, Options, ResolveError, ResolverEnvironment};

    fn no_solution() -> NoSolutionError {
        let name: PackageName = "example".parse().expect("valid package name");
        let reason = UnavailablePackage::NoIndex;
        let current_environment = MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
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
        let environment = ResolverEnvironment::universal(Vec::new());
        let options = Options::default();
        let selector = CandidateSelector::for_resolution(
            &options,
            &Manifest::simple(Vec::new()),
            &environment,
        );
        let python_requirement = PythonRequirement::from_marker_environment(
            &current_environment,
            RequiresPython::greater_than_equal_version(&Version::new([3_u64, 12])),
        );
        NoSolutionError::new(
            ErrorTree::External(External::Custom(
                PubGrubPackage::base(name.clone()),
                Range::full(),
                UnavailableReason::Package(reason.clone()),
            )),
            RootDependencyProof::default(),
            InMemoryIndex::default(),
            FxHashMap::default(),
            FxHashMap::default(),
            FxHashMap::default(),
            selector,
            python_requirement,
            IndexLocations::default(),
            IndexCapabilities::default(),
            FxHashMap::from_iter([(name, reason)]),
            FxHashMap::default(),
            ForkUrls::default(),
            ForkIndexes::default(),
            environment,
            current_environment,
            None,
            BTreeSet::default(),
            options,
        )
    }

    #[test]
    fn rejection_context_does_not_require_source_locations() {
        let error = no_solution();
        assert!(error.source().is_none());
        assert_eq!(error.diagnostic_info().count(), 1);

        let mut output = String::new();
        write_error_chain_with_options(
            &error,
            &Hints::none(),
            ErrorOptions::default()
                .with_width_override(1000)
                .with_diagnostic(diagnostic_for_error)
                .with_stream(&mut output),
        )
        .expect("writing to a string cannot fail");
        insta::assert_snapshot!(anstream::adapter::strip_str(&output), @"
        error: example was not found in the provided package locations
          info: Packages were unavailable because index lookups were disabled and no additional package locations were provided

        hint: Provide additional package locations with `--find-links <uri>`
        ");

        assert!(diagnostic_for_error(&Box::new(no_solution())).is_some());
        assert!(diagnostic_for_error(&Arc::new(no_solution())).is_some());
        assert!(diagnostic_for_error(&ResolveError::NoSolution(Box::new(no_solution()))).is_none());
    }
}
