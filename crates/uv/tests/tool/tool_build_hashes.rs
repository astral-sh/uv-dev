//! Build dependency hash verification for tool commands.

use std::collections::BTreeMap;
use std::process::Command;

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::assert::PathAssert;
use assert_fs::fixture::{ChildPath, FileWriteBin, FileWriteStr, PathChild};
use indoc::indoc;
use insta::assert_snapshot;
use predicates::prelude::predicate;
use sha2::{Digest, Sha256};

use uv_static::EnvVars;
use uv_test::packse::{generate_wheel, generate_wheel_with_files};
use uv_test::{TestContext, uv_snapshot};

const WRONG_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Create a tool project whose backend records whether it was imported.
fn build_hash_context() -> Result<(TestContext, String)> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let (filename, wheel) = generate_wheel(
        &"build-dependency".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let hash = hex::encode(Sha256::digest(&wheel));
    context
        .temp_dir
        .child("wheels")
        .child(filename)
        .write_binary(&wheel)?;
    let (filename, wheel) = generate_wheel_with_files(
        &"hash-tool".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[
            ("hash_tool/cli.py", "def main():\n    print('tool-ok')\n"),
            (
                "hash_tool-1.0.0.dist-info/entry_points.txt",
                "[console_scripts]\nhash-tool = hash_tool.cli:main\n",
            ),
        ],
    );
    context
        .temp_dir
        .child("wheels")
        .child(filename)
        .write_binary(&wheel)?;
    let project = context.temp_dir.child("project");
    project.child("pyproject.toml").write_str(indoc! {r#"
        [build-system]
        requires = ["build-dependency==1.0.0"]
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    project.child("backend.py").write_str(indoc! {r#"
        import shutil
        from pathlib import Path

        import build_dependency

        Path(__file__).with_name("backend-executed").touch()

        def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
            wheel = Path(__file__).parent.parent / "wheels" / "hash_tool-1.0.0-py3-none-any.whl"
            shutil.copyfile(wheel, Path(wheel_directory) / wheel.name)
            return wheel.name
    "#})?;
    Ok((context.with_filter((hash.clone(), "[BUILD_HASH]")), hash))
}

fn write_hashed_constraint(
    context: &TestContext,
    requirement: &str,
    hash: &str,
) -> Result<ChildPath> {
    let constraints = context.temp_dir.child("constraints.txt");
    constraints.write_str(&format!("{requirement} --hash=sha256:{hash}\n"))?;
    Ok(constraints)
}

fn assert_receipt_constraint(context: &TestContext, specifier: &str, hash: &str) {
    let expected = format!(
        r#"build-constraint-dependencies = [{{ name = "build-dependency", specifier = "{specifier}", hashes = ["sha256:{hash}"] }}]"#
    );
    context
        .temp_dir
        .child("tools/hash-tool/uv-receipt.toml")
        .assert(predicate::str::contains(expected));
}

fn install(context: &TestContext, requirement: &str) -> Command {
    let mut command = context.tool_install();
    command
        .arg(requirement)
        .args(["--no-index", "--find-links", "wheels"])
        .args(["--build-constraint", "constraints.txt", "--no-cache"])
        .env(EnvVars::PATH, context.temp_dir.child("bin").as_os_str());
    command
}

fn upgrade(context: &TestContext) -> Command {
    let mut command = context.tool_upgrade();
    command
        .args(["hash-tool", "--reinstall", "--no-cache"])
        .env(EnvVars::PATH, context.temp_dir.child("bin").as_os_str());
    command
}

#[test]
fn install_persists_trusted_build_hash_for_upgrade() -> Result<()> {
    let (context, hash) = build_hash_context()?;
    let constraints = write_hashed_constraint(&context, "build-dependency==1.0.0", &hash)?;

    uv_snapshot!(context.filters(), install(&context, "hash-tool @ ./project").arg("--no-preview"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + hash-tool==1.0.0 (from file://[TEMP_DIR]/project)
    Installed 1 executable: hash-tool
    ");
    insta::with_settings!({ filters => context.filters() }, {
        assert_snapshot!(context.read("tools/hash-tool/uv-receipt.toml"), @r#"
        [tool]
        requirements = [{ name = "hash-tool", directory = "[TEMP_DIR]/project" }]
        build-constraint-dependencies = [{ name = "build-dependency", specifier = "==1.0.0", hashes = ["sha256:[BUILD_HASH]"] }]
        entrypoints = [
            { name = "hash-tool", install-path = "[TEMP_DIR]/bin/hash-tool", from = "hash-tool" },
        ]

        [tool.options]
        no-index = true
        find-links = ["file://[TEMP_DIR]/wheels"]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    fs_err::remove_file(&constraints)?;
    fs_err::remove_file(context.temp_dir.child("project/backend-executed"))?;
    uv_snapshot!(context.filters(), upgrade(&context).arg("--no-preview"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Modified hash-tool environment
     ~ hash-tool==1.0.0 (from file://[TEMP_DIR]/project)
    Nothing to upgrade
    ");
    context
        .temp_dir
        .child("project/backend-executed")
        .assert(predicate::path::exists());
    Ok(())
}

#[test]
fn locked_install_persists_trusted_build_hash_for_upgrade() -> Result<()> {
    let (context, hash) = build_hash_context()?;
    let constraints = write_hashed_constraint(&context, "build-dependency==1.0.0", &hash)?;

    install(&context, "hash-tool @ ./project")
        .arg("--preview-features=tool-install-locks")
        .assert()
        .success();
    assert_receipt_constraint(&context, "==1.0.0", &hash);

    fs_err::remove_file(&constraints)?;
    fs_err::remove_file(context.temp_dir.child("project/backend-executed"))?;
    upgrade(&context)
        .arg("--preview-features=tool-install-locks")
        .assert()
        .success();
    context
        .temp_dir
        .child("project/backend-executed")
        .assert(predicate::path::exists());
    Ok(())
}

#[test]
fn install_requires_build_hash_before_importing_backend() -> Result<()> {
    let (context, _) = build_hash_context()?;
    context
        .temp_dir
        .child("constraints.txt")
        .write_str("build-dependency==1.0.0\n")?;

    uv_snapshot!(context.filters(), install(&context, "hash-tool @ ./project")
        .arg("--no-preview")
        .env(EnvVars::UV_REQUIRE_BUILD_HASHES, "true"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    warning: The `--require-build-hashes` option is experimental and may change without warning. Pass `--preview-features build-dependency-hashes` to disable this warning.
    error: Failed to build `hash-tool @ file://[TEMP_DIR]/project`
      cause: Failed to resolve requirements from `build-system.requires`
      cause: No solution found when resolving: `build-dependency==1.0.0`
      cause: Hash-checking is required, but no trusted hash was provided for: `build-dependency`
    ");
    context
        .temp_dir
        .child("project/backend-executed")
        .assert(predicate::path::missing());
    Ok(())
}

#[test]
fn install_rejects_mismatched_build_hash_before_importing_backend() -> Result<()> {
    let (context, _) = build_hash_context()?;
    write_hashed_constraint(&context, "build-dependency==1.0.0", WRONG_HASH)?;

    uv_snapshot!(context.filters(), install(&context, "hash-tool @ ./project").arg("--no-preview"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to build `hash-tool @ file://[TEMP_DIR]/project`
      cause: Failed to install requirements from `build-system.requires`
      cause: Failed to download `build-dependency==1.0.0`
      cause: Hash mismatch for `build-dependency==1.0.0`

             Expected:
               sha256:0000000000000000000000000000000000000000000000000000000000000000

             Computed:
               sha256:[BUILD_HASH]
    ");
    context
        .temp_dir
        .child("project/backend-executed")
        .assert(predicate::path::missing());
    Ok(())
}

#[test]
fn locked_install_rejects_mismatched_build_hash_before_importing_backend() -> Result<()> {
    let (context, _) = build_hash_context()?;
    write_hashed_constraint(&context, "build-dependency==1.0.0", WRONG_HASH)?;

    install(&context, "hash-tool @ ./project")
        .arg("--preview-features=tool-install-locks")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Hash mismatch for `build-dependency==1.0.0`",
        ));
    context
        .temp_dir
        .child("project/backend-executed")
        .assert(predicate::path::missing());
    Ok(())
}

#[test]
fn install_rejects_mismatched_build_hash_during_name_inference() -> Result<()> {
    let (context, _) = build_hash_context()?;
    write_hashed_constraint(&context, "build-dependency==1.0.0", WRONG_HASH)?;

    uv_snapshot!(context.filters(), install(&context, "./project").arg("--no-preview"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to install requirements from `build-system.requires`
      cause: Failed to download `build-dependency==1.0.0`
      cause: Hash mismatch for `build-dependency==1.0.0`

             Expected:
               sha256:0000000000000000000000000000000000000000000000000000000000000000

             Computed:
               sha256:[BUILD_HASH]
    ");
    context
        .temp_dir
        .child("project/backend-executed")
        .assert(predicate::path::missing());
    Ok(())
}

#[test]
fn run_rejects_mismatched_build_hash_before_importing_backend() -> Result<()> {
    let (context, _) = build_hash_context()?;
    write_hashed_constraint(&context, "build-dependency==1.0.0", WRONG_HASH)?;

    uv_snapshot!(context.filters(), context.tool_run()
        .args(["--from", "./project"])
        .args(["--no-index", "--find-links", "wheels"])
        .args(["--build-constraint", "constraints.txt", "--no-cache"])
        .arg("hash-tool"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to resolve `--with` requirement
      cause: Failed to install requirements from `build-system.requires`
      cause: Failed to download `build-dependency==1.0.0`
      cause: Hash mismatch for `build-dependency==1.0.0`

             Expected:
               sha256:0000000000000000000000000000000000000000000000000000000000000000

             Computed:
               sha256:[BUILD_HASH]
    ");
    context
        .temp_dir
        .child("project/backend-executed")
        .assert(predicate::path::missing());
    Ok(())
}

#[test]
fn install_ignores_unpinned_build_hash_during_upgrade() -> Result<()> {
    let (context, _) = build_hash_context()?;
    let constraints = write_hashed_constraint(&context, "build-dependency>=1.0.0", WRONG_HASH)?;

    install(&context, "hash-tool @ ./project")
        .arg("--no-preview")
        .assert()
        .success();
    insta::with_settings!({ filters => context.filters() }, {
        assert_snapshot!(context.read("tools/hash-tool/uv-receipt.toml"), @r#"
        [tool]
        requirements = [{ name = "hash-tool", directory = "[TEMP_DIR]/project" }]
        build-constraint-dependencies = [{ name = "build-dependency", specifier = ">=1.0.0", hashes = ["sha256:0000000000000000000000000000000000000000000000000000000000000000"] }]
        entrypoints = [
            { name = "hash-tool", install-path = "[TEMP_DIR]/bin/hash-tool", from = "hash-tool" },
        ]

        [tool.options]
        no-index = true
        find-links = ["file://[TEMP_DIR]/wheels"]
        exclude-newer = "2024-03-25T00:00:00Z"
        "#);
    });

    fs_err::remove_file(&constraints)?;
    fs_err::remove_file(context.temp_dir.child("project/backend-executed"))?;
    upgrade(&context).arg("--no-preview").assert().success();
    context
        .temp_dir
        .child("project/backend-executed")
        .assert(predicate::path::exists());
    Ok(())
}

#[test]
fn locked_install_ignores_unpinned_build_hash_during_upgrade() -> Result<()> {
    let (context, _) = build_hash_context()?;
    let constraints = write_hashed_constraint(&context, "build-dependency>=1.0.0", WRONG_HASH)?;

    install(&context, "hash-tool @ ./project")
        .arg("--preview-features=tool-install-locks")
        .assert()
        .success();
    assert_receipt_constraint(&context, ">=1.0.0", WRONG_HASH);

    fs_err::remove_file(&constraints)?;
    fs_err::remove_file(context.temp_dir.child("project/backend-executed"))?;
    upgrade(&context)
        .arg("--preview-features=tool-install-locks")
        .assert()
        .success();
    context
        .temp_dir
        .child("project/backend-executed")
        .assert(predicate::path::exists());
    Ok(())
}
