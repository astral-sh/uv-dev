use anyhow::{Context, Result, bail};
use std::env;
use std::io::Write;
use std::path::Path;
use uv_cli_types::exit::ExitStatus;
use uv_static::TarBackend;

/// PEP 517 hook to build a source distribution.
pub fn build_sdist(sdist_directory: &Path) -> Result<ExitStatus> {
    let filename = uv_build_backend::build_source_dist(
        &env::current_dir()?,
        sdist_directory,
        uv_version::version(),
        false,
        TarBackend::from_env(),
    )?;
    // Tell the build frontend about the name of the artifact we built
    writeln!(&mut std::io::stdout(), "{filename}").context("stdout is closed")?;
    Ok(ExitStatus::Success)
}

/// PEP 517 hook to build a wheel.
pub fn build_wheel(
    wheel_directory: &Path,
    metadata_directory: Option<&Path>,
) -> Result<ExitStatus> {
    let filename = uv_build_backend::build_wheel(
        &env::current_dir()?,
        wheel_directory,
        metadata_directory,
        uv_version::version(),
        false,
    )?;
    // Tell the build frontend about the name of the artifact we built
    writeln!(&mut std::io::stdout(), "{filename}").context("stdout is closed")?;
    Ok(ExitStatus::Success)
}

/// PEP 660 hook to build a wheel.
pub fn build_editable(
    wheel_directory: &Path,
    metadata_directory: Option<&Path>,
) -> Result<ExitStatus> {
    let filename = uv_build_backend::build_editable(
        &env::current_dir()?,
        wheel_directory,
        metadata_directory,
        uv_version::version(),
        false,
    )?;
    // Tell the build frontend about the name of the artifact we built
    writeln!(&mut std::io::stdout(), "{filename}").context("stdout is closed")?;
    Ok(ExitStatus::Success)
}

/// Not used from Python code, exists for symmetry with PEP 517.
pub fn get_requires_for_build_sdist() -> Result<ExitStatus> {
    bail!("uv does not support extra requires")
}

/// Not used from Python code, exists for symmetry with PEP 517.
pub fn get_requires_for_build_wheel() -> Result<ExitStatus> {
    bail!("uv does not support extra requires")
}

/// PEP 517 hook to just emit metadata through `.dist-info`.
pub fn prepare_metadata_for_build_wheel(metadata_directory: &Path) -> Result<ExitStatus> {
    let filename = uv_build_backend::metadata(
        &env::current_dir()?,
        metadata_directory,
        uv_version::version(),
    )?;
    // Tell the build frontend about the name of the artifact we built
    writeln!(&mut std::io::stdout(), "{filename}").context("stdout is closed")?;
    Ok(ExitStatus::Success)
}

/// Not used from Python code, exists for symmetry with PEP 660.
pub fn get_requires_for_build_editable() -> Result<ExitStatus> {
    bail!("uv does not support extra requires")
}

/// PEP 660 hook to just emit metadata through `.dist-info`.
pub fn prepare_metadata_for_build_editable(metadata_directory: &Path) -> Result<ExitStatus> {
    let filename = uv_build_backend::metadata(
        &env::current_dir()?,
        metadata_directory,
        uv_version::version(),
    )?;
    // Tell the build frontend about the name of the artifact we built
    writeln!(&mut std::io::stdout(), "{filename}").context("stdout is closed")?;
    Ok(ExitStatus::Success)
}
