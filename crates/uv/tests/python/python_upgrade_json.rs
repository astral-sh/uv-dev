use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::{fixture::ChildPath, prelude::*};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use url::Url;

use uv_python::downloads::{ManagedPythonDownloadList, PythonDownloadRequest};
use uv_static::EnvVars;
use uv_test::TestContext;

const CPYTHON_RELEASES: &str =
    "https://github.com/astral-sh/python-build-standalone/releases/download";

struct PythonFixture {
    key: String,
    metadata: Value,
}

/// Select an exact native distribution from the embedded catalog.
fn python_metadata(context: &TestContext, version: &str) -> Result<PythonFixture> {
    let downloads = ManagedPythonDownloadList::new_only_embedded()?;
    let request = version.parse::<PythonDownloadRequest>()?.fill_platform()?;
    let download = downloads.find(&request)?;
    let url = download
        .download_urls(Some(CPYTHON_RELEASES), None)?
        .into_iter()
        .next()
        .context("CPython fixture has no download URL")?;
    let catalog: Map<String, Value> = serde_json::from_slice(&fs_err::read(
        context
            .workspace_root
            .join("crates/uv-python/download-metadata.json"),
    )?)?;
    let metadata = catalog
        .into_iter()
        .find_map(|(_, metadata)| {
            (metadata["url"].as_str() == Some(url.as_str())).then_some(metadata)
        })
        .context("CPython fixture is missing from the download metadata")?;
    Ok(PythonFixture {
        key: download.key().to_string(),
        metadata,
    })
}

/// Reuse a checksum-verified real archive, then make the test's catalog entirely local.
fn python_fixture(context: &TestContext, version: &str) -> Result<PythonFixture> {
    let mut fixture = python_metadata(context, version)?;
    let url = Url::parse(
        fixture.metadata["url"]
            .as_str()
            .context("CPython fixture has no URL")?,
    )?;
    let filename = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .context("CPython fixture URL has no filename")?
        .replace("%2B", "-");
    let sha256 = fixture.metadata["sha256"]
        .as_str()
        .context("CPython fixture has no SHA-256 digest")?;
    let cache_filename = format!(
        "{}-{filename}",
        sha256
            .get(..9)
            .context("CPython fixture has an invalid SHA-256 digest")?
    );

    let archive_context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();
    let mut prepare = archive_context.python_install();
    let archive_cache = prepare
        .get_envs()
        .find_map(|(name, value)| (name == EnvVars::UV_PYTHON_CACHE_DIR).then_some(value))
        .flatten()
        .filter(|value| !value.is_empty())
        .map_or_else(
            || archive_context.cache_dir.join("python-archive-fixture"),
            PathBuf::from,
        );
    let archive_cache = if archive_cache.is_absolute() {
        archive_cache
    } else {
        archive_context.temp_dir.join(archive_cache)
    };
    let archive_path = archive_cache.join(cache_filename);
    if !archive_path.try_exists()? {
        prepare
            .arg(&fixture.key)
            .args(["--no-bin", "--no-registry"])
            .env(EnvVars::UV_PYTHON_CACHE_DIR, &archive_cache)
            .assert()
            .success();
    }
    let contents = fs_err::read(&archive_path)?;
    assert_eq!(hex::encode(Sha256::digest(&contents)), sha256);
    let archive = context.temp_dir.child(filename);
    archive.write_binary(&contents)?;
    fixture.metadata["url"] = json!(
        Url::from_file_path(archive.path())
            .map_err(|()| anyhow::anyhow!("failed to create the fixture archive URL"))?
    );
    Ok(fixture)
}

fn write_catalog(context: &TestContext, fixtures: &[PythonFixture]) -> Result<ChildPath> {
    let catalog = fixtures
        .iter()
        .map(|fixture| (fixture.key.clone(), fixture.metadata.clone()))
        .collect::<Map<_, _>>();
    let path = context.temp_dir.child("python-downloads.json");
    path.write_str(&serde_json::to_string(&catalog)?)?;
    Ok(path)
}

fn local_catalog(command: &mut Command, catalog: &Path) {
    command
        .args(["--no-config", "--offline"])
        .arg("--python-downloads-json-url")
        .arg(catalog)
        .env(EnvVars::UV_PYTHON_CACHE_DIR, "")
        .env_remove(EnvVars::UV_PYTHON_INSTALL_MIRROR)
        .env_remove(EnvVars::UV_PYPY_INSTALL_MIRROR)
        .env_remove(EnvVars::RUST_LOG);
}

