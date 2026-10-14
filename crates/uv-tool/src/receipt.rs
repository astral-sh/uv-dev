use std::path::Path;

use serde::Deserialize;

use crate::Tool;

/// A `uv-receipt.toml` file tracking the installation of a tool.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ToolReceipt {
    pub(crate) tool: Tool,
}

impl ToolReceipt {
    /// Read a [`ToolReceipt`] from the given path.
    pub(crate) fn from_path(path: &Path) -> Result<Self, crate::Error> {
        let contents = fs_err::read_to_string(path)?;
        toml::from_str(&contents)
            .map_err(|err| crate::Error::ReceiptRead(path.to_owned(), Box::new(err)))
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

impl From<Tool> for ToolReceipt {
    fn from(tool: Tool) -> Self {
        Self { tool }
    }
}
