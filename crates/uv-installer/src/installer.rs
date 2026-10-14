use std::sync::Arc;

use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use tokio::sync::oneshot;
use tracing::{instrument, warn};

use uv_cache::Cache;
use uv_distribution_types::CachedDist;
use uv_install_wheel::{Layout, LinkMode};
use uv_preview::Preview;
use uv_python_interpreter::PythonEnvironment;
use uv_threads::initialize_rayon_once;

/// A failure while installing wheels into a Python environment.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error(
        "Symlink-based installation is not supported with `--no-cache`. The created environment will be rendered unusable by the removal of the cache."
    )]
    SymlinkWithoutCache,
    #[error("`install_blocking` task panicked")]
    WorkerPanicked,
    #[error("Failed to install: {} ({wheel})", wheel.filename())]
    Wheel {
        wheel: Box<CachedDist>,
        #[source]
        source: uv_install_wheel::Error,
    },
}

pub struct Installer<'a> {
    venv: &'a PythonEnvironment,
    link_mode: LinkMode,
    cache: Option<&'a Cache>,
    reporter: Option<Arc<dyn Reporter>>,
    /// The name of the [`Installer`].
    name: Option<String>,
    /// The metadata associated with the [`Installer`].
    metadata: bool,
    /// Preview settings for the installer.
    preview: Preview,
}

/// Owned installation settings that can be moved to a worker thread.
struct InstallContext {
    layout: Layout,
    relocatable: bool,
    link_mode: LinkMode,
    reporter: Option<Arc<dyn Reporter>>,
    name: Option<String>,
    metadata: bool,
    preview: Preview,
}

impl<'a> Installer<'a> {
    /// Initialize a new installer.
    pub fn new(venv: &'a PythonEnvironment, preview: Preview) -> Self {
        Self {
            venv,
            link_mode: LinkMode::default(),
            cache: None,
            reporter: None,
            name: Some("uv".to_string()),
            metadata: true,
            preview,
        }
    }

    /// Set the [`LinkMode`][`uv_install_wheel::LinkMode`] to use for this installer.
    #[must_use]
    pub fn with_link_mode(self, link_mode: LinkMode) -> Self {
        Self { link_mode, ..self }
    }

    /// Set the [`Cache`] to use for this installer.
    #[must_use]
    pub fn with_cache(self, cache: &'a Cache) -> Self {
        Self {
            cache: Some(cache),
            ..self
        }
    }

    /// Set the [`Reporter`] to use for this installer.
    #[must_use]
    pub fn with_reporter(self, reporter: Arc<dyn Reporter>) -> Self {
        Self {
            reporter: Some(reporter),
            ..self
        }
    }

    /// Set the `installer_name` to something other than `"uv"`.
    #[must_use]
    pub fn with_installer_name(self, installer_name: Option<String>) -> Self {
        Self {
            name: installer_name,
            ..self
        }
    }

    /// Set whether to install uv-specifier files in the dist-info directory.
    #[must_use]
    pub fn with_installer_metadata(self, installer_metadata: bool) -> Self {
        Self {
            metadata: installer_metadata,
            ..self
        }
    }

    /// Install a set of wheels into a Python virtual environment.
    #[instrument(skip_all, fields(num_wheels = %wheels.len()))]
    pub async fn install(self, wheels: Vec<CachedDist>) -> Result<Vec<CachedDist>, InstallError> {
        let context = self.prepare_install()?;
        let (tx, rx) = oneshot::channel();

        rayon::spawn(move || {
            let result = context.install(wheels);

            // This may fail if the main task was cancelled.
            let _ = tx.send(result);
        });

        rx.await.map_err(|_| InstallError::WorkerPanicked)?
    }

    /// Install a set of wheels into a Python virtual environment synchronously.
    #[instrument(skip_all, fields(num_wheels = %wheels.len()))]
    pub fn install_blocking(
        self,
        wheels: Vec<CachedDist>,
    ) -> Result<Vec<CachedDist>, InstallError> {
        self.prepare_install()?.install(wheels)
    }

