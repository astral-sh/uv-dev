use std::error::Error;

use uv_errors::{Diagnostic, Hinted, Hints};

use crate::commands::pip::install::ExternallyManagedError;
use crate::commands::project::ProjectError;
use crate::commands::project::add::AddDependencyError;
use crate::commands::project::remove::DependencyNotFoundError;
use crate::commands::project::run::RecursionLimitError;
use crate::commands::python::install::InvalidUpgradeRequestError;
use crate::commands::tool::NoExecutablesError;
use crate::commands::tool::run::{ToolRunScriptError, ToolRunUsageError};
use uv_build_commands::Error as BuildError;
use uv_command_support::Printer;

/// Format an error chain with the default user-facing hints and output settings.
pub(crate) fn write_error_chain(err: &anyhow::Error, printer: Printer) -> std::fmt::Result {
    uv_errors::write_error_chain_with_options(
        err.as_ref(),
        &hints_for_error(err),
        uv_errors::ErrorOptions::default()
            .with_diagnostic(diagnostic_for_error)
            .with_stream(printer.stderr_important()),
    )
}

/// Resolve presentation data without changing an error or its source chain.
pub(crate) fn diagnostic_for_error<'a>(error: &'a (dyn Error + 'static)) -> Option<Diagnostic<'a>> {
    uv_settings::diagnostic_for_error(error)
        .or_else(|| uv_workspace::pyproject::diagnostic_for_error(error))
        .or_else(|| uv_pypi_types::diagnostic_for_error(error))
}

/// Walk an error chain and collect hint strings from all known error types.
///
/// This is the central "hint for error" function. It walks the full error chain
/// (via `anyhow::Error::chain`) and tries to downcast each error to known types
/// that implement [`Hinted`]. All hint rendering logic should be consolidated here.
pub(crate) fn hints_for_error(err: &anyhow::Error) -> Hints<'static> {
    let mut hints = Hints::none();
    for cause in err.chain() {
        collect_hint::<AddDependencyError>(cause, &mut hints);
        collect_hint::<ToolRunUsageError>(cause, &mut hints);
        collect_hint::<Box<uv_resolver::NoSolutionError>>(cause, &mut hints);
        collect_hint::<uv_resolver::NoSolutionError>(cause, &mut hints);
        collect_hint::<uv_resolver::ResolveError>(cause, &mut hints);
        collect_hint::<uv_lock::LockError>(cause, &mut hints);
        collect_hint::<uv_lock_operations::LockError>(cause, &mut hints);
        collect_hint::<uv_resolve_operations::Error>(cause, &mut hints);
        collect_hint::<uv_install_operations::Error>(cause, &mut hints);
        collect_hint::<ToolRunScriptError>(cause, &mut hints);
        collect_hint::<RecursionLimitError>(cause, &mut hints);
        collect_hint::<DependencyNotFoundError>(cause, &mut hints);
        collect_hint::<ProjectError>(cause, &mut hints);
        collect_hint::<uv_environment_operations::EnvironmentError>(cause, &mut hints);
        collect_hint::<uv_python_discovery::PythonSelectionError>(cause, &mut hints);
        collect_hint::<NoExecutablesError>(cause, &mut hints);
        collect_hint::<ExternallyManagedError>(cause, &mut hints);
        collect_hint::<InvalidUpgradeRequestError>(cause, &mut hints);
        collect_hint::<BuildError>(cause, &mut hints);
        collect_hint::<uv_build_backend::Error>(cause, &mut hints);
        collect_hint::<uv_build_frontend::Error>(cause, &mut hints);
        collect_hint::<uv_python_discovery::Error>(cause, &mut hints);
        collect_hint::<uv_installer::IncompatibleWheelError>(cause, &mut hints);
        collect_hint::<uv_installer::PlanError>(cause, &mut hints);
        collect_hint::<uv_distribution::Error>(cause, &mut hints);
        collect_hint::<uv_python_interpreter::BrokenLink>(cause, &mut hints);
        collect_hint::<uv_lock::PylockTomlError>(cause, &mut hints);
        collect_hint::<uv_pip_commands::PylockResolutionError>(cause, &mut hints);
        collect_hint::<uv_requirements_txt::MakeEditableError>(cause, &mut hints);
        collect_hint::<uv_python_interpreter::InterpreterError>(cause, &mut hints);
        collect_hint::<uv_workspace::pyproject::SourceError>(cause, &mut hints);
        collect_hint::<uv_distribution::LoweringError>(cause, &mut hints);
        collect_hint::<uv_virtualenv::Error>(cause, &mut hints);
        collect_hint::<uv_client::Error>(cause, &mut hints);
        #[cfg(not(feature = "self-update"))]
        collect_hint::<crate::ExternallyInstalledError>(cause, &mut hints);
    }
    hints
}

