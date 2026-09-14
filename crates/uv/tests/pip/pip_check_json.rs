use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use anyhow::{Context, Result};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::{FileWriteStr, PathChild};
use indoc::indoc;
use serde_json::Value;

use uv_fs::{PortablePathBuf, Simplified, write_atomic_sync};
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_static::EnvVars;
use uv_test::json_schema::JsonSchema;
use uv_test::packse::generate_wheel;
use uv_test::{TestContext, copy_dir_ignore, uv_snapshot};

static CHECK_SCHEMA: LazyLock<std::result::Result<JsonSchema, String>> = LazyLock::new(|| {
    JsonSchema::new(include_str!(
        "../../../../docs/reference/internals/pip-check.schema.json"
    ))
    .map_err(|error| error.to_string())
});

static CHECK_JSONL_SCHEMA: LazyLock<std::result::Result<JsonSchema, String>> =
    LazyLock::new(|| {
        JsonSchema::new(include_str!(
            "../../../../docs/reference/internals/pip-check-jsonl.schema.json"
        ))
        .map_err(|error| error.to_string())
    });

fn parse_check(contents: &[u8]) -> Result<serde_json::Value> {
    CHECK_SCHEMA
        .as_ref()
        .map_err(|error| anyhow::anyhow!("invalid pip check schema: {error}"))?
        .parse(contents)
        .context("pip check schema mismatch")
}

fn check_json(context: &TestContext) -> Command {
    let mut command = context.pip_check();
    command.args([
        "--offline",
        "--output-format",
        "json",
        "--preview-features",
        "json-output",
    ]);
    command
}

fn check_jsonl(context: &TestContext) -> Command {
    let mut command = context.pip_check();
    command.args([
        "--offline",
        "--output-format",
        "jsonl",
        "--preview-features",
        "jsonl",
    ]);
    command
}

fn parse_check_jsonl(contents: &[u8]) -> Result<Value> {
    anyhow::ensure!(
        contents.ends_with(b"\n"),
        "incomplete JSONL pip-check record"
    );
    let lines = std::str::from_utf8(contents)?.lines().collect::<Vec<_>>();
    anyhow::ensure!(lines.len() == 1, "expected one JSONL pip-check result");
    let mut report = CHECK_JSONL_SCHEMA
        .as_ref()
        .map_err(|error| anyhow::anyhow!("invalid JSONL pip-check schema: {error}"))?
        .parse(lines[0].as_bytes())?;
    let event_type = report
        .as_object_mut()
        .context("JSONL pip-check report is not an object")?
        .remove("type");
    anyhow::ensure!(
        event_type == Some(Value::String("result".to_owned())),
        "JSONL pip-check event is not a result"
    );
    parse_check(&serde_json::to_vec(&report)?)
}

fn install_wheel(
    context: &TestContext,
    name: &str,
    version: &str,
    target: Option<&Path>,
) -> Result<PathBuf> {
    let name = name.parse::<PackageName>()?;
    let version = version.parse::<Version>()?;
    let (filename, contents) =
        generate_wheel(&name, &version, &[], &BTreeMap::new(), None, "py3-none-any");
    let wheel = context.temp_dir.child(filename);
    fs_err::write(&wheel, contents)?;
    let mut install = context.pip_install();
    install.args(["--offline", "--no-index", "--no-deps"]);
    if let Some(target) = target {
        install.arg("--target").arg(target);
    }
    install.arg(wheel.path()).assert().success();
    let site_packages = target.map_or_else(|| context.site_packages(), Path::to_path_buf);
    Ok(site_packages.join(format!("{}-{version}.dist-info", name.as_dist_info_name())))
}

fn write_metadata(path: &Path, name: &str, version: &str, extra: &str) -> Result<()> {
    // Installed metadata can be hardlinked to the wheel cache.
    write_atomic_sync(
        path.join("METADATA"),
        format!("Metadata-Version: 2.3\nName: {name}\nVersion: {version}\n{extra}\n"),
    )?;
    Ok(())
}

