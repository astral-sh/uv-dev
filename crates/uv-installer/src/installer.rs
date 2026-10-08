use std::sync::Arc;

use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use tokio::sync::oneshot;
use tracing::{instrument, warn};

use uv_cache::Cache;
use uv_distribution_types::CachedDist;
use uv_install_wheel::{Layout, LinkMode};
use uv_preview::Preview;
use uv_python_interpreter::{EnvironmentLock, PythonEnvironment};
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
    destination_lock: Option<Arc<EnvironmentLock>>,
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

impl<'a> Installer<'a> {
    /// Initialize a new installer.
    pub fn new(venv: &'a PythonEnvironment, preview: Preview) -> Self {
        Self {
            venv,
            destination_lock: venv.destination_lock(),
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
        let Self {
            venv,
            destination_lock,
            cache,
            link_mode,
            reporter,
            name: installer_name,
            metadata: installer_metadata,
            preview,
        } = self;

        if cache.is_some_and(Cache::is_temporary) {
            if link_mode.is_symlink() {
                return Err(InstallError::SymlinkWithoutCache);
            }
        }

        let (tx, rx) = oneshot::channel();

        let layout = venv.interpreter().layout();
        let relocatable = venv.relocatable();
        // Initialize the threadpool with the user settings.
        initialize_rayon_once();
        rayon::spawn(move || {
            let _destination_lock = destination_lock;
            let result = install(
                wheels,
                &layout,
                installer_name.as_deref(),
                link_mode,
                reporter.as_ref(),
                relocatable,
                installer_metadata,
                preview,
            );

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
        if self.cache.is_some_and(Cache::is_temporary) {
            if self.link_mode.is_symlink() {
                return Err(InstallError::SymlinkWithoutCache);
            }
        }

        install(
            wheels,
            &self.venv.interpreter().layout(),
            self.name.as_deref(),
            self.link_mode,
            self.reporter.as_ref(),
            self.venv.relocatable(),
            self.metadata,
            self.preview,
        )
    }
}

/// Install a set of wheels into a Python virtual environment synchronously.
#[instrument(skip_all, fields(num_wheels = %wheels.len()))]
fn install(
    wheels: Vec<CachedDist>,
    layout: &Layout,
    installer_name: Option<&str>,
    link_mode: LinkMode,
    reporter: Option<&Arc<dyn Reporter>>,
    relocatable: bool,
    installer_metadata: bool,
    preview: Preview,
) -> Result<Vec<CachedDist>, InstallError> {
    // Initialize the threadpool with the user settings.
    initialize_rayon_once();
    let state = uv_install_wheel::InstallState::new(preview);
    wheels.par_iter().try_for_each(|wheel| {
        uv_install_wheel::install_wheel(
            layout,
            relocatable,
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
            installer_name,
            installer_metadata,
            link_mode,
            &state,
        )
        .map_err(|source| InstallError::Wheel {
            wheel: Box::new(wheel.clone()),
            source,
        })?;

        if let Some(reporter) = reporter.as_ref() {
            reporter.on_install_progress(wheel);
        }

        Ok::<(), InstallError>(())
    })?;
    if let Err(err) = state.warn_package_conflicts() {
        warn!("Checking for conflicts between packages failed: {err}");
    }

    Ok(wheels)
}

pub trait Reporter: Send + Sync {
    /// Callback to invoke when a dependency is installed.
    fn on_install_progress(&self, wheel: &CachedDist);

    /// Callback to invoke when the resolution is complete.
    fn on_install_complete(&self);
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Duration;

    use anyhow::Result;
    use tokio::sync::oneshot;
    use uv_cache::Cache;
    use uv_cache_info::CacheInfo;
    use uv_cache_key::cache_digest;
    use uv_distribution_types::{CachedDist, CachedRegistryDist};
    use uv_fs::{LockedFile, LockedFileMode};
    use uv_preview::Preview;
    use uv_pypi_types::HashDigests;
    use uv_python_discovery::find_environment;
    use uv_python_interpreter::{EnvironmentLock, PythonEnvironment};
    use uv_python_types::{EnvironmentPreference, PythonPreference, PythonRequest, Target};

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

    struct InstallationGate {
        started: Mutex<Option<oneshot::Sender<()>>>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl Reporter for InstallationGate {
        fn on_install_progress(&self, _wheel: &CachedDist) {
            if let Some(started) = self.started.lock().expect("gate mutex").take() {
                let _ = started.send(());
            }
            let _ = self.release.lock().expect("gate mutex").recv();
        }

        fn on_install_complete(&self) {}
    }

    #[tokio::test]
    async fn cancelled_installer_retains_destination_and_temporary_cache() -> Result<()> {
        let cache = Cache::temp()?.init().await?;
        let root = cache.root().join("environment");
        let wheel = cache.root().join("wheel");
        let dist_info = wheel.join("example-1.0.0.dist-info");
        fs_err::create_dir_all(&dist_info)?;
        fs_err::write(wheel.join("example.py"), "value = 1")?;
        fs_err::write(
            dist_info.join("METADATA"),
            "Metadata-Version: 2.1\nName: example\nVersion: 1.0.0\n",
        )?;
        fs_err::write(
            dist_info.join("WHEEL"),
            "Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
        )?;
        fs_err::write(
            dist_info.join("RECORD"),
            "example.py,,\nexample-1.0.0.dist-info/METADATA,,\nexample-1.0.0.dist-info/WHEEL,,\nexample-1.0.0.dist-info/RECORD,,\n",
        )?;

        let guard = EnvironmentLock::acquire(&[root.clone()], &cache).await?;
        let environment = environment()
            .with_target(Target::from(root.clone()))?
            .with_destination_lock(&guard);
        let key = fs_err::canonicalize(&root)?;
        let lock_path =
            std::env::temp_dir().join(format!("uv-environment-{}.lock", cache_digest(&key)));
        let (started, ready) = oneshot::channel();
        let (release, receiver) = mpsc::channel();
        let reporter = Arc::new(InstallationGate {
            started: Mutex::new(Some(started)),
            release: Mutex::new(receiver),
        });
        let distribution = CachedDist::Registry(CachedRegistryDist {
            filename: "example-1.0.0-py3-none-any.whl".parse()?,
            path: wheel.into_boxed_path(),
            hashes: HashDigests::empty(),
            cache_info: CacheInfo::default(),
            build_info: None,
        });
        let task = tokio::spawn(async move {
            Installer::new(&environment, Preview::default())
                .with_reporter(reporter)
                .install(vec![distribution])
                .await
        });
        tokio::time::timeout(Duration::from_secs(30), ready).await??;
        task.abort();
        assert!(task.await.expect_err("caller was cancelled").is_cancelled());
        drop(cache);
        drop(guard);

        assert!(root.join("example.py").is_file());
        assert!(
            LockedFile::acquire_no_wait(&lock_path, LockedFileMode::Exclusive, root.display())
                .is_none()
        );

        release.send(())?;
        let admitted = tokio::time::timeout(
            Duration::from_secs(30),
            LockedFile::acquire(&lock_path, LockedFileMode::Exclusive, root.display()),
        )
        .await??;
        assert!(
            !root.exists(),
            "worker cache lease is released before destination admission"
        );
        drop(admitted);
        Ok(())
    }
}
