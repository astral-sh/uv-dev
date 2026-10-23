use std::path::{Path, PathBuf};

/// Find the sibling `uv` executable, preferring the launcher's matching suffix.
pub(super) fn get_uv_path(
    current_exe_parent: &Path,
    suffix: Option<&str>,
) -> std::io::Result<PathBuf> {
    // First try to find a matching suffixed `uv`, e.g. `uv@1.2.3(.exe)`
    let uv_with_suffix = suffix.map(|suffix| current_exe_parent.join(format!("uv{suffix}")));
    if let Some(uv_with_suffix) = &uv_with_suffix {
        #[expect(clippy::print_stderr, reason = "printing a very rare warning")]
        match uv_with_suffix.try_exists() {
            Ok(true) => return Ok(uv_with_suffix.to_owned()),
            Ok(false) => { /* definitely not there, proceed to fallback */ }
            Err(err) => {
                // We don't know if `uv@1.2.3` exists, something errored when checking.
                // We *could* blindly use `uv@1.2.3` in this case, as the code below does, however
                // in this extremely narrow corner case it's *probably* better to default to `uv`,
                // since we don't want to mess up existing users who weren't using suffixes?
                eprintln!(
                    "warning: failed to determine if `{}` exists, trying `uv` instead: {err}",
                    uv_with_suffix.display()
                );
            }
        }
    }

    // Then just look for good ol' `uv`
    let uv = current_exe_parent.join(format!("uv{}", std::env::consts::EXE_SUFFIX));
    // If we are sure the `uv` binary does not exist, display a clearer error message.
    // If we're not certain if uv exists (try_exists == Err), keep going and hope it works.
    if matches!(uv.try_exists(), Ok(false)) {
        let message = if let Some(uv_with_suffix) = uv_with_suffix {
            format!(
                "Could not find the `uv` binary at either of:\n  {}\n  {}",
                uv_with_suffix.display(),
                uv.display(),
            )
        } else {
            format!("Could not find the `uv` binary at: {}", uv.display())
        };
        Err(std::io::Error::new(std::io::ErrorKind::NotFound, message))
    } else {
        Ok(uv)
    }
}

#[cfg(test)]
mod tests {
    use fs_err as fs;

    use super::get_uv_path;

    #[test]
    fn prefers_matching_suffix() -> std::io::Result<()> {
        let directory = tempfile::TempDir::new()?;
        let plain = directory
            .path()
            .join(format!("uv{}", std::env::consts::EXE_SUFFIX));
        let suffix = format!("@1.2.3{}", std::env::consts::EXE_SUFFIX);
        let suffixed = directory.path().join(format!("uv{suffix}"));
        fs::write(&plain, [])?;
        fs::write(&suffixed, [])?;
        assert_eq!(get_uv_path(directory.path(), Some(&suffix))?, suffixed);
        Ok(())
    }

    #[test]
    fn falls_back_to_unsuffixed_binary() -> std::io::Result<()> {
        let directory = tempfile::TempDir::new()?;
        let plain = directory
            .path()
            .join(format!("uv{}", std::env::consts::EXE_SUFFIX));
        fs::write(&plain, [])?;
        assert_eq!(get_uv_path(directory.path(), Some("@missing"))?, plain);
        assert_eq!(get_uv_path(directory.path(), None)?, plain);
        Ok(())
    }

    #[test]
    fn reports_missing_binary() -> std::io::Result<()> {
        let directory = tempfile::TempDir::new()?;
        let error = get_uv_path(directory.path(), Some("@missing"))
            .expect_err("both candidates are absent");
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        Ok(())
    }
}
