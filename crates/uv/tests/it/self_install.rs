use anyhow::Result;
use assert_fs::prelude::*;
use uv_test::uv_snapshot;

#[test]
fn requires_preview() {
    let context = uv_test::test_context_with_versions!(&[]);
    uv_snapshot!(context.command().args(["self", "install"]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Native self-installation is experimental; pass `--preview-features self-management` to enable it
    ");
}

#[test]
#[cfg(any(not(windows), feature = "windows-gui-bin"))]
fn installs_running_distribution() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let bin = context.temp_dir.child("bin");
    let mut command = context.command();
    command
        .args([
            "self",
            "install",
            "--preview-features",
            "self-management",
            "--no-modify-path",
            "--install-dir",
        ])
        .arg(bin.path());
    let output = command.output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let installed = bin.child(format!("uv{}", std::env::consts::EXE_SUFFIX));
    assert!(
        std::process::Command::new(installed.path())
            .arg("--version")
            .status()?
            .success()
    );
    bin.child(format!("uvx{}", std::env::consts::EXE_SUFFIX))
        .assert(predicates::path::is_file());
    let receipt: serde_json::Value =
        serde_json::from_slice(&fs_err::read(bin.child(".uv-receipt.json"))?)?;
    assert_eq!(receipt["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(receipt["modify_path"], false);
    assert_eq!(receipt["provider"]["source"], "uv");
    assert_eq!(
        receipt["install_prefix"],
        fs_err::canonicalize(bin.path())?.to_string_lossy().as_ref()
    );
    assert!(command.status()?.success());
    Ok(())
}

#[test]
#[cfg(any(not(windows), feature = "windows-gui-bin"))]
fn reinstalls_missing_executable_with_matching_receipt() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let bin = context.temp_dir.child("bin");
    let mut command = context.command();
    command
        .args([
            "self",
            "install",
            "--preview-features",
            "self-management",
            "--no-modify-path",
            "--install-dir",
        ])
        .arg(bin.path());
    assert!(command.status()?.success());
    let executable = bin.child(format!("uv{}", std::env::consts::EXE_SUFFIX));
    let receipt = bin.child(".uv-receipt.json");
    let mut data: serde_json::Value = serde_json::from_slice(&fs_err::read(receipt.path())?)?;
    data["source"]["owner"] = serde_json::json!("example");
    fs_err::write(receipt.path(), serde_json::to_vec(&data)?)?;
    fs_err::remove_file(executable.path())?;
    assert!(command.status()?.success());
    executable.assert(predicates::path::is_file());
    let repaired: serde_json::Value = serde_json::from_slice(&fs_err::read(receipt.path())?)?;
    assert_eq!(repaired["source"]["owner"], "example");

    let foreign = context.temp_dir.child("foreign");
    foreign.create_dir_all()?;
    data["install_prefix"] = serde_json::json!(foreign.path());
    fs_err::write(receipt.path(), serde_json::to_vec(&data)?)?;
    fs_err::remove_file(executable.path())?;
    uv_snapshot!(context.filters(), command, @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The install receipt at `[TEMP_DIR]/bin/.uv-receipt.json` belongs to a different installation at `[TEMP_DIR]/foreign`
    ");
    executable.assert(predicates::path::missing());
    assert_eq!(fs_err::read(receipt.path())?, serde_json::to_vec(&data)?);
    Ok(())
}

#[test]
#[cfg(any(not(windows), feature = "windows-gui-bin"))]
fn unmanaged_install_has_no_receipt() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let bin = context.temp_dir.child("bin");
    assert!(
        context
            .command()
            .args([
                "self",
                "install",
                "--preview-features",
                "self-management",
                "--unmanaged"
            ])
            .arg(bin.path())
            .status()?
            .success()
    );
    bin.child(".uv-receipt.json")
        .assert(predicates::path::missing());
    Ok(())
}
