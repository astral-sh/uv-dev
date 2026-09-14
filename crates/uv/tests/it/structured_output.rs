use std::path::Path;
use std::process::Output;
#[cfg(unix)]
use std::process::Stdio;

use anyhow::{Context, Result};
use assert_cmd::assert::OutputAssertExt;
use fs_err as fs;
use indoc::indoc;
use serde_json::{Value, json};
use url::Url;

use uv_fs::{PortablePathBuf, Simplified};
use uv_static::EnvVars;
use uv_test::json_schema::JsonSchema;
use uv_test::jsonl::{JsonlOutput, JsonlResultExpectation};
use uv_test::{TestContext, site_packages_path};

fn offline_context() -> TestContext {
    uv_test::test_context_with_versions!(&["3.12"])
        .with_tool_dirs()
        .with_env(EnvVars::UV_NO_CONFIG, "1")
        .with_env(EnvVars::UV_NO_BUILD, "1")
        .with_env(EnvVars::UV_OFFLINE, "1")
        .with_env(
            EnvVars::UV_PREVIEW_FEATURES,
            "json-output,jsonl,workspace-metadata",
        )
}

fn write_distribution(site_packages: &Path, editable: Option<&Path>) -> Result<()> {
    let dist_info = site_packages.join("fixture-1.0.dist-info");
    fs::create_dir_all(&dist_info)?;
    fs::write(
        dist_info.join("METADATA"),
        "Metadata-Version: 2.1\nName: fixture\nVersion: 1.0\n",
    )?;
    if let Some(editable) = editable {
        let url = Url::from_file_path(editable)
            .map_err(|()| anyhow::anyhow!("invalid editable fixture path"))?;
        fs::write(
            dist_info.join("direct_url.json"),
            serde_json::to_vec(&json!({"url": url, "dir_info": {"editable": true}}))?,
        )?;
    }
    Ok(())
}

/// Create a real tool environment with an inert receipt. The command is never executed.
fn create_tool(context: &TestContext, command_name: &str) -> Result<()> {
    let tool = context.temp_dir.join("tools/fixture");
    fs::create_dir_all(context.temp_dir.join("tools"))?;
    context
        .venv()
        .arg(&tool)
        .args(["--python", "3.12", "--no-project"])
        .assert()
        .success();
    write_distribution(&site_packages_path(&tool, "python3.12"), None)?;
    fs::write(
        tool.join("uv-receipt.toml"),
        toml::to_string(&json!({
            "tool": {
                "requirements": ["fixture==1.0"],
                "entrypoints": [{
                    "name": command_name,
                    "install-path": context.temp_dir.join("bin/fixture"),
                    "from": "fixture"
                }]
            }
        }))?,
    )?;
    Ok(())
}

fn parse_report(
    output: &Output,
    jsonl: bool,
    schema: &JsonSchema,
    jsonl_schema: &JsonSchema,
) -> Result<Value> {
    if !jsonl {
        return schema.parse(&output.stdout);
    }
    let parsed = JsonlOutput::parse(jsonl_schema, output, JsonlResultExpectation::Required)?;
    assert!(parsed.progress.is_empty());
    let mut report = parsed.result.context("missing final report")?;
    report
        .as_object_mut()
        .context("report is not an object")?
        .remove("type");
    schema.parse(&serde_json::to_vec(&report)?)
}

#[test]
fn tool_list_serialized_controls_ignore_color() -> Result<()> {
    let context = offline_context();
    let mut command_name: String = (0_u8..=31).map(char::from).collect();
    command_name.push_str("\u{7f}\u{85}\u{2028}\u{2029}\x1b[31mquoted\"\\path\x1b[0m");
    create_tool(&context, &command_name)?;
    let schema = JsonSchema::new(include_str!(
        "../../../../docs/reference/internals/tool-list.schema.json"
    ))?;
    let jsonl_schema = JsonSchema::new(include_str!(
        "../../../../docs/reference/internals/tool-list-jsonl.schema.json"
    ))?;

    for color in ["auto", "never", "always"] {
        for (format, jsonl) in [("json", false), ("jsonl", true)] {
            for quiet in 0..=2 {
                let mut command = context.tool_list();
                command
                    .args(["--output-format", format, "--color", color])
                    .args(std::iter::repeat_n("--quiet", quiet));
                let output = command.assert().success();
                let output = output.get_output();
                if quiet == 2 {
                    assert!(output.stdout.is_empty());
                    if jsonl {
                        JsonlOutput::parse(
                            &jsonl_schema,
                            output,
                            JsonlResultExpectation::Forbidden,
                        )?;
                    }
                    continue;
                }
                let report = parse_report(output, jsonl, &schema, &jsonl_schema)?;
                assert_eq!(report["tools"][0]["commands"][0]["name"], command_name);
            }
        }
    }
    Ok(())
}

