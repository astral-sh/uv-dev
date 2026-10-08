use std::os::unix::fs::PermissionsExt;

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use indoc::indoc;
use serde_json::json;

use uv_static::EnvVars;
use uv_test::uv_snapshot;

#[test]
fn python_find_reports_invalid_version_components() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filter((r"(?m)^([ \t]*)\{.*\}$", "$1[QUERY RESPONSE]"))
        .with_filter((r" at line \d+ column \d+", " at [LOCATION]"));
    let script = context.temp_dir.child("query.py");
    script.write_str(indoc! {r#"
        import json
        from pathlib import Path
        import subprocess
        import sys

        response = json.loads(subprocess.check_output([sys.executable, *sys.argv[1:]], text=True))
        response["sys_executable"] = str(Path(__file__).with_name("mock-python"))
        response["markers"]["python_full_version"] = json.loads(Path(__file__).with_name("version.json").read_text())
        print(json.dumps(response))
    "#})?;
    let executable = context.temp_dir.child("mock-python");
    let python = context
        .python_command()
        .get_program()
        .to_string_lossy()
        .replace('\'', "'\\''");
    let script_path = script.path().to_string_lossy().replace('\'', "'\\''");
    executable.write_str(&format!(
        "#!/bin/sh\nexec '{python}' '{script_path}' \"$@\"\n"
    ))?;
    fs_err::set_permissions(executable.path(), std::fs::Permissions::from_mode(0o755))?;
    let version_file = context.temp_dir.child("version.json");

    version_file.write_str(&json!("3.12.0").to_string())?;
    context
        .python_find()
        .arg("--no-cache")
        .arg(executable.path())
        .env(EnvVars::UV_NO_WRAP, "1")
        .assert()
        .success();

    version_file.write_str(&json!("3").to_string())?;
    uv_snapshot!(context.filters(), context.python_find().arg("--no-cache").arg(executable.path()).env(EnvVars::UV_NO_WRAP, "1"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to inspect Python interpreter from provided path at `mock-python`
      cause: Querying Python at `[TEMP_DIR]/mock-python` returned an invalid response: invalid `python_full_version` value `3`: expected at least 3 release components

             [stdout]
             [QUERY RESPONSE]
    ");

    version_file.write_str(&json!("3.9999.0").to_string())?;
    uv_snapshot!(context.filters(), context.python_find().arg("--no-cache").arg(executable.path()).env(EnvVars::UV_NO_WRAP, "1"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to inspect Python interpreter from provided path at `mock-python`
      cause: Querying Python at `[TEMP_DIR]/mock-python` returned an invalid response: invalid `python_full_version` value `3.9999.0`: the first 3 release components must be at most 255

             [stdout]
             [QUERY RESPONSE]
    ");
    Ok(())
}
