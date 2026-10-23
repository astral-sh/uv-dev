use std::fmt::{Display, Formatter};

/// The policy used to satisfy required resolution environments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum RequiredEnvironmentsMode {
    /// Require every active dependency to provide a compatible wheel.
    RequireWheels,
}

impl Display for RequiredEnvironmentsMode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RequireWheels => write!(f, "require-wheels"),
        }
    }
}
