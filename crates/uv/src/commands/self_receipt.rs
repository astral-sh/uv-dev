//! Legacy standalone receipt discovery shared by self-management commands.

use std::path::PathBuf;

use anyhow::Result;

const AXOUPDATER_CONFIG_PATH: &str = "AXOUPDATER_CONFIG_PATH";
const AXOUPDATER_CONFIG_WORKING_DIR: &str = "AXOUPDATER_CONFIG_WORKING_DIR";

/// Find the receipt path for the given app name. Returns `Ok(None)` if the receipt
/// definitely doesn't exist.
pub(super) fn find_receipt_path(app_name: &str) -> Result<Option<PathBuf>> {
    for prefix in receipt_prefixes(app_name)? {
        let receipt_path = prefix.join(format!("{app_name}-receipt.json"));
        if receipt_path.exists() {
            return Ok(Some(receipt_path));
        }
    }
    Ok(None)
}

/// List all possible locations for the receipt file for a given app name,
/// taking into account axoupdater-specific environment variable overrides.
fn receipt_prefixes(app_name: &str) -> Result<Vec<PathBuf>> {
    if std::env::var_os(AXOUPDATER_CONFIG_WORKING_DIR).is_some() {
        return Ok(vec![std::env::current_dir()?]);
    }

    if let Some(path) = std::env::var_os(AXOUPDATER_CONFIG_PATH) {
        return Ok(vec![PathBuf::from(path)]);
    }

    let mut prefixes = Vec::new();

    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        let path = PathBuf::from(path).join(app_name);
        if path.exists() {
            prefixes.push(path);
        }
    }

    #[cfg(windows)]
    if let Some(path) = std::env::var_os("LOCALAPPDATA") {
        prefixes.push(PathBuf::from(path).join(app_name));
    }

    #[cfg(not(windows))]
    if let Ok(path) = etcetera::home_dir() {
        prefixes.push(path.join(".config").join(app_name));
    }

    Ok(prefixes)
}
