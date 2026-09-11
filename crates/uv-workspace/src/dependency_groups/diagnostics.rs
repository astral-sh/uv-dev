use std::error::Error;
use std::ops::Range;
use std::path::Path;
use std::str::FromStr;

use uv_errors::{Diagnostic, Info, SourceAnnotation, SourceFile, SourceSnippet};
use uv_fs::Simplified;
use uv_normalize::{DEV_DEPENDENCIES, GroupName};
use uv_pep440::VersionSpecifiers;
use uv_pypi_types::DependencyGroupSpecifier;
use uv_toml::SourcePathSegment::{Index, Key};
use uv_toml::{SourceMap, SourcePathSegment};

use crate::pyproject::PyProjectToml;

use super::{DependencyGroupError, DependencyGroupErrorInner, GroupInclude};

/// Resolve source locations without changing the semantic dependency-group error or its sources.
pub fn diagnostic_for_error<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
    let error = error.downcast_ref::<DependencyGroupError>().or_else(|| {
        error
            .downcast_ref::<Box<DependencyGroupError>>()
            .map(AsRef::as_ref)
    })?;
    let context = error.diagnostic.as_deref()?;
    let mut diagnostic = Diagnostic::default().with_snippet(context.primary.clone());
    for related in &context.related {
        diagnostic = diagnostic
            .with_info(Info::new(related.message.clone()).with_snippet(related.snippet.clone()));
    }
    Some(Diagnostic::default().with_source(diagnostic))
}

/// A parsed project source shared by Python-requirement declarations and include locations.
pub struct PythonRequirementsSource<'a> {
    source: SourceFile,
    map: Option<SourceMap<'a>>,
    groups_match_source: bool,
}

impl<'a> PythonRequirementsSource<'a> {
    /// Capture the `pyproject.toml` in this directory without reopening it while rendering.
    pub fn new(path: &Path, pyproject: &'a PyProjectToml) -> Self {
        let source = SourceFile::new(
            path.join("pyproject.toml").portable_display().to_string(),
            pyproject.raw.as_str(),
        );
        let map = SourceMap::parse(&pyproject.raw).ok();
        let groups_match_source = map
            .as_ref()
            .is_some_and(|map| validate_dependency_groups_source(map, pyproject).is_some());
        Self {
            source,
            map,
            groups_match_source,
        }
    }

    /// The exact retained source used by this map.
    pub fn source_file(&self) -> &SourceFile {
        &self.source
    }

    /// The source map when the retained syntax could be parsed.
    pub fn source_map(&self) -> Option<&SourceMap<'a>> {
        self.map.as_ref()
    }

    /// Locate an authored dependency-group Python requirement.
    pub fn requires_python_source(
        &self,
        group: &GroupName,
        requires_python: &VersionSpecifiers,
    ) -> SourceSnippet<'static> {
        requires_python_source(&self.source, self.map.as_ref(), group, requires_python)
    }

    /// Locate an include occurrence in the retained dependency-group declarations.
    pub fn include_source(&self, include: &GroupInclude) -> Option<SourceSnippet<'static>> {
        let range = include_span(self.map.as_ref()?, include)?;
        Some(include_source_snippet(
            &self.source,
            SourceAnnotation::secondary(range).with_label("included here"),
            self.groups_match_source,
        ))
    }
}

fn requires_python_source(
    source: &SourceFile,
    map: Option<&SourceMap<'_>>,
    group: &GroupName,
    requires_python: &VersionSpecifiers,
) -> SourceSnippet<'static> {
    let span = map.and_then(|map| requires_python_span(map, group, requires_python));
    let mut snippet = SourceSnippet::new(source.clone());
    if let Some(span) = span {
        snippet = snippet.with_annotation(SourceAnnotation::primary(span).with_label(format!(
            "group `{group}` requires Python `{requires_python}`"
        )));
    }
    snippet
}

fn requires_python_span(
    map: &SourceMap<'_>,
    group: &GroupName,
    requires_python: &VersionSpecifiers,
) -> Option<Range<usize>> {
    let parent = [Key("tool"), Key("uv"), Key("dependency-groups")];
    let key = original_group_key(map, &parent, group)?;
    let path = [
        Key("tool"),
        Key("uv"),
        Key("dependency-groups"),
        Key(key),
        Key("requires-python"),
    ];
    let declared = VersionSpecifiers::from_str(map.string(&path)?).ok()?;
    if &declared != requires_python {
        return None;
    }
    map.span(&path)
}

#[derive(Debug)]
pub(super) enum DependencyGroupProvenance {
    Includes(Vec<GroupInclude>),
    Settings(GroupName),
}

