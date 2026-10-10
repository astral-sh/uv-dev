use std::borrow::Cow;
use std::fmt::{Display, Formatter, Write};
use std::ops::{Deref, DerefMut};
use std::str::FromStr;

#[derive(
    Default,
    Debug,
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
)]
#[rkyv(derive(Debug))]
#[repr(transparent)]
pub struct CPythonAbiVariants(u8);

bitflags::bitflags! {
    impl CPythonAbiVariants: u8 {
        /// A freethreading build of CPython without GIL, Python 3.13+.
        const Freethreading = 1 << 0;
        /// Historical, not needed on CPython 3.3+.
        const WideUnicode = 1 << 1;
        /// A debug build of CPython.
        const Debug = 1 << 2;
        /// Historical, not needed on CPython 3.4+, but only removed with CPython 3.8+.
        ///
        /// <https://github.com/python/cpython/issues/80888#issuecomment-1093821707>
        const Pymalloc = 1 << 3;
    }
}

impl Deref for CPythonAbiVariants {
    type Target = u8;

    fn deref(&self) -> &u8 {
        &self.0
    }
}

impl DerefMut for CPythonAbiVariants {
    fn deref_mut(&mut self) -> &mut u8 {
        &mut self.0
    }
}

impl Display for CPythonAbiVariants {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        // Preserve the known order of flags, as observed from CPython itself.
        // TODO(konsti): Is there a canonical source for that?
        if self.contains(Self::Freethreading) {
            // https://peps.python.org/pep-0703/#build-configuration-changes
            // Python 3.13+ only, but it makes more sense to just rely on the sysconfig var.
            f.write_char('t')?;
        }
        if self.contains(Self::Debug) {
            f.write_char('d')?;
        }
        if self.contains(Self::Pymalloc) {
            // Not used anymore, now always implied
            f.write_char('m')?;
        }
        if self.contains(Self::WideUnicode) {
            // Not used anymore, now always implied
            f.write_char('u')?;
        }
        Ok(())
    }
}

impl CPythonAbiVariants {
    fn from_char(suffix: char) -> Option<Self> {
        match suffix {
            't' => Some(Self::Freethreading),
            'u' => Some(Self::WideUnicode),
            'd' => Some(Self::Debug),
            'm' => Some(Self::Pymalloc),
            _ => None,
        }
    }
}

/// A tag to represent the ABI compatibility of a Python distribution.
///
/// This is the second segment in the wheel filename, following the language tag. For example,
/// in `cp39-none-manylinux_2_24_x86_64.whl`, the ABI tag is `none`.
#[derive(
    Debug,
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    rkyv::Archive,
    rkyv::Deserialize,
    rkyv::Serialize,
)]
#[rkyv(derive(Debug))]
pub enum AbiTag {
    /// Ex) `none`
    None,
    /// Ex) `abi3`
    Abi3,
    /// Ex) `abi3t`
    Abi3T,
    /// Ex) `cp39m`, `cp310t`
    CPython {
        python_version: (u8, u8),
        variant: CPythonAbiVariants,
    },
    /// Ex) `pypy39_pp73`
    PyPy {
        python_version: Option<(u8, u8)>,
        implementation_version: (u8, u8),
    },
    /// Ex) `graalpy240_310_native`
    GraalPy {
        python_version: (u8, u8),
        implementation_version: (u8, u8),
    },
    /// Ex) `pyston_23_x86_64_linux_gnu`
    Pyston { implementation_version: (u8, u8) },
}

impl AbiTag {
    /// Return `true` if this is one of the stable ABI tags.
    pub fn is_stable_abi(self) -> bool {
        matches!(self, Self::Abi3 | Self::Abi3T)
    }

