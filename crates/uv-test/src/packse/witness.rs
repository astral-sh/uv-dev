//! Conservative whole-domain proofs for a fixed project version assignment.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;

use uv_normalize::{ExtraName, PackageName};
use uv_pep440::{VersionSpecifiers, release_specifiers_to_ranges};
use uv_pep508::{MarkerExpression, MarkerTree, MarkerValueVersion, Requirement, VersionOrUrl};

use super::oracle::{Selection, validate_scenario};
use super::project::ScenarioProject;
use super::scenario::{Scenario, ScenarioDocument};

/// A sufficient satisfiability proof over the project's entire supported Python domain.
///
/// The activation markers and extra sets are conservative over-approximations. A rejected
/// assignment is not evidence that the project is unsatisfiable.
#[derive(Debug, Serialize)]
pub struct MarkerWitnessCertificate {
    requires_python: VersionSpecifiers,
    assignment: Selection,
    activations: BTreeMap<PackageName, CertifiedActivation>,
    checked_requirements: usize,
    activation_updates: usize,
    max_work: usize,
}

impl MarkerWitnessCertificate {
    /// The fixed assignment certified against the complete scenario document.
    pub fn assignment(&self) -> &Selection {
        &self.assignment
    }

    /// The number of package versions in the proposed fixed assignment.
    pub fn assigned_packages(&self) -> usize {
        self.assignment.len()
    }

    /// The number of packages reached by the conservative marker closure.
    pub fn activated_packages(&self) -> usize {
        self.activations.len()
    }

    /// Requirement evaluations, including repeated visits after activation growth.
    pub fn checked_requirements(&self) -> usize {
        self.checked_requirements
    }
}

#[derive(Debug, Eq, PartialEq, Serialize)]
struct CertifiedActivation {
    /// `None` denotes the marker that is true in every environment.
    marker: Option<String>,
    extras: BTreeSet<ExtraName>,
}

#[derive(Clone, Debug)]
struct Activation {
    marker: MarkerTree,
    extras: BTreeSet<ExtraName>,
}

/// Certify a fixed assignment using marker-conditioned dependency reachability.
///
/// All optional project dependencies and groups are roots. Package-extra predicates must be
/// additive; each package's extra set is the union of its requests across every environment.
/// This deliberately checks some extra requirements in a larger domain than necessary, so a
/// proof is sufficient but incomplete. No concrete environment or resolver output is sampled.
/// `max_work` bounds requirement evaluations; exhausting it does not imply unsatisfiability.
pub fn certify_project_marker_witness(
    document: &ScenarioDocument,
    assignment: &Selection,
    max_work: usize,
) -> Result<MarkerWitnessCertificate> {
    ensure!(max_work > 0, "the witness work budget must be positive");
    let scenario = document.scenario()?;
    let project = ScenarioProject::new(&scenario)?;
    let requirements = project.requirements(&project.all_selection())?;
    validate_scenario(&scenario, &requirements)?;

    let requires_python = scenario
        .root
        .requires_python
        .as_ref()
        .context("a marker witness requires an explicit root Python range")?;
    let python_range = release_specifiers_to_ranges(requires_python.clone());
    ensure!(
        !python_range.is_empty(),
        "a marker witness requires a nonempty root Python range"
    );
    let python3 = release_specifiers_to_ranges(">=3,<4".parse()?);
    ensure!(
        python_range.intersection(&python3.complement()).is_empty(),
        "a marker witness requires a Python 3 root range"
    );
    ensure!(
        assignment.len() == scenario.packages.len()
            && scenario
                .packages
                .keys()
                .all(|name| assignment.contains_key(name)),
        "a marker witness must assign every scenario package"
    );
    for (name, version) in assignment {
        ensure!(
            scenario.packages[name].versions.contains_key(version),
            "the scenario has no {name}=={version}"
        );
    }

    let domain = python_marker(requires_python);
    ensure!(
        !domain.is_false(),
        "the root Python marker domain must not be empty"
    );
    let mut state = WitnessState {
        scenario: &scenario,
        assignment,
        activations: BTreeMap::new(),
        checked_requirements: 0,
        activation_updates: 0,
        max_work,
    };
    let no_extras = BTreeSet::new();
    for requirement in &requirements {
        state.activate(requirement, domain, &no_extras)?;
    }

    loop {
        let mut changed = false;
        for (name, activation) in state.activations.clone() {
            let metadata = &scenario.packages[&name].versions[&assignment[&name]];
            for requirement in metadata.requires.iter().chain(
                activation
                    .extras
                    .iter()
                    .filter_map(|extra| metadata.extras.get(extra))
                    .flatten(),
            ) {
                changed |= state.activate(requirement, activation.marker, &activation.extras)?;
            }
        }
        if !changed {
            break;
        }
    }

    Ok(MarkerWitnessCertificate {
        requires_python: requires_python.clone(),
        assignment: assignment.clone(),
        activations: state
            .activations
            .into_iter()
            .map(|(name, activation)| {
                (
                    name,
                    CertifiedActivation {
                        marker: activation.marker.try_to_string(),
                        extras: activation.extras,
                    },
                )
            })
            .collect(),
        checked_requirements: state.checked_requirements,
        activation_updates: state.activation_updates,
        max_work,
    })
}

