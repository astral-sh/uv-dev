#[cfg(feature = "schemars")]
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fmt::{Display, Formatter};
use std::ops::Deref;
use std::str::FromStr;

use thiserror::Error;
use uv_pep440::Version;
use uv_pep508::{MarkerEnvironment, StringVersion};
use uv_static::EnvVars;

use crate::implementation::ImplementationName;
use crate::{PythonBuildName, PythonBuildRequest};

#[derive(Error, Debug)]
pub enum BuildRevisionError {
    #[error("`{0}` is not valid unicode: {1:?}")]
    NotUnicode(&'static str, OsString),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PythonVersion(StringVersion);

impl From<StringVersion> for PythonVersion {
    fn from(version: StringVersion) -> Self {
        Self(version)
    }
}

impl Deref for PythonVersion {
    type Target = StringVersion;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromStr for PythonVersion {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let version = StringVersion::from_str(s)
            .map_err(|err| format!("Python version `{s}` could not be parsed: {err}"))?;
        if version.is_dev() {
            return Err(format!("Python version `{s}` is a development release"));
        }
        if version.is_local() {
            return Err(format!("Python version `{s}` is a local version"));
        }
        if version.epoch() != 0 {
            return Err(format!("Python version `{s}` has a non-zero epoch"));
        }
        if let Some(major) = version.release().first()
            && u8::try_from(*major).is_err()
        {
            return Err(format!(
                "Python version `{s}` has an invalid major version ({major})"
            ));
        }
        if let Some(minor) = version.release().get(1)
            && u8::try_from(*minor).is_err()
        {
            return Err(format!(
                "Python version `{s}` has an invalid minor version ({minor})"
            ));
        }
        if let Some(patch) = version.release().get(2)
            && u8::try_from(*patch).is_err()
        {
            return Err(format!(
                "Python version `{s}` has an invalid patch version ({patch})"
            ));
        }

        Ok(Self(version))
    }
}

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for PythonVersion {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("PythonVersion")
    }

    fn json_schema(_generator: &mut schemars::generate::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": r"^3\.\d+(\.\d+)?$",
            "description": "A Python version specifier, e.g. `3.11` or `3.12.4`."
        })
    }
}

impl<'de> serde::Deserialize<'de> for PythonVersion {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = PythonVersion;

            fn expecting(&self, f: &mut Formatter) -> std::fmt::Result {
                f.write_str("a string")
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                PythonVersion::from_str(v).map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_str(Visitor)
    }
}

impl Display for PythonVersion {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl PythonVersion {
    /// Return a [`MarkerEnvironment`] compatible with the given [`PythonVersion`], based on
    /// a base [`MarkerEnvironment`].
    ///
    /// The returned [`MarkerEnvironment`] will preserve the base environment's platform markers,
    /// but override its Python version markers.
    pub fn markers(&self, base: MarkerEnvironment) -> MarkerEnvironment {
        let mut markers = base;

        // Ex) `implementation_version == "3.12.0"`
        if markers.implementation_name() == "cpython" {
            let python_full_version = self.python_full_version();
            markers = markers.with_implementation_version(StringVersion {
                // Retain the verbatim representation, provided by the user.
                string: self.0.to_string(),
                version: python_full_version,
            });
        }

        // Ex) `python_full_version == "3.12.0"`
        let python_full_version = self.python_full_version();
        markers = markers.with_python_full_version(StringVersion {
            // Retain the verbatim representation, provided by the user.
            string: self.0.to_string(),
            version: python_full_version,
        });

        // Ex) `python_version == "3.12"`
        let python_version = self.python_version();
        markers = markers.with_python_version(StringVersion {
            string: python_version.to_string(),
            version: python_version,
        });

        markers
    }

    /// Return the `python_version` marker corresponding to this Python version.
    ///
    /// This should include exactly a major and minor version, but no patch version.
    ///
    /// Ex) `python_version == "3.12"`
    pub fn python_version(&self) -> Version {
        let major = self.release().first().copied().unwrap_or(0);
        let minor = self.release().get(1).copied().unwrap_or(0);
        Version::new([major, minor])
    }