    /// Return a pretty string representation of the ABI tag.
    pub fn pretty(self) -> Option<Cow<'static, str>> {
        match self {
            Self::None => None,
            Self::Abi3 => None,
            Self::Abi3T => Some(Cow::Borrowed("stable ABI for free-threaded CPython")),
            Self::CPython {
                variant,
                python_version,
            } => {
                // We only need to handle freethreading here:
                // * Debug is ABI compatible (https://github.com/python/cpython/issues/80646)
                // * Wide unicode is always on in supported versions
                // * pymalloc is always on in supported versions

                // https://peps.python.org/pep-0703/#build-configuration-changes
                let prefix = if variant.contains(CPythonAbiVariants::Freethreading) {
                    "free-threaded "
                } else {
                    ""
                };
                Some(Cow::Owned(format!(
                    "{}CPython {}.{}",
                    prefix, python_version.0, python_version.1
                )))
            }
            Self::PyPy {
                implementation_version,
                ..
            } => Some(Cow::Owned(format!(
                "PyPy {}.{}",
                implementation_version.0, implementation_version.1
            ))),
            Self::GraalPy {
                implementation_version,
                ..
            } => Some(Cow::Owned(format!(
                "GraalPy {}.{}",
                implementation_version.0, implementation_version.1
            ))),
            Self::Pyston { .. } => Some(Cow::Borrowed("Pyston")),
        }
    }
}

impl std::fmt::Display for AbiTag {
    /// Format an [`AbiTag`] as a string.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Abi3 => write!(f, "abi3"),
            Self::Abi3T => write!(f, "abi3t"),
            Self::CPython {
                variant,
                python_version: (major, minor),
            } => {
                write!(f, "cp{major}{minor}{variant}")
            }
            Self::PyPy {
                python_version: Some((py_major, py_minor)),
                implementation_version: (impl_major, impl_minor),
            } => {
                write!(f, "pypy{py_major}{py_minor}_pp{impl_major}{impl_minor}")
            }
            Self::PyPy {
                python_version: None,
                implementation_version: (impl_major, impl_minor),
            } => {
                write!(f, "pypy_{impl_major}{impl_minor}")
            }
            Self::GraalPy {
                python_version: (py_major, py_minor),
                implementation_version: (impl_major, impl_minor),
            } => {
                write!(
                    f,
                    "graalpy{impl_major}{impl_minor}_{py_major}{py_minor}_native"
                )
            }
            Self::Pyston {
                implementation_version: (impl_major, impl_minor),
            } => {
                write!(f, "pyston_{impl_major}{impl_minor}_x86_64_linux_gnu")
            }
        }
    }
}

/// The context-independent reason that a compact ABI version could not be parsed.
enum ParseVersionError {
    MissingMajor,
    InvalidMajor,
    MissingMinor,
    InvalidMinor,
}

/// Parse a compact version from a string (e.g., convert `39` into `(3, 9)`).
fn parse_version(version: &str) -> Result<(u8, u8), ParseVersionError> {
    let major = version
        .as_bytes()
        .first()
        .ok_or(ParseVersionError::MissingMajor)?
        .checked_sub(b'0')
        .filter(|digit| *digit < 10)
        .ok_or(ParseVersionError::InvalidMajor)?;
    let minor = version
        .get(1..)
        .ok_or(ParseVersionError::MissingMinor)?
        .parse::<u8>()
        .map_err(|_| ParseVersionError::InvalidMinor)?;
    Ok((major, minor))
}

