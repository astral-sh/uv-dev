use std::borrow::Cow;

use owo_colors::OwoColorize;

use uv_errors::{Hint, Info};

use super::{ExcludeNewerVersionDetail, PackageRange, PubGrubHint};
use crate::exclude_newer::EffectiveExcludeNewerSource;
use crate::pubgrub::PubGrubPackage;
use crate::python_requirement::PythonRequirementSource;

impl PubGrubHint {
    /// Explain the resolver decision separately from any suggested action.
    pub(crate) fn diagnostic_info(&self) -> Info<'static> {
        match self {
            Self::PrereleaseAvailable {
                package, version, ..
            } => Info::new(format!(
                "Pre-releases are available for `{}` in the requested range (e.g., {}), but pre-releases weren't enabled",
                package.cyan(),
                version.cyan(),
            )),
            Self::BuildPrereleaseAvailable { package, version } => Info::new(format!(
                "Only pre-releases of `{}` (e.g., {}) match these build requirements, and build environments can't enable pre-releases automatically",
                package.cyan(),
                version.cyan(),
            )),
            Self::PrereleaseRequested { name, range, .. } => Info::new(format!(
                "`{}` was requested with a pre-release marker (e.g., {}), but pre-releases weren't enabled",
                name.cyan(),
                PackageRange::compatibility(&PubGrubPackage::base(name.clone()), range, None)
                    .cyan(),
            )),
            Self::BuildPrereleaseRequested { name, range } => Info::new(format!(
                "`{}` was requested with a pre-release marker (e.g., {}), but build environments can't opt into pre-releases automatically",
                name.cyan(),
                PackageRange::compatibility(&PubGrubPackage::base(name.clone()), range, None)
                    .cyan(),
            )),
            Self::NoIndex => Info::new(
                "Packages were unavailable because index lookups were disabled and no additional package locations were provided",
            ),
            Self::InvalidPackageMetadata { package, reason } => Info::new(format!(
                "Metadata for `{}` could not be parsed",
                package.cyan(),
            ))
            .with_details(reason.to_string()),
            Self::InvalidPackageStructure { package, reason } => {
                Info::new(format!("The structure of `{}` was invalid", package.cyan()))
                    .with_details(reason.to_string())
            }
            Self::InvalidVersionMetadata {
                package,
                version,
                reason,
            } => Info::new(format!(
                "Metadata for `{}` ({}) could not be parsed",
                package.cyan(),
                format!("v{version}").cyan(),
            ))
            .with_details(reason.clone()),
            Self::InvalidVersionStructure {
                package,
                version,
                reason,
            } => Info::new(format!(
                "The structure of `{}` ({}) was invalid",
                package.cyan(),
                format!("v{version}").cyan(),
            ))
            .with_details(reason.clone()),
            Self::InconsistentVersionMetadata {
                package,
                version,
                reason,
            } => Info::new(format!(
                "Metadata for `{}` ({}) was inconsistent",
                package.cyan(),
                format!("v{version}").cyan(),
            ))
            .with_details(reason.clone()),
            Self::RequiresPython {
                source,
                requires_python,
                name,
                package_set,
                package_requires_python,
            } => {
                let package = PubGrubPackage::base(name.clone());
                let package_range = PackageRange::compatibility(&package, package_set, None);
                let supports = if package_range.plural() {
                    "support"
                } else {
                    "supports"
                };
                let reason = match source {
                    PythonRequirementSource::RequiresPython => format!(
                        "The `requires-python` value ({}) includes Python versions that are not supported by your dependencies (e.g., {} only {} {})",
                        requires_python.cyan(),
                        package_range.cyan(),
                        supports,
                        package_requires_python.cyan(),
                    ),
                    PythonRequirementSource::PythonVersion => format!(
                        "The `--python-version` value ({}) includes Python versions that are not supported by your dependencies (e.g., {} only {} {})",
                        requires_python.cyan(),
                        package_range.cyan(),
                        supports,
                        package_requires_python.cyan(),
                    ),
                    PythonRequirementSource::Interpreter => format!(
                        "The Python interpreter uses a Python version that is not supported by your dependencies (e.g., {} only {} {})",
                        package_range.cyan(),
                        supports,
                        package_requires_python.cyan(),
                    ),
                };
                Info::new(reason)
            }
            Self::DependsOnWorkspacePackage {
                package,
                dependency,
                workspace,
            } => {
                let owner = if *workspace {
                    "one of your workspace members"
                } else {
                    "your project"
                };
                Info::new(format!(
                    "The package `{}` depends on the package `{}` but the name is shadowed by {owner}",
                    package.cyan(),
                    dependency.cyan(),
                ))
            }
            Self::DependsOnItself { package, workspace } => {
                let owner = if *workspace {
                    "workspace member"
                } else {
                    "project"
                };
                Info::new(format!(
                    "The {owner} `{}` depends on itself at an incompatible version. This is likely a mistake",
                    package.cyan(),
                ))
            }
            Self::UncheckedIndex {
                name,
                range,
                found_index,
                next_index,
            } => Info::new(format!(
                "`{}` was found on {}, but not at the requested version ({}). A compatible version may be available on a subsequent index (e.g., {}). By default, uv only considers versions published on the first index that contains a package, to avoid dependency confusion attacks",
                name.cyan(),
                found_index.without_credentials().cyan(),
                PackageRange::compatibility(&PubGrubPackage::base(name.clone()), range, None)
                    .cyan(),
                next_index.without_credentials().cyan(),
            )),
            Self::ForbiddenIndex {
                index,
                any_successful_response,
            } => Info::new(if *any_successful_response {
                format!(
                    "An index ({}) returned a {} error, but uv received a successful response from another request to the index",
                    index.without_credentials().cyan(),
                    "403 Forbidden".red(),
                )
            } else {
                format!(
                    "An index ({}) returned a {} error",
                    index.without_credentials().cyan(),
                    "403 Forbidden".red(),
                )
            }),
            Self::ExcludeNewer {
                package,
                source,
                exclude_newer,
                matching_version,
            } => {
                let setting = match source {
                    EffectiveExcludeNewerSource::Package => {
                        format!("`{}`", "exclude-newer-package".green())
                    }
                    EffectiveExcludeNewerSource::Global => {
                        format!("`{}`", "exclude-newer".green())
                    }
                    EffectiveExcludeNewerSource::Index => {
                        format!("the index-specific `{}` setting", "exclude-newer".green())
                    }
                };
                let latest = matching_version
                    .as_ref()
                    .map_or_else(String::new, ExcludeNewerVersionDetail::summary);
                Info::new(format!(
                    "`{}` was filtered by {setting} to only include packages uploaded before {}.{latest}",
                    package.cyan(),
                    exclude_newer.cyan(),
                ))
            }
            Self::DisjointPythonVersion { python_version } => Info::new(format!(
                "While the active Python version is {}, the resolution failed for other Python versions supported by your project",
                python_version.cyan(),
            )),
            Self::DisjointEnvironment => {
                Info::new("The resolution failed for an environment that is not the current one")
            }
            Self::Offline
            | Self::InvalidPackageNetwork { .. }
            | Self::InvalidVersionNetwork { .. }
            | Self::IncompatibleBuildRequirement { .. }
            | Self::UnauthorizedIndex { .. }
            | Self::NoBuild { .. }
            | Self::NoBinary { .. }
            | Self::LanguageTags { .. }
            | Self::AbiTags { .. }
            | Self::PlatformTags { .. } => Info::new(self.to_string()),
        }
    }

    /// Return an instruction the user can follow to change the resolution.
    pub(crate) fn actionable_hint(&self) -> Option<Hint<'static>> {
        let message: Cow<'static, str> = match self {
            Self::PrereleaseAvailable {
                package,
                package_override,
                ..
            }
            | Self::PrereleaseRequested {
                name: package,
                package_override,
                ..
            } => {
                let argument = if *package_override {
                    format!("--prerelease-package {package}=allow")
                } else {
                    "--prerelease=allow".to_string()
                };
                format!("Use `{}` to allow pre-releases", argument.green()).into()
            }
            Self::BuildPrereleaseAvailable { package, version } => format!(
                "Add `{}` to `build-system.requires`, `[tool.uv.extra-build-dependencies]`, or supply it via `uv build --build-constraint`",
                format!("{package}>={version}").cyan(),
            )
            .into(),
            Self::BuildPrereleaseRequested { name, range } => format!(
                "Add `{}` to `build-system.requires`, `[tool.uv.extra-build-dependencies]`, or supply it via `uv build --build-constraint`",
                PackageRange::compatibility(&PubGrubPackage::base(name.clone()), range, None)
                    .cyan(),
            )
            .into(),
            Self::NoIndex => format!(
                "Provide additional package locations with `{}`",
                "--find-links <uri>".green(),
            )
            .into(),
            Self::RequiresPython {
                source,
                package_requires_python,
                ..
            } => match source {
                PythonRequirementSource::RequiresPython => format!(
                    "Use a more restrictive `requires-python` value, such as `{}`",
                    package_requires_python.cyan(),
                )
                .into(),
                PythonRequirementSource::PythonVersion => {
                    "Use a higher `--python-version` value".into()
                }
                PythonRequirementSource::Interpreter => {
                    "Pass a `--python-version` value to raise the minimum supported version".into()
                }
            },
            Self::DependsOnWorkspacePackage {
                dependency,
                workspace,
                ..
            } => {
                let owner = if *workspace {
                    "workspace member"
                } else {
                    "project"
                };
                format!(
                    "Rename the {owner} `{}` to avoid shadowing the third-party package",
                    dependency.cyan(),
                )
                .into()
            }
            Self::DependsOnItself { package, workspace } => {
                let owner = if *workspace {
                    "workspace member"
                } else {
                    "project"
                };
                format!(
                    "If you intended to depend on a third-party package named `{}`, rename the {owner} `{}` to avoid creating a conflict",
                    package.cyan(),
                    package.cyan(),
                )
                .into()
            }
            Self::UncheckedIndex { .. } => format!(
                "If all indexes are equally trusted, use `{}` to consider all versions from all indexes",
                "--index-strategy unsafe-best-match".green(),
            )
            .into(),
            Self::ForbiddenIndex {
                index,
                any_successful_response,
            } => {
                if *any_successful_response {
                    format!(
                        "If the failing package is not present on {}, add `ignore-error-codes = [403]` to the index's `[[tool.uv.index]]` entry to continue searching across indexes",
                        index.without_credentials().cyan(),
                    )
                    .into()
                } else {
                    format!(
                        "Check that the index URL ({}) is correct and the credentials are valid",
                        index.without_credentials().cyan(),
                    )
                    .into()
                }
            }
            Self::ExcludeNewer {
                package, source, ..
            } => match source {
                EffectiveExcludeNewerSource::Package => format!(
                    "Remove `{}` for `{}` or update it to a later date",
                    "exclude-newer-package".green(),
                    package.cyan(),
                )
                .into(),
                EffectiveExcludeNewerSource::Global => format!(
                    "Use `{}` to override the cutoff for `{}`",
                    "exclude-newer-package".green(),
                    package.cyan(),
                )
                .into(),
                EffectiveExcludeNewerSource::Index => format!(
                    "Update the index's `exclude-newer` cutoff, set it to `false`, or use `{}` to override the cutoff for `{}`",
                    "exclude-newer-package".green(),
                    package.cyan(),
                )
                .into(),
            },
            Self::DisjointPythonVersion { .. } => {
                "Limit your project's supported Python versions using `requires-python`".into()
            }
            Self::DisjointEnvironment => {
                "Limit the environments with `tool.uv.environments`".into()
            }
            Self::Offline
            | Self::InvalidPackageMetadata { .. }
            | Self::InvalidPackageStructure { .. }
            | Self::InvalidPackageNetwork { .. }
            | Self::InvalidVersionMetadata { .. }
            | Self::InconsistentVersionMetadata { .. }
            | Self::InvalidVersionStructure { .. }
            | Self::IncompatibleBuildRequirement { .. }
            | Self::InvalidVersionNetwork { .. }
            | Self::UnauthorizedIndex { .. }
            | Self::NoBuild { .. }
            | Self::NoBinary { .. }
            | Self::LanguageTags { .. }
            | Self::AbiTags { .. }
            | Self::PlatformTags { .. } => return None,
        };
        Some(Hint::new(message))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::ops::Bound;

    use reqwest::StatusCode;

    use uv_configuration::{NoBinary, NoBuild};
    use uv_distribution_types::{IndexUrl, RequiresPython};
    use uv_errors::{Diagnostic, ErrorOptions, Hints, write_error_chain_with_options};
    use uv_normalize::PackageName;
    use uv_pep440::Version;
    use uv_warnings::anstream;

    use super::*;
    use crate::pubgrub::Range;

    #[derive(Debug, thiserror::Error)]
    #[error("No compatible packages")]
    struct Rejection(PubGrubHint);

    fn package() -> PackageName {
        "example".parse().expect("valid package name")
    }

    fn version(value: &str) -> Version {
        value.parse().expect("valid version")
    }

    fn index() -> IndexUrl {
        "https://user:secret@example.com/simple"
            .parse()
            .expect("valid index URL")
    }

    fn requires_python(source: PythonRequirementSource) -> PubGrubHint {
        PubGrubHint::RequiresPython {
            source,
            requires_python: RequiresPython::from_specifiers(
                ">=3.8".parse().expect("valid Python specifier"),
            ),
            name: package(),
            package_set: Range::singleton(version("1.0.0")),
            package_requires_python: Range::from_range_bounds((
                Bound::Included(version("3.9")),
                Bound::Unbounded,
            )),
        }
    }

    fn exclude_newer(source: EffectiveExcludeNewerSource) -> PubGrubHint {
        PubGrubHint::ExcludeNewer {
            package: package(),
            source,
            exclude_newer: "2025-01-01T00:00:00Z".parse().expect("valid timestamp"),
            matching_version: Some(ExcludeNewerVersionDetail {
                version: version("2.0.0"),
                publish_date: Some("2025-01-02T00:00:00Z".to_string()),
                singleton: true,
            }),
        }
    }

    fn render(hint: PubGrubHint) -> String {
        let hints: Hints<'_> = hint.actionable_hint().into_iter().collect();
        let mut output = String::new();
        write_error_chain_with_options(
            &Rejection(hint),
            &hints,
            ErrorOptions::default()
                .with_width_override(1000)
                .with_diagnostic(|error| {
                    error
                        .downcast_ref::<Rejection>()
                        .map(|error| Diagnostic::default().with_info(error.0.diagnostic_info()))
                })
                .with_stream(&mut output),
        )
        .expect("writing to a string cannot fail");
        anstream::adapter::strip_str(&output).to_string()
    }

    #[test]
    fn rejection_details_are_informational() {
        let hints = [
            PubGrubHint::Offline,
            PubGrubHint::InvalidVersionMetadata {
                package: package(),
                version: version("1.0.0"),
                reason: "Missing required `Name` field\nThe metadata is incomplete".to_string(),
            },
            PubGrubHint::InvalidVersionNetwork {
                package: package(),
                version: version("1.0.0"),
                status: StatusCode::UNAUTHORIZED,
            },
            PubGrubHint::IncompatibleBuildRequirement {
                package: package(),
                version: version("1.0.0"),
                requires_python: ">=3.12".parse().expect("valid Python specifier"),
                python_version: version("3.11"),
            },
            PubGrubHint::UnauthorizedIndex { index: index() },
            PubGrubHint::NoBuild {
                package: package(),
                option: NoBuild::All,
            },
            PubGrubHint::NoBinary {
                package: package(),
                option: NoBinary::All,
            },
            PubGrubHint::LanguageTags {
                package: package(),
                version: version("1.0.0"),
                tags: BTreeSet::from(["cp310".parse().expect("valid language tag")]),
                best: Some("cp312".parse().expect("valid language tag")),
            },
            PubGrubHint::AbiTags {
                package: package(),
                version: version("1.0.0"),
                tags: BTreeSet::from(["cp310".parse().expect("valid ABI tag")]),
                best: None,
            },
            PubGrubHint::PlatformTags {
                package: package(),
                version: version("1.0.0"),
                tags: BTreeSet::from(["win_amd64".parse().expect("valid platform tag")]),
            },
        ];
        for hint in &hints {
            assert!(hint.actionable_hint().is_none());
        }
        insta::assert_snapshot!(hints.into_iter().map(render).collect::<Vec<_>>().join("\n"), @"
        error: No compatible packages
          info: Packages were unavailable because the network was disabled. When the network is disabled, registry packages may only be read from the cache.

        error: No compatible packages
          info: Metadata for `example` (v1.0.0) could not be parsed
            |
            | Missing required `Name` field
            | The metadata is incomplete

        error: No compatible packages
          info: Metadata for `example` (v1.0.0) could not be fetched; the server returned: `401 Unauthorized`

        error: No compatible packages
          info: The source distribution for `example` (v1.0.0) does not include static metadata. Generating metadata for this package requires Python >=3.12, but Python 3.11 is installed.

        error: No compatible packages
          info: An index URL (https://example.com/simple) could not be queried due to a lack of valid authentication credentials (401 Unauthorized)

        error: No compatible packages
          info: Wheels are required for `example` because building from source is disabled for all packages (i.e., with `--no-build`)

        error: No compatible packages
          info: A source distribution is required for `example` because using pre-built wheels is disabled for all packages (i.e., with `--no-binary`)

        error: No compatible packages
          info: You require CPython 3.12 (`cp312`), but we only found wheels for `example` (v1.0.0) with the following Python implementation tag: `cp310`

        error: No compatible packages
          info: Wheels are available for `example` (v1.0.0) with the following Python ABI tag: `cp310`

        error: No compatible packages
          info: Wheels are available for `example` (v1.0.0) on the following platform: `win_amd64`
        ");
    }

    #[test]
    fn mixed_rejections_keep_context_and_action_separate() {
        let hints = [
            PubGrubHint::PrereleaseAvailable {
                package: package(),
                version: version("2.0.0a1"),
                package_override: false,
            },
            requires_python(PythonRequirementSource::RequiresPython),
            PubGrubHint::UncheckedIndex {
                name: package(),
                range: Range::singleton(version("2.0.0")),
                found_index: index(),
                next_index: "https://other:private@secondary.example.com/simple"
                    .parse()
                    .expect("valid index URL"),
            },
            PubGrubHint::ForbiddenIndex {
                index: index(),
                any_successful_response: true,
            },
            exclude_newer(EffectiveExcludeNewerSource::Index),
        ];
        insta::assert_snapshot!(hints.into_iter().map(render).collect::<Vec<_>>().join("\n"), @"
        error: No compatible packages
          info: Pre-releases are available for `example` in the requested range (e.g., 2.0.0a1), but pre-releases weren't enabled

        hint: Use `--prerelease=allow` to allow pre-releases

        error: No compatible packages
          info: The `requires-python` value (>=3.8) includes Python versions that are not supported by your dependencies (e.g., example==1.0.0 only supports >=3.9)

        hint: Use a more restrictive `requires-python` value, such as `>=3.9`

        error: No compatible packages
          info: `example` was found on https://example.com/simple, but not at the requested version (example==2.0.0). A compatible version may be available on a subsequent index (e.g., https://secondary.example.com/simple). By default, uv only considers versions published on the first index that contains a package, to avoid dependency confusion attacks

        hint: If all indexes are equally trusted, use `--index-strategy unsafe-best-match` to consider all versions from all indexes

        error: No compatible packages
          info: An index (https://example.com/simple) returned a 403 Forbidden error, but uv received a successful response from another request to the index

        hint: If the failing package is not present on https://example.com/simple, add `ignore-error-codes = [403]` to the index's `[[tool.uv.index]]` entry to continue searching across indexes

        error: No compatible packages
          info: `example` was filtered by the index-specific `exclude-newer` setting to only include packages uploaded before 2025-01-01T00:00:00Z. The requested version, v2.0.0, was published at 2025-01-02T00:00:00Z.

        hint: Update the index's `exclude-newer` cutoff, set it to `false`, or use `exclude-newer-package` to override the cutoff for `example`
        ");
    }

    #[test]
    fn actionable_rejection_variants_provide_instructions() {
        let hints = [
            PubGrubHint::PrereleaseAvailable {
                package: package(),
                version: version("2.0.0a1"),
                package_override: false,
            },
            PubGrubHint::PrereleaseRequested {
                name: package(),
                range: Range::singleton(version("2.0.0a1")),
                package_override: true,
            },
            PubGrubHint::BuildPrereleaseAvailable {
                package: package(),
                version: version("2.0.0a1"),
            },
            PubGrubHint::BuildPrereleaseRequested {
                name: package(),
                range: Range::singleton(version("2.0.0a1")),
            },
            PubGrubHint::NoIndex,
            requires_python(PythonRequirementSource::RequiresPython),
            requires_python(PythonRequirementSource::PythonVersion),
            requires_python(PythonRequirementSource::Interpreter),
            PubGrubHint::DependsOnWorkspacePackage {
                package: "parent".parse().expect("valid package name"),
                dependency: package(),
                workspace: true,
            },
            PubGrubHint::DependsOnItself {
                package: package(),
                workspace: false,
            },
            PubGrubHint::UncheckedIndex {
                name: package(),
                range: Range::singleton(version("2.0.0")),
                found_index: index(),
                next_index: "https://secondary.example.com/simple"
                    .parse()
                    .expect("valid index URL"),
            },
            PubGrubHint::ForbiddenIndex {
                index: index(),
                any_successful_response: true,
            },
            PubGrubHint::ForbiddenIndex {
                index: index(),
                any_successful_response: false,
            },
            exclude_newer(EffectiveExcludeNewerSource::Package),
            exclude_newer(EffectiveExcludeNewerSource::Global),
            exclude_newer(EffectiveExcludeNewerSource::Index),
            PubGrubHint::DisjointPythonVersion {
                python_version: version("3.12"),
            },
            PubGrubHint::DisjointEnvironment,
        ];
        let hints = hints
            .iter()
            .map(|hint| hint.actionable_hint().expect("actionable rejection"))
            .collect::<Hints<'_>>()
            .to_string();
        insta::assert_snapshot!(anstream::adapter::strip_str(&hints), @"

        hint: Use `--prerelease=allow` to allow pre-releases
        hint: Use `--prerelease-package example=allow` to allow pre-releases
        hint: Add `example>=2.0.0a1` to `build-system.requires`, `[tool.uv.extra-build-dependencies]`, or supply it via `uv build --build-constraint`
        hint: Add `example==2.0.0a1` to `build-system.requires`, `[tool.uv.extra-build-dependencies]`, or supply it via `uv build --build-constraint`
        hint: Provide additional package locations with `--find-links <uri>`
        hint: Use a more restrictive `requires-python` value, such as `>=3.9`
        hint: Use a higher `--python-version` value
        hint: Pass a `--python-version` value to raise the minimum supported version
        hint: Rename the workspace member `example` to avoid shadowing the third-party package
        hint: If you intended to depend on a third-party package named `example`, rename the project `example` to avoid creating a conflict
        hint: If all indexes are equally trusted, use `--index-strategy unsafe-best-match` to consider all versions from all indexes
        hint: If the failing package is not present on https://example.com/simple, add `ignore-error-codes = [403]` to the index's `[[tool.uv.index]]` entry to continue searching across indexes
        hint: Check that the index URL (https://example.com/simple) is correct and the credentials are valid
        hint: Remove `exclude-newer-package` for `example` or update it to a later date
        hint: Use `exclude-newer-package` to override the cutoff for `example`
        hint: Update the index's `exclude-newer` cutoff, set it to `false`, or use `exclude-newer-package` to override the cutoff for `example`
        hint: Limit your project's supported Python versions using `requires-python`
        hint: Limit the environments with `tool.uv.environments`
        ");
    }

    #[test]
    fn combined_display_retains_rejection_context() {
        let hint = exclude_newer(EffectiveExcludeNewerSource::Index);
        insta::assert_snapshot!(anstream::adapter::strip_str(&hint.to_string()), @"`example` was filtered by the index-specific `exclude-newer` setting to only include packages uploaded before 2025-01-01T00:00:00Z. The requested version, v2.0.0, was published at 2025-01-02T00:00:00Z. Consider updating that index's cutoff, setting it to `false`, or using `exclude-newer-package` to override the cutoff for this package.");
    }
}
