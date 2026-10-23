use std::collections::BTreeMap;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use uv_python_types::PythonDownloadKey;

/// Convert catalog entries to canonical installation keys, rejecting invalid platform,
/// variant, and version fields.
#[pyfunction]
fn python_download_keys(metadata: &str) -> PyResult<BTreeMap<String, String>> {
    let downloads: BTreeMap<String, PythonDownloadKey> =
        serde_json::from_str(metadata).map_err(|error| PyValueError::new_err(error.to_string()))?;
    downloads
        .into_iter()
        .map(|(name, download)| {
            download
                .installation_key()
                .map(|key| (name.clone(), key.to_string()))
                .map_err(|error| {
                    PyValueError::new_err(format!("Invalid download `{name}`: {error}"))
                })
        })
        .collect()
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(python_download_keys, module)?)?;
    Ok(())
}
