use std::collections::Bound;

use uv_distribution_types::{RequiresPython, RequiresPythonRange};
use uv_pep440::Version;
use uv_pep508::{MarkerEnvironment, MarkerTree};
use uv_python::{Interpreter, PythonVersion};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PythonRequirement {
    source: PythonRequirementSource,
    /// The exact installed version of Python.
    exact: Version,
    /// The installed version of Python.
    installed: RequiresPython,
    /// The target version of Python; that is, the version of Python for which we are resolving
    /// dependencies. This is typically the same as the installed version, but may be different
    /// when specifying an alternate Python version for the resolution.
    target: RequiresPython,
    /// The canonical marker representation of the target Python requirement.
    target_marker: MarkerTree,
}

impl PythonRequirement {
    fn new(
        source: PythonRequirementSource,
        exact: Version,
        installed: RequiresPython,
        target: RequiresPython,
    ) -> Self {
        let target_marker = target.to_marker_tree();
        Self {
            source,
            exact,
            installed,
            target,
            target_marker,
        }
    }

    /// Create a [`PythonRequirement`] to resolve against both an [`Interpreter`] and a
    /// [`PythonVersion`].
    pub fn from_python_version(interpreter: &Interpreter, python_version: &PythonVersion) -> Self {
        let exact = interpreter.python_full_version().version.clone();
        let installed = interpreter
            .python_full_version()
            .version
            .only_release()
            .without_trailing_zeros();
        let target = python_version
            .python_full_version()
            .only_release()
            .without_trailing_zeros();
        Self::new(
            PythonRequirementSource::PythonVersion,
            exact,
            RequiresPython::greater_than_equal_version(&installed),
            RequiresPython::greater_than_equal_version(&target),
        )
    }

    /// Create a [`PythonRequirement`] to resolve against both an [`Interpreter`] and a
    /// [`MarkerEnvironment`].
    pub fn from_requires_python(
        interpreter: &Interpreter,
        requires_python: RequiresPython,
    ) -> Self {
        Self::from_marker_environment(interpreter.markers(), requires_python)
    }

    /// Create a [`PythonRequirement`] to resolve against an [`Interpreter`].
    pub fn from_interpreter(interpreter: &Interpreter) -> Self {
        let exact = interpreter
            .python_full_version()
            .version
            .clone()
            .without_trailing_zeros();
        let installed = interpreter
            .python_full_version()
            .version
            .only_release()
            .without_trailing_zeros();
        Self::new(
            PythonRequirementSource::Interpreter,
            exact,
            RequiresPython::greater_than_equal_version(&installed),
            RequiresPython::greater_than_equal_version(&installed),
        )
    }

    /// Create a [`PythonRequirement`] from a [`MarkerEnvironment`] and a
    /// specific `Requires-Python` directive.
    ///
    /// This has the same "source" as
    /// [`PythonRequirement::from_requires_python`], but is useful for
    /// constructing a `PythonRequirement` without an [`Interpreter`].
    pub fn from_marker_environment(
        marker_env: &MarkerEnvironment,
        requires_python: RequiresPython,
    ) -> Self {
        let exact = marker_env
            .python_full_version()
            .version
            .clone()
            .without_trailing_zeros();
        let installed = marker_env
            .python_full_version()
            .version
            .only_release()
            .without_trailing_zeros();
        Self::new(
            PythonRequirementSource::RequiresPython,
            exact,
            RequiresPython::greater_than_equal_version(&installed),
            requires_python,
        )
    }

    /// Narrow the [`PythonRequirement`] to the given version, if it's stricter (i.e., greater)
    /// than the current `Requires-Python` minimum.
    ///
    /// Returns `None` if the given range is not narrower than the current range.
    pub(crate) fn narrow(&self, target: &RequiresPythonRange) -> Option<Self> {
        Some(Self::new(
            self.source,
            self.exact.clone(),
            self.installed.clone(),
            self.target.narrow(target)?,
        ))
    }

    /// Split the [`PythonRequirement`] at the given version.
    ///
    /// For example, if the current requirement is `>=3.10`, and the split point is `3.11`, then
    /// the result will be `>=3.10 and <3.11` and `>=3.11`.
    pub(crate) fn split(&self, at: Bound<Version>) -> Option<(Self, Self)> {
        let (lower, upper) = self.target.split(at)?;
        Some((
            Self::new(
                self.source,
                self.exact.clone(),
                self.installed.clone(),
                lower,
            ),
            Self::new(
                self.source,
                self.exact.clone(),
                self.installed.clone(),
                upper,
            ),
        ))
    }

    /// Returns `true` if the minimum version of Python required by the target is greater than the
    /// installed version.
    pub(crate) fn raises(&self, target: &RequiresPythonRange) -> bool {
        target.lower() > self.target.range().lower()
    }

    /// Return the exact version of Python.
    pub(crate) fn exact(&self) -> &Version {
        &self.exact
    }

    /// Return the installed version of Python.
    pub(crate) fn installed(&self) -> &RequiresPython {
        &self.installed
    }

    /// Return the target version of Python.
    pub(crate) fn target(&self) -> &RequiresPython {
        &self.target
    }

    /// Return the source of the [`PythonRequirement`].
    pub(crate) fn source(&self) -> PythonRequirementSource {
        self.source
    }

