//! Like `wheel.rs`, but for installing wheels that have already been unzipped, rather than
//! reading from a zip file.

use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use fs_err::File;
use tracing::{instrument, trace};

use uv_distribution_filename::WheelFilename;
use uv_pep440::Version;
use uv_pypi_types::{DirectUrl, Metadata10};

use crate::linker::{InstallState, LinkMode, link_wheel_files};
use crate::wheel::{
    LibKind, ValidatedWheel, WheelFile, dist_info_metadata, find_dist_info, install_data,
    parse_scripts, read_record, write_installer_metadata, write_record, write_script_entrypoints,
};
use crate::{Error, Layout};

/// Return the path at which the wheel's `.dist-info` directory will be installed.
pub fn installed_dist_info_path(
    layout: &Layout,
    wheel: impl AsRef<Path>,
) -> Result<PathBuf, Error> {
    let (dist_info_prefix, site_packages) = wheel_destination(layout, wheel.as_ref())?;
    Ok(site_packages.join(format!("{dist_info_prefix}.dist-info")))
}

/// Return the wheel's `.dist-info` prefix and target `site-packages` directory.
fn wheel_destination<'layout>(
    layout: &'layout Layout,
    wheel: &Path,
) -> Result<(String, &'layout Path), Error> {
    let dist_info_prefix = find_dist_info(wheel)?;
    let wheel_file_path = wheel.join(format!("{dist_info_prefix}.dist-info/WHEEL"));
    let wheel_text = fs_err::read_to_string(wheel_file_path)?;
    let site_packages = match WheelFile::parse(&wheel_text)?.lib_kind() {
        LibKind::Pure => &layout.scheme.purelib,
        LibKind::Plat => &layout.scheme.platlib,
    };
    Ok((dist_info_prefix, site_packages))
}

/// Install the given wheel to the given venv
///
/// The caller must ensure that the wheel is compatible to the environment.
///
/// <https://packaging.python.org/en/latest/specifications/binary-distribution-format/#installing-a-wheel-distribution-1-0-py32-none-any-whl>
///
/// Wheel 1.0: <https://www.python.org/dev/peps/pep-0427/>
#[instrument(skip_all, fields(wheel = %filename))]
pub fn install_wheel<Cache: serde::Serialize, Build: serde::Serialize>(
    layout: &Layout,
    relocatable: bool,
    wheel: impl AsRef<Path>,
    filename: &WheelFilename,
    direct_url: Option<&DirectUrl>,
    cache_info: Option<&Cache>,
    build_info: Option<&Build>,
    installer: Option<&str>,
    installer_metadata: bool,
    link_mode: LinkMode,
    state: &InstallState,
) -> Result<(), Error> {
    let wheel = wheel.as_ref();
    let (dist_info_prefix, site_packages) = wheel_destination(layout, wheel)?;
    let metadata = dist_info_metadata(&dist_info_prefix, wheel)?;
    let Metadata10 { name, version } = Metadata10::parse_pkg_info(&metadata)
        .map_err(|err| Error::InvalidWheel(err.to_string()))?;

    let version = Version::from_str(&version)?;

    // Validate the wheel name and version.
    if !uv_flags::contains(uv_flags::EnvironmentFlags::SKIP_WHEEL_FILENAME_CHECK) {
        if name != filename.name {
            return Err(Error::MismatchedName(name, filename.name.clone()));
        }

        if version != filename.version && version != filename.version.clone().without_local() {
            return Err(Error::MismatchedVersion(version, filename.version.clone()));
        }
    }

    // We're going step by step though
    // https://packaging.python.org/en/latest/specifications/binary-distribution-format/#installing-a-wheel-distribution-1-0-py32-none-any-whl
    // > 1.a Parse distribution-1.0.dist-info/WHEEL.
    // > 1.b Check that installer is compatible with Wheel-Version. Warn if minor version is greater, abort if major version is greater.
    // > 1.c If Root-Is-Purelib == ‘true’, unpack archive into purelib (site-packages).
    // > 1.d Else unpack archive into platlib (site-packages).
    let validated_wheel = ValidatedWheel::new(layout, wheel, &dist_info_prefix)?;
    trace!(?name, "Extracting wheel files");
    link_wheel_files(link_mode, site_packages, &validated_wheel, state, filename)?;
    trace!(?name, "Extracted wheel files");

    // Read the RECORD file.
    let mut record_file = File::open(wheel.join(format!("{dist_info_prefix}.dist-info/RECORD")))?;
    let mut record = read_record(&mut record_file)?;

    let (console_scripts, gui_scripts) =
        parse_scripts(wheel, &dist_info_prefix, None, layout.python_version.1)?;

    if console_scripts.is_empty() && gui_scripts.is_empty() {
        trace!(?name, "No entrypoints");
    } else {
        trace!(?name, "Writing entrypoints");

        fs_err::create_dir_all(&layout.scheme.scripts)?;
        write_script_entrypoints(
            layout,
            relocatable,
            site_packages,
            &console_scripts,
            &mut record,
            false,
        )?;
        write_script_entrypoints(
            layout,
            relocatable,
            site_packages,
            &gui_scripts,
            &mut record,
            true,
        )?;
    }

    // 2.a Unpacked archive includes distribution-1.0.dist-info/ and (if there is data) distribution-1.0.data/.
    // 2.b Move each subtree of distribution-1.0.data/ onto its destination path. Each subdirectory of distribution-1.0.data/ is a key into a dict of destination directories, such as distribution-1.0.data/(purelib|platlib|headers|scripts|data). The initially supported paths are taken from distutils.command.install.
    let data_dir = site_packages.join(format!("{dist_info_prefix}.data"));
    if data_dir.is_dir() {
        install_data(
            layout,
            relocatable,
            site_packages,
            &data_dir,
            &name,
            &console_scripts,
            &gui_scripts,
            &mut record,
        )?;
        // 2.c If applicable, update scripts starting with #!python to point to the correct interpreter.
        // Script are unsupported through data
        // 2.e Remove empty distribution-1.0.data directory.
        remove_wheel_data_dir(data_dir)?;
    } else {
        trace!(?name, "No data");
    }

    if installer_metadata {
        trace!(?name, "Writing installer metadata");
        write_installer_metadata(
            site_packages,
            &dist_info_prefix,
            true,
            direct_url,
            cache_info,
            build_info,
            installer,
            &mut record,
        )?;
    }

    trace!(?name, "Writing record");
    write_record(site_packages, &dist_info_prefix, record)?;

    Ok(())
}

