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
fn configuration_error_redacts_inline_url_credentials() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let source = "index = [{ url = 'https://user:sentinel-secret@example.invalid/simple?X-Amz%2DSignature=sentinel-signature&safe=value', explicit = \"yes\" }]\r\n";
    context.temp_dir.child("uv.toml").write_str(source)?;

    let text = context
        .command()
        .env("COLUMNS", "300")
        .args(["--config-file=uv.toml", "--offline", "cache", "dir"])
        .output()?;
    let json = context
        .command()
        .args([
            "--error-format=json",
            "--config-file=uv.toml",
            "--offline",
            "cache",
            "dir",
        ])
        .output()?;
    assert_eq!(text.status.code(), Some(2));
    assert_eq!(json.status.code(), text.status.code());
    assert!(text.stdout.is_empty());
    assert!(json.stdout.is_empty());

    let expected = source
        .replace("sentinel-secret", &"*".repeat("sentinel-secret".len()))
        .replace(
            "sentinel-signature",
            &"*".repeat("sentinel-signature".len()),
        );
    let text_stderr = String::from_utf8(text.stderr)?;
    assert!(text_stderr.contains(expected.trim_end_matches(['\r', '\n'])));
    for output in [text_stderr.as_str(), std::str::from_utf8(&json.stderr)?] {
        assert!(!output.contains("sentinel-secret"));
        assert!(!output.contains("sentinel-signature"));
    }

    let report: Value = serde_json::from_slice(&json.stderr)?;
    let window = report["errors"]
        .as_array()
        .context("error chain is an array")?
        .iter()
        .flat_map(|error| error["sources"].as_array().into_iter().flatten())
        .flat_map(|source| source["windows"].as_array().into_iter().flatten())
        .next()
        .context("the TOML parser retains the source window")?;
    assert_eq!(window["text"], expected);
    let start = source
        .find("\"yes\"")
        .context("invalid boolean in fixture")?;
    assert_eq!(
        window["annotations"][0]["range"],
        serde_json::json!({
            "start": {"line": 1, "byte_column": start},
            "end": {"line": 1, "byte_column": start + "\"yes\"".len()},
        })
    );
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

#[test]
#[cfg(feature = "test-python")]
fn json_source_marker_suggestion_identifies_a_usable_edit() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let source = "# café\n[project]\nname = 'project'\nversion = '0.1.0'\nrequires-python = '>=3.12'\n[tool.uv.sources]\ndemo = [\n  { url = 'https://user:sentinel-secret@example.com/one.whl', marker = \"sys_platform == 'linux'\" },\n  { url = 'https://user:sentinel-secret@example.com/two.whl', marker = \"python_version >= '3.12'\" },\n]\n";
    let pyproject = context.temp_dir.child("pyproject.toml");
    pyproject.write_str(source)?;

    let output = context
        .lock()
        .args(["--offline", "--error-format=json"])
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    let report: Value = serde_json::from_slice(&output.stderr)?;
    let errors = report["errors"]
        .as_array()
        .context("error chain is an array")?;
    let redacted_url = format!(
        "https://user:{}@example.com/two.whl",
        "*".repeat("sentinel-secret".len())
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("sentinel-secret"));
    assert!(errors.iter().any(|error| {
        error["sources"].as_array().is_some_and(|sources| {
            sources.iter().any(|source| {
                source["kind"] == "snippet"
                    && source["windows"][0]["text"]
                        .as_str()
                        .is_some_and(|text| text.contains(&redacted_url))
            })
        })
    }));
    let suggestion = errors
        .iter()
        .flat_map(|error| error["hints"].as_array().into_iter().flatten())
        .find_map(|hint| hint.get("suggestion"))
        .context("the marker conflict has an exact edit")?;
    assert_eq!(suggestion["applicability"], "display_only");
    assert_eq!(suggestion["source"]["kind"], "location");
    let edits = suggestion["edits"]
        .as_array()
        .context("edits are an array")?;
    let [edit] = edits.as_slice() else {
        anyhow::bail!("expected one marker edit");
    };
    let offset = |position: &Value| -> Result<usize> {
        let line = usize::try_from(position["line"].as_u64().context("one-based line")?)?;
        let column = usize::try_from(
            position["byte_column"]
                .as_u64()
                .context("zero-based UTF-8 byte column")?,
        )?;
        Ok(source
            .split_inclusive('\n')
            .take(line.checked_sub(1).context("line must be positive")?)
            .map(str::len)
            .sum::<usize>()
            + column)
    };
    let start = offset(&edit["range"]["start"])?;
    let end = offset(&edit["range"]["end"])?;
    let replacement = edit["replacement"]
        .as_str()
        .context("replacement is a string")?;
    let mut updated = source.to_string();
    updated.replace_range(start..end, replacement);
    pyproject.write_str(&updated)?;
    let output = context.lock().arg("--offline").output()?;
    assert_eq!(output.status.code(), Some(0));
    Ok(())
}
