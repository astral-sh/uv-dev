use std::str::FromStr;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyTuple;
use uv_normalize::PackageName;
use uv_pep440::{Version, VersionSpecifiers};
use uv_pep508::{Requirement, VerbatimUrl, VersionOrUrl};

/// A PEP 440 version parsed and compared by uv.
#[pyclass(
    frozen,
    eq,
    ord,
    hash,
    skip_from_py_object,
    module = "uv_dev",
    name = "Version"
)]
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct PyVersion {
    version: Version,
}

#[pymethods]
impl PyVersion {
    #[new]
    fn new(value: &str) -> PyResult<Self> {
        Version::from_str(value)
            .map(|version| Self { version })
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    fn __str__(&self) -> String {
        self.version.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Version(\"{}\")", self.version)
    }

    #[getter]
    fn release<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.version.release().iter())
    }

    #[getter]
    fn is_prerelease(&self) -> bool {
        self.version.any_prerelease()
    }
}

/// PEP 440 version comparisons, without resolver prerelease selection policy.
#[pyclass(
    frozen,
    skip_from_py_object,
    module = "uv_dev",
    name = "VersionSpecifiers"
)]
#[derive(Clone)]
pub(crate) struct PyVersionSpecifiers {
    specifiers: VersionSpecifiers,
}

#[pymethods]
impl PyVersionSpecifiers {
    #[new]
    fn new(value: &str) -> PyResult<Self> {
        VersionSpecifiers::from_str(value)
            .map(|specifiers| Self { specifiers })
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    fn __str__(&self) -> String {
        self.specifiers.to_string()
    }

    fn __contains__(&self, version: &PyVersion) -> bool {
        self.specifiers.contains(&version.version)
    }
}

/// A dependency requirement parsed by uv's PEP 508 implementation.
#[pyclass(frozen, module = "uv_dev", name = "Requirement")]
pub(crate) struct PyRequirement {
    requirement: Requirement<VerbatimUrl>,
}

#[pymethods]
impl PyRequirement {
    #[new]
    fn new(value: &str) -> PyResult<Self> {
        Requirement::from_str(value)
            .map(|requirement| Self { requirement })
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    fn __str__(&self) -> String {
        self.requirement.to_string()
    }

    #[getter]
    fn name(&self) -> &str {
        self.requirement.name.as_ref()
    }

    #[getter]
    fn extras<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.requirement.extras.iter().map(AsRef::<str>::as_ref))
    }

    #[getter]
    fn specifiers(&self) -> Option<PyVersionSpecifiers> {
        match &self.requirement.version_or_url {
            Some(VersionOrUrl::VersionSpecifier(specifiers)) => Some(PyVersionSpecifiers {
                specifiers: specifiers.clone(),
            }),
            Some(VersionOrUrl::Url(_)) | None => None,
        }
    }

    #[getter]
    fn url(&self) -> Option<String> {
        match &self.requirement.version_or_url {
            Some(VersionOrUrl::Url(url)) => Some(url.to_string()),
            Some(VersionOrUrl::VersionSpecifier(_)) | None => None,
        }
    }

    #[getter]
    fn marker(&self) -> Option<String> {
        self.requirement.marker.try_to_string()
    }
}

/// Validate and normalize a distribution name with uv's implementation.
#[pyfunction]
pub(crate) fn normalize_package_name(value: &str) -> PyResult<String> {
    PackageName::from_str(value)
        .map(|name| name.to_string())
        .map_err(|error| PyValueError::new_err(error.to_string()))
}