#[derive(Debug)]
pub(super) struct DependencyGroupDiagnostic {
    primary: SourceSnippet<'static>,
    related: Vec<RelatedLocation>,
}

#[derive(Debug)]
struct RelatedLocation {
    message: String,
    snippet: SourceSnippet<'static>,
}

impl DependencyGroupDiagnostic {
    pub(super) fn new(
        path: &Path,
        pyproject: &PyProjectToml,
        error: &DependencyGroupErrorInner,
        provenance: DependencyGroupProvenance,
    ) -> Option<Self> {
        let map = SourceMap::parse(&pyproject.raw).ok()?;
        let source = SourceFile::new(
            path.join("pyproject.toml").portable_display().to_string(),
            pyproject.raw.as_str(),
        );
        let groups_match_source = validate_dependency_groups_source(&map, pyproject).is_some();

        match error {
            DependencyGroupErrorInner::GroupNotFound(group, parent) => {
                let DependencyGroupProvenance::Includes(includes) = provenance else {
                    return None;
                };
                let last = includes.last()?;
                if &last.group != parent || &last.included != group {
                    return None;
                }
                Self::from_includes(
                    &source,
                    &map,
                    groups_match_source,
                    &includes,
                    "undefined group",
                )
            }
            DependencyGroupErrorInner::DevGroupInclude(parent) => {
                let DependencyGroupProvenance::Includes(includes) = provenance else {
                    return None;
                };
                let last = includes.last()?;
                if &last.group != parent || last.included != *DEV_DEPENDENCIES {
                    return None;
                }
                let mut diagnostic = Self::from_includes(
                    &source,
                    &map,
                    groups_match_source,
                    &includes,
                    "the standard `dev` group is not defined",
                )?;
                diagnostic.with_legacy_dev_location(&source, &map);
                Some(diagnostic)
            }
            DependencyGroupErrorInner::DependencyGroupCycle(cycle) => {
                let DependencyGroupProvenance::Includes(includes) = provenance else {
                    return None;
                };
                if !cycle
                    .0
                    .iter()
                    .eq(includes.iter().map(|include| &include.group))
                    || &includes.last()?.included != cycle.0.first()?
                {
                    return None;
                }
                Self::from_includes(
                    &source,
                    &map,
                    groups_match_source,
                    &includes,
                    "closes the cycle",
                )
            }
            DependencyGroupErrorInner::SettingsGroupNotFound(group) => {
                let DependencyGroupProvenance::Settings(source_group) = provenance else {
                    return None;
                };
                if &source_group != group {
                    return None;
                }
                Some(Self::from_settings(&source, &map, group)?)
            }
            DependencyGroupErrorInner::SettingsDevGroupInclude => {
                let DependencyGroupProvenance::Settings(group) = provenance else {
                    return None;
                };
                if group != *DEV_DEPENDENCIES {
                    return None;
                }
                let mut diagnostic = Self::from_settings(&source, &map, &group)?;
                diagnostic.with_legacy_dev_location(&source, &map);
                Some(diagnostic)
            }
            DependencyGroupErrorInner::GroupParseError(..)
            | DependencyGroupErrorInner::DependencyObjectSpecifierNotSupported(..) => None,
        }
    }

    fn from_includes(
        source: &SourceFile,
        map: &SourceMap<'_>,
        groups_match_source: bool,
        includes: &[GroupInclude],
        label: &'static str,
    ) -> Option<Self> {
        let (last, parents) = includes.split_last()?;
        let range = include_span(map, last)?;
        let primary = include_source_snippet(
            source,
            SourceAnnotation::primary(range).with_label(label),
            groups_match_source,
        );
        let related = parents
            .iter()
            .filter_map(|include| {
                let range = include_span(map, include)?;
                let snippet = include_source_snippet(
                    source,
                    SourceAnnotation::secondary(range).with_label("included here"),
                    groups_match_source,
                );
                Some(RelatedLocation {
                    message: format!(
                        "Group `{}` is included by `{}` here",
                        include.included, include.group
                    ),
                    snippet,
                })
            })
            .collect();
        Some(Self { primary, related })
    }

    fn from_settings(source: &SourceFile, map: &SourceMap<'_>, group: &GroupName) -> Option<Self> {
        let parent = [Key("tool"), Key("uv"), Key("dependency-groups")];
        let key = original_group_key(map, &parent, group)?;
        let range = map.key_span(&parent, key)?;
        let primary = SourceSnippet::new(source.clone())
            .with_annotation(SourceAnnotation::primary(range).with_label("undefined group"));
        Some(Self {
            primary,
            related: Vec::new(),
        })
    }

