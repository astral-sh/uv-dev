use std::error::Error;
use std::str::FromStr;
use std::sync::LazyLock;

use owo_colors::OwoColorize;
use rustc_hash::FxHashMap;
use version_ranges::Ranges;

use uv_distribution_types::{DerivationChain, DerivationStep};
use uv_errors::{Diagnostic, Hints, Info};
use uv_normalize::PackageName;
use uv_pep440::{Version, strip_local_version_sentinels};

use crate::printer::Printer;

use self::registry::metadata_for_error;

mod registry;

static SUGGESTIONS: LazyLock<FxHashMap<PackageName, PackageName>> = LazyLock::new(|| {
    let suggestions: Vec<(String, String)> =
        serde_json::from_str(include_str!("suggestions.json")).unwrap();
    suggestions
        .iter()
        .map(|(k, v)| {
            (
                PackageName::from_str(k).unwrap(),
                PackageName::from_str(v).unwrap(),
            )
        })
        .collect()
});

/// Format an error chain with the default user-facing hints and output settings.
pub(crate) fn write_error_chain(err: &anyhow::Error, printer: Printer) -> std::fmt::Result {
    uv_errors::write_error_chain_with_options(
        err.as_ref(),
        &Hints::none(),
        uv_errors::ErrorOptions::default()
            .with_format(printer.error_format())
            .with_diagnostic(diagnostic_for_error)
            .with_stream(printer.stderr_important()),
    )
}

/// Resolve presentation data for one concrete error, without changing its source chain.
pub(super) fn diagnostic_for_error<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
    // Transparent wrappers display their inner root without exposing it as a separate source.
    // Resolve that root's presentation and hints at this same visible node.
    let mut presentation: Option<Diagnostic<'a>> = None;
    let mut owners = Vec::new();
    let mut current = Some(error);
    while let Some(error) = current {
        let metadata = metadata_for_error(error);
        // A smart pointer has no presentation of its own. Some native providers also accept
        // boxed errors, so resolving both nodes would duplicate the inner error's context.
        if !metadata.is_smart_pointer
            && let Some(native) = uv_publish::diagnostic_for_error(error)
                .or_else(|| uv_requirements_txt::diagnostic_for_error(error))
                .or_else(|| uv_scripts::diagnostic_for_error(error))
                .or_else(|| uv_distribution::diagnostic_for_error(error))
                .or_else(|| uv_resolver::diagnostic_for_error(error))
                .or_else(|| uv_settings::diagnostic_for_error(error))
                .or_else(|| uv_workspace::pyproject::diagnostic_for_error(error))
                .or_else(|| uv_workspace::dependency_groups::diagnostic_for_error(error))
                .or_else(|| uv_workspace::diagnostic_for_error(error))
                .or_else(|| crate::commands::project::diagnostics::diagnostic_for_error(error))
        {
            presentation = Some(match presentation {
                Some(presentation) => native.with_presentation_override(presentation),
                None => native,
            });
        }
        current = metadata.transparent;
        owners.push(metadata);
    }

    // Native inner context precedes any additions owned by a transparent wrapper.
    // This collection is independent of which presentation override takes precedence.
    let mut info = Vec::new();
    let mut hints = Hints::none();
    for owner in owners.into_iter().rev() {
        info.extend(owner.info);
        hints.extend(owner.hints);
    }
    if hints.is_empty() && info.is_empty() {
        presentation
    } else {
        Some(
            info.into_iter()
                .fold(presentation.unwrap_or_default(), Diagnostic::with_info)
                .with_hints(hints),
        )
    }
}

/// Suggest corrected package names and retain actionable suggestions from a distribution error.
pub(crate) fn dist_hints(name: &PackageName, cause_hints: Hints<'_>) -> Hints<'static> {
    let mut hints = Hints::none();
    if let Some(suggestion) = SUGGESTIONS.get(name) {
        hints.push(format!(
            "`{}` is often confused for `{}`. Did you mean to install `{}` instead?",
            name.cyan(),
            suggestion.cyan(),
            suggestion.cyan(),
        ));
    }
    hints.extend(cause_hints);
    hints.into_owned()
}

/// Explain why a distribution was included in the resolution.
fn dist_info(
    name: &PackageName,
    version: Option<&Version>,
    chain: &DerivationChain,
) -> Option<Info<'static>> {
    (!chain.is_empty()).then(|| Info::new(format_chain(name, version, chain)))
}

