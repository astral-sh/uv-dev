//! Validate command output against checked-in draft-07 JSON Schemas.

use anyhow::{Context, Result, ensure};
use serde_json::Value;

/// A compiled JSON Schema for command-output assertions.
pub struct JsonSchema {
    validator: jsonschema::Validator,
}

impl JsonSchema {
    /// Compile a draft-07 schema, including its format constraints.
    pub fn new(contents: &str) -> Result<Self> {
        let schema: Value =
            serde_json::from_str(contents).context("invalid JSON schema document")?;
        let validator = jsonschema::draft7::options()
            .should_validate_formats(true)
            .build(&schema)
            .map_err(|error| anyhow::anyhow!("invalid JSON schema: {error}"))?;
        Ok(Self { validator })
    }

    /// Parse command output and report every schema violation with its instance path.
    pub fn parse(&self, contents: &[u8]) -> Result<Value> {
        let value = serde_json::from_slice(contents).context("command output is not valid JSON")?;
        let errors = self
            .validator
            .iter_errors(&value)
            .map(|error| format!("{}: {error}", error.instance_path()))
            .collect::<Vec<_>>();
        ensure!(
            errors.is_empty(),
            "JSON output violates its schema:\n{}",
            errors.join("\n")
        );
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::JsonSchema;

    #[test]
    fn rejects_invalid_schemas() {
        assert!(JsonSchema::new("{").is_err());
        assert!(JsonSchema::new(r#"{"type": "not-a-type"}"#).is_err());
    }

    #[test]
    fn reports_all_invalid_fields() -> anyhow::Result<()> {
        let schema = JsonSchema::new(
            r#"{
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "source": {"type": "string", "format": "uri"}
                },
                "required": ["name", "source"]
            }"#,
        )?;
        schema.parse(br#"{"name": "example", "source": "https://example.org/"}"#)?;
        let error = schema
            .parse(br#"{"name": 42, "source": "not a URI"}"#)
            .expect_err("both fields should violate the schema")
            .to_string();
        assert!(error.contains("/name"), "{error}");
        assert!(error.contains("/source"), "{error}");
        Ok(())
    }
}