    /// Return the `python_full_version` marker corresponding to this Python version.
    ///
    /// This should include exactly a major, minor, and patch version (even if it's zero), along
    /// with any pre-release or post-release information.
    ///
    /// Ex) `python_full_version == "3.12.0b1"`
    pub fn python_full_version(&self) -> Version {
        let major = self.release().first().copied().unwrap_or(0);
        let minor = self.release().get(1).copied().unwrap_or(0);
        let patch = self.release().get(2).copied().unwrap_or(0);
        Version::new([major, minor, patch])
            .with_pre(self.0.pre())
            .with_post(self.0.post())
    }

    /// Return the full parsed Python version.
    pub fn version(&self) -> &Version {
        &self.0.version
    }

    /// Return the full parsed Python version.
    pub(crate) fn into_version(self) -> Version {
        self.0.version
    }

    /// Return the major version of this Python version.
    pub fn major(&self) -> u8 {
        u8::try_from(self.0.release().first().copied().unwrap_or(0)).expect("invalid major version")
    }

    /// Return the minor version of this Python version.
    pub fn minor(&self) -> u8 {
        u8::try_from(self.0.release().get(1).copied().unwrap_or(0)).expect("invalid minor version")
    }

    /// Return the patch version of this Python version, if set.
    pub fn patch(&self) -> Option<u8> {
        self.0
            .release()
            .get(2)
            .copied()
            .map(|patch| u8::try_from(patch).expect("invalid patch version"))
    }
}

/// Get the environment variable name for the build constraint for a given implementation.
pub(crate) fn python_build_revision_variable(implementation: ImplementationName) -> &'static str {
    match implementation {
        ImplementationName::CPython => EnvVars::UV_PYTHON_CPYTHON_BUILD,
        ImplementationName::PyPy => EnvVars::UV_PYTHON_PYPY_BUILD,
        ImplementationName::GraalPy => EnvVars::UV_PYTHON_GRAALPY_BUILD,
        ImplementationName::Pyodide => EnvVars::UV_PYTHON_PYODIDE_BUILD,
    }
}

/// Get the build revision number from the environment variable for a given implementation.
fn python_build_revision_from_env(
    implementation: ImplementationName,
) -> Result<Option<String>, BuildRevisionError> {
    let variable = python_build_revision_variable(implementation);

    build_revision_from_env(variable)
}

/// Get the build revision for an explicitly requested build name.
pub(crate) fn python_named_build_revision_from_env() -> Result<Option<String>, BuildRevisionError> {
    build_revision_from_env(EnvVars::UV_PYTHON_BUILD_REVISION)
}

pub(crate) fn build_revision_from_env(
    variable: &'static str,
) -> Result<Option<String>, BuildRevisionError> {
    let Some(revision_os) = env::var_os(variable) else {
        return Ok(None);
    };

    let revision = revision_os
        .into_string()
        .map_err(|raw| BuildRevisionError::NotUnicode(variable, raw))?;

    let trimmed = revision.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    Ok(Some(trimmed.to_string()))
}

/// Environment revision pins for either named builds or unnamed implementations.
#[derive(Debug)]
pub(crate) enum PythonBuildRevisionPins {
    Named(Option<String>),
    Unnamed(BTreeMap<ImplementationName, String>),
}

impl PythonBuildRevisionPins {
    /// Read only the variables applicable to the request. Without an implementation, read pins
    /// for every implementation so discovery and listing remain implementation-independent.
    pub(crate) fn from_env(
        build_request: Option<&PythonBuildRequest>,
        implementation: Option<ImplementationName>,
    ) -> Result<Self, BuildRevisionError> {
        if build_request.is_some_and(|request| request.build_name().is_some()) {
            return Ok(Self::Named(python_named_build_revision_from_env()?));
        }
        let mut revisions = BTreeMap::new();
        match implementation {
            Some(implementation) => {
                if let Some(revision) = python_build_revision_from_env(implementation)? {
                    revisions.insert(implementation, revision);
                }
            }
            None => {
                for implementation in ImplementationName::iter_all() {
                    if let Some(revision) = python_build_revision_from_env(implementation)? {
                        revisions.insert(implementation, revision);
                    }
                }
            }
        }
        Ok(Self::Unnamed(revisions))
    }