/// Format a [`DerivationChain`] as a human-readable error message.
fn format_chain(name: &PackageName, version: Option<&Version>, chain: &DerivationChain) -> String {
    /// Format a step in the [`DerivationChain`] as a human-readable error message.
    fn format_step(step: &DerivationStep, range: Option<Ranges<Version>>) -> String {
        if let Some(range) =
            range.filter(|range| *range != Ranges::empty() && *range != Ranges::full())
        {
            if let Some(extra) = &step.extra {
                if let Some(version) = step.version.as_ref() {
                    // Ex) `flask[dotenv]>=1.0.0` (v1.2.3)
                    format!(
                        "`{}{}` ({})",
                        format!("{}[{}]", step.name, extra).cyan(),
                        range.cyan(),
                        format!("v{version}").cyan(),
                    )
                } else {
                    // Ex) `flask[dotenv]>=1.0.0`
                    format!(
                        "`{}{}`",
                        format!("{}[{}]", step.name, extra).cyan(),
                        range.cyan(),
                    )
                }
            } else if let Some(group) = &step.group {
                if let Some(version) = step.version.as_ref() {
                    // Ex) `flask:dev>=1.0.0` (v1.2.3)
                    format!(
                        "`{}{}` ({})",
                        format!("{}:{}", step.name, group).cyan(),
                        range.cyan(),
                        format!("v{version}").cyan(),
                    )
                } else {
                    // Ex) `flask:dev>=1.0.0`
                    format!(
                        "`{}{}`",
                        format!("{}:{}", step.name, group).cyan(),
                        range.cyan(),
                    )
                }
            } else {
                if let Some(version) = step.version.as_ref() {
                    // Ex) `flask>=1.0.0` (v1.2.3)
                    format!(
                        "`{}{}` ({})",
                        step.name.cyan(),
                        range.cyan(),
                        format!("v{version}").cyan(),
                    )
                } else {
                    // Ex) `flask>=1.0.0`
                    format!("`{}{}`", step.name.cyan(), range.cyan())
                }
            }
        } else {
            if let Some(extra) = &step.extra {
                if let Some(version) = step.version.as_ref() {
                    // Ex) `flask[dotenv]` (v1.2.3)
                    format!(
                        "`{}` ({})",
                        format!("{}[{}]", step.name, extra).cyan(),
                        format!("v{version}").cyan(),
                    )
                } else {
                    // Ex) `flask[dotenv]`
                    format!("`{}`", format!("{}[{}]", step.name, extra).cyan())
                }
            } else if let Some(group) = &step.group {
                if let Some(version) = step.version.as_ref() {
                    // Ex) `flask:dev` (v1.2.3)
                    format!(
                        "`{}` ({})",
                        format!("{}:{}", step.name, group).cyan(),
                        format!("v{version}").cyan(),
                    )
                } else {
                    // Ex) `flask:dev`
                    format!("`{}`", format!("{}:{}", step.name, group).cyan())
                }
            } else {
                if let Some(version) = step.version.as_ref() {
                    // Ex) `flask` (v1.2.3)
                    format!("`{}` ({})", step.name.cyan(), format!("v{version}").cyan())
                } else {
                    // Ex) `flask`
                    format!("`{}`", step.name.cyan())
                }
            }
        }
    }

    let mut message = if let Some(version) = version {
        format!(
            "`{}` ({}) was included because",
            name.cyan(),
            format!("v{version}").cyan()
        )
    } else {
        format!("`{}` was included because", name.cyan())
    };
    let mut range: Option<Ranges<Version>> = None;
    for (i, step) in chain.iter().enumerate() {
        if i > 0 {
            message = format!("{message} {} which depends on", format_step(step, range));
        } else {
            message = format!("{message} {} depends on", format_step(step, range));
        }
        range = Some(strip_local_version_sentinels(&step.range));
    }
    if let Some(range) = range.filter(|range| *range != Ranges::empty() && *range != Ranges::full())
    {
        message = format!("{message} `{}{}`", name.cyan(), range.cyan());
    } else {
        message = format!("{message} `{}`", name.cyan());
    }
    message
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::path::Path;
    use std::sync::Arc;

    use assert_fs::prelude::*;
    use insta::{assert_json_snapshot, assert_snapshot};
    use reqwest::StatusCode;

    use uv_cache::Cache;
    use uv_client::{BaseClientBuilder, Connectivity};
    use uv_configuration::{
        BuildOptions, DependencyGroupsWithDefaults, ExtrasSpecification, InstallOptions,
    };
    use uv_distribution::MetadataError;
    use uv_distribution_types::{DerivationChain, DerivationStep, IsBuildBackendError};
    use uv_errors::{
        ErrorFormat, ErrorOptions, HintOrdering, Hinted, Hints, write_error_chain_with_options,
    };
    use uv_fs::Simplified;
    use uv_normalize::DefaultExtras;
    use uv_pep440::Version;
    use uv_pep508::{MarkerEnvironment, MarkerEnvironmentBuilder};
    use uv_platform_tags::{Arch, Os, Platform, Tags, TagsOptions};
    use uv_publish::PublishSendError;
    use uv_pypi_types::ResolverMarkerEnvironment;
    use uv_redacted::DisplaySafeUrl;
    use uv_requirements_txt::{RequirementsTxt, SourceCache};
    use uv_resolver::{Lock, PylockTomlError, ResolveError};
    use uv_scripts::Pep723Metadata;
    use uv_settings::FilesystemOptions;
    use uv_types::AnyErrorBuild;
    use uv_workspace::dependency_groups::{DependencyGroupError, FlatDependencyGroups};
    use uv_workspace::pyproject::{PyProjectToml, PyprojectTomlError, SourceError};
    use uv_workspace::{DiscoveryOptions, Workspace, WorkspaceCache, WorkspaceErrorKind};
    use version_ranges::Ranges;

    use crate::commands::pip::{self, operations};
    use crate::commands::project::ProjectError;

    use super::diagnostic_for_error;

    fn format_error(error: &(dyn Error + 'static)) -> String {
        let mut output = String::new();
        write_error_chain_with_options(
            error,
            &Hints::none(),
            ErrorOptions::default()
                .with_width_override(80)
                .with_diagnostic(diagnostic_for_error)
                .with_stream(&mut output),
        )
        .unwrap();
        anstream::adapter::strip_str(&output).to_string()
    }

    fn hidden_root<'a>(mut error: &'a (dyn Error + 'static)) -> &'a (dyn Error + 'static) {
        while let Some(source) = super::registry::metadata_for_error(error).transparent {
            error = source;
        }
        error
    }

    #[test]
    fn resolves_diagnostics_through_transparent_operation_errors() -> anyhow::Result<()> {
        let error = pip::operations::Error::Anyhow(anyhow::Error::new(PublishSendError::Status(
            StatusCode::BAD_REQUEST,
            "Use /upload/ instead.".to_string(),
        )));
        let mut output = String::new();
        write_error_chain_with_options(
            &error,
            &Hints::none(),
            ErrorOptions::default()
                .with_diagnostic(diagnostic_for_error)
                .with_stream(&mut output),
        )?;
        assert_snapshot!(anstream::adapter::strip_str(&output), @"
        error: Server returned status code 400 Bad Request
          info: The server included the following context:
            |
            | Use /upload/ instead.
        ");
        Ok(())
    }

    #[test]
    fn settings_parse_error_retains_source() -> anyhow::Result<()> {
        let file = assert_fs::NamedTempFile::new("uv.toml")?;
        file.write_str(indoc::indoc! {r#"
            index-url = "https://user:first-secret@example.com/simple"
            preview-features = 123
            publish-url = "https://user:second-secret@example.com/legacy/"
        "#})?;
        let error = FilesystemOptions::from_file(file.path())
            .expect_err("invalid preview setting in test input");
        assert!(
            error
                .source()
                .expect("settings retain the original TOML cause")
                .is::<Box<toml::de::Error>>()
        );

        // Rendering must use the input that failed, not a later version of the file.
        file.write_str("preview-features = []\n")?;
        let error = Arc::new(error);
        let mut output = String::new();
        write_error_chain_with_options(
            &error,
            &Hints::none(),
            ErrorOptions::default()
                .with_diagnostic(diagnostic_for_error)
                .with_stream(&mut output),
        )?;
        let output = anstream::adapter::strip_str(&output);
        let display_path = regex::escape(&file.path().user_display().to_string());
        let source_path = regex::escape(&file.path().portable_display().to_string());
        let filters = [
            (display_path.as_str(), "[CONFIG]"),
            (source_path.as_str(), "[CONFIG]"),
        ];
        insta::with_settings!({ filters => filters }, {
            assert_snapshot!(output, @"
            error: Failed to parse: `[CONFIG]`
              cause: invalid type: integer `123`, expected a boolean or a list of preview feature names
               --> [CONFIG]:2:20
                |
              2 | preview-features = 123
                |                    ^^^
            ");
        });
        Ok(())
    }

    #[test]
    fn resolves_toml_diagnostics_through_boxed_errors() {
        let error = PyProjectToml::from_string("[project]\n".to_owned(), "pyproject.toml")
            .expect_err("missing project name in test input");
        assert!(error.source().is_none());
        assert!(diagnostic_for_error(&error).is_some());
        assert!(diagnostic_for_error(&Box::new(error)).is_some());
        let error = PyProjectToml::from_string("[project]\n".to_owned(), "pyproject.toml")
            .expect_err("missing project name in test input");
        assert!(diagnostic_for_error(&Arc::new(error)).is_some());
    }

    #[test]
    fn script_parse_error_retains_original_coordinates() {
        let script = "print('café')\r\n# /// script\r\n# requires-python = \">=3.11\"\r\n# dependencies = [\"\"]\r\n# ///\r\nTOKEN = 'not metadata'\r\n";
        let error = || {
            Pep723Metadata::parse_with_name(script.as_bytes(), "script.py")
                .expect_err("empty requirement in script metadata")
        };
        assert!(error().source().is_none());
        assert!(diagnostic_for_error(&Box::new(error())).is_some());
        assert!(diagnostic_for_error(&Arc::new(error())).is_some());
        assert_snapshot!(format_error(&error()), @r#"
        error: Empty field is not allowed for PEP508
           --> script.py:4:19
            |
          4 | # dependencies = [""]
            |                   ^^
        "#);
    }

    #[test]
    fn remote_script_source_name_is_redacted() -> anyhow::Result<()> {
        let url = DisplaySafeUrl::parse("https://user:secret@example.com/script.py")?;
        let error = Pep723Metadata::parse_with_name(
            b"# /// script\n# dependencies = [\"\"]\n# ///\n",
            url.to_string(),
        )
        .expect_err("empty requirement in remote script metadata");
        assert_snapshot!(format_error(&error), @r#"
        error: Empty field is not allowed for PEP508
           --> https://user:****@example.com/script.py:2:19
            |
          2 | # dependencies = [""]
            |                   ^^
        "#);
        Ok(())
    }

    #[test]
    fn formats_dependency_group_source_through_project_metadata() -> anyhow::Result<()> {
        let pyproject = PyProjectToml::from_string(
            indoc::indoc! {r#"
                [project]
                name = "demo"
                version = "0.1.0"

                [dependency-groups]
                dev = [{ include-group = "missing" }]
            "#}
            .to_string(),
            "project/pyproject.toml",
        )?;
        let error = || {
            FlatDependencyGroups::from_pyproject_toml(Path::new("project"), &pyproject)
                .expect_err("the included group is missing")
        };
        let project = ProjectError::Metadata(MetadataError::DependencyGroup(error()));
        let owner = hidden_root(&project);

        assert!(owner.is::<DependencyGroupError>());
        assert!(std::ptr::eq(
            project.source().expect("the actual semantic cause"),
            owner.source().expect("the dependency-group cause"),
        ));
        assert!(diagnostic_for_error(&Box::new(error())).is_some());
        assert!(diagnostic_for_error(&Arc::new(error())).is_some());
        assert_snapshot!(format_error(&project), @r#"
        error: Project `demo @ project` has malformed dependency groups
          cause: Failed to find group `missing` included by `dev`
           --> project/pyproject.toml:6:26
            |
          6 | dev = [{ include-group = "missing" }]
            |                          ^^^^^^^^^ undefined group
        "#);
        Ok(())
    }

    #[test]
    fn formats_unregistered_erased_build_owner() {
        #[derive(Debug, thiserror::Error)]
        #[error("Third-party build backend failed")]
        struct CustomError(#[source] std::io::Error);

        impl Hinted for CustomError {
            fn hints(&self) -> Hints<'_> {
                Hints::from("Set the backend-specific environment variable")
                    .with_ordering(HintOrdering::Last)
            }
        }

        impl IsBuildBackendError for CustomError {
            fn is_build_backend_error(&self) -> bool {
                true
            }

            fn is_user_failure(&self) -> bool {
                true
            }
        }

        let error =
            AnyErrorBuild::from(CustomError(std::io::Error::other("backend-specific cause")));
        assert_snapshot!(format_error(&error), @"
        error: Third-party build backend failed
          cause: backend-specific cause

        hint: Set the backend-specific environment variable
        ");
    }

    #[test]
    fn formats_native_metadata_through_erased_build_owners() {
        let resolution = AnyErrorBuild::from(uv_dispatch::BuildDispatchError::Resolve(
            ResolveError::Dependencies(
                Box::new(ResolveError::Distribution(uv_distribution::Error::NoBuild)),
                "sklearn".parse().unwrap(),
                Version::new([1, 0]),
                DerivationChain::default(),
            ),
        ));
        let publish = AnyErrorBuild::from(uv_dispatch::BuildDispatchError::Anyhow(
            anyhow::Error::new(PublishSendError::Status(
                StatusCode::BAD_REQUEST,
                "Invalid package metadata".to_string(),
            )),
        ));

        assert_snapshot!(format!("{}\n{}", format_error(&resolution), format_error(&publish)), @"
        error: Failed to resolve dependencies for package `sklearn==1.0`
          cause: Building source distributions is disabled

        hint: `sklearn` is often confused for `scikit-learn`. Did you mean to install
              `scikit-learn` instead?

        error: Server returned status code 400 Bad Request
          info: The server included the following context:
            |
            | Invalid package metadata
        ");
    }

    #[test]
    fn formats_hints_through_tool_errors() {
        let error = ProjectError::Tool(uv_tool::Error::VirtualEnvError(
            uv_virtualenv::Error::Exists {
                name: "virtual environment",
                path: "tool-env".into(),
            },
        ));

        assert_snapshot!(format_error(&error), @"
        error: A virtual environment already exists at: tool-env

        hint: Use the `--clear` flag or set `UV_VENV_CLEAR=1` to replace the existing
              virtual environment
        ");
    }

    #[test]
    fn resolves_transparent_client_roots() {
        let client = || uv_client::Error::from(uv_client::ErrorKind::NoIndex("demo".to_owned()));
        let tool = uv_tool::Error::EnvironmentError(uv_python::Error::ManagedPython(
            uv_python::managed::Error::Download(
                uv_python::downloads::Error::RemotePythonDownloadsJSONClient(Box::new(client())),
            ),
        ));
        let osv = uv_audit::osv::Error::Client(client());

        assert!(tool.source().is_none());
        assert!(osv.source().is_none());
        assert!(hidden_root(&tool).is::<uv_client::Error>());
        assert!(hidden_root(&osv).is::<uv_client::Error>());
    }

    #[test]
    fn resolves_transparent_lock_roots() {
        let lock =
            uv_resolver::LockError::from(uv_configuration::ScopedOverrideSourceError::Index {
                package: "demo".parse().unwrap(),
                dependency: "dependency".parse().unwrap(),
            });
        let error = uv_resolver::PylockTomlError::from(lock);

        assert!(error.source().is_none());
        assert!(hidden_root(&error).is::<uv_configuration::ScopedOverrideSourceError>());
    }

    #[tokio::test]
    async fn formats_requirements_source_through_shared_error() {
        let directory = assert_fs::TempDir::new().unwrap();
        let requirements = directory.child("requirements.txt");
        let error = RequirementsTxt::parse_str(
            "-e demo==1.0\n",
            requirements.path(),
            directory.path(),
            &BaseClientBuilder::default().connectivity(Connectivity::Offline),
            &mut SourceCache::default(),
        )
        .await
        .expect_err("a registry requirement cannot be editable");

        let source_path = regex::escape(&requirements.path().portable_display().to_string());
        insta::with_settings!({ filters => [(source_path.as_str(), "requirements.txt")] }, {
            assert_snapshot!(format_error(&Arc::new(error)), @"
            error: Unsupported editable requirement
               --> requirements.txt:1:1
                |
              1 | -e demo==1.0
                | ^^^^^^^^^^^^ not editable
              cause: Registry requirements cannot be editable

            hint: Editable requirements must refer to a local directory
            ");
        });
    }

    #[test]
    fn formats_source_hints_through_pyproject_errors() {
        let error = PyprojectTomlError::from(SourceError::OverlappingMarkers {
            left: "sys_platform == 'win32'".to_string(),
            right: "python_full_version >= '3.12'".to_string(),
            replacement: "python_full_version >= '3.12' and sys_platform != 'win32'"
                .parse()
                .unwrap(),
            suggestion: None,
        });

        assert_snapshot!(format_error(&error), @"
        error: Failed to parse `tool.uv.sources`
          cause: Source markers must be disjoint, but the following markers overlap:
                 `sys_platform == 'win32'` and `python_full_version >= '3.12'`.

        hint: replace `python_full_version >= '3.12'` with `python_full_version >=
              '3.12' and sys_platform != 'win32'`
        ");
    }

    #[test]
    fn formats_transparent_root_metadata() {
        let error = ProjectError::Operation(operations::Error::Anyhow(anyhow::Error::new(
            uv_publish::PublishSendError::Status(
                http::StatusCode::BAD_REQUEST,
                "Invalid package metadata".to_string(),
            ),
        )));

        assert_snapshot!(format_error(&error), @"
        error: Server returned status code 400 Bad Request
          info: The server included the following context:
            |
            | Invalid package metadata
        ");
    }

    #[test]
    fn formats_hints_through_transparent_build_errors() {
        let error = uv_build_frontend::Error::RequirementsResolve(
            "build-system.requires",
            uv_types::AnyErrorBuild::from(uv_dispatch::BuildDispatchError::Anyhow(
                anyhow::Error::new(uv_build_backend::Error::PortableGlob {
                    field: "tool.uv.build-backend.source-include".to_string(),
                    source: uv_globfilter::PortableGlobError::InvalidCharacterUv {
                        glob: "[".to_string(),
                        pos: 0,
                        invalid: '[',
                    },
                }),
            )),
        );
        let error = ProjectError::Operation(operations::Error::Anyhow(anyhow::Error::new(error)));

        assert_snapshot!(format_error(&error), @"
        error: Failed to resolve requirements from build-system.requires
          cause: Unsupported glob expression in: tool.uv.build-backend.source-include
          cause: Invalid character `[` at position 0 in glob: `[`

        hint: Characters can be escaped with a backslash
        ");
    }

    #[test]
    fn formats_distribution_hints_at_their_owner() {
        let error = ResolveError::Dependencies(
            Box::new(ResolveError::Distribution(uv_distribution::Error::NoBuild)),
            "sklearn".parse().unwrap(),
            Version::new([1, 0]),
            DerivationChain::default(),
        );
        let error = ProjectError::Operation(operations::Error::Resolve(error));

        assert_snapshot!(format_error(&error), @"
        error: Failed to resolve dependencies for package `sklearn==1.0`
          cause: Building source distributions is disabled

        hint: `sklearn` is often confused for `scikit-learn`. Did you mean to install
              `scikit-learn` instead?
        ");
    }

    #[test]
    fn formats_distribution_context_at_its_owner() -> anyhow::Result<()> {
        let chain = [DerivationStep::new(
            "foo".parse()?,
            None,
            None,
            Some(Version::new([1, 0])),
            Ranges::full(),
        )]
        .into_iter()
        .collect();
        let error = ResolveError::Dependencies(
            Box::new(ResolveError::Distribution(uv_distribution::Error::NoBuild)),
            "bar".parse()?,
            Version::new([2, 0]),
            chain,
        );
        let error = operations::Error::Resolve(error);

        assert!(error.hints().is_empty());
        assert_snapshot!(format_error(&error), @"
        error: Failed to resolve dependencies for package `bar==2.0`
          info: `bar` (v2.0) was included because `foo` (v1.0) depends on `bar`
          cause: Building source distributions is disabled
        ");
        Ok(())
    }

    #[test]
    fn formats_context_once_through_smart_pointer_owners() -> anyhow::Result<()> {
        let boxed = Box::new(PublishSendError::Status(
            StatusCode::BAD_REQUEST,
            "Invalid package metadata".to_string(),
        ));
        let shared = Arc::new(PublishSendError::Status(
            StatusCode::BAD_REQUEST,
            "Invalid package metadata".to_string(),
        ));
        let errors: [&(dyn Error + 'static); 2] = [&boxed, &shared];
        let mut reports = Vec::new();
        for error in errors {
            let mut output = String::new();
            write_error_chain_with_options(
                error,
                &Hints::none(),
                ErrorOptions::default()
                    .with_format(ErrorFormat::Json)
                    .with_diagnostic(diagnostic_for_error)
                    .with_stream(&mut output),
            )?;
            let report: serde_json::Value = serde_json::from_str(&output)?;
            reports.push(report["errors"][0]["info"].clone());
        }

        assert_json_snapshot!(reports, @r#"
        [
          [
            {
              "details": "Invalid package metadata",
              "message": "The server included the following context:"
            }
          ],
          [
            {
              "details": "Invalid package metadata",
              "message": "The server included the following context:"
            }
          ]
        ]
        "#);
        Ok(())
    }

    #[test]
    fn formats_native_lock_context_once_through_wrappers() -> anyhow::Result<()> {
        let lock: Lock = toml::from_str(
            r#"
version = 1
revision = 3
requires-python = ">=3.12"

[[package]]
name = "example"
version = "1.0.0"
source = { path = "example-1.0.0-cp312-cp312-macosx_11_0_arm64.whl" }
wheels = [{ filename = "example-1.0.0-cp312-cp312-macosx_11_0_arm64.whl", hash = "sha256:1111111111111111111111111111111111111111111111111111111111111111" }]
"#,
        )?;
        let markers = ResolverMarkerEnvironment::from(MarkerEnvironment::try_from(
            MarkerEnvironmentBuilder {
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
            },
        )?);
        let tags = Tags::from_env(
            Platform::new(
                Os::Manylinux {
                    major: 2,
                    minor: 28,
                },
                Arch::X86_64,
            ),
            (3, 12),
            "cpython",
            (3, 12),
            TagsOptions {
                manylinux_compatible: true,
                ..TagsOptions::default()
            },
        )?;
        let make_error = || {
            lock.to_resolution(
                Path::new("."),
                lock.packages(),
                None,
                &markers,
                &tags,
                &ExtrasSpecification::default().with_defaults(DefaultExtras::default()),
                &DependencyGroupsWithDefaults::none(),
                &BuildOptions::default(),
                &InstallOptions::default(),
            )
            .expect_err("a macOS-only wheel is incompatible with Linux")
        };

        let direct = make_error();
        let boxed = Box::new(make_error());
        let shared = Arc::new(make_error());
        let transparent = PylockTomlError::from(make_error());
        let errors: [&(dyn Error + 'static); 4] = [&direct, &boxed, &shared, &transparent];
        for error in errors {
            let mut output = String::new();
            write_error_chain_with_options(
                error,
                &Hints::none(),
                ErrorOptions::default()
                    .with_format(ErrorFormat::Json)
                    .with_diagnostic(diagnostic_for_error)
                    .with_stream(&mut output),
            )?;
            let report: serde_json::Value = serde_json::from_str(&output)?;
            let errors = report["errors"].as_array().expect("structured error chain");
            assert_eq!(errors.len(), 1);
            assert_eq!(
                errors[0]["info"].as_array().expect("owned context").len(),
                1
            );
            assert_eq!(
                errors[0]["hints"].as_array().expect("owned actions").len(),
                1
            );
        }
        Ok(())
    }

    #[test]
    fn formats_hints_through_boxed_and_shared_sources() {
        #[derive(Debug, thiserror::Error)]
        #[error("Failed to load project metadata")]
        struct Metadata(#[source] Arc<uv_distribution::Error>);

        let error = Metadata(Arc::new(uv_distribution::Error::MetadataLowering(
            uv_distribution::MetadataError::LoweringError(
                "demo".parse().unwrap(),
                Box::new(uv_distribution::LoweringError::MissingIndex {
                    package: "demo".parse().unwrap(),
                    index: "private".parse().unwrap(),
                    declaration_target: uv_distribution::IndexDeclarationTarget::Project,
                    configured_index_origin: Some(uv_distribution_types::Origin::Project),
                    diagnostic: None,
                }),
            ),
        )));

        assert_snapshot!(format_error(&error), @"
        error: Failed to load project metadata
          cause: Failed to parse entry: `demo`
          cause: Package `demo` references an undeclared index: `private`
          info: Index `private` was found in a project-level `uv.toml`, but indexes
                referenced via `tool.uv.sources` must be defined in the project's
                `pyproject.toml`

        hint: Define index `private` in the project's `pyproject.toml`
        ");
    }

    #[tokio::test]
    async fn formats_workspace_diagnostics_through_lowering_errors() -> anyhow::Result<()> {
        #[derive(Debug, thiserror::Error)]
        #[error("Failed to load project metadata")]
        struct Metadata(#[source] Arc<uv_distribution::Error>);

        let root = assert_fs::TempDir::new()?;
        root.child("pyproject.toml").write_str(indoc::indoc! {r#"
            [tool.uv.workspace]
            members = ["packages/first", "packages/second"]
        "#})?;
        for member in ["first", "second"] {
            root.child(format!("packages/{member}/pyproject.toml"))
                .write_str(indoc::indoc! {r#"
                    [project]
                    name = "duplicate"
                    version = "1.0"
                "#})?;
        }
        let cache = Cache::from_path(root.path().join(".cache"));
        let workspace_cache = WorkspaceCache::default();
        let workspace_error = Workspace::discover(
            root.path(),
            &DiscoveryOptions {
                stop_discovery_at: Some(root.path().to_path_buf()),
                ..DiscoveryOptions::default()
            },
            &cache,
            &workspace_cache,
        )
        .await
        .expect_err("the workspace has duplicate package names");
        assert!(matches!(
            workspace_error.as_ref(),
            WorkspaceErrorKind::DuplicatePackage { .. }
        ));
        assert!(workspace_error.source().is_none());

        let error = Metadata(Arc::new(uv_distribution::Error::MetadataLowering(
            MetadataError::LoweringError(
                "dependency".parse()?,
                Box::new(uv_distribution::LoweringError::Workspace(workspace_error)),
            ),
        )));
        let mut output = String::new();
        write_error_chain_with_options(
            &error,
            &Hints::none(),
            ErrorOptions::default()
                .with_format(ErrorFormat::Json)
                .with_diagnostic(diagnostic_for_error)
                .with_stream(&mut output),
        )?;
        let report: serde_json::Value = serde_json::from_str(&output)?;
        let errors = report["errors"]
            .as_array()
            .expect("the report contains an error chain");
        assert_eq!(errors.len(), 3);
        assert_eq!(errors[2]["sources"].as_array().map(Vec::len), Some(1));
        assert_eq!(errors[2]["info"].as_array().map(Vec::len), Some(1));
        let root_path = regex::escape(&root.path().portable_display().to_string());
        insta::with_settings!({filters => [(root_path.as_str(), "[ROOT]")]}, {
            assert_json_snapshot!(&errors[2], @r#"
            {
              "info": [
                {
                  "message": "The name was first declared here",
                  "sources": [
                    {
                      "kind": "snippet",
                      "name": "[ROOT]/packages/first/pyproject.toml",
                      "windows": [
                        {
                          "annotations": [
                            {
                              "kind": "secondary",
                              "label": "first declared here",
                              "range": {
                                "end": {
                                  "byte_column": 18,
                                  "line": 2
                                },
                                "start": {
                                  "byte_column": 7,
                                  "line": 2
                                }
                              }
                            }
                          ],
                          "line_start": 2,
                          "text": "name = \"duplicate\"\n"
                        }
                      ]
                    }
                  ]
                }
              ],
              "message": "Two workspace members are both named `duplicate`",
              "sources": [
                {
                  "kind": "snippet",
                  "name": "[ROOT]/packages/second/pyproject.toml",
                  "windows": [
                    {
                      "annotations": [
                        {
                          "kind": "primary",
                          "label": "duplicate name",
                          "range": {
                            "end": {
                              "byte_column": 18,
                              "line": 2
                            },
                            "start": {
                              "byte_column": 7,
                              "line": 2
                            }
                          }
                        }
                      ],
                      "line_start": 2,
                      "text": "name = \"duplicate\"\n"
                    }
                  ]
                }
              ]
            }
            "#);
        });
        Ok(())
    }

    #[test]
    fn formats_presentation_through_boxed_and_shared_sources() {
        #[derive(Debug, thiserror::Error)]
        #[error("Failed to publish a boxed request")]
        struct Boxed(#[source] Box<uv_publish::PublishSendError>);

        #[derive(Debug, thiserror::Error)]
        #[error("Failed to publish a shared request")]
        struct Shared(#[source] Arc<uv_publish::PublishSendError>);

        let error = || {
            uv_publish::PublishSendError::Status(
                http::StatusCode::BAD_REQUEST,
                "Invalid package metadata".to_string(),
            )
        };
        let boxed = Boxed(Box::new(error()));
        let shared = Shared(Arc::new(error()));

        assert_snapshot!(format!("{}\n{}", format_error(&boxed), format_error(&shared)), @"
        error: Failed to publish a boxed request
          cause: Server returned status code 400 Bad Request
          info: The server included the following context:
            |
            | Invalid package metadata

        error: Failed to publish a shared request
          cause: Server returned status code 400 Bad Request
          info: The server included the following context:
            |
            | Invalid package metadata
        ");
    }
}