#[test]
fn pip_check_json_empty_and_preview() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_keys()
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin()
        .with_filtered_exe_suffix();

    let output = uv_snapshot!(context.filters(), context.pip_check()
        .args(["--offline", "--output-format", "json"]), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "environment": {
        "path": "[VENV]/",
        "python": {
          "path": "[VENV]/[BIN]/[PYTHON]",
          "version": "3.12.[X]",
          "implementation": "cpython",
          "key": "cpython-3.12.[X]-[PLATFORM]"
        }
      },
      "target": {
        "python_version": "3.12.[X]",
        "python_platform": null
      },
      "packages_checked": 0,
      "diagnostics": []
    }

    ----- stderr -----
    warning: The `--output-format json` option is experimental and the schema may change without warning. Pass `--preview-features json-output` to disable this warning.
    "#);
    let report = parse_check(&output.stdout)?;
    assert_eq!(report["packages_checked"], 0);
    assert_eq!(report["diagnostics"], serde_json::json!([]));
    assert_eq!(report["target"]["python_platform"], serde_json::Value::Null);
    let acknowledged = check_json(&context).assert().success().stderr("");
    assert_eq!(acknowledged.get_output().stdout, output.stdout);

    uv_snapshot!(context.pip_check().args(["--offline", "--output-format", "text"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 0 packages in [TIME]
    All installed packages are compatible
    ");
    Ok(())
}

#[test]
fn pip_check_jsonl_empty_and_preview() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let json = check_json(&context).assert().success();
    let expected = parse_check(&json.get_output().stdout)?;
    let unacknowledged = context
        .pip_check()
        .args(["--offline", "--output-format", "jsonl"])
        .assert()
        .success();
    assert_eq!(
        parse_check_jsonl(&unacknowledged.get_output().stdout)?,
        expected
    );
    let stderr = String::from_utf8_lossy(&unacknowledged.get_output().stderr);
    assert_eq!(
        stderr
            .matches("The JSONL output format is experimental")
            .count(),
        1
    );
    assert!(!stderr.contains("The `--output-format json` option is experimental"));
    let acknowledged = check_jsonl(&context).assert().success().stderr("");
    assert_eq!(
        parse_check_jsonl(&acknowledged.get_output().stdout)?,
        expected
    );
    Ok(())
}

#[test]
fn pip_check_jsonl_completed_checks() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let package = install_wheel(&context, "diag-jsonl", "1.0.0", None)?;
    write_metadata(
        &package,
        "diag-jsonl",
        "1.0.0",
        "Requires-Python: >=3.12\nRequires-Dist: diag-missing>=2 ; sys_platform == 'win32'\n",
    )?;
    let metadata = fs_err::read(package.join("METADATA"))?;
    let target = [
        "--python-version",
        "3.11",
        "--python-platform",
        "x86_64-pc-windows-msvc",
    ];
    let json = check_json(&context)
        .args(target)
        .assert()
        .code(1)
        .stderr("");
    let expected = parse_check(&json.get_output().stdout)?;
    assert_eq!(expected["packages_checked"], 1);
    assert_eq!(expected["diagnostics"].as_array().map(Vec::len), Some(2));
    let normal = check_jsonl(&context)
        .args(target)
        .assert()
        .code(1)
        .stderr("");
    assert_eq!(parse_check_jsonl(&normal.get_output().stdout)?, expected);
    for argument in ["--quiet", "--no-progress"] {
        let output = check_jsonl(&context)
            .args(target)
            .arg(argument)
            .assert()
            .code(1)
            .stderr("");
        assert_eq!(output.get_output().stdout, normal.get_output().stdout);
    }
    check_jsonl(&context)
        .args(target)
        .arg("-qq")
        .assert()
        .code(1)
        .stdout("")
        .stderr("");
    assert_eq!(fs_err::read(package.join("METADATA"))?, metadata);

    write_metadata(&package, "diag-jsonl", "1.0.0", "")?;
    let json = check_json(&context).args(target).assert().success();
    let compatible = check_jsonl(&context).args(target).assert().success();
    let report = parse_check_jsonl(&compatible.get_output().stdout)?;
    assert_eq!(report, parse_check(&json.get_output().stdout)?);
    assert_eq!(report["packages_checked"], 1);
    assert_eq!(report["diagnostics"], serde_json::json!([]));
    Ok(())
}