struct WitnessState<'a> {
    scenario: &'a Scenario,
    assignment: &'a Selection,
    activations: BTreeMap<PackageName, Activation>,
    checked_requirements: usize,
    activation_updates: usize,
    max_work: usize,
}

impl WitnessState<'_> {
    fn activate(
        &mut self,
        requirement: &Requirement,
        parent: MarkerTree,
        extras: &BTreeSet<ExtraName>,
    ) -> Result<bool> {
        self.checked_requirements = self
            .checked_requirements
            .checked_add(1)
            .filter(|checked| *checked <= self.max_work)
            .with_context(|| {
                format!("marker witness work exceeds {} requirements", self.max_work)
            })?;

        let marker = requirement
            .marker
            .simplify_extras_with(|extra| extras.contains(extra))
            .simplify_not_extras_with(|extra| !extras.contains(extra));
        let marker = parent.and(marker);
        if marker.is_false() {
            return Ok(false);
        }

        let version = self
            .assignment
            .get(&requirement.name)
            .with_context(|| format!("the witness is missing `{requirement}`"))?;
        let metadata = self
            .scenario
            .packages
            .get(&requirement.name)
            .and_then(|package| package.versions.get(version))
            .with_context(|| format!("the scenario has no {}=={version}", requirement.name))?;
        match &requirement.version_or_url {
            Some(VersionOrUrl::VersionSpecifier(specifier)) => ensure!(
                specifier.contains(version),
                "{}=={version} does not satisfy `{requirement}`",
                requirement.name
            ),
            Some(VersionOrUrl::Url(_)) => {
                bail!("the scenario oracle does not model direct URLs: {requirement}");
            }
            None => {}
        }
        if let Some(requires_python) = &metadata.requires_python {
            ensure!(
                marker.implies(python_marker(requires_python)).is_true(),
                "{}=={version} does not declare support for its entire reachable Python domain `{}`",
                requirement.name,
                marker
                    .try_to_string()
                    .unwrap_or_else(|| "all environments".to_owned()),
            );
        }

        let activation = self
            .activations
            .entry(requirement.name.clone())
            .or_insert_with(|| Activation {
                marker: MarkerTree::FALSE,
                extras: BTreeSet::new(),
            });
        let combined = activation.marker.or(marker);
        let mut changed = combined != activation.marker;
        activation.marker = combined;
        for extra in &requirement.extras {
            changed |= activation.extras.insert(extra.clone());
        }
        if changed {
            self.activation_updates += 1;
        }
        Ok(changed)
    }
}

