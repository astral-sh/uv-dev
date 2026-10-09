use std::sync::Arc;

use uv_distribution_types::{InstalledDist, InstalledDistKind, InstalledEggInfoFile};
use uv_install_wheel::Layout;
use uv_python_interpreter::EnvironmentLock;

/// Uninstall a package from the specified Python environment.
pub async fn uninstall(
    dist: &InstalledDist,
    layout: &Layout,
    destination_lock: Option<Arc<EnvironmentLock>>,
) -> Result<uv_install_wheel::Uninstall, UninstallError> {
    let uninstall = tokio::task::spawn_blocking({
        let dist = dist.clone();
        let layout = layout.clone();
        move || {
            let _destination_lock = destination_lock;
            match dist.kind {
                InstalledDistKind::Registry(_) | InstalledDistKind::Url(_) => Ok(
                    uv_install_wheel::uninstall_wheel(dist.install_path(), &dist, &layout)?,
                ),
                InstalledDistKind::EggInfoDirectory(_) => {
                    Ok(uv_install_wheel::uninstall_egg(dist.install_path(), &dist)?)
                }
                InstalledDistKind::LegacyEditable(dist) => {
                    Ok(uv_install_wheel::uninstall_legacy_editable(&dist.egg_link)?)
                }
                InstalledDistKind::EggInfoFile(dist) => Err(UninstallError::Distutils(dist)),
            }
        }
    })
    .await??;

    Ok(uninstall)
}

#[derive(thiserror::Error, Debug)]
pub enum UninstallError {
    #[error(
        "Unable to uninstall `{0}`. distutils-installed distributions do not include the metadata required to uninstall safely."
    )]
    Distutils(InstalledEggInfoFile),
    #[error(transparent)]
    Uninstall(#[from] uv_install_wheel::Error),
    #[error(transparent)]
    Join(#[from] tokio::task::JoinError),
}
