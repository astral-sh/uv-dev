//! All-or-none opt-in settings for mutation measurements.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

pub(super) const INPUT_NAMES: [&str; 3] = [
    "UV_BENCH_UNLINK_SCRATCH",
    "UV_BENCH_WHEEL_PATH",
    "UV_BENCH_WHEEL_SHA256",
];

#[derive(Debug)]
pub(super) struct Inputs {
    pub(super) scratch: PathBuf,
    pub(super) wheel: PathBuf,
    pub(super) sha256: String,
}

pub(super) fn read(settings: [Option<OsString>; 3]) -> Result<Option<Inputs>> {
    let [scratch, wheel, sha256] = settings;
    let (scratch, wheel, sha256) = match (scratch, wheel, sha256) {
        (None, None, None) => return Ok(None),
        (Some(scratch), Some(wheel), Some(sha256)) => (scratch, wheel, sha256),
        _ => bail!("set all of {} together", INPUT_NAMES.join(", ")),
    };
    ensure!(
        !scratch.is_empty() && !wheel.is_empty() && !sha256.is_empty(),
        "wheel-unlink input settings must not be empty"
    );
    let sha256 = sha256
        .into_string()
        .map_err(|_| anyhow::anyhow!("wheel SHA-256 must be UTF-8"))?;
    ensure!(
        sha256.len() == 64 && sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "UV_BENCH_WHEEL_SHA256 must contain 64 hexadecimal digits"
    );
    let scratch = fs_err::canonicalize(Path::new(&scratch))
        .context("wheel-unlink scratch directory must already exist")?;
    ensure!(
        fs_err::metadata(&scratch)?.is_dir(),
        "wheel-unlink scratch path is not a directory"
    );
    let wheel = PathBuf::from(wheel);
    ensure!(
        fs_err::metadata(&wheel)
            .context("wheel-unlink source wheel must already exist")?
            .is_file(),
        "wheel-unlink source path is not a file"
    );
    Ok(Some(Inputs {
        scratch,
        wheel,
        sha256,
    }))
}
