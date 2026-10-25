use std::hash::{Hash, Hasher};

use crate::Error;

/// How a directory is installed once both packaging and editability are known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectoryInstallMode {
    Wheel,
    Editable,
    Virtual,
}

/// A directory preference whose other setting has not been determined yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectorySourcePreference {
    Unspecified,
    Editable,
    NonEditable,
    Package,
    Virtual,
}

/// Directory installation state, retaining unset preferences until discovery resolves them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectorySourceMode {
    Resolved(DirectoryInstallMode),
    Deferred(DirectorySourcePreference),
}

impl DirectorySourceMode {
    pub(crate) fn from_flags(
        editable: Option<bool>,
        virtual_project: Option<bool>,
    ) -> Result<Self, Error> {
        Ok(match (editable, virtual_project) {
            (Some(true), Some(true)) => return Err(Error::EditableVirtualDirectory),
            (Some(true), Some(false)) => Self::Resolved(DirectoryInstallMode::Editable),
            (Some(false), Some(false)) => Self::Resolved(DirectoryInstallMode::Wheel),
            (Some(false), Some(true)) => Self::Resolved(DirectoryInstallMode::Virtual),
            (None, None) => Self::Deferred(DirectorySourcePreference::Unspecified),
            (Some(true), None) => Self::Deferred(DirectorySourcePreference::Editable),
            (Some(false), None) => Self::Deferred(DirectorySourcePreference::NonEditable),
            (None, Some(false)) => Self::Deferred(DirectorySourcePreference::Package),
            (None, Some(true)) => Self::Deferred(DirectorySourcePreference::Virtual),
        })
    }

    /// A directory known to be packaged, retaining an unset editability preference.
    pub fn packaged(editable: Option<bool>) -> Self {
        match editable {
            Some(true) => Self::Resolved(DirectoryInstallMode::Editable),
            Some(false) => Self::Resolved(DirectoryInstallMode::Wheel),
            None => Self::Deferred(DirectorySourcePreference::Package),
        }
    }

    pub fn editable(self) -> Option<bool> {
        match self {
            Self::Resolved(DirectoryInstallMode::Editable)
            | Self::Deferred(DirectorySourcePreference::Editable) => Some(true),
            Self::Resolved(DirectoryInstallMode::Wheel | DirectoryInstallMode::Virtual)
            | Self::Deferred(DirectorySourcePreference::NonEditable) => Some(false),
            Self::Deferred(
                DirectorySourcePreference::Unspecified
                | DirectorySourcePreference::Package
                | DirectorySourcePreference::Virtual,
            ) => None,
        }
    }

    pub fn virtual_project(self) -> Option<bool> {
        match self {
            Self::Resolved(DirectoryInstallMode::Virtual)
            | Self::Deferred(DirectorySourcePreference::Virtual) => Some(true),
            Self::Resolved(DirectoryInstallMode::Wheel | DirectoryInstallMode::Editable)
            | Self::Deferred(DirectorySourcePreference::Package) => Some(false),
            Self::Deferred(
                DirectorySourcePreference::Unspecified
                | DirectorySourcePreference::Editable
                | DirectorySourcePreference::NonEditable,
            ) => None,
        }
    }

    /// Override editability without turning a virtual project into an installable package.
    #[must_use]
    pub fn with_editable(self, editable: bool) -> Self {
        match (self.virtual_project(), editable) {
            (Some(true), true) => self,
            (Some(true), false) => Self::Resolved(DirectoryInstallMode::Virtual),
            (Some(false), true) => Self::Resolved(DirectoryInstallMode::Editable),
            (Some(false), false) => Self::Resolved(DirectoryInstallMode::Wheel),
            (None, true) => Self::Deferred(DirectorySourcePreference::Editable),
            (None, false) => Self::Deferred(DirectorySourcePreference::NonEditable),
        }
    }
}

impl Hash for DirectorySourceMode {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Directory distribution hashes include the original preferences in this order.
        self.editable().hash(state);
        self.virtual_project().hash(state);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    use super::{DirectoryInstallMode, DirectorySourceMode};
    use crate::Error;

    #[test]
    fn directory_preferences_round_trip() -> Result<(), Error> {
        for editable in [None, Some(false), Some(true)] {
            for virtual_project in [None, Some(false), Some(true)] {
                if editable == Some(true) && virtual_project == Some(true) {
                    assert!(DirectorySourceMode::from_flags(editable, virtual_project).is_err());
                    continue;
                }
                let mode = DirectorySourceMode::from_flags(editable, virtual_project)?;
                assert_eq!(mode.editable(), editable);
                assert_eq!(mode.virtual_project(), virtual_project);
                let mut original = DefaultHasher::new();
                editable.hash(&mut original);
                virtual_project.hash(&mut original);
                let mut encoded = DefaultHasher::new();
                mode.hash(&mut encoded);
                assert_eq!(encoded.finish(), original.finish());

                let overridden = mode.with_editable(true);
                assert_eq!(overridden.virtual_project(), virtual_project);
                if virtual_project != Some(true) {
                    assert_eq!(overridden.editable(), Some(true));
                } else {
                    assert_eq!(overridden, mode);
                }
                assert_eq!(mode.with_editable(false).editable(), Some(false));
                assert_eq!(mode.with_editable(false).virtual_project(), virtual_project);
            }
        }
        assert_eq!(
            DirectorySourceMode::packaged(Some(true)),
            DirectorySourceMode::Resolved(DirectoryInstallMode::Editable),
        );
        Ok(())
    }
}
