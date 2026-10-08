use uv_add_command::add::AddDependencyError;
use uv_cli_output::printer::Printer;
use uv_errors::{Hinted, Hints};
use uv_pip_command::install::ExternallyManagedError;
use uv_project::ProjectError;
use uv_python_command::install::InvalidUpgradeRequestError;
use uv_remove_command::remove::DependencyNotFoundError;
use uv_run_command::run::RecursionLimitError;
use uv_tool_command::common::NoExecutablesError;
use uv_tool_command::run::{ToolRunScriptError, ToolRunUsageError};
use uv_version_command::version::MissingProjectVersionError;

/// Format an error chain with the default user-facing hints and output settings.
pub(crate) fn write_error_chain(err: &anyhow::Error, printer: Printer) -> std::fmt::Result {
    uv_errors::write_error_chain_with_options(
        err.as_ref(),
        &hints_for_error(err),
        uv_errors::ErrorOptions::default().with_stream(printer.stderr_important()),
    )
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
        collect_hint::<uv_operations::error::Error>(cause, &mut hints);
        collect_hint::<ToolRunScriptError>(cause, &mut hints);
        collect_hint::<RecursionLimitError>(cause, &mut hints);
        collect_hint::<DependencyNotFoundError>(cause, &mut hints);
        collect_hint::<ProjectError>(cause, &mut hints);
        collect_hint::<NoExecutablesError>(cause, &mut hints);
        collect_hint::<ExternallyManagedError>(cause, &mut hints);
        collect_hint::<MissingProjectVersionError>(cause, &mut hints);
        collect_hint::<InvalidUpgradeRequestError>(cause, &mut hints);
        collect_hint::<uv_build_command::build_frontend::Error>(cause, &mut hints);
        collect_hint::<uv_build_backend::Error>(cause, &mut hints);
        collect_hint::<uv_build_frontend::Error>(cause, &mut hints);
        collect_hint::<uv_python_discovery::Error>(cause, &mut hints);
        collect_hint::<uv_installer::IncompatibleWheelError>(cause, &mut hints);
        collect_hint::<uv_installer::PlanError>(cause, &mut hints);
        collect_hint::<uv_distribution::Error>(cause, &mut hints);
        collect_hint::<uv_python_interpreter::BrokenLink>(cause, &mut hints);
        collect_hint::<uv_lock::PylockTomlError>(cause, &mut hints);
        collect_hint::<uv_pylock_command::PylockResolutionError>(cause, &mut hints);
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
    use insta::assert_debug_snapshot;

    use uv_workspace::pyproject::{PyprojectTomlError, SourceError};

    use super::hints_for_error;

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
}

#[cfg(test)]
mod error_tests {
    use std::io::{Error, ErrorKind};

    use anyhow::bail;
    use insta::{allow_duplicates, assert_snapshot};

    use uv_cli_error::UvError;

    #[test]
    fn contextual_operations_keep_their_classification_and_cause() -> anyhow::Result<()> {
        for (kind, user_failure) in [
            (ErrorKind::NotFound, true),
            (ErrorKind::PermissionDenied, false),
        ] {
            let error = uv_operations::error::Error::Requirements(uv_requirements::Error::Io(
                Error::new(kind, "requirements failure"),
            ));
            let error = UvError::from(
                error
                    .with_resolution_context("script")
                    .with_resolution_context("tool"),
            );
            let ((UvError::User(error), true) | (UvError::Unexpected(error), false)) =
                (error, user_failure)
            else {
                bail!("operation classification changed with context");
            };
            allow_duplicates! {
                assert_snapshot!(format!("{error:#}"), @"Failed to resolve tool requirement: requirements failure");
            }
            assert!(
                error
                    .downcast_ref::<uv_operations::error::Error>()
                    .is_some()
            );
        }
        Ok(())
    }

    #[test]
    fn resolution_context_leaves_other_errors_unchanged() -> anyhow::Result<()> {
        let error = uv_operations::error::Error::Io(Error::new(
            ErrorKind::PermissionDenied,
            "cache write failed",
        ));
        let UvError::Unexpected(error) = UvError::from(error.with_resolution_context("tool"))
        else {
            bail!("operation classification changed with context");
        };
        assert_snapshot!(format!("{error:#}"), @"cache write failed");
        assert!(
            error
                .downcast_ref::<uv_operations::error::Error>()
                .is_some()
        );
        Ok(())
    }

    #[test]
    fn project_requirements_use_operation_classification() {
        let error = uv_project::ProjectError::Requirements(uv_requirements::Error::Io(Error::new(
            ErrorKind::NotFound,
            "requirements failure",
        )));
        assert!(matches!(UvError::from(error), UvError::User(_)));
    }
}
