//! Installation of unpacked wheels into a Python environment.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use fs_err::File;
use tracing::{instrument, trace};

use uv_distribution_filename::WheelFilename;
use uv_pep440::Version;
use uv_pypi_types::{DirectUrl, Metadata10};

use crate::linker::{InstallState, LinkMode, link_wheel_files};
use crate::wheel::{
    LibKind, ValidatedWheel, WheelFile, dist_info_file, find_dist_info, install_data,
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
    let wheel_file_path = dist_info_file(wheel, &dist_info_prefix, "WHEEL");
    let wheel_text = fs_err::read_to_string(wheel_file_path)?;
    let site_packages = match WheelFile::parse(&wheel_text)?.lib_kind() {
        LibKind::Pure => &layout.scheme.purelib,
        LibKind::Plat => &layout.scheme.platlib,
    };
    Ok((dist_info_prefix, site_packages))
}

/// Install the given unpacked wheel into a Python environment.
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
    let metadata = fs_err::read(dist_info_file(wheel, &dist_info_prefix, "METADATA"))?;
    let Metadata10 { name, version } = Metadata10::parse_pkg_info(&metadata)
        .map_err(|err| Error::InvalidWheel(err.to_string()))?;

    let version = Version::from_str(&version)?;

    // Validate the wheel name and version.
    if !uv_flags::contains(uv_flags::EnvironmentFlags::SKIP_WHEEL_FILENAME_CHECK) {
        if name != filename.name {
            return Err(Error::MismatchedName(name, filename.name.clone()));
        }

        if !filename.matches_version(&version) {
            return Err(Error::MismatchedVersion(version, filename.version.clone()));
        }
    }

    // Parse `RECORD` and entry points and validate script destinations before linking wheel files.
    let validated_wheel = ValidatedWheel::new(layout, wheel, &dist_info_prefix)?;
    let record_file = File::open(dist_info_file(wheel, &dist_info_prefix, "RECORD"))?;
    let mut record = read_record(record_file)?;
    let entrypoints = parse_scripts(wheel, &dist_info_prefix, layout.python_version.1)?;

    trace!(?name, "Extracting wheel files");
    link_wheel_files(link_mode, site_packages, &validated_wheel, state, filename)?;
    trace!(?name, "Extracted wheel files");

    if entrypoints.is_empty() {
        trace!(?name, "No entrypoints");
    } else {
        trace!(?name, "Writing entrypoints");

        fs_err::create_dir_all(&layout.scheme.scripts)?;
        for (scripts, is_gui) in [
            (&entrypoints.console_scripts, false),
            (&entrypoints.gui_scripts, true),
        ] {
            write_script_entrypoints(
                layout,
                relocatable,
                site_packages,
                scripts,
                &mut record,
                is_gui,
            )?;
        }
    }

    // Move wheel data into the interpreter's installation scheme.
    let data_dir = site_packages.join(format!("{dist_info_prefix}.data"));
    if data_dir.is_dir() {
        install_data(
            layout,
            relocatable,
            site_packages,
            &data_dir,
            &name,
            &entrypoints,
            &mut record,
        )?;
        fs_err::remove_dir_all(data_dir)?;
    } else {
        trace!(?name, "No data");
    }

    if installer_metadata {
        trace!(?name, "Writing installer metadata");
        write_installer_metadata(
            site_packages,
            &dist_info_prefix,
            direct_url,
            cache_info,
            build_info,
            installer,
            &mut record,
        )?;
    }

    trace!(?name, "Writing record");
    let record_path = dist_info_file(site_packages, &dist_info_prefix, "RECORD");
    write_record(&record_path, record)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use assert_fs::prelude::*;
    use uv_distribution_filename::WheelFilename;
    use uv_pypi_types::Scheme;

    use super::install_wheel;
    use crate::{Error, InstallState, Layout, LinkMode};

    #[test]
    fn invalid_entry_points_leave_environment_untouched() -> anyhow::Result<()> {
        let temp = assert_fs::TempDir::new()?;
        let wheel = temp.child("wheel");
        wheel.child("example/__init__.py").touch()?;
        let dist_info = wheel.child("example-1.0.0.dist-info");
        dist_info
            .child("METADATA")
            .write_str("Metadata-Version: 2.1\nName: example\nVersion: 1.0.0\n")?;
        dist_info
            .child("WHEEL")
            .write_str("Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n")?;
        dist_info
            .child("entry_points.txt")
            .write_str("[console_scripts]\nbroken = missing_colon\n")?;
        dist_info.child("RECORD").write_str(concat!(
            "example/__init__.py,,\n",
            "example-1.0.0.dist-info/METADATA,,\n",
            "example-1.0.0.dist-info/WHEEL,,\n",
            "example-1.0.0.dist-info/entry_points.txt,,\n",
            "example-1.0.0.dist-info/RECORD,,\n",
        ))?;

        let venv = temp.child("venv");
        let site_packages = venv.child("site-packages");
        site_packages.create_dir_all()?;
        let layout = Layout {
            sys_executable: venv.path().join("bin/python"),
            python_version: (3, 12),
            os_name: "posix".to_owned(),
            scheme: Scheme {
                purelib: site_packages.to_path_buf(),
                platlib: site_packages.to_path_buf(),
                scripts: venv.path().join("bin"),
                data: venv.to_path_buf(),
                include: venv.path().join("include"),
            },
        };
        let filename: WheelFilename = "example-1.0.0-py3-none-any.whl".parse()?;
        let error = install_wheel::<(), ()>(
            &layout,
            false,
            wheel.path(),
            &filename,
            None,
            None,
            None,
            None,
            true,
            LinkMode::Copy,
            &InstallState::default(),
        )
        .unwrap_err();

        assert!(
            matches!(error, Error::InvalidWheel(message) if message.contains("invalid console script"))
        );
        assert!(!site_packages.child("example").exists());
        assert!(!site_packages.child("example-1.0.0.dist-info").exists());
        Ok(())
    }
}