    /// Prepare owned installation settings and initialize the worker pool.
    fn prepare_install(self) -> Result<InstallContext, InstallError> {
        if self.cache.is_some_and(Cache::is_temporary) && self.link_mode.is_symlink() {
            return Err(InstallError::SymlinkWithoutCache);
        }

        let context = InstallContext {
            layout: self.venv.interpreter().layout(),
            relocatable: self.venv.relocatable(),
            link_mode: self.link_mode,
            reporter: self.reporter,
            name: self.name,
            metadata: self.metadata,
            preview: self.preview,
        };
        initialize_rayon_once();
        Ok(context)
    }
}

impl InstallContext {
    /// Install a set of wheels into a Python virtual environment synchronously.
    #[instrument(skip_all, fields(num_wheels = %wheels.len()))]
    fn install(&self, wheels: Vec<CachedDist>) -> Result<Vec<CachedDist>, InstallError> {
        let state = uv_install_wheel::InstallState::new(self.preview);
        wheels.par_iter().try_for_each(|wheel| {
            uv_install_wheel::install_wheel(
                &self.layout,
                self.relocatable,
                wheel.path(),
                wheel.filename(),
                wheel
                    .parsed_url()
                    .map(uv_pypi_types::DirectUrl::from)
                    .as_ref(),
                if wheel.cache_info().is_empty() {
                    None
                } else {
                    Some(wheel.cache_info())
                },
                wheel.build_info(),
                self.name.as_deref(),
                self.metadata,
                self.link_mode,
                &state,
            )
            .map_err(|source| InstallError::Wheel {
                wheel: Box::new(wheel.clone()),
                source,
            })?;

            if let Some(reporter) = &self.reporter {
                reporter.on_install_progress(wheel);
            }

            Ok::<(), InstallError>(())
        })?;
        if let Some(reporter) = &self.reporter {
            reporter.on_install_complete();
        }
        if let Err(err) = state.warn_package_conflicts() {
            warn!("Checking for conflicts between packages failed: {err}");
        }

        Ok(wheels)
    }
}

pub trait Reporter: Send + Sync {
    /// Callback to invoke when a dependency is installed.
    fn on_install_progress(&self, wheel: &CachedDist);

    /// Callback to invoke when all wheels have been installed.
    fn on_install_complete(&self);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use uv_cache::Cache;
    use uv_distribution_types::CachedDist;
    use uv_preview::Preview;
    use uv_python_discovery::find_environment;
    use uv_python_interpreter::PythonEnvironment;
    use uv_python_types::{EnvironmentPreference, PythonPreference, PythonRequest};

    use super::{Installer, Reporter};

    fn environment() -> PythonEnvironment {
        let _preview = uv_preview::test::with_features(&[]);
        let cache = Cache::temp().expect("cache should be available");
        find_environment(
            &PythonRequest::Any,
            EnvironmentPreference::Any,
            PythonPreference::System,
            None,
            &cache,
        )
        .expect("Python environment should be available")
    }

    #[test]
    fn default_installer_name() {
        let environment = environment();

        let installer = Installer::new(&environment, Preview::default());

        assert_eq!(installer.name.as_deref(), Some("uv"));
    }

    #[test]
    fn custom_installer_name() {
        let environment = environment();

        let installer = Installer::new(&environment, Preview::default())
            .with_installer_name(Some("client".to_string()));

        assert_eq!(installer.name.as_deref(), Some("client"));
    }

    #[test]
    fn disabled_installer_name() {
        let environment = environment();

        let installer = Installer::new(&environment, Preview::default()).with_installer_name(None);

        assert_eq!(installer.name, None);
    }

    #[test]
    fn reports_installation_complete() {
        struct CompletionReporter(AtomicUsize);

        impl Reporter for CompletionReporter {
            fn on_install_progress(&self, _wheel: &CachedDist) {
                panic!("an empty installation cannot report progress");
            }

            fn on_install_complete(&self) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }

        let environment = environment();
        let reporter = Arc::new(CompletionReporter(AtomicUsize::new(0)));
        Installer::new(&environment, Preview::default())
            .with_reporter(reporter.clone())
            .install_blocking(Vec::new())
            .expect("empty installation should succeed");
        assert_eq!(reporter.0.load(Ordering::Relaxed), 1);
    }
}