    fn with_legacy_dev_location(&mut self, source: &SourceFile, map: &SourceMap<'_>) {
        if let Some(range) = map.key_span(&[Key("tool"), Key("uv")], "dev-dependencies") {
            self.related.push(RelatedLocation {
                message: "Legacy development dependencies are defined here".to_string(),
                snippet: SourceSnippet::new(source.clone()).with_annotation(
                    SourceAnnotation::secondary(range)
                        .with_label("legacy development dependencies"),
                ),
            });
        }
    }
}

/// Resolve normalized names to their unique decoded TOML spelling.
fn original_group_key<'a>(
    map: &'a SourceMap<'_>,
    parent: &[SourcePathSegment<'_>],
    name: &GroupName,
) -> Option<&'a str> {
    let mut keys = map
        .keys(parent)?
        .filter(|key| GroupName::from_str(key).is_ok_and(|candidate| &candidate == name));
    let key = keys.next()?;
    keys.next().is_none().then_some(key)
}

fn include_span(map: &SourceMap<'_>, include: &GroupInclude) -> Option<Range<usize>> {
    let key = original_group_key(map, &[Key("dependency-groups")], &include.group)?;
    let path = [
        Key("dependency-groups"),
        Key(key),
        Index(include.index),
        Key("include-group"),
    ];
    if GroupName::from_str(map.string(&path)?).ok()? != include.included {
        return None;
    }
    map.span(&path)
}

/// Keep a known include location when the complete semantic input cannot be matched to the source.
fn include_source_snippet(
    source: &SourceFile,
    annotation: SourceAnnotation<'static>,
    groups_match_source: bool,
) -> SourceSnippet<'static> {
    let snippet = SourceSnippet::new(source.clone()).with_annotation(annotation);
    if groups_match_source {
        snippet
    } else {
        snippet.without_source_text()
    }
}