#[test]
fn pip_check_json_all_diagnostics() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_keys()
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin()
        .with_filtered_exe_suffix();
    let mixed = install_wheel(&context, "diag-mixed", "1.0.0", None)?;
    install_wheel(&context, "diag-installed", "2.0.0", None)?;
    let metadata = install_wheel(&context, "diag-metadata", "1.0.0", None)?;
    let tags = install_wheel(&context, "diag-tags", "1.0.0", None)?;
    install_wheel(&context, "diag-duplicate", "1.0.0", None)?;
    let duplicate_target = context.temp_dir.child("duplicate-install");
    let duplicate = install_wheel(
        &context,
        "diag-duplicate",
        "2.0.0",
        Some(duplicate_target.path()),
    )?;
    copy_dir_ignore(
        duplicate,
        context
            .site_packages()
            .join("diag_duplicate-2.0.0.dist-info"),
    )?;

    write_metadata(
        &mixed,
        "diag-mixed",
        "1.0.0",
        indoc! {r#"
            Requires-Python: >=99
            Requires-Dist: diag-missing-z >=2
            Requires-Dist: diag-installed <2
            Requires-Dist: diag-missing-a ; sys_platform == "win32"
            Requires-Dist: diag-skipped ; sys_platform != "win32"
            Requires-Dist: diag-extra ; extra == "speed"
        "#},
    )?;
    write_atomic_sync(
        mixed.join("WHEEL"),
        "Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py2-none-any\n",
    )?;
    fs_err::remove_file(metadata.join("METADATA"))?;
    fs_err::remove_file(tags.join("WHEEL"))?;
    let metadata_before = fs_err::read(mixed.join("METADATA"))?;
    let wheel_before = fs_err::read(mixed.join("WHEEL"))?;

    let command = || {
        let mut command = check_json(&context);
        command.args([
            "--python-version",
            "3.11",
            "--python-platform",
            "x86_64-pc-windows-msvc",
        ]);
        command
    };
    let output = uv_snapshot!(context.filters(), command(), @r#"
    exit_code: 1 (failure)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "environment": {
        "path": "[VENV]/",
        "python": {
          "path": "[VENV]/[BIN]/[PYTHON]",
          "version": "3.12.[X]",
          "implementation": "cpython",
          "key": "cpython-3.12.[X]-[PLATFORM]"
        }
      },
      "target": {
        "python_version": "3.11.0",
        "python_platform": "x86_64-pc-windows-msvc"
      },
      "packages_checked": 6,
      "diagnostics": [
        {
          "package": "diag-duplicate",
          "kind": "duplicate_package",
          "paths": [
            "[SITE_PACKAGES]/diag_duplicate-1.0.0.dist-info",
            "[SITE_PACKAGES]/diag_duplicate-2.0.0.dist-info"
          ]
        },
        {
          "package": "diag-metadata",
          "kind": "metadata_unavailable",
          "path": "[SITE_PACKAGES]/diag_metadata-1.0.0.dist-info"
        },
        {
          "package": "diag-mixed",
          "kind": "incompatible_dependency",
          "requirement": "diag-installed<2",
          "installed_version": "2.0.0"
        },
        {
          "package": "diag-mixed",
          "kind": "incompatible_platform"
        },
        {
          "package": "diag-mixed",
          "kind": "incompatible_python_version",
          "requires_python": ">=99",
          "installed_version": "3.12.[X]"
        },
        {
          "package": "diag-mixed",
          "kind": "missing_dependency",
          "requirement": "diag-missing-a ; sys_platform == 'win32'"
        },
        {
          "package": "diag-mixed",
          "kind": "missing_dependency",
          "requirement": "diag-missing-z>=2"
        },
        {
          "package": "diag-tags",
          "kind": "tags_unavailable",
          "path": "[SITE_PACKAGES]/diag_tags-1.0.0.dist-info"
        }
      ]
    }
    "#);
    let report = parse_check(&output.stdout)?;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(report["packages_checked"], 6);
    assert_eq!(
        report["target"],
        serde_json::json!({
            "python_version": "3.11.0",
            "python_platform": "x86_64-pc-windows-msvc",
        })
    );
    assert_eq!(report["diagnostics"].as_array().map(Vec::len), Some(8));
    let jsonl = check_jsonl(&context)
        .args([
            "--python-version",
            "3.11",
            "--python-platform",
            "x86_64-pc-windows-msvc",
        ])
        .assert()
        .code(1)
        .stderr("");
    assert_eq!(parse_check_jsonl(&jsonl.get_output().stdout)?, report);
    let repeated = command().assert().code(1).stderr("");
    assert_eq!(repeated.get_output().stdout, output.stdout);
    assert_eq!(fs_err::read(mixed.join("METADATA"))?, metadata_before);
    assert_eq!(fs_err::read(mixed.join("WHEEL"))?, wheel_before);
    Ok(())
}

