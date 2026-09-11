use std::error::Error;
use std::ops::Range;
use std::path::Path;
use std::str::FromStr;

use uv_errors::{Diagnostic, Info, SourceAnnotation, SourceFile, SourceSnippet};
use uv_fs::Simplified;
use uv_normalize::{DEV_DEPENDENCIES, GroupName};
use uv_pep508::{Requirement, VerbatimUrl, VersionOrUrl};
use uv_pypi_types::DependencyGroupSpecifier;
use uv_toml::SourcePathSegment::{Index, Key};
use uv_toml::{SourceMap, SourcePathSegment};

use crate::pyproject::PyProjectToml;

use super::{DependencyGroupError, DependencyGroupErrorInner};

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

/// An exact include occurrence encountered during semantic group traversal.
#[derive(Debug, Clone)]
pub(super) struct GroupInclude {
    pub(super) group: GroupName,
    pub(super) index: usize,
    pub(super) included: GroupName,
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
        let visibility = SourceVisibility::new(&map, pyproject);

        match error {
            DependencyGroupErrorInner::GroupNotFound(group, parent) => {
                let DependencyGroupProvenance::Includes(includes) = provenance else {
                    return None;
                };
                let last = includes.last()?;
                if &last.group != parent || &last.included != group {
                    return None;
                }
                Self::from_includes(&source, &map, &visibility, &includes, "undefined group")
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
                    &visibility,
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
                Self::from_includes(&source, &map, &visibility, &includes, "closes the cycle")
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
        visibility: &SourceVisibility,
        includes: &[GroupInclude],
        label: &'static str,
    ) -> Option<Self> {
        let (last, parents) = includes.split_last()?;
        let range = include_span(map, last)?;
        let primary = visibility.restrict(
            source,
            range.clone(),
            SourceSnippet::new(source.clone())
                .with_annotation(SourceAnnotation::primary(range).with_label(label)),
        );
        let related = parents
            .iter()
            .filter_map(|include| {
                let range = include_span(map, include)?;
                let snippet = visibility.restrict(
                    source,
                    range.clone(),
                    SourceSnippet::new(source.clone()).with_annotation(
                        SourceAnnotation::secondary(range).with_label("included here"),
                    ),
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
        // Other `tool.uv` fields can share an inline table with these settings. Their values are
        // outside the dependency-group model, so only the exact location is shown.
        let primary = SourceSnippet::new(source.clone())
            .with_annotation(SourceAnnotation::primary(range).with_label("undefined group"))
            .without_source_text();
        Some(Self {
            primary,
            related: Vec::new(),
        })
    }

    fn with_legacy_dev_location(&mut self, source: &SourceFile, map: &SourceMap<'_>) {
        if let Some(range) = map.key_span(&[Key("tool"), Key("uv")], "dev-dependencies") {
            self.related.push(RelatedLocation {
                message: "Legacy development dependencies are defined here".to_string(),
                snippet: SourceSnippet::new(source.clone())
                    .with_annotation(
                        SourceAnnotation::secondary(range)
                            .with_label("legacy development dependencies"),
                    )
                    .without_source_text(),
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

/// Source ranges that cannot be safely displayed alongside an include.
///
/// Group keys and include targets are validated names. Other entries are displayed only when the
/// decoded value is a registry requirement without arbitrary marker text.
struct SourceVisibility {
    /// An unavailable map means the semantic document and retained syntax could not be matched.
    private: Option<Vec<Range<usize>>>,
}

impl SourceVisibility {
    fn new(map: &SourceMap<'_>, pyproject: &PyProjectToml) -> Self {
        Self {
            private: private_requirement_spans(map, pyproject),
        }
    }

    fn restrict(
        &self,
        source: &SourceFile,
        range: Range<usize>,
        snippet: SourceSnippet<'static>,
    ) -> SourceSnippet<'static> {
        if self.can_show(source, range) {
            snippet
        } else {
            snippet.without_source_text()
        }
    }

    fn can_show(&self, source: &SourceFile, range: Range<usize>) -> bool {
        let Some(private) = &self.private else {
            return false;
        };
        let Some(window) = source.line_range_for_span(range) else {
            return false;
        };
        // A comment can contain arbitrary user text. Treat even a possible comment delimiter as
        // private instead of trying to redact or reinterpret it.
        if source
            .text()
            .get(window.clone())
            .is_none_or(|line| line.contains('#'))
        {
            return false;
        }
        !private
            .iter()
            .any(|private| private.start < window.end && window.start < private.end)
    }
}

/// Classify the exact decoded values, including entries not yet reached by semantic traversal.
/// Raw TOML can escape or split URL punctuation across lines, so source text is not a URL parser.
fn private_requirement_spans(
    map: &SourceMap<'_>,
    pyproject: &PyProjectToml,
) -> Option<Vec<Range<usize>>> {
    let Some(groups) = &pyproject.dependency_groups else {
        return map
            .span(&[Key("dependency-groups")])
            .is_none()
            .then(Vec::new);
    };
    if map.keys(&[Key("dependency-groups")])?.count() != groups.keys().count() {
        return None;
    }

    let mut private = Vec::new();
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
                    if !is_registry_requirement(decoded) {
                        private.push(map.span(&path)?);
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
                DependencyGroupSpecifier::Object(_) => private.push(map.span(&path)?),
            }
        }
    }
    Some(private)
}

fn is_registry_requirement(value: &str) -> bool {
    // Marker and comment suffixes are arbitrary user text, even when parsing discards them or
    // simplifies the marker to true.
    if value.contains(';') || value.contains('#') {
        return false;
    }
    let Ok(requirement) = Requirement::<VerbatimUrl>::from_str(value) else {
        return false;
    };
    match requirement.version_or_url {
        Some(VersionOrUrl::Url(_)) => false,
        Some(VersionOrUrl::VersionSpecifier(_)) | None => true,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::str::FromStr;

    use anyhow::{Context, Result};
    use uv_errors::SourceFile;
    use uv_normalize::GroupName;
    use uv_toml::SourceMap;

    use crate::dependency_groups::FlatDependencyGroups;
    use crate::pyproject::PyProjectToml;

    use super::{GroupInclude, SourceVisibility, include_span};

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
        let failure = FlatDependencyGroups::from_dependency_groups(&groups, &BTreeMap::new())
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
        let failure = FlatDependencyGroups::from_dependency_groups(&groups, &BTreeMap::new())
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
    fn visibility_uses_decoded_requirements_and_rendered_windows() -> Result<()> {
        let source = r#"[dependency-groups]
safe = [
    "private @ https://user:password@example.com/private-1.0.0-py3-none-any.whl",
    { include-group = "missing" },
]
same-line = [{ include-group = "missing" }, "private \u0040 https\u003a//user:password@example.com/private-1.0.0-py3-none-any.whl"]
continued = [
    """private @ https://example.com/private-1.0.0-py3-none-any.whl?token=\
        sentinel-secret""", { include-group = "missing" },
]
"#
        .replace('\n', "\r\n");
        let pyproject = PyProjectToml::from_string(source, Path::new("pyproject.toml"))?;
        let map = SourceMap::parse(&pyproject.raw)?;
        let source = SourceFile::new("pyproject.toml", pyproject.raw.as_str());
        let visibility = SourceVisibility::new(&map, &pyproject);
        let cases = [("safe", 1), ("same-line", 0), ("continued", 1)];
        let mut visible = Vec::new();
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
            visible.push(visibility.can_show(&source, range));
        }
        insta::assert_debug_snapshot!(visible, @"
        [
            true,
            false,
            false,
        ]
        ");
        Ok(())
    }

    #[test]
    fn visibility_rejects_unproven_values() -> Result<()> {
        let source = r#"[dependency-groups]
registry = ["typing-extensions>=4", { include-group = "missing" }]
marker = ["safe; python_version >= '0' or python_version < '0'", { include-group = "missing" }]
comment-string = ["safe \u0023 sentinel-secret", { include-group = "missing" }]
unknown = [{ include-group = "missing" }, { private = "sentinel-secret" }]
comment = [{ include-group = "missing" }] # sentinel-secret
"#;
        let pyproject =
            PyProjectToml::from_string(source.to_string(), Path::new("pyproject.toml"))?;
        let map = SourceMap::parse(source)?;
        let source = SourceFile::new("pyproject.toml", source);
        let visibility = SourceVisibility::new(&map, &pyproject);
        let cases = [
            ("registry", 1),
            ("marker", 1),
            ("comment-string", 1),
            ("unknown", 0),
            ("comment", 0),
        ];
        let mut visible = Vec::new();
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
            visible.push(visibility.can_show(&source, range));
        }
        insta::assert_debug_snapshot!(visible, @"
        [
            true,
            false,
            false,
            false,
            false,
        ]
        ");
        Ok(())
    }

    #[test]
    fn mismatched_source_is_location_only() -> Result<()> {
        let pyproject = PyProjectToml::from_string(
            "[dependency-groups]\nroot = [{ include-group = 'missing' }, 'safe']\n".to_string(),
            Path::new("pyproject.toml"),
        )?;
        let changed = "[dependency-groups]\nroot = [{ include-group = 'missing' }, 'private @ https://user:password@example.com/private.whl']\n";
        let map = SourceMap::parse(changed)?;
        let source = SourceFile::new("pyproject.toml", changed);
        let range = include_span(
            &map,
            &GroupInclude {
                group: GroupName::from_str("root")?,
                index: 0,
                included: GroupName::from_str("missing")?,
            },
        )
        .context("the include should have a source span")?;

        insta::assert_debug_snapshot!(SourceVisibility::new(&map, &pyproject).can_show(&source, range), @"false");
        Ok(())
    }
}
