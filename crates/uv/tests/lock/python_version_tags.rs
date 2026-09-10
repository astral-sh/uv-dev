use std::process::Command;

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::formatdoc;

use uv_test::TestContext;
use uv_test::packse::{PackseServer, scenario::Scenario};

fn index(tags: &[&str]) -> Result<PackseServer> {
    let scenario: Scenario = toml::from_str(&formatdoc! {
        r#"
        name = "wheel-python-version-tags"

        [root]
        requires = ["wheel-python-version-test==1.0.0"]

        [expected]
        satisfiable = false

        [packages.wheel-python-version-test.versions."1.0.0"]
        requires_python = ">=3.7"
        sdist = false
        wheel_tags = {}
        "#,
        serde_json::to_string(tags)?,
    })?;
    Ok(PackseServer::from_scenario(&scenario))
}

fn lock_command(context: &TestContext, server: &PackseServer) -> Command {
    let mut command = context.lock();
    command
        .arg("--no-config")
        .arg("--default-index")
        .arg(server.index_url())
        .arg("--no-build")
        .arg("--no-python-downloads")
        .env("NO_PROXY", "*");
    for key in [
        "UV_INDEX",
        "UV_DEFAULT_INDEX",
        "UV_INDEX_URL",
        "UV_EXTRA_INDEX_URL",
        "UV_FIND_LINKS",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITHUB_ENTERPRISE_TOKEN",
    ] {
        command.env_remove(key);
    }
    command
}

fn write_project(context: &TestContext) -> Result<()> {
    context.temp_dir.child("pyproject.toml").write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = "==3.12.*"
        dependencies = ["wheel-python-version-test==1.0.0"]
        "#,
    )?;
    Ok(())
}

/// A version-only rejection should name the Python tags, even when the ABI is `none` or names a
/// different implementation version. No wheel is installed or built by this test.
#[test]
fn lock_python_version_tag_hints() -> Result<()> {
    for (tags, expected) in [
        (
            &["cp37-cp37m-any", "cp310-cp310-any", "cp311-cp311-any"][..],
            "Python version tags: `cp37`, `cp310`, `cp311`",
        ),
        (&["cp311-none-any"][..], "Python version tag: `cp311`"),
        (
            &["pp310-pypy310_pp73-any"][..],
            "Python version tag: `pp310`",
        ),
        (
            &["graalpy310-graalpy240_310_native-any"][..],
            "Python version tag: `graalpy310`",
        ),
        // Report literal filename tags, without claiming that they describe exact compatibility.
        (&["cp312-cp310-any"][..], "Python version tag: `cp312`"),
    ] {
        let context = uv_test::test_context!("3.12");
        let server = index(tags)?;
        write_project(&context)?;
        let output = lock_command(&context, &server).output()?;
        let stderr = str::from_utf8(&output.stderr)?;
        assert_eq!(output.status.code(), Some(1), "{tags:?}: {stderr}");
        assert!(
            stderr.contains("no wheels with a matching Python version tag"),
            "{tags:?}: {stderr}"
        );
        let expected = format!(
            "hint: Wheels are available for `wheel-python-version-test` (v1.0.0) with the following {expected}"
        );
        assert!(stderr.contains(&expected), "{tags:?}: {stderr}");
        assert!(!stderr.contains("Python ABI tag"), "{tags:?}: {stderr}");
        assert!(!context.temp_dir.child("uv.lock").exists());
    }
    Ok(())
}

/// Stable ABI wheels remain acceptable; the diagnostic change must not narrow wheel selection.
#[test]
fn lock_stable_abi_wheels() -> Result<()> {
    for tag in ["cp39-abi3-any", "cp313-abi3t-any"] {
        let context = uv_test::test_context!("3.12");
        let server = index(&[tag])?;
        write_project(&context)?;
        lock_command(&context, &server).assert().success();
        let lock = context.read("uv.lock");
        assert!(lock.contains(&format!("wheel_python_version_test-1.0.0-{tag}.whl")));
        lock_command(&context, &server)
            .arg("--offline")
            .arg("--locked")
            .assert()
            .success();
        assert_eq!(context.read("uv.lock"), lock);
    }
    Ok(())
}