    /// Return the revision pin for this build, if applicable.
    ///
    /// Implementation-specific pins, such as `UV_PYTHON_CPYTHON_BUILD`, do not
    /// constrain named builds. Named-build pins do not constrain unnamed builds.
    pub(crate) fn get(
        &self,
        implementation: Option<ImplementationName>,
        build_name: Option<&PythonBuildName>,
    ) -> Option<&str> {
        match (self, build_name) {
            (Self::Named(revision), Some(_)) => revision.as_deref(),
            (Self::Unnamed(revisions), None) => implementation
                .and_then(|implementation| revisions.get(&implementation))
                .map(String::as_str),
            (Self::Named(_), None) | (Self::Unnamed(_), Some(_)) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::ffi::OsString;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;
    use std::str::FromStr;

    use uv_pep440::{Prerelease, PrereleaseKind, Version};
    #[cfg(unix)]
    use uv_static::EnvVars;

    #[cfg(unix)]
    use super::{BuildRevisionError, PythonBuildRevisionPins};
    use crate::PythonVersion;
    #[cfg(unix)]
    use crate::{ImplementationName, PythonBuildRequest};

    #[test]
    #[cfg(unix)]
    fn build_revision_pins_ignore_unrelated_variables() -> Result<(), BuildRevisionError> {
        let invalid = OsString::from_vec(vec![0xff]);
        let build_request = PythonBuildRequest::from_str("custom").expect("Valid build name");
        temp_env::with_vars(
            [
                (EnvVars::UV_PYTHON_BUILD_REVISION, Some(invalid.clone())),
                (
                    EnvVars::UV_PYTHON_CPYTHON_BUILD,
                    Some(OsString::from("cpython")),
                ),
                (EnvVars::UV_PYTHON_PYPY_BUILD, Some(invalid.clone())),
            ],
            || -> Result<(), BuildRevisionError> {
                let pins =
                    PythonBuildRevisionPins::from_env(None, Some(ImplementationName::CPython))?;
                assert_eq!(
                    pins.get(Some(ImplementationName::CPython), None),
                    Some("cpython")
                );
                assert_eq!(
                    pins.get(
                        Some(ImplementationName::CPython),
                        build_request.build_name()
                    ),
                    None
                );
                assert!(PythonBuildRevisionPins::from_env(None, None).is_err());
                assert!(PythonBuildRevisionPins::from_env(Some(&build_request), None).is_err());
                Ok(())
            },
        )?;
        temp_env::with_vars(
            [
                (
                    EnvVars::UV_PYTHON_BUILD_REVISION,
                    Some(OsString::from("named")),
                ),
                (EnvVars::UV_PYTHON_CPYTHON_BUILD, Some(invalid)),
            ],
            || -> Result<(), BuildRevisionError> {
                let pins = PythonBuildRevisionPins::from_env(Some(&build_request), None)?;
                assert_eq!(
                    pins.get(
                        Some(ImplementationName::CPython),
                        build_request.build_name()
                    ),
                    Some("named")
                );
                assert_eq!(pins.get(None, build_request.build_name()), Some("named"));
                assert_eq!(pins.get(Some(ImplementationName::CPython), None), None);
                Ok(())
            },
        )
    }

    #[test]
    fn python_markers() {
        let version = PythonVersion::from_str("3.11.0").expect("valid python version");
        assert_eq!(version.python_version(), Version::new([3, 11]));
        assert_eq!(version.python_version().to_string(), "3.11");
        assert_eq!(version.python_full_version(), Version::new([3, 11, 0]));
        assert_eq!(version.python_full_version().to_string(), "3.11.0");

        let version = PythonVersion::from_str("3.11").expect("valid python version");
        assert_eq!(version.python_version(), Version::new([3, 11]));
        assert_eq!(version.python_version().to_string(), "3.11");
        assert_eq!(version.python_full_version(), Version::new([3, 11, 0]));
        assert_eq!(version.python_full_version().to_string(), "3.11.0");

        let version = PythonVersion::from_str("3.11.8a1").expect("valid python version");
        assert_eq!(version.python_version(), Version::new([3, 11]));
        assert_eq!(version.python_version().to_string(), "3.11");
        assert_eq!(
            version.python_full_version(),
            Version::new([3, 11, 8]).with_pre(Some(Prerelease {
                kind: PrereleaseKind::Alpha,
                number: 1
            }))
        );
        assert_eq!(version.python_full_version().to_string(), "3.11.8a1");
    }
}