#[test]
fn pip_check_json_configured_target() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let package = install_wheel(&context, "diag-python", "1.0.0", None)?;
    write_metadata(
        &package,
        "diag-python",
        "1.0.0",
        "Requires-Python: >=3.12\n",
    )?;
    let compatible = check_json(&context).assert().success();
    let compatible = parse_check(&compatible.get_output().stdout)?;

    context.temp_dir.child("uv.toml").write_str(indoc! {r#"
        [pip]
        python-version = "3.11"
        python-platform = "linux"
    "#})?;
    let incompatible = check_json(&context).assert().code(1).stderr("");
    let incompatible = parse_check(&incompatible.get_output().stdout)?;
    assert_eq!(incompatible["environment"], compatible["environment"]);
    assert_eq!(
        incompatible["target"],
        serde_json::json!({"python_version": "3.11.0", "python_platform": "linux"})
    );
    assert_eq!(
        incompatible["diagnostics"],
        serde_json::json!([{
            "package": "diag-python",
            "kind": "incompatible_python_version",
            "requires_python": ">=3.12",
            "installed_version": compatible["environment"]["python"]["version"],
        }])
    );

    let overridden = check_json(&context)
        .args(["--python-version", "3.12", "--python-platform", "windows"])
        .assert()
        .success();
    let overridden = parse_check(&overridden.get_output().stdout)?;
    assert_eq!(
        overridden["target"],
        serde_json::json!({"python_version": "3.12.0", "python_platform": "windows"})
    );
    Ok(())
}

#[test]
fn pip_check_json_omits_invalid_metadata_values() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let package = install_wheel(&context, "diag-values", "1.0.0", None)?;
    let metadata = package.join("METADATA");
    write_metadata(
        &package,
        "diag-values",
        "1.0.0",
        indoc! {r"
            Requires-Dist: diag-direct @ 'https://user:check-secret@example.com/diag_direct-1.0.0-py3-none-any.whl?X-Amz-Signature=signature-secret'
            Provides-Extra: https://user:extra-secret@example.com/private.whl?sig=extra-signature
        "},
    )?;
    let original = fs_err::read(&metadata)?;
    let repaired = check_json(&context)
        .env(EnvVars::RUST_LOG, "warn")
        .assert()
        .code(1);
    let report = parse_check(&repaired.get_output().stdout)?;
    assert_eq!(
        report["diagnostics"],
        serde_json::json!([{
            "package": "diag-values",
            "kind": "missing_dependency",
            "requirement": "diag-direct @ https://user:****@example.com/diag_direct-1.0.0-py3-none-any.whl?X-Amz-Signature=****",
        }])
    );
    let repaired_jsonl = check_jsonl(&context)
        .env(EnvVars::RUST_LOG, "warn")
        .assert()
        .code(1);
    assert_eq!(
        parse_check_jsonl(&repaired_jsonl.get_output().stdout)?,
        report
    );
    for output in [
        &repaired.get_output().stdout,
        &repaired.get_output().stderr,
        &repaired_jsonl.get_output().stdout,
        &repaired_jsonl.get_output().stderr,
    ] {
        let output = String::from_utf8_lossy(output);
        for value in [
            "check-secret",
            "signature-secret",
            "extra-secret",
            "extra-signature",
        ] {
            assert!(!output.contains(value));
        }
    }
    assert_eq!(fs_err::read(&metadata)?, original);

    for (name, version, extra) in [
        ("https://user:invalid-name-secret@example.com/", "1.0.0", ""),
        ("diag-values", "invalid-version-secret", ""),
        (
            "diag-values",
            "1.0.0",
            "Requires-Python: invalid-python-secret\n",
        ),
        (
            "diag-values",
            "1.0.0",
            "Requires-Dist: diag-direct ; invalid-requirement-secret == 'value'\n",
        ),
    ] {
        write_metadata(&package, name, version, extra)?;
        let contents = fs_err::read(&metadata)?;
        let invalid = check_json(&context)
            .env(EnvVars::RUST_LOG, "warn")
            .assert()
            .code(1);
        let report = parse_check(&invalid.get_output().stdout)?;
        assert_eq!(
            report["diagnostics"],
            serde_json::json!([{
                "package": "diag-values",
                "kind": "metadata_unavailable",
                "path": PortablePathBuf::from(package.simplified()).to_string(),
            }])
        );
        for output in [&invalid.get_output().stdout, &invalid.get_output().stderr] {
            assert!(!String::from_utf8_lossy(output).contains("-secret"));
        }
        assert_eq!(fs_err::read(&metadata)?, contents);
    }

    fs_err::remove_file(&metadata)?;
    fs_err::create_dir(&metadata)?;
    let unreadable = check_json(&context).assert().code(1);
    let report = parse_check(&unreadable.get_output().stdout)?;
    assert_eq!(report["diagnostics"][0]["kind"], "metadata_unavailable");
    assert!(metadata.is_dir());

    fs_err::remove_dir(&metadata)?;
    write_metadata(&package, "diag-values", "1.0.0", "")?;
    let wheel = package.join("WHEEL");
    write_atomic_sync(
        &wheel,
        "Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: invalid-wheel-secret-canary\n",
    )?;
    let contents = fs_err::read(&wheel)?;
    let invalid = check_json(&context)
        .env(EnvVars::RUST_LOG, "warn")
        .assert()
        .code(1);
    let report = parse_check(&invalid.get_output().stdout)?;
    assert_eq!(
        report["diagnostics"],
        serde_json::json!([{
            "package": "diag-values",
            "kind": "tags_unavailable",
            "path": PortablePathBuf::from(package.simplified()).to_string(),
        }])
    );
    for output in [&invalid.get_output().stdout, &invalid.get_output().stderr] {
        assert!(!String::from_utf8_lossy(output).contains("wheel-secret"));
    }
    assert_eq!(fs_err::read(&wheel)?, contents);
    Ok(())
}

#[test]
fn pip_check_json_quiet() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let package = install_wheel(&context, "diag-quiet", "1.0.0", None)?;
    write_metadata(
        &package,
        "diag-quiet",
        "1.0.0",
        "Requires-Dist: diag-missing\n",
    )?;

    let normal = check_json(&context).assert().code(1).stderr("");
    let quiet = check_json(&context).arg("-q").assert().code(1).stderr("");
    assert_eq!(quiet.get_output().stdout, normal.get_output().stdout);
    parse_check(&quiet.get_output().stdout)?;
    check_json(&context)
        .arg("-qq")
        .assert()
        .code(1)
        .stdout("")
        .stderr("");
    context
        .pip_check()
        .args(["--output-format", "text", "-q"])
        .assert()
        .code(1)
        .stdout("")
        .stderr("");
    Ok(())
}

