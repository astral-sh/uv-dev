use std::collections::BTreeMap;
use std::str::FromStr;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use uv_normalize::ExtraName;
use uv_pep508::{MarkerEnvironment, MarkerTree};

/// A marker parsed, simplified, and evaluated by uv.
#[pyclass(
    frozen,
    eq,
    hash,
    skip_from_py_object,
    module = "uv_dev",
    name = "Marker"
)]
#[derive(Clone, Eq, PartialEq, Hash)]
pub(crate) struct PyMarker {
    marker: MarkerTree,
}

#[pymethods]
impl PyMarker {
    #[new]
    fn new(value: &str) -> PyResult<Self> {
        MarkerTree::from_str(value)
            .map(|marker| Self { marker })
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    /// The simplified expression, or `None` when the marker simplifies to true.
    #[getter]
    fn normalized(&self) -> Option<String> {
        self.marker.try_to_string()
    }

    #[getter]
    fn is_true(&self) -> bool {
        self.marker.is_true()
    }

    #[getter]
    fn is_false(&self) -> bool {
        self.marker.is_false()
    }

    /// Evaluate against a complete PEP 508 environment, without host defaults.
    #[pyo3(signature = (environment, *, extras = Vec::new()))]
    fn evaluate(
        &self,
        environment: BTreeMap<String, String>,
        extras: Vec<String>,
    ) -> PyResult<bool> {
        evaluate_marker(self.marker, environment, extras)
    }
}

pub(crate) fn evaluate_marker(
    marker: MarkerTree,
    environment: BTreeMap<String, String>,
    extras: Vec<String>,
) -> PyResult<bool> {
    let environment = serde_json::to_value(environment)
        .and_then(serde_json::from_value::<MarkerEnvironment>)
        .map_err(|error| PyValueError::new_err(format!("Invalid marker environment: {error}")))?;
    let extras = extras
        .into_iter()
        .map(|extra| {
            ExtraName::from_str(&extra).map_err(|error| PyValueError::new_err(error.to_string()))
        })
        .collect::<PyResult<Vec<_>>>()?;
    Ok(marker.evaluate(&environment, &extras))
}
