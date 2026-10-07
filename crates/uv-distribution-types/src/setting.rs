#[cfg(feature = "schemars")]
use std::borrow::Cow;
use std::fmt;
use std::hash::{Hash, Hasher};

#[cfg(feature = "schemars")]
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};

use crate::RequirementsInput;

/// A setting and the declarations responsible for its effective value.
///
/// Sources are diagnostic metadata: serialization, equality, and hashing depend only on the value.
/// Scalar precedence should select the entire setting, so a value cannot retain a shadowed source.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Sourced<T> {
    value: T,
    #[serde(skip)]
    sources: SettingSources,
}

impl<T> Sourced<T> {
    pub fn new(value: T, source: SettingSource) -> Self {
        Self {
            value,
            sources: SettingSources(Box::new([source])),
        }
    }

    pub fn value(&self) -> &T {
        &self.value
    }

    pub fn sources(&self) -> &SettingSources {
        &self.sources
    }

    pub fn into_parts(self) -> (T, SettingSources) {
        (self.value, self.sources)
    }

    #[must_use]
    pub fn with_sources(mut self, sources: &SettingSources) -> Self {
        self.sources.extend(sources);
        self
    }

    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> Sourced<U> {
        Sourced {
            value: map(self.value),
            sources: self.sources,
        }
    }

    /// Associate a deserialized setting with its configuration file and key.
    pub fn set_source(&mut self, source: SettingSource) {
        self.sources = SettingSources(Box::new([source]));
    }

    /// Record the parent input through which a requirements directive was reached.
    pub fn included_by(&mut self, location: &RequirementsLocation) {
        for source in &mut self.sources.0 {
            if let SettingSource::Requirements { included_by, .. } = source {
                included_by.push(location.clone());
            }
        }
    }
}

impl Sourced<bool> {
    /// Enable a cumulative flag if either input enables it, retaining every enabling declaration.
    #[must_use]
    pub fn or(mut self, other: Self) -> Self {
        if !self.value {
            return other;
        }
        if other.value {
            self.sources.extend(&other.sources);
        }
        self
    }
}

impl<T> From<T> for Sourced<T> {
    fn from(value: T) -> Self {
        Self {
            value,
            sources: SettingSources::default(),
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for Sourced<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Sources are exposed separately so settings dumps focus on effective values.
        self.value.fmt(f)
    }
}

impl<T: PartialEq> PartialEq for Sourced<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T: Eq> Eq for Sourced<T> {}

impl<T: Hash> Hash for Sourced<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

#[cfg(feature = "schemars")]
impl<T: JsonSchema> JsonSchema for Sourced<T> {
    fn inline_schema() -> bool {
        T::inline_schema()
    }

    fn schema_name() -> Cow<'static, str> {
        T::schema_name()
    }

    fn schema_id() -> Cow<'static, str> {
        T::schema_id()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        T::json_schema(generator)
    }

    fn _schemars_private_non_optional_json_schema(generator: &mut SchemaGenerator) -> Schema {
        T::_schemars_private_non_optional_json_schema(generator)
    }

    fn _schemars_private_is_option() -> bool {
        T::_schemars_private_is_option()
    }
}

/// The declarations that contribute to an effective setting, in encounter order.
#[derive(Debug, Clone, Default)]
pub struct SettingSources(Box<[SettingSource]>);

impl SettingSources {
    fn iter(&self) -> impl Iterator<Item = &SettingSource> {
        self.0.iter()
    }

    pub fn extend(&mut self, other: &Self) {
        let mut sources = std::mem::take(&mut self.0).into_vec();
        for source in &other.0 {
            if !sources.contains(source) {
                sources.push(source.clone());
            }
        }
        self.0 = sources.into_boxed_slice();
    }

    /// Explain indirect enabling declarations. An explicit command-line flag needs no extra hint.
    pub fn enabled_hints(&self, flag: &'static str) -> impl Iterator<Item = String> + '_ {
        self.iter().filter_map(move |source| match source {
            SettingSource::CommandLine(_) => None,
            SettingSource::Environment(_)
            | SettingSource::Configuration { .. }
            | SettingSource::Requirements { .. } => {
                Some(format!("`{flag}` was enabled by {source}"))
            }
        })
    }
}

/// The source of a setting. Environment values are omitted and remote inputs use redacted URLs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingSource {
    CommandLine(&'static str),
    Environment(&'static str),
    Configuration {
        input: RequirementsInput,
        key: String,
    },
    Requirements {
        location: RequirementsLocation,
        /// Include sites, ordered from the nearest parent to the root input.
        included_by: Vec<RequirementsLocation>,
    },
}

impl fmt::Display for SettingSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CommandLine(flag) => write!(f, "command-line argument `{flag}`"),
            Self::Environment(name) => write!(f, "environment variable `{name}`"),
            Self::Configuration { input, key } => match input {
                RequirementsInput::Stdin => write!(f, "`{key}` in script metadata read from stdin"),
                RequirementsInput::Local(_) | RequirementsInput::Remote(_) => {
                    write!(f, "`{key}` in `{}`", input.user_display())
                }
            },
            Self::Requirements {
                location,
                included_by,
            } => {
                location.fmt(f)?;
                if !included_by.is_empty() {
                    f.write_str(" (included from ")?;
                    for (index, parent) in included_by.iter().rev().enumerate() {
                        if index > 0 {
                            f.write_str(" -> ")?;
                        }
                        parent.fmt(f)?;
                    }
                    f.write_str(")")?;
                }
                Ok(())
            }
        }
    }
}

/// A declaration or include site in a requirements input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementsLocation {
    pub input: RequirementsInput,
    /// The one-based line number.
    pub line: usize,
}

impl fmt::Display for RequirementsLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.input {
            RequirementsInput::Stdin => write!(f, "stdin at line {}", self.line),
            RequirementsInput::Local(_) | RequirementsInput::Remote(_) => {
                write!(f, "`{}` at line {}", self.input.user_display(), self.line)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    use super::{SettingSource, Sourced};

    /// Diagnostic metadata must not change serialized configuration or semantic cache identity.
    #[test]
    fn value_identity() -> Result<(), serde_json::Error> {
        let first = Sourced::new(true, SettingSource::CommandLine("--no-index"));
        let second = Sourced::new(true, SettingSource::Environment("UV_REQUIRE_HASHES"));
        assert_eq!(first, second);

        let hash = |value: &Sourced<bool>| {
            let mut hasher = DefaultHasher::new();
            value.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash(&first), hash(&second));
        assert_eq!(serde_json::to_string(&first)?, "true");
        assert_eq!(serde_json::from_str::<Sourced<bool>>("true")?, first);
        Ok(())
    }
}
