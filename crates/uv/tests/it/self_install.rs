#[cfg(unix)]
use anyhow::Context;
#[cfg(any(not(windows), feature = "windows-gui-bin"))]
use anyhow::Result;
#[cfg(unix)]
use assert_cmd::assert::OutputAssertExt;
#[cfg(any(not(windows), feature = "windows-gui-bin"))]
use assert_fs::prelude::*;
#[cfg(unix)]
use uv_fs::{LockedFile, LockedFileMode};
#[cfg(any(not(windows), feature = "windows-gui-bin"))]
use uv_static::EnvVars;
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
fn installs_running_distribution_with_numeric_no_modify_path() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filter((r"uv [0-9]+\.[0-9]+\.[0-9]+[^ ]*", "uv [VERSION]"))
        .with_filter((r"\([a-z0-9_]+-[a-z0-9_-]+\)", "([TARGET])"));
    let bin = context.temp_dir.child("bin");
    uv_snapshot!(context.filters(), context.command().args([
        "self", "install", "--preview-features", "self-management", "--install-dir",
    ]).arg(bin.path()).env(EnvVars::UV_NO_MODIFY_PATH, "1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed uv [VERSION] to [TEMP_DIR]/bin
    ");
    let installed = bin.child(format!("uv{}", std::env::consts::EXE_SUFFIX));
    uv_snapshot!(context.filters(), context.external_command(installed.path()).arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    uv [VERSION] ([TARGET])
    ");
    bin.child(format!("uvx{}", std::env::consts::EXE_SUFFIX))
        .assert(predicates::path::is_file());
    let receipt: serde_json::Value = serde_json::from_str(&context.read("bin/.uv-receipt.json"))?;
    assert_eq!(receipt["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(receipt["modify_path"], false);
    assert_eq!(receipt["provider"]["source"], "uv");
    assert_eq!(
        receipt["install_prefix"],
        fs_err::canonicalize(bin.path())?.to_string_lossy().as_ref()
    );
    uv_snapshot!(context.filters(), context.command().args([
        "self", "install", "--preview-features", "self-management", "--install-dir",
    ]).arg(bin.path()).env(EnvVars::UV_NO_MODIFY_PATH, "1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed uv [VERSION] to [TEMP_DIR]/bin
    ");
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

#[cfg(unix)]
#[test]
fn legacy_receipt_falls_back_from_xdg_config_home() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_filter((
        r"Installed uv [0-9]+\.[0-9]+\.[0-9]+[^ ]*",
        "Installed uv [VERSION]",
    ));
    let original = context.temp_dir.child("original/bin");
    context
        .command()
        .args([
            "self",
            "install",
            "--preview-features",
            "self-management",
            "--no-modify-path",
            "--install-dir",
        ])
        .arg(original.path())
        .assert()
        .success();
    let native_receipt = original.child(".uv-receipt.json");
    let mut receipt: serde_json::Value =
        serde_json::from_slice(&fs_err::read(native_receipt.path())?)?;
    receipt["source"]["owner"] = serde_json::json!("custom-owner");
    receipt["install_prefix"] = serde_json::json!(context.temp_dir.child("original").path());
    receipt["provider"]["source"] = serde_json::json!("cargo-dist");
    let legacy = context.home_dir.child(".config/uv/uv-receipt.json");
    fs_err::create_dir_all(legacy.parent().context("legacy receipt parent")?)?;
    fs_err::write(legacy.path(), serde_json::to_vec(&receipt)?)?;
    fs_err::remove_file(native_receipt.path())?;
    let config = context.temp_dir.child("alternate-config");
    config.create_dir_all()?;
    let destination = context.temp_dir.child("destination");
    uv_snapshot!(context.filters(), context.external_command(original.child("uv").path()).args([
        "self", "install", "--preview-features", "self-management", "--no-modify-path", "--install-dir",
    ]).arg(destination.path()).env(EnvVars::XDG_CONFIG_HOME, config.path()), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Installed uv [VERSION] to [TEMP_DIR]/destination
    "#);
    let receipt: serde_json::Value =
        serde_json::from_slice(&fs_err::read(destination.child(".uv-receipt.json"))?)?;
    assert_eq!(receipt["source"]["owner"], "custom-owner");
    Ok(())
}

#[cfg(unix)]
#[test]
fn installation_lock_precedes_receipt_validation() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let bin = context.temp_dir.child("bin");
    bin.create_dir_all()?;
    bin.child(".uv-receipt.json").write_str("{}")?;
    let lock = LockedFile::acquire_no_wait(
        bin.child(".uv-install.lock"),
        LockedFileMode::Exclusive,
        "fixture installation",
    )
    .context("failed to acquire fixture installation lock")?;
    uv_snapshot!(context.filters(), context.command().args([
        "self", "install", "--preview-features", "self-management", "--no-modify-path", "--install-dir",
    ]).arg(bin.path()).env(EnvVars::UV_LOCK_TIMEOUT, "0"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Timeout ([TIME]) when waiting for lock on `uv installation` at `bin/.uv-install.lock`, is another uv process running? You can set `UV_LOCK_TIMEOUT` to increase the timeout.
    "#);
    drop(lock);
    uv_snapshot!(context.filters(), context.command().args([
        "self", "install", "--preview-features", "self-management", "--no-modify-path", "--install-dir",
    ]).arg(bin.path()), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse install receipt at `[TEMP_DIR]/bin/.uv-receipt.json`
      cause: missing field `install_prefix` at line 1 column 2
    "#);
    assert_eq!(fs_err::read(bin.child(".uv-receipt.json"))?, b"{}");
    bin.child("uv").assert(predicates::path::missing());
    Ok(())
}

/// Reinstalling before restarting the shell leaves the configured PATH entry unchanged.
#[cfg(unix)]
#[test]
fn repeated_install_with_configured_shell() {
    let context = uv_test::test_context_with_versions!(&[]).with_filter((
        r"Installed uv [0-9]+\.[0-9]+\.[0-9]+[^ ]*",
        "Installed uv [VERSION]",
    ));
    let bin = context.temp_dir.child("bin");
    uv_snapshot!(context.filters(), context.command().args([
        "self", "install", "--preview-features", "self-management", "--install-dir",
    ]).arg(bin.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed uv [VERSION] to [TEMP_DIR]/bin
    Created configuration file: [HOME]/.bash_profile
    Created configuration file: [HOME]/.bashrc
    Restart your shell to apply changes
    ");
    let profile = context.read(context.home_dir.child(".bash_profile"));
    let rc = context.read(context.home_dir.child(".bashrc"));
    uv_snapshot!(context.filters(), context.command().args([
        "self", "install", "--preview-features", "self-management", "--install-dir",
    ]).arg(bin.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed uv [VERSION] to [TEMP_DIR]/bin
    Restart your shell to apply changes
    ");
    assert_eq!(
        profile,
        context.read(context.home_dir.child(".bash_profile"))
    );
    assert_eq!(rc, context.read(context.home_dir.child(".bashrc")));
}

/// The windowed launcher remains alive while its child replaces the installation.
#[cfg(all(windows, feature = "windows-gui-bin"))]
#[test]
fn reinstalls_from_running_windowed_launcher() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_filter((
        r"Installed uv [0-9]+\.[0-9]+\.[0-9]+[^ ]*",
        "Installed uv [VERSION]",
    ));
    let bin = context.temp_dir.child("bin");
    uv_snapshot!(context.filters(), context.command().args([
        "self", "install", "--preview-features", "self-management", "--no-modify-path", "--install-dir",
    ]).arg(bin.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed uv [VERSION] to [TEMP_DIR]/bin
    ");
    let unrelated = bin.child(".uv-install-backup-unrelated/sentinel.exe");
    unrelated.write_str("unrelated")?;
    uv_snapshot!(context.filters(), context.external_command(bin.child("uvw.exe").path()).args([
        "self", "install", "--preview-features", "self-management", "--no-modify-path", "--install-dir",
    ]).arg(bin.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed uv [VERSION] to [TEMP_DIR]/bin
    ");
    uv_snapshot!(context.filters(), context.command().args([
        "self", "install", "--preview-features", "self-management", "--no-modify-path", "--install-dir",
    ]).arg(bin.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed uv [VERSION] to [TEMP_DIR]/bin
    ");
    unrelated.assert("unrelated");
    let entries = fs_err::read_dir(bin.path())?.collect::<std::io::Result<Vec<_>>>()?;
    let backups = entries
        .into_iter()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".uv-install-backup-")
        })
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();
    assert_eq!(
        backups,
        [std::ffi::OsString::from(".uv-install-backup-unrelated")]
    );
    let receipt: serde_json::Value = serde_json::from_str(&context.read("bin/.uv-receipt.json"))?;
    assert_eq!(receipt["provider"]["source"], "uv");
    Ok(())
}

/// A versioned executable must not install different unsuffixed binaries beside it.
#[cfg(unix)]
#[test]
fn rejects_different_adjacent_distribution() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let source = context.temp_dir.child("source");
    source.create_dir_all()?;
    let executable = source.child("uv@current");
    fs_err::copy(uv_test::get_bin!(), executable.path())?;
    source.child("uv").write_str("different uv")?;
    source.child("uvx").write_str("different uvx")?;
    let destination = context.temp_dir.child("destination");
    destination.child("uv").write_str("installed uv")?;
    destination.child("uvx").write_str("installed uvx")?;
    uv_snapshot!(context.filters(), context.external_command(executable.path()).args([
        "self", "install", "--preview-features", "self-management", "--no-modify-path", "--install-dir",
    ]).arg(destination.path()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Cannot install from `[TEMP_DIR]/source`: `uv` does not identify the running executable `[TEMP_DIR]/source/uv@current`
    ");
    assert_eq!(context.read("destination/uv"), "installed uv");
    assert_eq!(context.read("destination/uvx"), "installed uvx");
    destination
        .child(".uv-receipt.json")
        .assert(predicates::path::missing());
    Ok(())
}

/// Hard links to the running binary and symlinked sibling launchers identify one distribution.
#[cfg(unix)]
#[test]
fn installs_linked_running_distribution() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filter((r"uv [0-9]+\.[0-9]+\.[0-9]+[^ ]*", "uv [VERSION]"))
        .with_filter((r"\([a-z0-9_]+-[a-z0-9_-]+\)", "([TARGET])"));
    let source = context.temp_dir.child("source");
    source.create_dir_all()?;
    let executable = source.child("uv@current");
    let built = uv_test::get_bin!();
    fs_err::copy(&built, executable.path())?;
    fs_err::hard_link(executable.path(), source.child("uv").path())?;
    fs_err::os::unix::fs::symlink(
        built.parent().context("binary directory")?.join("uvx"),
        source.child("uvx").path(),
    )?;
    let destination = context.temp_dir.child("destination");
    uv_snapshot!(context.filters(), context.external_command(executable.path()).args([
        "self", "install", "--preview-features", "self-management", "--no-modify-path", "--install-dir",
    ]).arg(destination.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed uv [VERSION] to [TEMP_DIR]/destination
    ");
    uv_snapshot!(context.filters(), context.external_command(destination.child("uv").path()).arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    uv [VERSION] ([TARGET])
    ");
    destination.child("uvx").assert(predicates::path::is_file());
    let receipt: serde_json::Value =
        serde_json::from_str(&context.read("destination/.uv-receipt.json"))?;
    assert_eq!(receipt["version"], env!("CARGO_PKG_VERSION"));
    Ok(())
}
