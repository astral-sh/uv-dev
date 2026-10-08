use std::fmt::Display;
use std::str::FromStr;

use crate::{SourceDistFilename, SourceDistFilenameError, WheelFilename, WheelFilenameError};

/// A built distribution's parsed filename and exact on-disk spelling.
///
/// A backend can return a valid filename whose spelling differs from its normalized form.
/// Filesystem operations must use the raw spelling, while metadata checks use the parsed value.
#[derive(Debug, Clone)]
pub struct BuiltFilename<T> {
    parsed: T,
    raw: String,
}

impl<T> BuiltFilename<T> {
    /// Return the parsed distribution identity.
    pub fn parsed(&self) -> &T {
        &self.parsed
    }

    /// Return the exact filename used on disk.
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Separate the parsed identity from the exact on-disk spelling.
    pub fn into_parts(self) -> (T, String) {
        (self.parsed, self.raw)
    }

    /// Map the parsed value without changing the filename used on disk.
    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> BuiltFilename<U> {
        BuiltFilename {
            parsed: map(self.parsed),
            raw: self.raw,
        }
    }
}

impl<T: Display> From<T> for BuiltFilename<T> {
    /// Use a backend's canonical filename as its on-disk spelling.
    fn from(parsed: T) -> Self {
        Self {
            raw: parsed.to_string(),
            parsed,
        }
    }
}

impl BuiltFilename<WheelFilename> {
    /// Parse a wheel filename returned by a build backend.
    pub fn parse(raw: String) -> Result<Self, WheelFilenameError> {
        Ok(Self {
            parsed: WheelFilename::from_str(&raw)?,
            raw,
        })
    }
}

impl BuiltFilename<SourceDistFilename> {
    /// Parse a source distribution filename returned by a build backend.
    pub fn parse(raw: String) -> Result<Self, SourceDistFilenameError> {
        Ok(Self {
            parsed: SourceDistFilename::parsed_normalized_filename(&raw)?,
            raw,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::BuiltFilename;
    use crate::{DistFilename, SourceDistFilename, WheelFilename};

    #[test]
    fn retains_backend_filename_spelling() -> Result<(), Box<dyn std::error::Error>> {
        let wheel =
            BuiltFilename::<WheelFilename>::parse("Example_Pkg-1.0-py3-none-any.whl".to_owned())?;
        let source =
            BuiltFilename::<SourceDistFilename>::parse("Example_Pkg-1.0.tar.gz".to_owned())?;
        for (filename, raw, normalized) in [
            (
                wheel.map(DistFilename::WheelFilename),
                "Example_Pkg-1.0-py3-none-any.whl",
                "example_pkg-1.0-py3-none-any.whl",
            ),
            (
                source.map(DistFilename::SourceDistFilename),
                "Example_Pkg-1.0.tar.gz",
                "example_pkg-1.0.tar.gz",
            ),
        ] {
            assert_eq!(filename.raw(), raw);
            assert_eq!(filename.parsed().to_string(), normalized);
            let (parsed, raw_filename) = filename.into_parts();
            assert_eq!(raw_filename, raw);
            assert_eq!(BuiltFilename::from(parsed).raw(), normalized);
        }
        Ok(())
    }
}
