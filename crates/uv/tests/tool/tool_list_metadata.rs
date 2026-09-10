use std::path::{Path, PathBuf};

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use fs_err as fs;
use serde_json::{Value, json};
use url::Url;

use uv_fs::Simplified;
use uv_static::EnvVars;
use uv_test::{TestContext, site_packages_path, uv_snapshot};

fn tool_context() -> TestContext {
    uv_test::test_context_with_versions!(&["3.12"])
        .with_tool_dirs()
        .with_filtered_exe_suffix()
        .with_env(EnvVars::UV_NO_CONFIG, "1")
        .with_env(EnvVars::UV_NO_BUILD, "1")
        .with_env(EnvVars::UV_OFFLINE, "1")
}

/// Create a real, empty tool environment and an inert receipt. No entry point is installed or run.
fn create_tool(context: &TestContext, name: &str, requirements: &str) -> Result<PathBuf> {
    let tool = context.temp_dir.join("tools").join(name);
    fs::create_dir_all(context.temp_dir.join("tools"))?;
    context
        .venv()
        .arg(&tool)
        .args(["--python", "3.12", "--no-project"])
        .assert()
        .success();

    let entrypoint = context
        .temp_dir
        .join("bin")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    fs::write(
        tool.join("uv-receipt.toml"),
        format!(
            "[tool]\nrequirements = {requirements}\nentrypoints = [{{ name = {name:?}, install-path = {}, from = {name:?} }}]\n",
            serde_json::to_string(&entrypoint.to_string_lossy())?,
        ),
    )?;
    Ok(site_packages_path(&tool, "python3.12"))
}

fn write_distribution(
    site_packages: &Path,
    name: &str,
    version: &str,
    direct_url: Option<Value>,
) -> Result<PathBuf> {
    let dist_info = site_packages.join(format!("{}-{version}.dist-info", name.replace('-', "_")));
    fs::create_dir_all(&dist_info)?;
    fs::write(
        dist_info.join("METADATA"),
        format!("Metadata-Version: 2.1\nName: {name}\nVersion: {version}\n"),
    )?;
    if let Some(direct_url) = direct_url {
        fs::write(
            dist_info.join("direct_url.json"),
            serde_json::to_vec(&direct_url)?,
        )?;
    }
    Ok(dist_info)
}

fn local_source(path: &Path, editable: bool) -> Result<Value> {
    let url = Url::from_file_path(path).map_err(|()| anyhow::anyhow!("invalid fixture path"))?;
    Ok(json!({ "url": url, "dir_info": { "editable": editable } }))
}

