use std::collections::BTreeSet;
use std::ops::Range;
use std::str::FromStr;

use uv_distribution_types::RequirementProvenance;
use uv_errors::SourceFile;
use uv_normalize::{ExtraName, PackageName};
use uv_pep508::Requirement;
use uv_pypi_types::{RequiresDist, VerbatimParsedUrl};
use uv_toml::SourcePathSegment::{Index, Key};
use uv_toml::{SourceMap, SourcePathSegment};

/// Map a known-static PEP 621 input to the complete metadata sequence before source lowering.
///
/// The source is retained by the successful static metadata reader. Built or cached metadata has
/// no such input, and any mismatch in declaration order or semantics drops the entire mapping.
pub(super) fn project_requirement_sources(
    source: &SourceFile,
    metadata: &RequiresDist,
) -> Option<Box<[RequirementProvenance]>> {
    if metadata.requires_dist.is_empty() {
        return None;
    }

    let map = SourceMap::parse(source.text()).ok()?;
    if PackageName::from_str(map.string(&[Key("project"), Key("name")])?).ok()? != metadata.name {
        return None;
    }

    let dynamic = [Key("project"), Key("dynamic")];
    if let Some(count) = map.array_len(&dynamic) {
        for index in 0..count {
            let field = map.string(&[Key("project"), Key("dynamic"), Index(index)])?;
            if matches!(field, "dependencies" | "optional-dependencies") {
                return None;
            }
        }
    } else if map.span(&dynamic).is_some() {
        return None;
    }

    let dependencies = [Key("project"), Key("dependencies")];
    if map.span(&dependencies).is_none() && map.span(&[Key("tool"), Key("poetry")]).is_some() {
        return None;
    }

    let mut declarations = Vec::new();
    if map.span(&dependencies).is_some() {
        collect_requirements(&map, &dependencies, None, &mut declarations)?;
    }

    let optional_dependencies = [Key("project"), Key("optional-dependencies")];
    let mut provides_extra = Vec::new();
    if let Some(keys) = map.keys(&optional_dependencies) {
        let mut seen = BTreeSet::new();
        for key in keys {
            let extra = ExtraName::from_str(key).ok()?;
            if !seen.insert(extra.clone()) {
                return None;
            }
            collect_requirements(
                &map,
                &[Key("project"), Key("optional-dependencies"), Key(key)],
                Some(&extra),
                &mut declarations,
            )?;
            provides_extra.push(extra);
        }
    } else if map.span(&optional_dependencies).is_some() {
        return None;
    }

    if provides_extra.as_slice() != metadata.provides_extra.as_ref()
        || declarations.len() != metadata.requires_dist.len()
        || declarations
            .iter()
            .zip(metadata.requires_dist.iter())
            .any(|(declaration, requirement)| declaration.requirement != *requirement)
    {
        return None;
    }

    Some(
        declarations
            .into_iter()
            .map(|declaration| RequirementProvenance::new(source.clone(), declaration.range))
            .collect(),
    )
}

struct AuthoredRequirement {
    requirement: Requirement<VerbatimParsedUrl>,
    range: Range<usize>,
}