fn install(context: &TestContext, catalog: &Path, versions: &[&str]) {
    let mut command = context.python_install();
    local_catalog(&mut command, catalog);
    command.args(versions).assert().success();
}

fn upgrade(context: &TestContext, catalog: &Path) -> Command {
    let mut command = context.python_upgrade();
    local_catalog(&mut command, catalog);
    command.args([
        "--output-format",
        "json",
        "--preview-features",
        "json-output",
    ]);
    command
}

fn report(output: &Output, code: i32) -> Result<Value> {
    assert_eq!(output.status.code(), Some(code));
    let report: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(report["schema"], json!({"version": "preview"}));
    assert!(report["upgrades"].is_array());
    Ok(report)
}

/// Keep snapshots focused on request association and completed changes, not platform identity.
fn summary(report: &Value) -> Result<Value> {
    let entries = report["upgrades"]
        .as_array()
        .context("missing upgrade entries")?
        .iter()
        .map(|entry| {
            let versions = |field: &str| -> Result<Vec<Value>> {
                Ok(entry[field]
                    .as_array()
                    .context("missing installations")?
                    .iter()
                    .map(|installation| installation["version"].clone())
                    .collect())
            };
            let executables = entry["executables"]
                .as_array()
                .context("missing executable changes")?
                .iter()
                .map(|change| {
                    json!({"from": change["from"]["version"], "to": change["to"]["version"]})
                })
                .collect::<Vec<_>>();
            let errors = entry["errors"]
                .as_array()
                .context("missing errors")?
                .iter()
                .map(|error| error["kind"].clone())
                .collect::<Vec<_>>();
            Ok(json!({
                "request": entry["request"],
                "outcome": entry["outcome"],
                "from": versions("from")?,
                "to": entry["to"]["version"],
                "executables": executables,
                "errors": errors,
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(json!(entries))
}

#[test]
fn python_upgrade_json_noop() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();
    let fixture = python_fixture(&context, "cpython-3.12.9")?;
    let key = fixture.key.clone();
    let catalog = write_catalog(&context, &[fixture])?;

    let empty = report(&upgrade(&context, catalog.path()).output()?, 0)?;
    insta::assert_json_snapshot!(empty, @r#"
    {
      "errors": [],
      "schema": {
        "version": "preview"
      },
      "upgrades": []
    }
    "#);

    install(&context, catalog.path(), &["3.12.9"]);
    let installed = report(&upgrade(&context, catalog.path()).arg("3.12").output()?, 0)?;
    insta::assert_json_snapshot!(summary(&installed)?, @r#"
    [
      {
        "errors": [],
        "executables": [],
        "from": [
          "3.12.9"
        ],
        "outcome": "no_op",
        "request": "3.12",
        "to": "3.12.9"
      }
    ]
    "#);
    assert_eq!(installed["upgrades"][0]["from"][0]["key"], key);
    assert_eq!(
        installed["upgrades"][0]["from"][0],
        installed["upgrades"][0]["to"]
    );
    assert_eq!(
        installed["upgrades"][0]["to"]["version_parts"],
        json!({"major": 3, "minor": 12, "patch": 9})
    );
    Ok(())
}

#[test]
fn python_upgrade_json_patch_associates_retained_installations() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();
    let fixtures = [
        python_fixture(&context, "cpython-3.12.7")?,
        python_fixture(&context, "cpython-3.12.8")?,
        python_fixture(&context, "cpython-3.12.9")?,
    ];
    let catalog = write_catalog(&context, &fixtures)?;
    install(&context, catalog.path(), &["3.12.7", "3.12.8"]);

    let upgraded = report(&upgrade(&context, catalog.path()).arg("3.12").output()?, 0)?;
    insta::assert_json_snapshot!(summary(&upgraded)?, @r#"
    [
      {
        "errors": [],
        "executables": [
          {
            "from": "3.12.8",
            "to": "3.12.9"
          }
        ],
        "from": [
          "3.12.7",
          "3.12.8"
        ],
        "outcome": "upgraded",
        "request": "3.12",
        "to": "3.12.9"
      }
    ]
    "#);
    for fixture in &fixtures {
        assert!(
            context
                .temp_dir
                .child("managed")
                .child(&fixture.key)
                .is_dir()
        );
    }
    let executable = context
        .bin_dir
        .child(format!("python3.12{}", std::env::consts::EXE_SUFFIX));
    assert!(executable.is_file());
    assert_eq!(
        upgraded["upgrades"][0]["executables"][0]["path"],
        executable.path().to_string_lossy().replace('\\', "/")
    );
    assert_eq!(upgraded["upgrades"][0]["to"]["key"], fixtures[2].key);
    Ok(())
}

#[test]
fn python_upgrade_json_same_key_build_replacement() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();
    let fixture = python_fixture(&context, "cpython-3.12.9")?;
    let key = fixture.key.clone();
    let expected_build = fixture.metadata["build"]
        .as_str()
        .context("fixture has no build")?
        .to_owned();
    let catalog = write_catalog(&context, &[fixture])?;
    install(&context, catalog.path(), &["3.12.9"]);
    let build = context.temp_dir.child("managed").child(&key).child("BUILD");
    build.write_str("19000101")?;

    let upgraded = report(&upgrade(&context, catalog.path()).arg("3.12").output()?, 0)?;
    let entry = &upgraded["upgrades"][0];
    assert_eq!(entry["from"][0]["key"], key);
    assert_eq!(entry["to"]["key"], key);
    assert_eq!(entry["from"][0]["build"], "19000101");
    assert_eq!(entry["to"]["build"], expected_build);
    assert_eq!(fs_err::read_to_string(build.path())?, expected_build);
    insta::assert_json_snapshot!(summary(&upgraded)?, @r#"
    [
      {
        "errors": [],
        "executables": [],
        "from": [
          "3.12.9"
        ],
        "outcome": "upgraded",
        "request": "3.12",
        "to": "3.12.9"
      }
    ]
    "#);
    Ok(())
}

#[test]
fn python_upgrade_json_partial_failure() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();
    let old = python_fixture(&context, "cpython-3.12.8")?;
    let new = python_fixture(&context, "cpython-3.12.9")?;
    let mut missing = python_metadata(&context, "cpython-3.13.1")?;
    missing.metadata["url"] = json!(
        Url::from_file_path(context.temp_dir.child("missing.tar.gz").path())
            .map_err(|()| anyhow::anyhow!("failed to create missing archive URL"))?
    );
    missing.metadata["sha256"] = Value::Null;
    let missing_key = missing.key.clone();
    let catalog = write_catalog(&context, &[old, new, missing])?;
    install(&context, catalog.path(), &["3.12.8"]);

    // Reverse the argument order to ensure completed results are sorted independently of the
    // download completion order. The failed download must not hide the successful patch upgrade.
    let upgraded = report(
        &upgrade(&context, catalog.path())
            .args(["3.13", "3.12"])
            .output()?,
        1,
    )?;
    insta::assert_json_snapshot!(summary(&upgraded)?, @r#"
    [
      {
        "errors": [],
        "executables": [
          {
            "from": "3.12.8",
            "to": "3.12.9"
          }
        ],
        "from": [
          "3.12.8"
        ],
        "outcome": "upgraded",
        "request": "3.12",
        "to": "3.12.9"
      },
      {
        "errors": [
          "download"
        ],
        "executables": [],
        "from": [],
        "outcome": "failed",
        "request": "3.13",
        "to": null
      }
    ]
    "#);
    assert!(
        !context
            .temp_dir
            .child("managed")
            .child(missing_key)
            .exists()
    );
    assert!(
        upgraded["upgrades"][1]["errors"][0]["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty())
    );
    Ok(())
}

#[test]
fn python_upgrade_json_unmanaged_executable() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();
    let fixture = python_fixture(&context, "cpython-3.12.9")?;
    let catalog = write_catalog(&context, &[fixture])?;
    let executable = context
        .bin_dir
        .child(format!("python3.12{}", std::env::consts::EXE_SUFFIX));
    executable.write_str("unmanaged executable sentinel\n")?;

    let upgraded = report(&upgrade(&context, catalog.path()).arg("3.12").output()?, 0)?;
    insta::assert_json_snapshot!(summary(&upgraded)?, @r#"
    [
      {
        "errors": [],
        "executables": [],
        "from": [],
        "outcome": "installed",
        "request": "3.12",
        "to": "3.12.9"
      }
    ]
    "#);
    assert_eq!(
        fs_err::read_to_string(executable.path())?,
        "unmanaged executable sentinel\n"
    );
    Ok(())
}

#[test]
fn python_upgrade_json_quiet() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();
    let catalog = write_catalog(&context, &[])?;
    let normal = upgrade(&context, catalog.path()).output()?;
    let quiet = upgrade(&context, catalog.path()).arg("-q").output()?;
    assert_eq!(report(&normal, 0)?, report(&quiet, 0)?);
    assert!(quiet.stderr.is_empty());
    let silent = upgrade(&context, catalog.path()).arg("-qq").output()?;
    assert!(silent.status.success());
    assert!(silent.stdout.is_empty());
    assert!(silent.stderr.is_empty());
    Ok(())
}

#[test]
fn python_upgrade_json_reinstall_any_reports_executed_keys() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();
    let first = python_fixture(&context, "cpython-3.12.8")?;
    let second = python_fixture(&context, "cpython-3.13.1")?;
    let newest = python_metadata(&context, "cpython-3.13.2")?;
    let newest_key = newest.key.clone();
    let catalog = write_catalog(&context, &[first, second, newest])?;
    install(&context, catalog.path(), &["3.12.8", "3.13.1"]);

    let reinstalled = report(
        &upgrade(&context, catalog.path())
            .args(["--reinstall", "any"])
            .output()?,
        0,
    )?;
    insta::assert_json_snapshot!(summary(&reinstalled)?, @r#"
    [
      {
        "errors": [],
        "executables": [
          {
            "from": "3.12.8",
            "to": "3.12.8"
          }
        ],
        "from": [
          "3.12.8"
        ],
        "outcome": "reinstalled",
        "request": "any",
        "to": "3.12.8"
      },
      {
        "errors": [],
        "executables": [
          {
            "from": "3.13.1",
            "to": "3.13.1"
          }
        ],
        "from": [
          "3.13.1"
        ],
        "outcome": "reinstalled",
        "request": "any",
        "to": "3.13.1"
      }
    ]
    "#);
    assert!(!context.temp_dir.child("managed").child(newest_key).exists());
    Ok(())
}

#[test]
fn python_upgrade_json_finalization_failure() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();
    let fixture = python_fixture(&context, "cpython-3.12.9")?;
    let key = fixture.key.clone();
    let catalog = write_catalog(&context, &[fixture])?;
    install(&context, catalog.path(), &["3.12.9"]);
    let installation = context.temp_dir.child("managed").child(key);
    let marker = installation.child(if cfg!(windows) {
        "Lib/EXTERNALLY-MANAGED"
    } else {
        "lib/python3.12/EXTERNALLY-MANAGED"
    });
    fs_err::remove_file(marker.path())?;
    fs_err::create_dir(marker.path())?;

    let failed = report(&upgrade(&context, catalog.path()).arg("3.12").output()?, 2)?;
    insta::assert_json_snapshot!(summary(&failed)?, @r#"
    [
      {
        "errors": [
          "finalize"
        ],
        "executables": [],
        "from": [
          "3.12.9"
        ],
        "outcome": "failed",
        "request": "3.12",
        "to": "3.12.9"
      }
    ]
    "#);
    assert_eq!(failed["errors"][0]["kind"], "operation");
    assert!(installation.is_dir());
    Ok(())
}

#[test]
fn python_upgrade_json_bytecode_failure() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();
    let fixture = python_fixture(&context, "cpython-3.12.9")?;
    let key = fixture.key.clone();
    let catalog = write_catalog(&context, &[fixture])?;
    install(&context, catalog.path(), &["3.12.9"]);
    let installation = context.temp_dir.child("managed").child(key);
    let executable = installation.child(if cfg!(windows) {
        "python.exe"
    } else {
        "bin/python3.12"
    });
    executable.write_str("not a Python interpreter\n")?;

    let failed = report(
        &upgrade(&context, catalog.path())
            .args(["3.12", "--compile-bytecode"])
            .output()?,
        2,
    )?;
    insta::assert_json_snapshot!(summary(&failed)?, @r#"
    [
      {
        "errors": [
          "bytecode"
        ],
        "executables": [],
        "from": [
          "3.12.9"
        ],
        "outcome": "failed",
        "request": "3.12",
        "to": "3.12.9"
      }
    ]
    "#);
    assert_eq!(failed["errors"][0]["kind"], "bytecode");
    Ok(())
}