    /// A wrapper around `RequiresPython::simplify_markers`. See its docs for
    /// more info.
    ///
    /// When this `PythonRequirement` isn't `RequiresPython`, the given markers
    /// are returned unchanged.
    pub(crate) fn simplify_markers(&self, marker: MarkerTree) -> MarkerTree {
        self.target.simplify_markers(marker)
    }

    /// Return a [`MarkerTree`] representing the Python requirement.
    ///
    /// See: [`RequiresPython::to_marker_tree`]
    pub(crate) fn to_marker_tree(&self) -> MarkerTree {
        self.target_marker
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, PartialOrd, Hash, Ord)]
pub enum PythonRequirementSource {
    /// `--python-version`
    PythonVersion,
    /// `Requires-Python`
    RequiresPython,
    /// The discovered Python interpreter.
    Interpreter,
}

#[cfg(test)]
mod tests {
    use std::ops::Bound;

    use uv_distribution_types::RequiresPython;
    use uv_pep440::Version;
    use uv_pep508::{MarkerEnvironment, MarkerEnvironmentBuilder};

    use super::{PythonRequirement, PythonRequirementSource};

    fn requires_python(specifiers: &str) -> RequiresPython {
        RequiresPython::from_specifiers(
            specifiers.parse().expect("valid Python version specifiers"),
        )
    }

    fn python_requirement(source: PythonRequirementSource) -> PythonRequirement {
        PythonRequirement::new(
            source,
            "3.13rc1".parse().expect("valid Python version"),
            requires_python(">=3.13"),
            requires_python(">=3.9, <3.14"),
        )
    }

    fn assert_cached_marker(requirement: &PythonRequirement) {
        assert_eq!(
            requirement.to_marker_tree(),
            requirement.target().to_marker_tree()
        );
    }

    fn assert_metadata_preserved(actual: &PythonRequirement, original: &PythonRequirement) {
        assert_eq!(actual.exact(), original.exact());
        assert_eq!(actual.installed(), original.installed());
        assert_eq!(actual.source(), original.source());
    }

    #[test]
    fn cached_marker_is_initialized() {
        for source in [
            PythonRequirementSource::PythonVersion,
            PythonRequirementSource::RequiresPython,
            PythonRequirementSource::Interpreter,
        ] {
            let requirement = python_requirement(source);
            assert_eq!(requirement.source(), source);
            assert_eq!(requirement.installed(), &requires_python(">=3.13"));
            assert_eq!(requirement.target(), &requires_python(">=3.9, <3.14"));
            assert_cached_marker(&requirement);
        }

        let marker_environment = MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
            implementation_name: "cpython",
            implementation_version: "3.13.0rc1",
            os_name: "posix",
            platform_machine: "aarch64",
            platform_python_implementation: "CPython",
            platform_release: "1",
            platform_system: "Linux",
            platform_version: "1",
            python_full_version: "3.13.0rc1",
            python_version: "3.13",
            sys_platform: "linux",
        })
        .expect("valid marker environment");
        let requirement = PythonRequirement::from_marker_environment(
            &marker_environment,
            requires_python(">=3.9, <3.14"),
        );
        assert_eq!(
            requirement,
            python_requirement(PythonRequirementSource::RequiresPython)
        );
        assert_cached_marker(&requirement);
    }

    #[test]
    fn cached_marker_is_refreshed_after_narrowing() {
        let original = python_requirement(PythonRequirementSource::PythonVersion);
        let expected = requires_python(">=3.11, <3.13");
        let narrowed = original
            .narrow(expected.range())
            .expect("strictly narrower Python range");
        assert_eq!(narrowed.target(), &expected);
        assert_ne!(narrowed.to_marker_tree(), original.to_marker_tree());
        assert_cached_marker(&narrowed);
        assert_metadata_preserved(&narrowed, &original);

        assert!(original.narrow(original.target().range()).is_none());
        assert!(
            original
                .narrow(requires_python(">=3.8, <3.15").range())
                .is_none()
        );
        assert_eq!(
            original,
            python_requirement(PythonRequirementSource::PythonVersion)
        );
        assert_cached_marker(&original);
    }

    #[test]
    fn cached_marker_is_refreshed_after_splitting() {
        let original = python_requirement(PythonRequirementSource::Interpreter);
        for (bound, lower, upper) in [
            (
                Bound::Included(Version::new([3, 11])),
                ">=3.9, <3.11",
                ">=3.11, <3.14",
            ),
            (
                Bound::Excluded(Version::new([3, 11])),
                ">=3.9, <=3.11",
                ">3.11, <3.14",
            ),
        ] {
            let (lower_requirement, upper_requirement) =
                original.split(bound).expect("interior split point");
            assert_eq!(lower_requirement.target(), &requires_python(lower));
            assert_eq!(upper_requirement.target(), &requires_python(upper));
            for requirement in [lower_requirement, upper_requirement] {
                assert_ne!(requirement.to_marker_tree(), original.to_marker_tree());
                assert_cached_marker(&requirement);
                assert_metadata_preserved(&requirement, &original);
            }
        }

        assert!(
            original
                .split(Bound::Included(Version::new([3, 9])))
                .is_none()
        );
        assert_eq!(
            original,
            python_requirement(PythonRequirementSource::Interpreter)
        );
        assert_cached_marker(&original);
    }
}