impl AbiTag {
    /// Parse an [`AbiTag`] without attaching the input to errors.
    fn parse(s: &str) -> Result<Self, ParseAbiTagErrorKind> {
        use ParseAbiTagErrorKind as ErrorKind;

        /// Parse a Python version from a string (e.g., convert `39` into `(3, 9)`).
        fn parse_python_version(
            version_str: &str,
            implementation: &'static str,
        ) -> Result<(u8, u8), ErrorKind> {
            parse_version(version_str).map_err(|err| match err {
                ParseVersionError::MissingMajor => ErrorKind::MissingMajorVersion(implementation),
                ParseVersionError::InvalidMajor => ErrorKind::InvalidMajorVersion(implementation),
                ParseVersionError::MissingMinor => ErrorKind::MissingMinorVersion(implementation),
                ParseVersionError::InvalidMinor => ErrorKind::InvalidMinorVersion(implementation),
            })
        }

        /// Parse an implementation version from a string (e.g., convert `37` into `(3, 7)`).
        fn parse_impl_version(
            version_str: &str,
            implementation: &'static str,
        ) -> Result<(u8, u8), ErrorKind> {
            parse_version(version_str).map_err(|err| match err {
                ParseVersionError::MissingMajor => {
                    ErrorKind::MissingImplMajorVersion(implementation)
                }
                ParseVersionError::InvalidMajor => {
                    ErrorKind::InvalidImplMajorVersion(implementation)
                }
                ParseVersionError::MissingMinor => {
                    ErrorKind::MissingImplMinorVersion(implementation)
                }
                ParseVersionError::InvalidMinor => {
                    ErrorKind::InvalidImplMinorVersion(implementation)
                }
            })
        }

        if s == "none" {
            Ok(Self::None)
        } else if s == "abi3" {
            Ok(Self::Abi3)
        } else if s == "abi3t" {
            Ok(Self::Abi3T)
        } else if let Some(cp) = s.strip_prefix("cp") {
            // Ex) `cp39m`, `cp310t`
            let version_end = cp.find(|c: char| !c.is_ascii_digit()).unwrap_or(cp.len());
            let version_str = &cp[..version_end];
            let (major, minor) = parse_python_version(version_str, "CPython")?;
            let abi_suffixes = &cp[version_end..];
            let mut variant = CPythonAbiVariants::default();
            for suffix_char in abi_suffixes.chars() {
                let Some(suffix) = CPythonAbiVariants::from_char(suffix_char) else {
                    return Err(ParseAbiTagErrorKind::UnknownAbiTagSuffix(suffix_char));
                };

                if variant.contains(suffix) {
                    return Err(ParseAbiTagErrorKind::DuplicateAbiTagSuffix(suffix_char));
                }

                variant.insert(suffix);
            }
            Ok(Self::CPython {
                variant,
                python_version: (major, minor),
            })
        } else if let Some(rest) = s.strip_prefix("pypy") {
            if let Some(rest) = rest.strip_prefix('_') {
                // Ex) `pypy_73`
                let (impl_major, impl_minor) = parse_impl_version(rest, "PyPy")?;
                Ok(Self::PyPy {
                    python_version: None,
                    implementation_version: (impl_major, impl_minor),
                })
            } else {
                // Ex) `pypy39_pp73`
                let (version_str, rest) = rest
                    .split_once('_')
                    .ok_or(ParseAbiTagErrorKind::InvalidFormat("PyPy"))?;
                let (major, minor) = parse_python_version(version_str, "PyPy")?;
                let rest = rest
                    .strip_prefix("pp")
                    .ok_or(ParseAbiTagErrorKind::InvalidFormat("PyPy"))?;
                let (impl_major, impl_minor) = parse_impl_version(rest, "PyPy")?;
                Ok(Self::PyPy {
                    python_version: Some((major, minor)),
                    implementation_version: (impl_major, impl_minor),
                })
            }
        } else if let Some(rest) = s.strip_prefix("graalpy") {
            // Ex) `graalpy240_310_native`
            let (impl_ver_str, rest) = rest
                .split_once('_')
                .ok_or(ParseAbiTagErrorKind::InvalidFormat("GraalPy"))?;
            let (impl_major, impl_minor) = parse_impl_version(impl_ver_str, "GraalPy")?;
            let (py_ver_str, suffix) = rest
                .split_once('_')
                .ok_or(ParseAbiTagErrorKind::InvalidFormat("GraalPy"))?;
            if suffix != "native" {
                return Err(ParseAbiTagErrorKind::InvalidFormat("GraalPy"));
            }
            let (major, minor) = parse_python_version(py_ver_str, "GraalPy")?;
            Ok(Self::GraalPy {
                python_version: (major, minor),
                implementation_version: (impl_major, impl_minor),
            })
        } else if let Some(rest) = s.strip_prefix("pyston") {
            // Ex) `pyston_23_x86_64_linux_gnu`
            let rest = rest
                .strip_prefix("_")
                .ok_or(ParseAbiTagErrorKind::InvalidFormat("Pyston"))?;
            let rest = rest
                .strip_suffix("_x86_64_linux_gnu")
                .ok_or(ParseAbiTagErrorKind::InvalidFormat("Pyston"))?;
            let (impl_major, impl_minor) = parse_impl_version(rest, "Pyston")?;
            Ok(Self::Pyston {
                implementation_version: (impl_major, impl_minor),
            })
        } else {
            Err(ParseAbiTagErrorKind::UnknownFormat)
        }
    }
}

