use std::fmt;
use std::ops::Deref;
use std::path::Path;

use serde::Deserialize;

use crate::Tool;

/// A `uv-receipt.toml` file tracking the installation of a tool.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ToolReceipt {
    pub(crate) tool: Tool,

    /// The raw unserialized document.
    #[serde(skip)]
    pub(crate) raw: String,
}

impl ToolReceipt {
    /// Parse a [`ToolReceipt`] from a raw TOML string.
    pub(crate) fn from_string(raw: String) -> Result<Self, ToolReceiptParseError> {
        let tool = toml::from_str(&raw).map_err(|error| ToolReceiptParseError::new(error, &raw))?;
        Ok(Self { raw, ..tool })
    }

    ///  Read a [`ToolReceipt`] from the given path.
    pub(crate) fn from_path(path: &Path) -> Result<Self, crate::Error> {
        match fs_err::read_to_string(path) {
            Ok(contents) => Ok(Self::from_string(contents)
                .map_err(|err| crate::Error::ReceiptRead(path.to_owned(), Box::new(err)))?),
            Err(err) => Err(err.into()),
        }
    }

    /// Returns the TOML representation of this receipt.
    pub(crate) fn to_toml(&self) -> Result<String, toml_edit::ser::Error> {
        // We construct a TOML document manually instead of going through Serde to enable
        // the use of inline tables.
        let mut doc = toml_edit::DocumentMut::new();
        doc.insert("tool", toml_edit::Item::Table(self.tool.to_toml()?));

        Ok(doc.to_string())
    }
}

/// A receipt parse error whose diagnostic output excludes stored credentials.
///
/// Receipts can contain authenticated URLs. Both parser messages and source excerpts may repeat
/// those values, so only the location is displayed. The typed parser error is available via
/// [`Deref`], but is not exposed through the diagnostic source chain.
pub struct ToolReceiptParseError {
    error: toml::de::Error,
    location: Option<(usize, usize)>,
}

impl ToolReceiptParseError {
    fn new(error: toml::de::Error, input: &str) -> Self {
        let location = error.span().map(|span| {
            let mut line = 1;
            let mut column = 1;
            for (_, character) in input
                .char_indices()
                .take_while(|(offset, _)| *offset < span.start)
            {
                if character == '\n' {
                    line += 1;
                    column = 1;
                } else {
                    column += 1;
                }
            }
            (line, column)
        });
        Self { error, location }
    }
}

impl Deref for ToolReceiptParseError {
    type Target = toml::de::Error;

    fn deref(&self) -> &Self::Target {
        &self.error
    }
}

impl fmt::Display for ToolReceiptParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Invalid TOML tool receipt")?;
        if let Some((line, column)) = self.location {
            write!(formatter, " at line {line}, column {column}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for ToolReceiptParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolReceiptParseError")
            .field("location", &self.location)
            .finish_non_exhaustive()
    }
}

impl std::error::Error for ToolReceiptParseError {}

impl From<Tool> for ToolReceipt {
    fn from(tool: Tool) -> Self {
        Self {
            tool,
            raw: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::ops::Deref;

    use super::ToolReceipt;

    #[test]
    fn parse_diagnostics_omit_source_and_schema_values() {
        for raw in [
            "url = \"https://user:receipt-secret@example.com/\" trailing",
            "[tool]\nrequirements = \"https://user:receipt-secret@example.com/\"",
        ] {
            let error = ToolReceipt::from_string(raw.to_string()).expect_err("invalid receipt");
            assert!(error.deref().to_string().contains("receipt-secret"));
            assert!(error.span().is_some());
            assert!(
                error
                    .to_string()
                    .starts_with("Invalid TOML tool receipt at line ")
            );
            assert!(!error.to_string().contains("receipt-secret"));
            assert!(!format!("{error:?}").contains("receipt-secret"));
            assert!(error.source().is_none());
        }
    }
}