#[test]
fn tool_list_installed_sources() -> Result<()> {
    let context = tool_context();

    let registry = create_tool(&context, "registry", r#"[{ name = "registry" }]"#)?;
    write_distribution(&registry, "registry", "1.0", None)?;

    // An editable receipt must not override the current non-editable installation.
    let stale_source = serde_json::to_string(&context.temp_dir.join("stale-source"))?;
    let regular = create_tool(
        &context,
        "regular",
        &format!(r#"[{{ name = "regular", editable = {stale_source} }}]"#),
    )?;
    write_distribution(
        &regular,
        "regular",
        "2.0",
        Some(local_source(
            &context.temp_dir.join("regular-source"),
            false,
        )?),
    )?;

    let remote = create_tool(&context, "remote", r#"[{ name = "remote" }]"#)?;
    write_distribution(
        &remote,
        "remote",
        "3.0",
        Some(json!({
            "url": "https://example.invalid/fixture",
            "dir_info": { "editable": true }
        })),
    )?;

    // The receipt is deliberately stale, and the actual source no longer exists.
    let editable = create_tool(
        &context,
        "editable",
        r#"[{ name = "editable", specifier = "==99" }]"#,
    )?;
    let missing_source = context.temp_dir.join("source tree").join("é");
    assert!(!missing_source.exists());
    write_distribution(
        &editable,
        "editable",
        "4.0",
        Some(local_source(&missing_source, true)?),
    )?;

    let legacy = create_tool(&context, "legacy-tool", r#"[{ name = "legacy-tool" }]"#)?;
    let legacy_source = context.temp_dir.join("legacy-source");
    let egg_info = legacy_source.join("legacy_tool.egg-info");
    fs::create_dir_all(&egg_info)?;
    fs::write(
        egg_info.join("PKG-INFO"),
        "Metadata-Version: 1.2\nName: legacy-tool\nVersion: 5.0\n",
    )?;
    fs::write(
        legacy.join("legacy-tool.egg-link"),
        format!("{}\n", legacy_source.simplified_display()),
    )?;

    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stdout -----
    editable v4.0 (editable from [TEMP_DIR]/source tree/é)
    - editable
    legacy-tool v5.0 (editable from [TEMP_DIR]/legacy-source)
    - legacy-tool
    registry v1.0
    - registry
    regular v2.0
    - regular
    remote v3.0
    - remote
    ");

    Ok(())
}

#[test]
fn tool_list_editable_show_flags() -> Result<()> {
    let context = tool_context();
    let installed = create_tool(
        &context,
        "fixture",
        r#"[{ name = "fixture", specifier = "==99", extras = ["extra"] }, { name = "helper", specifier = "==2" }]"#,
    )?;
    write_distribution(
        &installed,
        "fixture",
        "1.0",
        Some(local_source(&context.temp_dir.join("source"), true)?),
    )?;

    uv_snapshot!(context.filters(), context.tool_list()
        .args(["--show-paths", "--show-version-specifiers", "--show-extras", "--show-with", "--show-python"]), @"
    exit_code: 0 (success)
    ----- stdout -----
    fixture v1.0 (editable from [TEMP_DIR]/source) [required: ==99] [extras: extra] [with: helper==2] [CPython 3.12.[X]] ([TEMP_DIR]/tools/fixture)
    - fixture ([TEMP_DIR]/bin/fixture)
    ");

    Ok(())
}

#[test]
fn tool_list_selects_first_installed_distribution() -> Result<()> {
    let context = tool_context();
    let installed = create_tool(&context, "fixture", r#"[{ name = "fixture" }]"#)?;
    write_distribution(&installed, "fixture", "1.0", None)?;
    write_distribution(
        &installed,
        "fixture",
        "2.0",
        Some(local_source(&context.temp_dir.join("other-source"), true)?),
    )?;

    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stdout -----
    fixture v1.0
    - fixture
    ");

    // The version-only accessor used by `uv tool run` retains the same selection.
    uv_snapshot!(context.filters(), context.tool_run(), @"
    exit_code: 2 (failure)
    ----- stdout -----
    Provide a command to run with `uv tool run <command>`.

    The following tools are installed:

    - fixture v1.0

    See `uv tool run --help` for more information.
    ");

    Ok(())
}

#[test]
fn tool_list_missing_installed_distribution() -> Result<()> {
    let context = tool_context();
    create_tool(&context, "fixture", r#"[{ name = "fixture" }]"#)?;

    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Failed find package `fixture` in tool environment
    ");

    Ok(())
}

#[test]
fn tool_list_preserves_environment_scan_errors() -> Result<()> {
    let context = tool_context().with_filtered_python_names();
    let installed = create_tool(&context, "fixture", r#"[{ name = "fixture" }]"#)?;
    write_distribution(&installed, "fixture", "1.0", None)?;
    let broken = write_distribution(&installed, "broken", "1.0", None)?;
    fs::write(broken.join("direct_url.json"), "{")?;

    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stderr -----
    Failed to read tool environment packages at `[TEMP_DIR]/tools/fixture`: Failed to read metadata from: `[TEMP_DIR]/tools/fixture/[PYTHON-LIB]/site-packages/broken-1.0.dist-info`
    ");

    Ok(())
}

#[test]
fn tool_list_editable_source_is_single_line() -> Result<()> {
    let context = tool_context();
    let installed = create_tool(&context, "fixture", r#"[{ name = "fixture" }]"#)?;
    let source = context
        .temp_dir
        .join("source-\x1b[31mred\x1b[0m\n\r\t\u{85}\u{2028}\u{2029}\x7f");
    write_distribution(
        &installed,
        "fixture",
        "1.0",
        Some(local_source(&source, true)?),
    )?;

    let output = context.tool_list().args(["--color", "never"]).output()?;
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout)?,
        format!(
            "fixture v1.0 (editable from {}red\\n\\r\\t\\u{{85}}\\u{{2028}}\\u{{2029}})\n- fixture\n",
            context.temp_dir.join("source-").simplified_display(),
        )
    );

    Ok(())
}
