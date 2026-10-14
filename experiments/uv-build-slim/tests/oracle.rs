//! Native reference: call the pinned upstream libraries without reimplementing them.
use std::{io::{self, BufRead}, str::FromStr};
use serde_json::{Value, json};
use uv_normalize::PackageName;
use uv_pypi_types::Metadata23;

fn evaluate(input: &Value) -> Result<Value, Box<dyn std::error::Error>> {
    match input["operation"].as_str().ok_or("missing operation")? {
        "normalize" => {
            let name = PackageName::from_str(input["value"].as_str().ok_or("missing name")?)?;
            Ok(json!({"normalized":name.to_string(),"dist_info":name.as_dist_info_name()}))
        }
        "metadata" => {
            let mut metadata = serde_json::to_value(Metadata23::default())?;
            for (key,value) in input["value"].as_object().ok_or("missing metadata")? {
                metadata[key] = value.clone();
            }
            let metadata: Metadata23 = serde_json::from_value(metadata)?;
            Ok(json!(metadata.core_metadata_format()))
        }
        "alphanumeric" => {
            let code = input["value"].as_u64().ok_or("missing codepoint")?;
            let code = u32::try_from(code)?;
            Ok(json!(char::from_u32(code).ok_or("invalid Unicode scalar")?.is_alphanumeric()))
        }
        operation => Err(format!("Unknown operation: {operation}").into()),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for line in io::stdin().lock().lines() {
        let input: Value = serde_json::from_str(&line?)?;
        let output = match evaluate(&input) {
            Ok(value) => json!({"ok":value}),
            Err(error) => json!({"error":error.to_string()}),
        };
        println!("{}", serde_json::to_string(&output)?);
    }
    Ok(())
}
