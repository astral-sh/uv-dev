use std::path::Path;

use anyhow::{Ok, Result};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::indoc;
use insta::assert_snapshot;

use uv_static::EnvVars;
use uv_test::{apply_filters, assert_path_missing, uv_snapshot};

const VERSION_QUERY_PROJECT: &str = indoc! {r#"
    [project]
    name = "myproject"
    version = '1.10.31'
    requires-python = ">=3.12"
"#};

// Print the version
#[test]
fn version_get() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(VERSION_QUERY_PROJECT)?;

    uv_snapshot!(context.filters(), context.version(), @"
    exit_code: 0 (success)
    ----- stdout -----
    myproject 1.10.31
    ");

    assert_eq!(context.read(&pyproject_toml), VERSION_QUERY_PROJECT);
    assert_path_missing(context.temp_dir.child("uv.lock"));
    assert_path_missing(&context.venv);
    Ok(())
}

// Print the version (json format)
#[test]
fn version_get_json() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(VERSION_QUERY_PROJECT)?;

    let output = uv_snapshot!(context.filters(), context.version()
        .arg("--output-format").arg("json"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "package_name": "myproject",
      "version": "1.10.31",
      "commit_info": null
    }
    "#);

    context
        .version()
        .args(["--output-format", "json", "--quiet"])
        .assert()
        .success()
        .stdout(String::from_utf8(output.stdout)?);
    context
        .version()
        .args(["--output-format", "json", "-qq"])
        .assert()
        .success()
        .stdout("");

    assert_eq!(context.read(&pyproject_toml), VERSION_QUERY_PROJECT);
    Ok(())
}

// Print the version (--short)
#[test]
fn version_get_short() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(VERSION_QUERY_PROJECT)?;

    uv_snapshot!(context.filters(), context.version()
        .arg("--short"), @"
    exit_code: 0 (success)
    ----- stdout -----
    1.10.31
    ");

    assert_eq!(context.read(&pyproject_toml), VERSION_QUERY_PROJECT);

    Ok(())
}

#[test]
fn bump_invalid_component_fails() {
    let context = uv_test::test_context_with_versions!(&[]);

    let output = uv_snapshot!(context.filters(), context.version().args(["--bump", "foo"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: invalid bump component `foo`

    Usage: uv version [OPTIONS] [VALUE]

    For more information, try '--help'.
    ");
    assert!(output.stderr.ends_with(b"\n"));
}

#[test]
fn bump_stable_with_value_fails() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.version().args(["--bump", "stable=1"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `--bump stable` does not accept a value

    Usage: uv version [OPTIONS] [VALUE]

    For more information, try '--help'.
    ");
}

#[test]
fn bump_empty_value_fails() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.version().args(["--bump", "patch="]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `--bump` values cannot be empty

    Usage: uv version [OPTIONS] [VALUE]

    For more information, try '--help'.
    ");
}

#[test]
fn bump_invalid_numeric_value_fails() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(context.filters(), context.version().args(["--bump", "dev=foo"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: invalid numeric value `foo` for `--bump dev`

    Usage: uv version [OPTIONS] [VALUE]

    For more information, try '--help'.
    ");
}

#[test]
fn bump_invalid_component_color() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    let output = context
        .version()
        .arg("--bump")
        .arg("foo")
        .env_remove(EnvVars::NO_COLOR)
        .env(EnvVars::CLICOLOR_FORCE, "1")
        .output()?;

    assert_eq!(output.status.code(), Some(2));
    let stderr = apply_filters(
        String::from_utf8_lossy(&output.stderr).into_owned(),
        context.filters(),
    );
    assert_snapshot!(
        format!("{stderr:?}"),
        @r#""\u{1b}[1m\u{1b}[31merror:\u{1b}[0m invalid bump component `foo`\n\n\u{1b}[1m\u{1b}[32mUsage:\u{1b}[0m \u{1b}[1m\u{1b}[36muv version\u{1b}[0m \u{1b}[36m[OPTIONS]\u{1b}[0m \u{1b}[36m[VALUE]\u{1b}[0m\n\nFor more information, try '\u{1b}[1m\u{1b}[36m--help\u{1b}[0m'.\n""#
    );

    Ok(())
}

// Set version --dry-run
#[test]
fn version_set_dry() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    let original = r#"
[project]
name = "myproject"
version = "1.10.31"
requires-python = ">=3.12"
"#;
    pyproject_toml.write_str(original)?;

    uv_snapshot!(context.filters(), context.version()
        .arg("1.2.3")
        .arg("--dry-run"), @"
    exit_code: 0 (success)
    ----- stdout -----
    myproject 1.10.31 => 1.2.3
    ");

    assert_eq!(context.read(&pyproject_toml), original);
    assert_path_missing(context.temp_dir.child("uv.lock"));
    assert_path_missing(&context.venv);
    Ok(())
}

// Bump version --dry-run
#[test]
fn version_major_dry() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    let original = r#"
[project]
name = "myproject"
version = "1.10.31"
requires-python = ">=3.12"
"#;
    pyproject_toml.write_str(original)?;

    uv_snapshot!(context.filters(), context.version()
        .arg("--bump").arg("major")
        .arg("--dry-run"), @"
    exit_code: 0 (success)
    ----- stdout -----
    myproject 1.10.31 => 2.0.0
    ");

    assert_eq!(context.read(&pyproject_toml), original);
    assert_path_missing(context.temp_dir.child("uv.lock"));
    assert_path_missing(&context.venv);
    Ok(())
}

/// Whether this build should include Git metadata in its version output.
fn git_version_info_expected() -> bool {
    if env!("PROFILE") != "release" && option_env!("UV_INTERNAL__BUILD_GIT_INFO") != Some("1") {
        return false;
    }

    let manifest_dir =
        std::env::var(EnvVars::CARGO_MANIFEST_DIR).expect("CARGO_MANIFEST_DIR not defined");
    let workspace_root = Path::new(&manifest_dir)
        .parent()
        .expect("parent of manifest dir missing")
        .parent()
        .expect("grandparent of manifest dir missing");
    workspace_root.join(".git").exists() && !workspace_root.join(".jj").exists()
}

// `uv self version`
// (also setup a honeypot project and make sure it's not used)
#[test]
fn self_version() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "myapp"
        version = "0.1.2"
        "#,
    )?;

    uv_snapshot!(context.filters(), context.self_version(), @"
    exit_code: 0 (success)
    ----- stdout -----
    uv [VERSION] ([COMMIT] DATE)
    ");

    let pyproject = context.read("pyproject.toml");
    assert_snapshot!(
        pyproject,
    @r#"

    [project]
    name = "myapp"
    version = "0.1.2"
    "#
    );
    Ok(())
}

// `uv self version --short`
// (also setup a honeypot project and make sure it's not used)
#[test]
fn self_version_short() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "myapp"
        version = "0.1.2"
        "#,
    )?;

    let filters = context
        .filters()
        .into_iter()
        .chain([(r"\d+\.\d+\.\d+(-alpha\.\d+)?(\+\d+)?", r"[VERSION]")])
        .collect::<Vec<_>>();
    uv_snapshot!(filters, context.self_version()
        .arg("--short"), @"
    exit_code: 0 (success)
    ----- stdout -----
    [VERSION]
    ");

    let pyproject = context.read("pyproject.toml");
    assert_snapshot!(
        pyproject,
    @r#"

    [project]
    name = "myapp"
    version = "0.1.2"
    "#
    );
    Ok(())
}

// `uv self version --output-format json`
// (also setup a honeypot project and make sure it's not used)
#[test]
fn self_version_json() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filter((
            r#"version": "\d+\.\d+\.\d+(-(alpha|beta|rc)\.\d+)?(\+\d+)?""#,
            r#"version": "[VERSION]""#,
        ))
        .with_filter((
            r#""short_commit_hash": ".*""#,
            r#""short_commit_hash": "[HASH]""#,
        ))
        .with_filter((r#""commit_hash": ".*""#, r#""commit_hash": "[LONGHASH]""#))
        .with_filter((r#"commit_date": ".*""#, r#"commit_date": "[DATE]""#))
        .with_filter((r#"last_tag": (".*"|null)"#, r#"last_tag": "[TAG]""#))
        .with_filter((
            r#"commits_since_last_tag": .*"#,
            r#"commits_since_last_tag": [COUNT]"#,
        ))
        .with_filter((r#"target_triple": ".*""#, r#"target_triple": "[TARGET]""#));

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "myapp"
        version = "0.1.2"
        "#,
    )?;

    let output = if git_version_info_expected() {
        uv_snapshot!(context.filters(), context.self_version()
          .arg("--output-format").arg("json"), @r#"
        exit_code: 0 (success)
        ----- stdout -----
        {
          "package_name": "uv",
          "version": "[VERSION]",
          "commit_info": {
            "short_commit_hash": "[HASH]",
            "commit_hash": "[LONGHASH]",
            "commit_date": "[DATE]",
            "last_tag": "[TAG]",
            "commits_since_last_tag": [COUNT]
          },
          "target_triple": "[TARGET]"
        }
        "#)
    } else {
        uv_snapshot!(context.filters(), context.self_version()
          .arg("--output-format").arg("json"), @r#"
      exit_code: 0 (success)
      ----- stdout -----
      {
        "package_name": "uv",
        "version": "[VERSION]",
        "commit_info": null,
        "target_triple": "[TARGET]"
      }
      "#)
    };

    context
        .self_version()
        .args(["--output-format", "json", "--quiet"])
        .assert()
        .success()
        .stdout(String::from_utf8(output.stdout)?);
    context
        .self_version()
        .args(["--output-format", "json", "-qq"])
        .assert()
        .success()
        .stdout("");

    let pyproject = context.read("pyproject.toml");
    assert_snapshot!(
        pyproject,
    @r#"

    [project]
    name = "myapp"
    version = "0.1.2"
    "#
    );
    Ok(())
}