#[test]
fn pip_check_json_setup_error_has_no_report() {
    let context = uv_test::test_context!("3.12");
    check_json(&context)
        .arg("--python")
        .arg(context.temp_dir.child("missing-python").path())
        .arg("--no-python-downloads")
        .assert()
        .code(2)
        .stdout("");
}

#[test]
fn pip_check_jsonl_setup_error_has_no_report() {
    let context = uv_test::test_context!("3.12");
    check_jsonl(&context)
        .arg("--python")
        .arg(context.temp_dir.child("missing-python").path())
        .arg("--no-python-downloads")
        .assert()
        .code(2)
        .stdout("");
}

#[test]
fn pip_check_json_schema_rejects_invalid_diagnostics() -> Result<()> {
    let report = serde_json::json!({
        "schema": {"version": "preview"},
        "environment": {
            "path": "/environment",
            "python": {
                "path": "/environment/bin/python",
                "version": "3.12.14",
                "implementation": "cpython",
                "key": "cpython-3.12.14-linux-x86_64-gnu",
            },
        },
        "target": {"python_version": "3.12.14", "python_platform": null},
        "packages_checked": 1,
        "diagnostics": [{
            "kind": "incompatible_dependency",
            "package": "example",
            "requirement": "dependency>=2",
            "installed_version": "1.0",
        }],
    });
    parse_check(&serde_json::to_vec(&report)?)?;

    for diagnostic in [
        serde_json::json!({"kind": "unknown", "package": "example"}),
        serde_json::json!({"kind": "missing_dependency", "package": "example"}),
        serde_json::json!({"kind": "missing_dependency", "package": "example", "requirement": null}),
        serde_json::json!({"kind": "incompatible_dependency", "package": "example", "requirement": "dependency>=2"}),
        serde_json::json!({"kind": "metadata_unavailable", "package": "example", "path": 1}),
        serde_json::json!({"kind": "tags_unavailable", "package": "example", "path": null}),
        serde_json::json!({"kind": "duplicate_package", "package": "example", "paths": [1]}),
        serde_json::json!({"kind": "incompatible_platform", "package": 1}),
    ] {
        let mut invalid = report.clone();
        invalid["diagnostics"] = serde_json::json!([diagnostic]);
        assert!(parse_check(&serde_json::to_vec(&invalid)?).is_err());
    }
    for field in ["python_version", "python_platform"] {
        let mut invalid = report.clone();
        invalid["target"][field] = serde_json::json!(42);
        assert!(parse_check(&serde_json::to_vec(&invalid)?).is_err());
    }
    Ok(())
}