#[test]
fn pip_list_serialized_path_ignores_color() -> Result<()> {
    let context = offline_context();
    context
        .venv()
        .arg(context.venv.path())
        .args(["--python", "3.12", "--no-project"])
        .assert()
        .success();
    let source = context.temp_dir.join("source\u{7f}\u{85}\u{2028}\u{2029}");
    fs::create_dir_all(&source)?;
    write_distribution(&context.site_packages(), Some(&source))?;
    let expected = json!([{
        "name": "fixture",
        "version": "1.0",
        "editable_project_location": source.simplified_display().to_string()
    }]);

    for color in ["auto", "never", "always"] {
        for quiet in 0..=2 {
            let mut command = context.pip_list();
            command
                .args(["--format", "json", "--color", color])
                .args(std::iter::repeat_n("--quiet", quiet));
            let output = command.assert().success();
            if quiet == 2 {
                assert!(output.get_output().stdout.is_empty());
            } else {
                let report: Value = serde_json::from_slice(&output.get_output().stdout)?;
                assert_eq!(report, expected);
            }
        }
    }
    Ok(())
}

fn create_workspace(context: &TestContext, workspace: &Path) -> Result<()> {
    fs::create_dir_all(workspace)?;
    fs::write(
        workspace.join("pyproject.toml"),
        indoc! {r#"
            [project]
            name = "fixture"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = []
        "#},
    )?;
    context
        .lock()
        .current_dir(workspace)
        .args(["--python", "3.12"])
        .assert()
        .success();
    Ok(())
}

#[test]
fn workspace_metadata_serialized_path_ignores_color() -> Result<()> {
    let context = offline_context();
    let workspace = context
        .temp_dir
        .join("workspace\u{7f}\u{85}\u{2028}\u{2029}");
    create_workspace(&context, &workspace)?;
    let expected = serde_json::to_value(PortablePathBuf::from(workspace.as_path()))?;
    let schema = JsonSchema::new(include_str!(
        "../../../../docs/reference/internals/metadata.schema.json"
    ))?;
    let jsonl_schema = JsonSchema::new(include_str!(
        "../../../../docs/reference/internals/metadata-jsonl.schema.json"
    ))?;

    for color in ["auto", "never", "always"] {
        for (format, jsonl) in [("json", false), ("jsonl", true)] {
            for quiet in 0..=2 {
                let mut command = context.workspace_metadata();
                command
                    .current_dir(&workspace)
                    .args([
                        "--frozen",
                        "--no-progress",
                        "--output-format",
                        format,
                        "--color",
                        color,
                    ])
                    .args(std::iter::repeat_n("--quiet", quiet));
                let output = command.assert().success();
                let output = output.get_output();
                if quiet == 2 {
                    assert!(output.stdout.is_empty());
                    if jsonl {
                        JsonlOutput::parse(
                            &jsonl_schema,
                            output,
                            JsonlResultExpectation::Forbidden,
                        )?;
                    }
                    continue;
                }
                let report = parse_report(output, jsonl, &schema, &jsonl_schema)?;
                assert_eq!(report["workspace_root"], expected);
            }
        }
    }
    Ok(())
}

fn create_export_workspace(context: &TestContext, member_path: &str) -> Result<()> {
    let workspace = context.temp_dir.path();
    let member = workspace.join(member_path);
    fs::create_dir_all(&member)?;
    fs::write(
        workspace.join("pyproject.toml"),
        toml::to_string(&json!({
            "project": {
                "name": "root",
                "version": "0.1.0",
                "requires-python": ">=3.12",
                "dependencies": ["member"]
            },
            "tool": {"uv": {
                "package": false,
                "sources": {"member": {"workspace": true}},
                "workspace": {"members": [member_path]}
            }}
        }))?,
    )?;
    fs::write(
        member.join("pyproject.toml"),
        indoc! {r#"
            [project]
            name = "member"
            version = "0.1.0"
            requires-python = ">=3.12"
            dependencies = []

            [tool.uv]
            package = false
        "#},
    )?;
    context.lock().args(["--python", "3.12"]).assert().success();
    Ok(())
}

fn assert_cyclonedx_workspace_path(bytes: &[u8], expected: &str) -> Result<()> {
    let bom: Value = serde_json::from_slice(bytes)?;
    assert_eq!(bom["bomFormat"], "CycloneDX");
    assert_eq!(bom["specVersion"], "1.5");
    let values: Vec<_> = bom["components"]
        .as_array()
        .context("missing CycloneDX components")?
        .iter()
        .filter(|component| component["name"] == "member")
        .flat_map(|component| component["properties"].as_array().into_iter().flatten())
        .filter(|property| property["name"] == "uv:workspace:path")
        .map(|property| property["value"].as_str())
        .collect();
    assert_eq!(values, [Some(expected)]);
    Ok(())
}

#[test]
fn cyclonedx_export_serialized_path_ignores_color() -> Result<()> {
    let context = offline_context().with_env(EnvVars::UV_PREVIEW_FEATURES, "sbom-export");
    let member_path = "member\u{7f}\u{85}\u{2028}\u{2029}";
    create_export_workspace(&context, member_path)?;

    for color in ["auto", "never", "always"] {
        for quiet in 0..=2 {
            for with_file in [false, true] {
                let output_file = context
                    .temp_dir
                    .join(format!("bom-{color}-{quiet}-{with_file}.json"));
                let mut command = context.export();
                command
                    .args([
                        "--frozen",
                        "--format",
                        "cyclonedx1.5",
                        "--no-hashes",
                        "--color",
                        color,
                    ])
                    .args(std::iter::repeat_n("--quiet", quiet));
                if with_file {
                    command.arg("--output-file").arg(&output_file);
                }
                let output = command.assert().success();
                let output = output.get_output();
                if !with_file || quiet == 0 {
                    assert_cyclonedx_workspace_path(&output.stdout, member_path)?;
                } else {
                    assert!(output.stdout.is_empty());
                }
                if with_file {
                    assert_cyclonedx_workspace_path(&fs::read(&output_file)?, member_path)?;
                }
            }
        }
    }
    Ok(())
}

fn assert_styled_output_file(output: &Output, output_file: &Path) -> Result<()> {
    assert!(output.stdout.windows(2).any(|bytes| bytes == b"\x1b["));
    let contents = fs::read(output_file)?;
    assert!(!contents.contains(&b'\x1b'));
    assert_eq!(
        contents,
        anstream::adapter::strip_bytes(&output.stdout).into_vec()
    );
    Ok(())
}

#[test]
fn text_export_and_compile_keep_styled_stdout() -> Result<()> {
    let context = offline_context();
    let workspace = context.temp_dir.join("workspace");
    create_workspace(&context, &workspace)?;
    fs::write(workspace.join("requirements.in"), "")?;

    for (format, export_file, compile_file) in [
        ("requirements.txt", "export.txt", "compile.txt"),
        ("pylock.toml", "pylock.export.toml", "pylock.compile.toml"),
    ] {
        let export_file = workspace.join(export_file);
        let output = context
            .export()
            .current_dir(&workspace)
            .args(["--frozen", "--format", format, "--color", "always"])
            .arg("--output-file")
            .arg(&export_file)
            .assert()
            .success();
        assert_styled_output_file(output.get_output(), &export_file)?;

        let compile_file = workspace.join(compile_file);
        let output = context
            .pip_compile()
            .current_dir(&workspace)
            .args(["requirements.in", "--format", format, "--color", "always"])
            .arg("--output-file")
            .arg(&compile_file)
            .assert()
            .success();
        assert_styled_output_file(output.get_output(), &compile_file)?;
    }
    Ok(())
}

/// A failed stdout write must not commit an output-file buffer.
#[test]
#[cfg(unix)]
fn cyclonedx_export_closed_pipe_leaves_output_file() -> Result<()> {
    let context = offline_context().with_env(EnvVars::UV_PREVIEW_FEATURES, "sbom-export");
    create_export_workspace(&context, "member")?;
    let output_file = context.temp_dir.join("bom.json");
    fs::write(&output_file, "untouched\n")?;

    for with_file in [false, true] {
        let (read, write) = nix::unistd::pipe()?;
        drop(read);
        let mut command = context.export();
        command.args([
            "--frozen",
            "--format",
            "cyclonedx1.5",
            "--no-hashes",
            "--color",
            "never",
        ]);
        if with_file {
            command.arg("--output-file").arg(&output_file);
        }
        command
            .stdout(Stdio::from(write))
            .assert()
            .code(2)
            .stderr(predicates::str::contains("Broken pipe"));
        assert_eq!(fs::read(&output_file)?, b"untouched\n");
    }
    Ok(())
}

/// A consumer that closes its pipe keeps the command's established output-error policy.
#[test]
#[cfg(unix)]
fn structured_output_closed_pipe_status() -> Result<()> {
    let context = offline_context();
    create_tool(&context, "fixture")?;
    let workspace = context.temp_dir.join("workspace");
    create_workspace(&context, &workspace)?;

    for format in ["json", "jsonl"] {
        let (read, write) = nix::unistd::pipe()?;
        drop(read);
        context
            .tool_list()
            .args(["--output-format", format, "--color", "never"])
            .stdout(Stdio::from(write))
            .assert()
            .success()
            .stderr("");

        let (read, write) = nix::unistd::pipe()?;
        drop(read);
        context
            .workspace_metadata()
            .current_dir(&workspace)
            .args([
                "--frozen",
                "--no-progress",
                "--output-format",
                format,
                "--color",
                "never",
            ])
            .stdout(Stdio::from(write))
            .assert()
            .code(2)
            .stderr("error: Broken pipe (os error 32)\n");
    }
    Ok(())
}
