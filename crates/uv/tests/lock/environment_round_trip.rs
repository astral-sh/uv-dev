use anyhow::Result;
use assert_fs::prelude::*;
use indoc::formatdoc;

use uv_test::uv_snapshot;

/// An environment implied by the project's Python bound remains part of the lock's inputs.
#[test]
fn lock_environment_entries_round_trip() -> Result<()> {
    for (setting, key, marker) in [
        (
            "environments",
            "supported-markers",
            "python_version >= '3.9'",
        ),
        ("environments", "supported-markers", "python_version >= '0'"),
        (
            "required-environments",
            "required-markers",
            "python_version >= '3.9'",
        ),
        (
            "required-environments",
            "required-markers",
            "python_version >= '0'",
        ),
    ] {
        let context = uv_test::test_context!("3.12");
        context
            .temp_dir
            .child("pyproject.toml")
            .write_str(&formatdoc! {
                r#"
            [project]
            name = "project"
            version = "0.1.0"
            requires-python = ">=3.9"

            [tool.uv]
            {setting} = ["{marker}"]
            "#
            })?;

        insta::allow_duplicates! {
            uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
            exit_code: 0 (success)
            ----- stderr -----
            Resolved 1 package in [TIME]
            ");
        }
        let lock = context.read("uv.lock");
        insta::allow_duplicates! {
            uv_snapshot!(context.filters(), context.lock().arg("--offline").arg("--locked"), @"
            exit_code: 0 (success)
            ----- stderr -----
            Resolved 1 package in [TIME]
            ");
        }
        assert_eq!(lock, context.read("uv.lock"));
        let value: toml::Value = toml::from_str(&lock)?;
        assert_eq!(
            value[key].as_array(),
            Some(&vec![toml::Value::String(
                "python_version >= '0'".to_owned()
            )])
        );
        insta::allow_duplicates! {
            uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
            exit_code: 0 (success)
            ----- stderr -----
            Resolved 1 package in [TIME]
            ");
        }
        assert_eq!(lock, context.read("uv.lock"));
    }
    Ok(())
}