fn collect_requirements(
    map: &SourceMap<'_>,
    parent: &[SourcePathSegment<'_>],
    extra: Option<&ExtraName>,
    declarations: &mut Vec<AuthoredRequirement>,
) -> Option<()> {
    for index in 0..map.array_len(parent)? {
        let mut path = parent.to_vec();
        path.push(Index(index));
        let decoded = map.string(&path)?;
        // Metadata parsing accepts some invalid PEP 508 spellings. Reapplying its fixups would
        // emit duplicate warnings, so source attribution is limited to strict requirements.
        let requirement = Requirement::<VerbatimParsedUrl>::from_str(decoded).ok()?;
        let range = map.span(&path)?;
        let requirement = if let Some(extra) = extra {
            requirement.with_extra_marker(extra.clone())
        } else {
            requirement
        };
        declarations.push(AuthoredRequirement { requirement, range });
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use anyhow::{Context, Result, bail};
    use uv_distribution_types::RequirementProvenance;
    use uv_errors::{Diagnostic, ErrorOptions, Hints, SourceFile, write_error_chain_with_options};
    use uv_pypi_types::{PyProjectToml, RequiresDist};

    use super::project_requirement_sources;

    #[derive(Debug, thiserror::Error)]
    #[error("project requirement declarations")]
    struct DeclarationError(Box<[RequirementProvenance]>);

    fn diagnostic_for_error<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
        let error = error.downcast_ref::<DeclarationError>()?;
        Some(
            error
                .0
                .iter()
                .fold(Diagnostic::default(), |diagnostic, source| {
                    diagnostic.with_snippet(source.snippet("declared here"))
                }),
        )
    }

    fn metadata(source: &str) -> Result<RequiresDist> {
        Ok(RequiresDist::from_pyproject_toml(
            PyProjectToml::from_toml(source, "pyproject.toml")?,
        )?)
    }

    fn render(sources: Box<[RequirementProvenance]>) -> Result<String> {
        let mut output = String::new();
        write_error_chain_with_options(
            &DeclarationError(sources),
            &Hints::none(),
            ErrorOptions::default()
                .with_width_override(usize::MAX)
                .with_diagnostic(diagnostic_for_error)
                .with_stream(&mut output),
        )?;
        Ok(anstream::adapter::strip_str(&output).to_string())
    }

    #[test]
    fn project_requirement_spans_follow_static_declarations() -> Result<()> {
        let source = "# café\r\n[project]\r\nname = 'root'\r\nversion = '0.1.0'\r\ndependencies = [\r\n  \"Demo.Pkg==1,>=1.2\",\r\n  \"other==2\",\r\n]\r\n[project.optional-dependencies]\r\n\"Z.Tools\" = [\"other==3\"]\r\n\"Dev.Tools\" = [\r\n  \"demo-pkg==4,>=5\",\r\n]\r\n";
        let sources = project_requirement_sources(
            &SourceFile::new("pyproject.toml", source),
            &metadata(source)?,
        )
        .context("static declarations should have exact sources")?;
        insta::assert_snapshot!(render(sources)?, @r#"
        error: project requirement declarations
            --> pyproject.toml:6:3
             |
           6 |   "Demo.Pkg==1,>=1.2",
             |   ^^^^^^^^^^^^^^^^^^^ declared here
             |
            ::: pyproject.toml:7:3
             |
           7 |   "other==2",
             |   ^^^^^^^^^^ declared here
            ::: pyproject.toml:10:14
             |
            ::: pyproject.toml:12:3
             |
          12 |   "demo-pkg==4,>=5",
             |   ^^^^^^^^^^^^^^^^^ declared here
        "#);
        Ok(())
    }

    #[test]
    fn project_requirement_sources_reject_metadata_mismatches() -> Result<()> {
        let source = "[project]\nname = 'root'\nversion = '0.1.0'\ndependencies = ['demo==1', 'other==2']\n[project.optional-dependencies]\n'Z.Tools' = ['demo==3']\n'Dev.Tools' = ['other==4']\n";
        let expected = metadata(source)?;
        let mut reordered = expected.clone();
        reordered.requires_dist.reverse();
        let mut reordered_extras = expected.clone();
        reordered_extras.provides_extra.reverse();
        let cases = [
            ("matching static input", source.to_string(), &expected),
            ("reordered metadata", source.to_string(), &reordered),
            ("reordered extras", source.to_string(), &reordered_extras),
            (
                "changed requirement",
                source.replace("demo==1", "demo==2"),
                &expected,
            ),
            (
                "dynamic dependencies",
                source.replace(
                    "version = '0.1.0'",
                    "version = '0.1.0'\ndynamic = ['dependencies']",
                ),
                &expected,
            ),
            (
                "dynamic optional dependencies",
                source.replace(
                    "version = '0.1.0'",
                    "version = '0.1.0'\ndynamic = ['optional-dependencies']",
                ),
                &expected,
            ),
            (
                "ambiguous normalized extra",
                source.replace("'Dev.Tools'", "'z_tools'"),
                &expected,
            ),
            (
                "different project",
                source.replace("name = 'root'", "name = 'different'"),
                &expected,
            ),
        ];
        let matches = cases
            .into_iter()
            .map(|(case, source, metadata)| {
                (
                    case,
                    project_requirement_sources(
                        &SourceFile::new("pyproject.toml", source),
                        metadata,
                    )
                    .is_some(),
                )
            })
            .collect::<Vec<_>>();
        insta::assert_debug_snapshot!(matches, @r#"
        [
            (
                "matching static input",
                true,
            ),
            (
                "reordered metadata",
                false,
            ),
            (
                "reordered extras",
                false,
            ),
            (
                "changed requirement",
                false,
            ),
            (
                "dynamic dependencies",
                false,
            ),
            (
                "dynamic optional dependencies",
                false,
            ),
            (
                "ambiguous normalized extra",
                false,
            ),
            (
                "different project",
                false,
            ),
        ]
        "#);
        Ok(())
    }

    #[test]
    fn project_requirement_occurrences_have_distinct_identity() -> Result<()> {
        let source = "[project]\nname = 'root'\nversion = '0.1.0'\ndependencies = [\n  'demo==1,>=2',\n  'demo==1,>=2',\n]\n";
        let sources = project_requirement_sources(
            &SourceFile::new("pyproject.toml", source),
            &metadata(source)?,
        )
        .context("both authored occurrences should be retained")?;
        let [first, second] = sources.as_ref() else {
            bail!("expected two authored occurrences");
        };
        assert!(first.unambiguous_with(&first.clone()).is_some());
        assert!(first.unambiguous_with(second).is_none());
        Ok(())
    }

    #[test]
    fn project_requirement_sources_keep_shared_lines() -> Result<()> {
        let source = "[project]\nname = 'root'\nversion = '0.1.0'\ndependencies = [\n  'demo==1,>=2', # required by the application\n  'other==2,>=3', 'direct @ https://example.com/direct-1.0.0-py3-none-any.whl',\n  \"marked==3,>=4; sys_platform != 'win32'\",\n]\n";
        let sources = project_requirement_sources(
            &SourceFile::new("pyproject.toml", source),
            &metadata(source)?,
        )
        .context("each declaration should have an exact source")?;
        insta::assert_snapshot!(render(sources)?, @"
        error: project requirement declarations
           --> pyproject.toml:5:3
           ::: pyproject.toml:6:3
           ::: pyproject.toml:6:19
           ::: pyproject.toml:7:3
        ");
        Ok(())
    }
}
