use std::path::Path;

use serde::Serialize;

use uv_fs::PortablePathBuf;
use uv_python::PythonEnvironment;
use uv_resolver::PythonReport;

#[derive(Serialize, Debug, Default)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
enum SchemaVersion {
    /// An unstable, experimental schema.
    #[default]
    Preview,
}

#[derive(Serialize, Debug, Default)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub(crate) struct SchemaReport {
    /// The version of the schema.
    version: SchemaVersion,
}

#[derive(Serialize, Debug)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub(crate) struct EnvironmentReport {
    /// The path to the environment.
    path: PortablePathBuf,
    /// The Python interpreter for the environment.
    python: PythonReport,
}

impl From<&PythonEnvironment> for EnvironmentReport {
    fn from(env: &PythonEnvironment) -> Self {
        Self {
            python: PythonReport::from(env.interpreter()),
            path: env.root().into(),
        }
    }
}

impl EnvironmentReport {
    /// Return the Python interpreter for the environment.
    pub(crate) fn python(&self) -> &PythonReport {
        &self.python
    }

    /// Return the path to the environment.
    pub(crate) fn path(&self) -> &Path {
        self.path.as_ref()
    }

    /// Set the path for this environment report.
    #[must_use]
    pub(crate) fn with_path(mut self, path: PortablePathBuf) -> Self {
        if let Ok(python_path) = self.python.path().strip_prefix(self.path) {
            let new_path = path.as_ref().to_path_buf().join(python_path);
            self.python = self.python.with_path(new_path.as_path().into());
        }
        self.path = path;
        self
    }
}
