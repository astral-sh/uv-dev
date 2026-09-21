//! The independently admitted marker domain of a scenario's universal lock.

use anyhow::{Result, ensure};
use uv_pep440::VersionSpecifiers;
use uv_pep508::{MarkerExpression, MarkerTree, MarkerValueVersion};

use super::scenario::Scenario;

/// Return the union of the explicitly supported environments, or the universal domain.
///
/// Project extras and PEP 751 list markers do not describe a concrete interpreter environment.
/// Overlapping entries are rejected because uv requires disjoint supported environments.
pub(super) fn lock_environment_marker(scenario: &Scenario) -> Result<MarkerTree> {
    let environments = &scenario.resolver_options.environments;
    if environments.is_empty() {
        return Ok(MarkerTree::TRUE);
    }

    let python = scenario
        .root
        .requires_python
        .as_ref()
        .map_or(MarkerTree::TRUE, python_marker);
    let mut domain = MarkerTree::FALSE;
    for &environment in environments {
        let mut has_extras = false;
        environment.visit_extras(|_, _| has_extras = true);
        ensure!(
            !has_extras
                && !environment
                    .to_dnf()
                    .iter()
                    .flatten()
                    .any(|expression| { matches!(expression, MarkerExpression::List { .. }) }),
            "supported lock environments must not use extra or list markers"
        );
        ensure!(
            !python.is_disjoint(environment),
            "a supported lock environment is disjoint from the root Python range"
        );
        ensure!(
            domain.is_disjoint(environment),
            "supported lock environments must be disjoint"
        );
        domain = domain.or(environment);
    }
    ensure!(
        !domain.is_false(),
        "supported lock environments must not be empty"
    );
    Ok(domain)
}

/// Build the full release-only Python condition, retaining exclusions and upper bounds.
pub(super) fn python_marker(specifiers: &VersionSpecifiers) -> MarkerTree {
    specifiers
        .iter()
        .fold(MarkerTree::TRUE, |marker, specifier| {
            marker.and(MarkerTree::expression(MarkerExpression::Version {
                key: MarkerValueVersion::PythonFullVersion,
                specifier: specifier.clone(),
            }))
        })
}

/// Put the same independently validated environment entries in every generated project.
pub(super) fn apply_lock_environments(
    scenario: &Scenario,
    pyproject: &mut serde_json::Value,
) -> Result<()> {
    lock_environment_marker(scenario)?;
    if !scenario.resolver_options.environments.is_empty() {
        let environments = scenario
            .resolver_options
            .environments
            .iter()
            .map(|environment| {
                environment
                    .try_to_string()
                    .unwrap_or_else(|| "python_version == '0' or python_version != '0'".to_owned())
            })
            .collect::<Vec<_>>();
        pyproject["tool"]["uv"]["environments"] = serde_json::json!(environments);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scenario(environments: &[&str]) -> Result<Scenario> {
        let mut scenario: Scenario = toml::from_str(
            r#"
name = "restricted-domain"
[root]
requires_python = ">=3.12,<3.15"
requires = []
[expected]
satisfiable = true
"#,
        )?;
        scenario.resolver_options.environments = environments
            .iter()
            .map(|marker| marker.parse())
            .collect::<Result<_, _>>()?;
        Ok(scenario)
    }

    #[test]
    fn admits_disjoint_environment_unions() -> Result<()> {
        assert!(lock_environment_marker(&scenario(&[])?)?.is_true());
        let scenario = scenario(&["sys_platform == 'win32'", "sys_platform == 'linux'"])?;
        assert_eq!(
            lock_environment_marker(&scenario)?,
            "sys_platform == 'linux' or sys_platform == 'win32'".parse()?
        );
        let mut project = serde_json::json!({"project": {"name": "example"}});
        apply_lock_environments(&scenario, &mut project)?;
        assert_eq!(
            project["tool"]["uv"]["environments"],
            serde_json::json!(["sys_platform == 'win32'", "sys_platform == 'linux'"])
        );
        Ok(())
    }

    #[test]
    fn rejects_ambiguous_or_non_environment_domains() -> Result<()> {
        let cases: &[(&[&str], &str)] = &[
            (
                &["python_version < '0'"],
                "disjoint from the root Python range",
            ),
            (
                &["python_version < '3.12'"],
                "disjoint from the root Python range",
            ),
            (
                &["sys_platform == 'linux'", "python_version >= '3.12'"],
                "disjoint",
            ),
            (&["extra == 'feature'"], "extra or list markers"),
            (&["'dev' in dependency_groups"], "extra or list markers"),
        ];
        let mut errors = Vec::new();
        for (markers, expected) in cases {
            let error = lock_environment_marker(&scenario(markers)?)
                .expect_err("the environment domain is unsupported");
            assert!(error.to_string().contains(expected));
            errors.push(error.to_string());
        }
        insta::assert_debug_snapshot!(errors, @r#"
        [
            "a supported lock environment is disjoint from the root Python range",
            "a supported lock environment is disjoint from the root Python range",
            "supported lock environments must be disjoint",
            "supported lock environments must not use extra or list markers",
            "supported lock environments must not use extra or list markers",
        ]
        "#);
        Ok(())
    }

    #[test]
    fn renders_an_explicit_universal_environment() -> Result<()> {
        let scenario = scenario(&["sys_platform == 'linux' or sys_platform != 'linux'"])?;
        let mut project = serde_json::json!({});
        apply_lock_environments(&scenario, &mut project)?;
        let rendered = project["tool"]["uv"]["environments"][0]
            .as_str()
            .expect("rendered marker");
        assert!(rendered.parse::<MarkerTree>()?.is_true());
        Ok(())
    }
}
