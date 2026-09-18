//! Like `wheel.rs`, but for installing wheels that have already been unzipped, rather than
//! reading from a zip file.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use fs_err::File;
use tracing::{instrument, trace};

use uv_distribution_filename::WheelFilename;
use uv_pep440::Version;
use uv_pypi_types::{DirectUrl, Metadata10};

use crate::linker::{InstallState, LinkMode, link_wheel_files};
use crate::wheel::{
    LibKind, ValidatedWheel, WheelFile, dist_info_metadata, find_dist_info, install_data,
    parse_scripts, read_record, wheel_entrypoint_paths, write_installer_metadata, write_record,
    write_script_entrypoints,
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

/// Return the script paths that an unpacked wheel will record after installation.
///
/// This does not write to the target environment. Script names use the same normalization and
/// `.data` relocation rules as wheel installation.
pub fn installed_entrypoint_paths(
    layout: &Layout,
    wheel: impl AsRef<Path>,
) -> Result<Vec<(String, PathBuf)>, Error> {
    let wheel = wheel.as_ref();
    let (dist_info_prefix, site_packages) = wheel_destination(layout, wheel)?;
    let metadata = dist_info_metadata(&dist_info_prefix, wheel)?;
    let metadata = Metadata10::parse_pkg_info(&metadata)
        .map_err(|err| Error::InvalidWheel(err.to_string()))?;
    wheel_entrypoint_paths(
        layout,
        wheel,
        &dist_info_prefix,
        &metadata.name,
        site_packages,
    )
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
    let validated_wheel =
        ValidatedWheel::new(layout, wheel, &dist_info_prefix, &name, site_packages)?;
    trace!(?name, "Extracting wheel files");
    link_wheel_files(link_mode, &validated_wheel, state, filename)?;
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
    let data_dir = validated_wheel.installed_data_dir();
    if data_dir.is_dir() {
        install_data(
            layout,
            relocatable,
            &validated_wheel,
            &name,
            &console_scripts,
            &gui_scripts,
            &mut record,
        )?;
        // 2.c If applicable, update scripts starting with #!python to point to the correct interpreter.
        // Script are unsupported through data
        // 2.e Remove empty distribution-1.0.data directory.
        fs_err::remove_dir_all(data_dir)?;
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::str::FromStr;

    use anyhow::Result;
    use assert_fs::prelude::*;
    use uv_distribution_filename::WheelFilename;
    use uv_preview::Preview;
    use uv_pypi_types::Scheme;

    use crate::{InstallState, Layout, LinkMode, read_record};

    use super::{install_wheel, installed_entrypoint_paths};

    #[test]
    fn projected_entrypoints_match_the_installed_record() -> Result<()> {
        let temp = assert_fs::TempDir::new()?;
        let wheel = temp.child("wheel");
        let root = temp.child("environment");
        let scripts = if cfg!(windows) { "Scripts" } else { "bin" };
        let layout = Layout {
            sys_executable: root.child(scripts).child("python").to_path_buf(),
            python_version: (3, 13),
            os_name: if cfg!(windows) { "nt" } else { "posix" }.to_string(),
            scheme: Scheme {
                purelib: root.child("site-packages").to_path_buf(),
                platlib: root.child("site-packages").to_path_buf(),
                scripts: root.child(scripts).to_path_buf(),
                data: root.to_path_buf(),
                include: root.child("include").to_path_buf(),
            },
        };
        let relocated = format!("demo-1.0.0.data/data/{scripts}/relocated");
        let files = [
            (
                "demo-1.0.0.dist-info/METADATA",
                "Metadata-Version: 2.1\nName: demo\nVersion: 1.0.0\n",
            ),
            (
                "demo-1.0.0.dist-info/WHEEL",
                "Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
            ),
            (
                "demo-1.0.0.dist-info/entry_points.txt",
                "[console_scripts]\nconsole.py = demo:main\n[gui_scripts]\ngui = demo:main\n",
            ),
            ("demo.py", "def main(): pass\n"),
            ("demo-1.0.0.data/scripts/console.py", "ignored wrapper"),
            ("demo-1.0.0.data/scripts/gui-script.py", "ignored wrapper"),
            ("demo-1.0.0.data/scripts/plain", "ordinary script"),
            (relocated.as_str(), "relocated script"),
        ];
        let mut record = String::new();
        for (path, contents) in files {
            wheel.child(path).write_str(contents)?;
            record.push_str(path);
            record.push_str(",,\n");
        }
        record.push_str("demo-1.0.0.dist-info/RECORD,,\n");
        wheel
            .child("demo-1.0.0.dist-info/RECORD")
            .write_str(&record)?;

        let mut projected = installed_entrypoint_paths(&layout, wheel.path())?;
        install_wheel::<(), ()>(
            &layout,
            false,
            wheel.path(),
            &WheelFilename::from_str("demo-1.0.0-py3-none-any.whl")?,
            None,
            None,
            None,
            None,
            false,
            LinkMode::Copy,
            &InstallState::new(Preview::default()),
        )?;
        let relative_scripts = pathdiff::diff_paths(&layout.scheme.scripts, &layout.scheme.purelib)
            .expect("paths share a root");
        let mut installed = read_record(fs_err::File::open(
            layout.scheme.purelib.join("demo-1.0.0.dist-info/RECORD"),
        )?)?
        .into_iter()
        .filter_map(|entry| {
            let path = PathBuf::from(&entry.path);
            let relative = path.strip_prefix(&relative_scripts).ok()?;
            let name = path.file_name()?.to_str()?.to_string();
            Some((name, layout.scheme.scripts.join(relative)))
        })
        .collect::<Vec<_>>();
        projected.sort_unstable();
        installed.sort_unstable();
        assert_eq!(projected, installed);
        Ok(())
    }
}