/// Build the full release-only Python condition, retaining exclusions and upper bounds.
fn python_marker(specifiers: &VersionSpecifiers) -> MarkerTree {
    specifiers
        .iter()
        .fold(MarkerTree::TRUE, |marker, specifier| {
            marker.and(MarkerTree::expression(MarkerExpression::Version {
                key: MarkerValueVersion::PythonFullVersion,
                specifier: specifier.clone(),
            }))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::packse::check::{LockScenarioFailureKind, ScenarioPlatform, ScenarioTarget};
    use crate::packse::generate::WitnessedProjectGraph;

    fn assignment(packages: &[(&str, &str)]) -> Selection {
        packages
            .iter()
            .map(|(name, version)| {
                (
                    name.parse().expect("valid package name"),
                    version.parse().expect("valid package version"),
                )
            })
            .collect()
    }

    fn activation(certificate: &MarkerWitnessCertificate, name: &str) -> MarkerTree {
        certificate.activations[&name.parse::<PackageName>().expect("valid package name")]
            .marker
            .as_ref()
            .map_or(MarkerTree::TRUE, |marker| {
                marker.parse().expect("valid activation marker")
            })
    }

    #[test]
    fn certifies_disjoint_transitive_markers() -> Result<()> {
        let document = ScenarioDocument::from_path(
            &super::super::workspace_root()
                .join("test/scenarios/fork/non-local-fork-marker-unreachable.toml"),
        )?;
        let assignment = assignment(&[("a", "1.0.0")]);
        let certificate = certify_project_marker_witness(&document, &assignment, 100)?;
        assert_eq!(certificate.assigned_packages(), 1);
        assert_eq!(certificate.activated_packages(), 1);
        assert_eq!(
            activation(&certificate, "a"),
            "python_full_version >= '3.12' and python_full_version < '3.15' and sys_platform == 'win32'"
                .parse()?,
        );
        assert!(
            WitnessedProjectGraph {
                document: document.clone(),
                assignment: assignment.clone(),
            }
            .certify_universal_witness()
            .is_err()
        );

        let overlapping = document
            .to_toml()?
            .replace("sys_platform != 'win32'", "sys_platform == 'win32'")
            .parse()?;
        let error = certify_project_marker_witness(&overlapping, &assignment, 100)
            .expect_err("the missing dependency is reachable on Windows");
        assert!(error.to_string().contains("the witness is missing"));
        assert_eq!(LockScenarioFailureKind::from_error(&error), None);
        Ok(())
    }

    #[test]
    fn checks_the_entire_reachable_python_domain() -> Result<()> {
        let contents = r#"
name = "conditional-python"
[root]
requires_python = ">=3.12,<3.15"
requires = ["a; python_full_version >= '3.13'"]
[expected]
satisfiable = true
[packages.a.versions."1.0.0"]
requires_python = ">=3.13,<3.15"
"#;
        let assignment = assignment(&[("a", "1.0.0")]);
        certify_project_marker_witness(&contents.parse()?, &assignment, 100)?;
        for narrower in [">=3.14,<3.15", ">=3.13,<3.15,!=3.14.*"] {
            let document = contents
                .replace(
                    "requires_python = \">=3.13,<3.15\"",
                    &format!("requires_python = \"{narrower}\""),
                )
                .parse()?;
            let error = certify_project_marker_witness(&document, &assignment, 100)
                .expect_err("the assigned version excludes a reachable Python region");
            assert!(error.to_string().contains("entire reachable Python domain"));
        }
        Ok(())
    }

    #[test]
    fn activation_growth_is_independent_of_root_order() -> Result<()> {
        let contents = r#"
name = "activation-cycle"
[root]
requires_python = ">=3.12,<3.15"
requires = ["a[feature]; sys_platform == 'win32'", "b"]
[expected]
satisfiable = true
[packages.a.versions."1.0.0"]
requires = ["b"]
extras = { feature = ["c"] }
[packages.b.versions."1.0.0"]
requires = ["a[feature]; sys_platform == 'win32'"]
[packages.c.versions."1.0.0"]
requires = ["b"]
"#;
        let assignment = assignment(&[("a", "1.0.0"), ("b", "1.0.0"), ("c", "1.0.0")]);
        let certificate = certify_project_marker_witness(&contents.parse()?, &assignment, 100)?;
        let reversed = contents
            .replace(
                r#"["a[feature]; sys_platform == 'win32'", "b"]"#,
                r#"["b", "a[feature]; sys_platform == 'win32'"]"#,
            )
            .parse()?;
        let reversed = certify_project_marker_witness(&reversed, &assignment, 100)?;
        assert_eq!(certificate.activations, reversed.activations);
        assert_eq!(certificate.activated_packages(), 3);
        assert_eq!(activation(&certificate, "a"), activation(&certificate, "c"));
        assert_eq!(
            activation(&certificate, "b"),
            python_marker(&">=3.12,<3.15".parse()?)
        );
        Ok(())
    }

    #[test]
    fn retains_unknown_requested_extras_conservatively() -> Result<()> {
        let contents = r#"
name = "unknown-extra"
[root]
requires_python = ">=3.12,<3.15"
requires = ["a[ghost]"]
[expected]
satisfiable = true
[packages.a.versions."1.0.0"]
requires = ["b; extra == 'ghost'"]
[packages.b.versions."1.0.0"]
"#;
        let certificate = certify_project_marker_witness(
            &contents.parse()?,
            &assignment(&[("a", "1.0.0"), ("b", "1.0.0")]),
            100,
        )?;
        assert_eq!(certificate.activated_packages(), 2);
        assert!(
            certificate.activations[&"a".parse::<PackageName>()?]
                .extras
                .contains(&"ghost".parse()?)
        );

        let missing = contents
            .replace("[packages.b.versions.\"1.0.0\"]", "")
            .parse()?;
        let error = certify_project_marker_witness(&missing, &assignment(&[("a", "1.0.0")]), 100)
            .expect_err("the undeclared requested extra can still activate metadata requirements");
        assert!(error.to_string().contains("the witness is missing"));
        Ok(())
    }

    #[test]
    fn extra_over_approximation_is_not_an_unsat_claim() -> Result<()> {
        let document: ScenarioDocument = r#"
name = "disjoint-extra-requests"
[root]
requires_python = ">=3.12,<3.15"
requires = ["a[x]; sys_platform == 'win32'", "a[y]; sys_platform != 'win32'"]
[expected]
satisfiable = true
[packages.a.versions."1.0.0"]
requires = ["missing; extra == 'x' and extra == 'y'"]
extras = { x = [], y = [] }
"#
        .parse()?;
        let assignment = assignment(&[("a", "1.0.0")]);
        let error = certify_project_marker_witness(&document, &assignment, 100)
            .expect_err("the global extra union deliberately over-approximates the conjunction");
        assert_eq!(LockScenarioFailureKind::from_error(&error), None);

        let targets = ScenarioTarget::matrix(
            &["3.12".parse().expect("valid Python version")],
            &[ScenarioPlatform::Windows, ScenarioPlatform::Linux],
        );
        let graph = WitnessedProjectGraph {
            document,
            assignment,
        };
        assert!(graph.check_witness(&targets)? > 0);
        Ok(())
    }

    #[test]
    fn rejects_incompatible_unsampled_extra_requirements() -> Result<()> {
        let document = r#"
name = "unsampled-extra"
[root]
requires_python = ">=3.12,<3.15"
requires = ["a[feature]; sys_platform == 'freebsd'"]
[expected]
satisfiable = true
[packages.a.versions."1.0.0"]
extras = { feature = ["b==2.0.0"] }
[packages.b.versions."1.0.0"]
"#
        .parse()?;
        let error = certify_project_marker_witness(
            &document,
            &assignment(&[("a", "1.0.0"), ("b", "1.0.0")]),
            100,
        )
        .expect_err("the fixed assignment must work outside the sampled platform matrix");
        assert!(error.to_string().contains("does not satisfy"));
        Ok(())
    }

    #[test]
    fn rejects_invalid_assignments_and_exhausted_budgets() -> Result<()> {
        let document: ScenarioDocument = r#"
name = "witness-budget"
[root]
requires_python = ">=3.12,<3.15"
requires = ["a"]
[expected]
satisfiable = true
[packages.a.versions."1.0.0"]
requires = ["b"]
[packages.b.versions."1.0.0"]
"#
        .parse()?;
        for invalid in [
            assignment(&[("a", "1.0.0")]),
            assignment(&[("a", "2.0.0"), ("b", "1.0.0")]),
        ] {
            assert!(certify_project_marker_witness(&document, &invalid, 100).is_err());
        }
        let assignment = assignment(&[("a", "1.0.0"), ("b", "1.0.0")]);
        for max_work in [0, 1] {
            let error = certify_project_marker_witness(&document, &assignment, max_work)
                .expect_err("the proof must fit in its work budget");
            assert_eq!(LockScenarioFailureKind::from_error(&error), None);
        }
        let certificate = certify_project_marker_witness(&document, &assignment, 100)?;
        assert!(certificate.checked_requirements() >= 2);
        Ok(())
    }

    #[test]
    fn keeps_unsupported_policies_outside_the_certificate() -> Result<()> {
        let contents = r#"
name = "witness-policies"
[root]
requires_python = ">=3.12,<3.15"
requires = ["a"]
[expected]
satisfiable = true
[packages.a.versions."1.0.0"]
"#;
        let assignment = assignment(&[("a", "1.0.0")]);
        for document in [
            contents.replace(">=3.12,<3.15", ">=3.12,<3.12"),
            contents.replace(">=3.12,<3.15", ">=2,<4"),
            contents.replace("requires = [\"a\"]", "requires = [\"uv-scenario-root\"]"),
            contents.replace(
                "requires = [\"a\"]",
                "requires = [\"a; extra == 'feature'\"]",
            ),
            format!("{contents}\nrequires = [\"a; extra != 'feature'\"]\n"),
        ] {
            let error = certify_project_marker_witness(&document.parse()?, &assignment, 100)
                .expect_err("unsupported policies do not produce a certificate");
            assert_eq!(LockScenarioFailureKind::from_error(&error), None);
        }
        Ok(())
    }
}