/// If `cause` can be downcast to `T`, collect its hints.
fn collect_hint<T: Hinted + std::error::Error + 'static>(
    cause: &(dyn std::error::Error + 'static),
    hints: &mut Hints<'static>,
) {
    if let Some(inner) = cause.downcast_ref::<T>() {
        hints.extend(inner.hints());
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use assert_fs::prelude::*;
    use insta::{assert_debug_snapshot, assert_snapshot};

    use uv_errors::{ErrorOptions, Hints, write_error_chain_with_options};
    use uv_fs::Simplified;
    use uv_lock_operations::LockError;
    use uv_project_commands::ProjectError;
    use uv_settings::{FilesystemOptions, LockedFlag, LockedSource};
    use uv_workspace::pyproject::{PyProjectToml, PyprojectTomlError, SourceError};

    use super::{diagnostic_for_error, hints_for_error};

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

        file.write_str("preview-features = []\n")?;
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
        let filters = [(display_path.as_str(), "[CONFIG]")];
        insta::with_settings!({ filters => filters }, {
            assert_snapshot!(output, @"
            error: Failed to parse: [CONFIG]
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
    fn pyproject_diagnostics_retain_parser_sources() {
        let error = uv_pypi_types::PyProjectToml::from_toml("123 - 456", "pyproject.toml")
            .expect_err("invalid TOML in test input");
        assert!(
            error
                .source()
                .expect("metadata retains the original syntax error")
                .is::<toml_edit::TomlError>()
        );
        assert!(diagnostic_for_error(&error).is_some());
        assert!(diagnostic_for_error(&Box::new(error)).is_some());

        let error =
            uv_pypi_types::PyProjectToml::from_toml("[project]\nname = 42\n", "pyproject.toml")
                .expect_err("invalid project name in test input");
        assert!(error.source().is_none());
        assert!(diagnostic_for_error(&error).is_some());

        let error = PyProjectToml::from_string("[project]\n".to_owned(), "pyproject.toml")
            .expect_err("missing project name in test input");
        assert!(error.source().is_none());
        assert!(diagnostic_for_error(&error).is_some());
        assert!(diagnostic_for_error(&Box::new(error)).is_some());
    }

    #[test]
    fn collects_source_hints_through_pyproject_errors() {
        let err = anyhow::Error::new(PyprojectTomlError::Source(SourceError::OverlappingMarkers(
            "sys_platform == 'win32'".to_string(),
            "python_version == '3.12'".to_string(),
            "python_version != '3.12'".to_string(),
        )));

        let hints = hints_for_error(&err);
        assert_debug_snapshot!(hints.iter().collect::<Vec<_>>(), @r#"
        [
            "replace `python_version == '3.12'` with `python_version != '3.12'`",
        ]
        "#);
    }

    #[test]
    fn collects_lock_hints_through_context() {
        let error =
            LockError::LockFormat("uv.lock".into(), 3, LockedSource::Cli(LockedFlag::Check));

        // Command context retains the lockfile's regeneration hint.
        let error = anyhow::Error::new(error).context("Failed to check the lockfile");

        let hints = hints_for_error(&error);
        assert_debug_snapshot!(hints.iter().collect::<Vec<_>>(), @r#"
        [
            "To regenerate the lockfile, run `uv lock --refresh --preview-features lockfile-format-check`.",
        ]
        "#);
    }

    #[test]
    fn collects_lock_hints_through_project_errors() {
        let error =
            LockError::LockFormat("uv.lock".into(), 3, LockedSource::Cli(LockedFlag::Check));

        // Project and command context retain the lockfile's regeneration hint.
        let error =
            anyhow::Error::new(ProjectError::from(error)).context("Failed to check the lockfile");

        let hints = hints_for_error(&error);
        assert_debug_snapshot!(hints.iter().collect::<Vec<_>>(), @r#"
        [
            "To regenerate the lockfile, run `uv lock --refresh --preview-features lockfile-format-check`.",
        ]
        "#);
    }
}
