use std::sync::Arc;

use anyhow::Result;
use tracing::debug;

use uv_cache::Cache;
use uv_client::BaseClientBuilder;
use uv_command_support::Printer;
use uv_fs::Simplified;
use uv_python_discovery::{
    PythonDownloadReporter, PythonInstallation, find_environment, report_interpreter,
};
use uv_python_interpreter::{EnvironmentLock, PythonEnvironment};
use uv_python_types::{
    EnvironmentPreference, Prefix, PythonArchitecture, PythonDownloads, PythonPreference,
    PythonRequest, Target,
};
use uv_settings::PythonInstallMirrors;

use crate::reporters::report_target_environment;

/// Select and prepare pip's destination while retaining its admission guard.
pub(super) async fn prepare_environment(
    python: Option<&str>,
    system: bool,
    target: Option<Target>,
    prefix: Option<Prefix>,
    python_preference: PythonPreference,
    python_arch: Option<PythonArchitecture>,
    python_downloads: PythonDownloads,
    install_mirrors: &PythonInstallMirrors,
    client_builder: &BaseClientBuilder<'_>,
    cache: &Cache,
    printer: Printer,
) -> Result<(PythonEnvironment, Option<Arc<EnvironmentLock>>)> {
    // Re-discover after destination admission: replacement may change the selected interpreter.
    let mut destination_lock: Option<Arc<EnvironmentLock>> = None;
    let mut admitted = false;
    let (environment, installation) = loop {
        let (environment, installation) = if target.is_some() || prefix.is_some() {
            let python_request = python.map(PythonRequest::parse);
            let reporter = PythonDownloadReporter::single(printer);

            let installation = PythonInstallation::find_or_download(
                python_request.as_ref(),
                EnvironmentPreference::from_system_flag(system, false),
                python_preference.with_system_flag(system),
                python_arch,
                python_downloads,
                client_builder,
                cache,
                Some(&reporter),
                install_mirrors.mirrors(),
                install_mirrors.python_downloads_json_url.as_deref(),
            )
            .await?;
            (
                PythonEnvironment::from_interpreter(installation.interpreter().clone()),
                Some(installation),
            )
        } else {
            let environment = find_environment(
                &python.map(PythonRequest::parse).unwrap_or_default(),
                EnvironmentPreference::from_system_flag(system, true),
                PythonPreference::default().with_system_flag(system),
                python_arch,
                cache,
            )?;
            (environment, None)
        };

        let destination = target
            .as_ref()
            .map(Target::root)
            .or_else(|| prefix.as_ref().map(Prefix::root))
            .unwrap_or_else(|| environment.root())
            .to_path_buf();
        let paths = [destination];
        let needs_admission = match destination_lock.as_ref() {
            Some(lock) => !lock.matches(&paths)?,
            None => !admitted,
        };
        if needs_admission {
            drop(destination_lock.take());
            destination_lock = EnvironmentLock::acquire_optional(&paths, cache).await?;
            admitted = true;
            if destination_lock.is_some() {
                continue;
            }
        }
        break (environment, installation);
    };

    if let Some(installation) = installation {
        report_interpreter(&installation, true, printer)?;
    } else {
        report_target_environment(&environment, cache, printer)?;
    }

    // Apply any `--target` or `--prefix` directories.
    let environment = if let Some(target) = target {
        debug!(
            "Using `--target` directory at `{}`",
            target.root().user_display()
        );
        environment.with_target(target)?
    } else if let Some(prefix) = prefix {
        debug!(
            "Using `--prefix` directory at `{}`",
            prefix.root().user_display()
        );
        environment.with_prefix(prefix)?
    } else {
        environment
    };

    let environment = if let Some(lock) = destination_lock.as_mut() {
        lock.finish_creation()?;
        environment.with_destination_lock(lock)
    } else {
        environment
    };

    Ok((environment, destination_lock))
}
