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
    pub(crate) fn from_string(raw: String) -> Result<Self, toml::de::Error> {
        let tool = toml::from_str(&raw)?;
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
    use std::str::FromStr;

    use uv_distribution_types::{Requirement, RequirementSource};
    use uv_pypi_types::VerbatimParsedUrl;
    use uv_settings::ToolOptions;

    use super::ToolReceipt;
    use crate::Tool;

    /// A locked upgrade must use the same archive hashes as the original URL override.
    #[test]
    fn override_url_hashes_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let fragment = format!("sha256={digest}");
        let requirement = uv_pep508::Requirement::<VerbatimParsedUrl>::from_str(&format!(
            "idna @ https://example.com/idna-3.3-py3-none-any.whl#{fragment}"
        ))?;
        let tool = Tool::new(
            Vec::new(),
            Vec::new(),
            vec![Requirement::from(requirement).into()],
            Vec::new(),
            Vec::new(),
            None,
            [],
            ToolOptions::default(),
        );
        let receipt = ToolReceipt::from_string(ToolReceipt::from(tool).to_toml()?)?;

        let [override_spec] = receipt.tool.overrides() else {
            return Err("expected one override".into());
        };
        let requirement = &override_spec.requirement;
        let RequirementSource::Url { url, .. } = &requirement.source else {
            return Err("expected a direct URL".into());
        };
        assert_eq!(url.fragment(), Some(fragment.as_str()));
        Ok(())
    }
}