impl FromStr for AbiTag {
    type Err = ParseAbiTagError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).map_err(|kind| ParseAbiTagError {
            kind,
            tag: s.to_string(),
        })
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{kind}: {tag}")]
pub struct ParseAbiTagError {
    kind: ParseAbiTagErrorKind,
    tag: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
enum ParseAbiTagErrorKind {
    #[error("Unknown ABI tag format")]
    UnknownFormat,
    #[error("Missing major version in {0} ABI tag")]
    MissingMajorVersion(&'static str),
    #[error("Invalid major version in {0} ABI tag")]
    InvalidMajorVersion(&'static str),
    #[error("Missing minor version in {0} ABI tag")]
    MissingMinorVersion(&'static str),
    #[error("Invalid minor version in {0} ABI tag")]
    InvalidMinorVersion(&'static str),
    #[error("Invalid {0} ABI tag format")]
    InvalidFormat(&'static str),
    #[error("Missing implementation major version in {0} ABI tag")]
    MissingImplMajorVersion(&'static str),
    #[error("Invalid implementation major version in {0} ABI tag")]
    InvalidImplMajorVersion(&'static str),
    #[error("Missing implementation minor version in {0} ABI tag")]
    MissingImplMinorVersion(&'static str),
    #[error("Invalid implementation minor version in {0} ABI tag")]
    InvalidImplMinorVersion(&'static str),
    #[error("Unknown suffix `{0}` in CPython ABI tag")]
    UnknownAbiTagSuffix(char),
    #[error("Duplicate suffix `{0}` in CPython ABI tag")]
    DuplicateAbiTagSuffix(char),
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use insta::assert_snapshot;

    use crate::abi_tag::{AbiTag, ParseAbiTagError, ParseAbiTagErrorKind};

    #[test]
    fn none_abi() {
        assert_eq!(AbiTag::from_str("none"), Ok(AbiTag::None));
        assert_eq!(AbiTag::None.to_string(), "none");
    }

    #[test]
    fn abi3() {
        assert_eq!(AbiTag::from_str("abi3"), Ok(AbiTag::Abi3));
        assert_eq!(AbiTag::Abi3.to_string(), "abi3");
        assert!(AbiTag::Abi3.is_stable_abi());
    }

    #[test]
    fn abi3t() {
        assert_eq!(AbiTag::from_str("abi3t"), Ok(AbiTag::Abi3T));
        assert_eq!(AbiTag::Abi3T.to_string(), "abi3t");
        assert!(AbiTag::Abi3T.is_stable_abi());
        assert_eq!(
            AbiTag::Abi3T.pretty().as_deref(),
            Some("stable ABI for free-threaded CPython")
        );
    }

    #[test]
    fn cpython_abi() {
        let tag = AbiTag::from_str("cp39").unwrap();
        assert_eq!(tag.to_string(), "cp39");
        assert_eq!(tag.pretty().as_deref(), Some("CPython 3.9"));

        let tag = AbiTag::from_str("cp37m").unwrap();
        assert_eq!(tag.to_string(), "cp37m");
        assert_eq!(tag.pretty().as_deref(), Some("CPython 3.7"));

        let tag = AbiTag::from_str("cp313t").unwrap();
        assert_eq!(tag.to_string(), "cp313t");
        assert_eq!(tag.pretty().as_deref(), Some("free-threaded CPython 3.13"));

        assert_eq!(
            AbiTag::from_str("cpXY"),
            Err(ParseAbiTagError {
                kind: ParseAbiTagErrorKind::MissingMajorVersion("CPython"),
                tag: "cpXY".to_string()
            })
        );
    }

    #[test]
    fn cpython_abi_invalid() {
        let err = AbiTag::from_str("cp39y").unwrap_err();
        assert_snapshot!(err, @"Unknown suffix `y` in CPython ABI tag: cp39y");

        let err = AbiTag::from_str("cp39dd").unwrap_err();
        assert_snapshot!(err, @"Duplicate suffix `d` in CPython ABI tag: cp39dd");
    }

    #[test]
    fn version_errors() {
        let errors = [
            "cp",
            "cp3",
            "cp3999",
            "cpXY",
            "pypy_",
            "pypy_XY",
            "pypy_3",
            "pypy_3999",
            "pypyX_pp",
            "pypy3_pp73",
            "pypy3999_pp73",
            "pypy39_pp",
            "pypy39_ppX",
            "pypy39_pp3",
            "pypy39_pp3999",
            "graalpy_39_native",
            "graalpyX_39_native",
            "graalpy3_39_native",
            "graalpy3999_39_native",
            "graalpy39__native",
            "graalpy39_X_native",
            "graalpy39_3_native",
            "graalpy39_3999_native",
            "graalpy39__wrong",
            "pyston__x86_64_linux_gnu",
            "pyston_X_x86_64_linux_gnu",
            "pyston_3_x86_64_linux_gnu",
            "pyston_3999_x86_64_linux_gnu",
        ]
        .map(|tag| AbiTag::from_str(tag).unwrap_err().to_string());
        assert_snapshot!(errors.join("\n"), @"
        Missing major version in CPython ABI tag: cp
        Invalid minor version in CPython ABI tag: cp3
        Invalid minor version in CPython ABI tag: cp3999
        Missing major version in CPython ABI tag: cpXY
        Missing implementation major version in PyPy ABI tag: pypy_
        Invalid implementation major version in PyPy ABI tag: pypy_XY
        Invalid implementation minor version in PyPy ABI tag: pypy_3
        Invalid implementation minor version in PyPy ABI tag: pypy_3999
        Invalid major version in PyPy ABI tag: pypyX_pp
        Invalid minor version in PyPy ABI tag: pypy3_pp73
        Invalid minor version in PyPy ABI tag: pypy3999_pp73
        Missing implementation major version in PyPy ABI tag: pypy39_pp
        Invalid implementation major version in PyPy ABI tag: pypy39_ppX
        Invalid implementation minor version in PyPy ABI tag: pypy39_pp3
        Invalid implementation minor version in PyPy ABI tag: pypy39_pp3999
        Missing implementation major version in GraalPy ABI tag: graalpy_39_native
        Invalid implementation major version in GraalPy ABI tag: graalpyX_39_native
        Invalid implementation minor version in GraalPy ABI tag: graalpy3_39_native
        Invalid implementation minor version in GraalPy ABI tag: graalpy3999_39_native
        Missing major version in GraalPy ABI tag: graalpy39__native
        Invalid major version in GraalPy ABI tag: graalpy39_X_native
        Invalid minor version in GraalPy ABI tag: graalpy39_3_native
        Invalid minor version in GraalPy ABI tag: graalpy39_3999_native
        Invalid GraalPy ABI tag format: graalpy39__wrong
        Missing implementation major version in Pyston ABI tag: pyston__x86_64_linux_gnu
        Invalid implementation major version in Pyston ABI tag: pyston_X_x86_64_linux_gnu
        Invalid implementation minor version in Pyston ABI tag: pyston_3_x86_64_linux_gnu
        Invalid implementation minor version in Pyston ABI tag: pyston_3999_x86_64_linux_gnu
        ");
    }

    #[test]
    fn pypy_abi() {
        let tag = AbiTag::PyPy {
            python_version: Some((3, 9)),
            implementation_version: (7, 3),
        };
        assert_eq!(AbiTag::from_str("pypy39_pp73"), Ok(tag));
        assert_eq!(tag.to_string(), "pypy39_pp73");

        let tag = AbiTag::PyPy {
            python_version: None,
            implementation_version: (7, 3),
        };
        assert_eq!(AbiTag::from_str("pypy_73").as_ref(), Ok(&tag));
        assert_eq!(tag.to_string(), "pypy_73");

        assert_eq!(
            AbiTag::from_str("pypy39"),
            Err(ParseAbiTagError {
                kind: ParseAbiTagErrorKind::InvalidFormat("PyPy"),
                tag: "pypy39".to_string()
            })
        );
        assert_eq!(
            AbiTag::from_str("pypy39_73"),
            Err(ParseAbiTagError {
                kind: ParseAbiTagErrorKind::InvalidFormat("PyPy"),
                tag: "pypy39_73".to_string()
            })
        );
        assert_eq!(
            AbiTag::from_str("pypy39_ppXY"),
            Err(ParseAbiTagError {
                kind: ParseAbiTagErrorKind::InvalidImplMajorVersion("PyPy"),
                tag: "pypy39_ppXY".to_string()
            })
        );
    }

    #[test]
    fn graalpy_abi() {
        let tag = AbiTag::GraalPy {
            python_version: (3, 10),
            implementation_version: (2, 40),
        };
        assert_eq!(AbiTag::from_str("graalpy240_310_native"), Ok(tag));
        assert_eq!(tag.to_string(), "graalpy240_310_native");

        assert_eq!(
            AbiTag::from_str("graalpy310"),
            Err(ParseAbiTagError {
                kind: ParseAbiTagErrorKind::InvalidFormat("GraalPy"),
                tag: "graalpy310".to_string()
            })
        );
        assert_eq!(
            AbiTag::from_str("graalpy310_240"),
            Err(ParseAbiTagError {
                kind: ParseAbiTagErrorKind::InvalidFormat("GraalPy"),
                tag: "graalpy310_240".to_string()
            })
        );
        assert_eq!(
            AbiTag::from_str("graalpy310_graalpyXY"),
            Err(ParseAbiTagError {
                kind: ParseAbiTagErrorKind::InvalidFormat("GraalPy"),
                tag: "graalpy310_graalpyXY".to_string()
            })
        );
        assert_eq!(
            AbiTag::from_str("graalpy240_310_wrong"),
            Err(ParseAbiTagError {
                kind: ParseAbiTagErrorKind::InvalidFormat("GraalPy"),
                tag: "graalpy240_310_wrong".to_string()
            })
        );
        assert_eq!(
            AbiTag::from_str("graalpy240_310_native_extra"),
            Err(ParseAbiTagError {
                kind: ParseAbiTagErrorKind::InvalidFormat("GraalPy"),
                tag: "graalpy240_310_native_extra".to_string()
            })
        );
    }

    #[test]
    fn pyston_abi() {
        let tag = AbiTag::Pyston {
            implementation_version: (2, 3),
        };
        assert_eq!(AbiTag::from_str("pyston_23_x86_64_linux_gnu"), Ok(tag));
        assert_eq!(tag.to_string(), "pyston_23_x86_64_linux_gnu");

        assert_eq!(
            AbiTag::from_str("pyston23_x86_64_linux_gnu"),
            Err(ParseAbiTagError {
                kind: ParseAbiTagErrorKind::InvalidFormat("Pyston"),
                tag: "pyston23_x86_64_linux_gnu".to_string()
            })
        );
        assert_eq!(
            AbiTag::from_str("pyston_XY_x86_64_linux_gnu"),
            Err(ParseAbiTagError {
                kind: ParseAbiTagErrorKind::InvalidImplMajorVersion("Pyston"),
                tag: "pyston_XY_x86_64_linux_gnu".to_string()
            })
        );
    }

    #[test]
    fn unknown_abi() {
        assert_eq!(
            AbiTag::from_str("unknown"),
            Err(ParseAbiTagError {
                kind: ParseAbiTagErrorKind::UnknownFormat,
                tag: "unknown".to_string(),
            })
        );
        assert_eq!(
            AbiTag::from_str(""),
            Err(ParseAbiTagError {
                kind: ParseAbiTagErrorKind::UnknownFormat,
                tag: String::new(),
            })
        );
    }
}
