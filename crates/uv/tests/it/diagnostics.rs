use anyhow::{Context, Result};
use assert_fs::prelude::*;
use serde_json::Value;

#[test]
fn json_configuration_error_has_safe_source_coordinates() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    context
        .temp_dir
        .child("uv.toml")
        .write_str("index-url = 'https://user:secret@example.com/simple'\nno-build = !\n")?;

    let output = context
        .command()
        .args([
            "--error-format=json",
            "--color=always",
            "--config-file=uv.toml",
            "--offline",
            "cache",
            "dir",
        ])
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.contains(&0x1b));
    let report: Value = serde_json::from_slice(&output.stderr)?;
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["coordinates"]["column_encoding"], "utf-8");
    let source = report["errors"]
        .as_array()
        .context("error chain is an array")?
        .iter()
        .flat_map(|error| error["sources"].as_array().into_iter().flatten())
        .find(|source| source["kind"] == "snippet")
        .context("the TOML parser retains its source location")?;
    assert_eq!(source["windows"][0]["line_start"], 2);
    assert_eq!(source["windows"][0]["text"], "no-build = !\n");
    assert_eq!(
        source["windows"][0]["annotations"][0]["range"]["start"],
        serde_json::json!({"line": 2, "byte_column": 11})
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("secret"));
    Ok(())
}

#[test]
#[cfg(feature = "test-python")]
fn json_resolution_error_keeps_failure_status() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("requirements.in")
        .write_str("pypyp==1,>=1.2\n")?;

    let text = context
        .pip_compile()
        .args(["requirements.in", "--offline"])
        .output()?;
    let json = context
        .pip_compile()
        .args(["requirements.in", "--offline", "--error-format=json"])
        .output()?;
    assert_eq!(text.status.code(), Some(1));
    assert_eq!(json.status.code(), text.status.code());
    assert!(json.stdout.is_empty());
    let report: Value = serde_json::from_slice(&json.stderr)?;
    assert_eq!(report["level"], "error");
    assert!(
        report["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty())
    );
    Ok(())
}

#[test]
fn json_error_format_does_not_change_clap_errors() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let output = context
        .command()
        .args(["--error-format=json", "--not-a-real-option"])
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    assert!(serde_json::from_slice::<Value>(&output.stderr).is_err());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected argument"));
    Ok(())
}