/// Remove an installed wheel's data directory, retrying transient filesystem errors.
///
/// Network filesystems can briefly retain `.nfs` files that prevent directory removal.
/// See: <https://github.com/astral-sh/uv/issues/12036>.
fn remove_wheel_data_dir(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    retry_wheel_data_cleanup(|| fs_err::remove_dir_all(path), std::thread::sleep)
}

fn retry_wheel_data_cleanup(
    mut remove: impl FnMut() -> io::Result<()>,
    mut sleep: impl FnMut(Duration),
) -> io::Result<()> {
    // Match the existing bounded file-operation retry budget: ten retries over about ten seconds.
    let mut delays = (0..10).map(|retry| Duration::from_millis(10 << retry));
    loop {
        match remove() {
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::ResourceBusy | io::ErrorKind::DirectoryNotEmpty
                ) =>
            {
                let Some(delay) = delays.next() else {
                    return Err(error);
                };
                sleep(delay);
            }
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as StdError;
    use std::fmt;
    use std::io;
    use std::sync::Arc;
    use std::time::Duration;

    use anyhow::{Context, Result, bail};
    use assert_fs::TempDir;

    use super::{remove_wheel_data_dir, retry_wheel_data_cleanup};

    #[derive(Debug)]
    struct CleanupError {
        attempt: usize,
        token: Arc<()>,
    }

    impl fmt::Display for CleanupError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "cleanup error {}", self.attempt)
        }
    }

    impl StdError for CleanupError {}

    fn cleanup_error(kind: io::ErrorKind, attempt: usize, token: &Arc<()>) -> io::Error {
        io::Error::new(
            kind,
            CleanupError {
                attempt,
                token: Arc::clone(token),
            },
        )
    }

    fn assert_original_error(
        error: &io::Error,
        kind: io::ErrorKind,
        attempt: usize,
        token: &Arc<()>,
    ) -> Result<()> {
        assert_eq!(error.kind(), kind);
        let marker = error
            .get_ref()
            .and_then(|source| source.downcast_ref::<CleanupError>())
            .context("terminal error lost its original marker")?;
        assert_eq!(marker.attempt, attempt);
        assert!(Arc::ptr_eq(&marker.token, token));
        assert_eq!(error.to_string(), format!("cleanup error {attempt}"));
        Ok(())
    }

    #[test]
    fn data_cleanup_retries_transient_errors() -> Result<()> {
        let directory = TempDir::new()?;
        let immediate = directory.path().join("immediate.data");
        fs_err::create_dir_all(immediate.join("nested"))?;
        fs_err::write(immediate.join("nested/owned.txt"), b"owned")?;
        remove_wheel_data_dir(&immediate)?;
        assert!(!immediate.try_exists()?);

        let retried = directory.path().join("retried.data");
        fs_err::create_dir_all(retried.join("nested"))?;
        fs_err::write(retried.join("nested/owned.txt"), b"owned")?;
        let mut attempts = 0;
        let mut delays = Vec::new();
        retry_wheel_data_cleanup(
            || {
                attempts += 1;
                match attempts {
                    1 => Err(io::ErrorKind::ResourceBusy.into()),
                    2 => Err(io::ErrorKind::DirectoryNotEmpty.into()),
                    _ => fs_err::remove_dir_all(&retried),
                }
            },
            |delay| delays.push(delay),
        )?;
        assert_eq!(attempts, 3);
        assert_eq!(
            delays,
            [Duration::from_millis(10), Duration::from_millis(20)]
        );
        assert!(!retried.try_exists()?);
        Ok(())
    }

    #[test]
    fn data_cleanup_preserves_terminal_errors() -> Result<()> {
        for kind in [
            io::ErrorKind::NotFound,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::Other,
        ] {
            let token = Arc::new(());
            let mut attempts = 0;
            let mut delays = Vec::new();
            let Err(error) = retry_wheel_data_cleanup(
                || {
                    attempts += 1;
                    Err(cleanup_error(kind, attempts, &token))
                },
                |delay| delays.push(delay),
            ) else {
                bail!("injected terminal cleanup error was ignored");
            };
            assert_eq!(attempts, 1);
            assert!(delays.is_empty());
            assert_original_error(&error, kind, 1, &token)?;
        }

        let token = Arc::new(());
        let mut attempts = 0;
        let mut delays = Vec::new();
        let Err(error) = retry_wheel_data_cleanup(
            || {
                attempts += 1;
                let kind = if attempts == 1 {
                    io::ErrorKind::ResourceBusy
                } else {
                    io::ErrorKind::PermissionDenied
                };
                Err(cleanup_error(kind, attempts, &token))
            },
            |delay| delays.push(delay),
        ) else {
            bail!("terminal cleanup error after a retry was ignored");
        };
        assert_eq!(attempts, 2);
        assert_eq!(delays, [Duration::from_millis(10)]);
        assert_original_error(&error, io::ErrorKind::PermissionDenied, 2, &token)?;
        Ok(())
    }

    #[test]
    fn data_cleanup_stops_after_retry_budget() -> Result<()> {
        let token = Arc::new(());
        let mut attempts: usize = 0;
        let mut delays = Vec::new();
        let Err(error) = retry_wheel_data_cleanup(
            || {
                attempts += 1;
                let kind = if attempts.is_multiple_of(2) {
                    io::ErrorKind::DirectoryNotEmpty
                } else {
                    io::ErrorKind::ResourceBusy
                };
                Err(cleanup_error(kind, attempts, &token))
            },
            |delay| delays.push(delay),
        ) else {
            bail!("cleanup retry budget was not enforced");
        };
        assert_eq!(attempts, 11);
        assert_eq!(
            delays,
            [10, 20, 40, 80, 160, 320, 640, 1280, 2560, 5120].map(Duration::from_millis)
        );
        assert_eq!(
            delays.iter().copied().sum::<Duration>(),
            Duration::from_millis(10230)
        );
        assert_original_error(&error, io::ErrorKind::ResourceBusy, 11, &token)?;
        Ok(())
    }
}