/// Check the exact decoded values, including entries not yet reached by semantic traversal.
fn validate_dependency_groups_source(map: &SourceMap<'_>, pyproject: &PyProjectToml) -> Option<()> {
    let Some(groups) = &pyproject.dependency_groups else {
        return map
            .span(&[Key("dependency-groups")])
            .is_none()
            .then_some(());
    };
    if map.keys(&[Key("dependency-groups")])?.count() != groups.keys().count() {
        return None;
    }

    for (name, specifiers) in groups {
        let key = original_group_key(map, &[Key("dependency-groups")], name)?;
        if map.array_len(&[Key("dependency-groups"), Key(key)])? != specifiers.len() {
            return None;
        }
        for (index, specifier) in specifiers.iter().enumerate() {
            let path = [Key("dependency-groups"), Key(key), Index(index)];
            match specifier {
                DependencyGroupSpecifier::Requirement(requirement) => {
                    let decoded = map.string(&path)?;
                    if decoded != requirement {
                        return None;
                    }
                }
                DependencyGroupSpecifier::IncludeGroup { include_group } => {
                    let mut keys = map.keys(&path)?;
                    if keys.next() != Some("include-group") || keys.next().is_some() {
                        return None;
                    }
                    let path = [
                        Key("dependency-groups"),
                        Key(key),
                        Index(index),
                        Key("include-group"),
                    ];
                    if GroupName::from_str(map.string(&path)?).ok()? != *include_group {
                        return None;
                    }
                }
                DependencyGroupSpecifier::Object(values) => {
                    if map.keys(&path)?.count() != values.len() {
                        return None;
                    }
                    for (object_key, expected) in values {
                        let value_path = [
                            Key("dependency-groups"),
                            Key(key),
                            Index(index),
                            Key(object_key),
                        ];
                        if map.string(&value_path)? != expected.as_str() {
                            return None;
                        }
                    }
                }
            }
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::str::FromStr;

    use anyhow::{Context, Result};
    use uv_normalize::GroupName;
    use uv_pep440::VersionSpecifiers;
    use uv_toml::SourceMap;

    use crate::dependency_groups::FlatDependencyGroups;
    use crate::pyproject::PyProjectToml;

    use super::{
        GroupInclude, include_span, requires_python_span, validate_dependency_groups_source,
    };

    #[test]
    fn include_spans_follow_normalized_keys_and_occurrences() -> Result<()> {
        let source = "# café\n[dependency-groups]\n\"Dev.Tools\" = [\"same\", { include-group = \"sAmE\" }, \"same\", { include-group = \"MISSING_Name\" }]\nsame = []\n";
        let map = SourceMap::parse(source)?;
        let group = GroupName::from_str("dev-tools")?;
        let ranges = [
            include_span(
                &map,
                &GroupInclude {
                    group: group.clone(),
                    index: 1,
                    included: GroupName::from_str("same")?,
                },
            ),
            include_span(
                &map,
                &GroupInclude {
                    group,
                    index: 3,
                    included: GroupName::from_str("missing-name")?,
                },
            ),
        ];
        let values = ranges
            .clone()
            .map(|range| range.and_then(|range| source.get(range)));
        let pyproject =
            PyProjectToml::from_string(source.to_string(), Path::new("pyproject.toml"))?;
        let groups = pyproject.dependency_groups.iter().flatten().collect();
        let failure =
            FlatDependencyGroups::from_dependency_groups(&groups, &BTreeMap::new(), false)
                .err()
                .context("the included group should be missing")?;
        insta::assert_debug_snapshot!((ranges, values, failure.provenance), @r#"
        (
            [
                Some(
                    69..75,
                ),
                Some(
                    105..119,
                ),
            ],
            [
                Some(
                    "\"sAmE\"",
                ),
                Some(
                    "\"MISSING_Name\"",
                ),
            ],
            Some(
                Includes(
                    [
                        GroupInclude {
                            group: GroupName(
                                "dev-tools",
                            ),
                            index: 3,
                            included: GroupName(
                                "missing-name",
                            ),
                        },
                    ],
                ),
            ),
        )
        "#);
        Ok(())
    }

    #[test]
    fn cycle_provenance_contains_only_cycle_edges() -> Result<()> {
        let pyproject = PyProjectToml::from_string(
            "[dependency-groups]\nfirst = [{ include-group = 'inner-a' }]\ninner-a = ['same', { include-group = 'inner-b' }]\ninner-b = ['same', 'same', { include-group = 'inner-a' }]\n".to_string(),
            Path::new("pyproject.toml"),
        )?;
        let groups = pyproject.dependency_groups.iter().flatten().collect();
        let failure =
            FlatDependencyGroups::from_dependency_groups(&groups, &BTreeMap::new(), false)
                .err()
                .context("the dependency groups should contain a cycle")?;

        insta::assert_debug_snapshot!(failure.provenance, @r#"
        Some(
            Includes(
                [
                    GroupInclude {
                        group: GroupName(
                            "inner-a",
                        ),
                        index: 1,
                        included: GroupName(
                            "inner-b",
                        ),
                    },
                    GroupInclude {
                        group: GroupName(
                            "inner-b",
                        ),
                        index: 2,
                        included: GroupName(
                            "inner-a",
                        ),
                    },
                ],
            ),
        )
        "#);
        Ok(())
    }

    #[test]
    fn include_spans_retain_occurrences_beside_multiline_values() -> Result<()> {
        let source = r#"[dependency-groups]
separate = [
    "demo @ https://example.com/demo-1.0.0-py3-none-any.whl",
    { include-group = "missing" },
]
same-line = [{ include-group = "missing" }, "demo \u0040 https\u003a//example.com/demo-1.0.0-py3-none-any.whl"]
continued = [
    """demo @ https://example.com/\
        demo-1.0.0-py3-none-any.whl""", { include-group = "missing" },
]
"#
        .replace('\n', "\r\n");
        let pyproject = PyProjectToml::from_string(source, Path::new("pyproject.toml"))?;
        let map = SourceMap::parse(&pyproject.raw)?;
        validate_dependency_groups_source(&map, &pyproject)
            .context("the semantic groups should match their source")?;
        let cases = [("separate", 1), ("same-line", 0), ("continued", 1)];
        let mut values = Vec::new();
        for (group, index) in cases {
            let range = include_span(
                &map,
                &GroupInclude {
                    group: GroupName::from_str(group)?,
                    index,
                    included: GroupName::from_str("missing")?,
                },
            )
            .context("the include should have a source span")?;
            values.push(pyproject.raw.get(range));
        }
        insta::assert_debug_snapshot!(values, @r#"
        [
            Some(
                "\"missing\"",
            ),
            Some(
                "\"missing\"",
            ),
            Some(
                "\"missing\"",
            ),
        ]
        "#);
        Ok(())
    }

    #[test]
    fn source_mapping_accepts_neighboring_values_and_comments() -> Result<()> {
        let source = r#"[dependency-groups]
registry = ["typing-extensions>=4", { include-group = "missing" }]
marker = ["safe; python_version >= '0' or python_version < '0'", { include-group = "missing" }]
comment-string = ["safe \u0023 explanatory note", { include-group = "missing" }]
unknown = [{ include-group = "missing" }, { note = "for another tool" }]
comment = [{ include-group = "missing" }] # needed for local tests
"#;
        let pyproject =
            PyProjectToml::from_string(source.to_string(), Path::new("pyproject.toml"))?;
        let map = SourceMap::parse(source)?;
        validate_dependency_groups_source(&map, &pyproject)
            .context("the semantic groups should match their source")?;
        let cases = [
            ("registry", 1),
            ("marker", 1),
            ("comment-string", 1),
            ("unknown", 0),
            ("comment", 0),
        ];
        let mut values = Vec::new();
        for (group, index) in cases {
            let range = include_span(
                &map,
                &GroupInclude {
                    group: GroupName::from_str(group)?,
                    index,
                    included: GroupName::from_str("missing")?,
                },
            )
            .context("the include should have a source span")?;
            values.push(source.get(range));
        }
        insta::assert_debug_snapshot!(values, @r#"
        [
            Some(
                "\"missing\"",
            ),
            Some(
                "\"missing\"",
            ),
            Some(
                "\"missing\"",
            ),
            Some(
                "\"missing\"",
            ),
            Some(
                "\"missing\"",
            ),
        ]
        "#);
        Ok(())
    }

    #[test]
    fn mismatched_source_is_location_only() -> Result<()> {
        let pyproject = PyProjectToml::from_string(
            "[dependency-groups]\nroot = [{ include-group = 'missing' }, 'safe']\n".to_string(),
            Path::new("pyproject.toml"),
        )?;
        let changed = "[dependency-groups]\nroot = [{ include-group = 'missing' }, 'other>=2']\n";
        let map = SourceMap::parse(changed)?;
        include_span(
            &map,
            &GroupInclude {
                group: GroupName::from_str("root")?,
                index: 0,
                included: GroupName::from_str("missing")?,
            },
        )
        .context("the include should have a source span")?;

        assert!(validate_dependency_groups_source(&map, &pyproject).is_none());
        Ok(())
    }

    #[test]
    fn mismatched_object_source_is_location_only() -> Result<()> {
        let pyproject = PyProjectToml::from_string(
            "[dependency-groups]\nroot = [{ include-group = 'missing' }, { note = 'first' }]\n"
                .to_string(),
            Path::new("pyproject.toml"),
        )?;
        for changed in [
            "[dependency-groups]\nroot = [{ include-group = 'missing' }, { note = 'second' }]\n",
            "[dependency-groups]\nroot = [{ include-group = 'missing' }, { other = 'first' }]\n",
            "[dependency-groups]\nroot = [{ include-group = 'missing' }, { note = 'first', other = 'second' }]\n",
        ] {
            let map = SourceMap::parse(changed)?;
            include_span(
                &map,
                &GroupInclude {
                    group: GroupName::from_str("root")?,
                    index: 0,
                    included: GroupName::from_str("missing")?,
                },
            )
            .context("the include should have a source span")?;
            assert!(validate_dependency_groups_source(&map, &pyproject).is_none());
        }
        Ok(())
    }

    #[test]
    fn group_python_bounds_use_exact_matching_values() -> Result<()> {
        let group = GroupName::from_str("dev-tools")?;
        let requires_python = VersionSpecifiers::from_str(">=3.12")?;
        let cases = [
            "[tool.uv.dependency-groups.\"Dev.Tools\"]\nrequires-python = '>=3.12'\n",
            "[tool.uv.dependency-groups]\n\"Dev.Tools\" = { \"requires\\u002dpython\" = \">=3.\\u0031\\u0032\" }\n",
            "[tool.uv.dependency-groups]\n\"Dev.Tools\" = { requires-python = '>=3.12', note = 'local tooling' }\n",
            "[tool.uv.dependency-groups]\n\"Dev.Tools\" = { requires-python = '>=3.12' } # local tooling\n",
            "[tool.uv.dependency-groups]\n\"Dev.Tools\" = { requires-python = '>=3.13' }\n",
            "[tool.uv.dependency-groups]\n\"Dev.Tools\" = { requires-python = '>=3.12' }\ndev_tools = { requires-python = '>=3.12' }\n",
        ];
        let mut locations = Vec::new();
        for source in cases {
            let map = SourceMap::parse(source)?;
            let span = requires_python_span(&map, &group, &requires_python);
            locations.push(span.and_then(|span| source.get(span)));
        }
        insta::assert_debug_snapshot!(locations, @r#"
        [
            Some(
                "'>=3.12'",
            ),
            Some(
                "\">=3.\\u0031\\u0032\"",
            ),
            Some(
                "'>=3.12'",
            ),
            Some(
                "'>=3.12'",
            ),
            None,
            None,
        ]
        "#);
        Ok(())
    }
}
