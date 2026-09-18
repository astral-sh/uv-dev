#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::process::Command;

use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::assert::PathAssert;
use assert_fs::fixture::{PathChild, PathCreateDir};
use predicates::prelude::predicate;

use uv_static::EnvVars;

#[cfg(windows)]
use uv_test::packse::generate_wheel_with_files;
use uv_test::{TestContext, uv_snapshot, venv_bin_path};

#[test]
fn tool_uninstall() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Install `black`
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("black"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 2 executables: black, blackd
    ");

    // After uninstalling the tool, it shouldn't be listed.
    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stderr -----
    No tools installed
    ");

    // After uninstalling the tool, we should be able to reinstall it.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("black==24.2.0")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 6 packages in [TIME]
    Installed 6 packages in [TIME]
     + black==24.2.0
     + click==8.1.7
     + mypy-extensions==1.0.0
     + packaging==24.0
     + pathspec==0.12.1
     + platformdirs==4.2.0
    Installed 2 executables: black, blackd
    ");
}

fn install_replaced_executable(context: &TestContext) {
    let links = context.workspace_root.join("test/links");
    context
        .tool_install()
        .arg("simple-launcher==0.1.0")
        .arg("--no-index")
        .arg("--find-links")
        .arg(&links)
        .assert()
        .success();
    context
        .tool_install()
        .arg("basic-app==0.1.0")
        .arg("--with-executables-from")
        .arg("simple-launcher==0.1.0")
        .arg("--force")
        .arg("--no-index")
        .arg("--find-links")
        .arg(&links)
        .assert()
        .success();
}

#[test]
fn tool_uninstall_preserves_replaced_executable() -> Result<()> {
    let context = uv_test::test_context!("3.13")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");
    install_replaced_executable(&context);

    let executable = bin_dir.child(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX));
    let source = venv_bin_path(tool_dir.child("basic-app").path())
        .join(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX));
    let receipt = tool_dir.child("basic-app").child("uv-receipt.toml");
    let receipt_contents = fs_err::read(receipt.path())?;
    let contents = fs_err::read(executable.path())?;
    assert_eq!(contents, fs_err::read(&source)?);
    let metadata = fs_err::symlink_metadata(executable.path())?;
    #[cfg(unix)]
    let identity = {
        assert!(metadata.is_symlink());
        assert_eq!(
            fs_err::canonicalize(executable.path())?,
            fs_err::canonicalize(&source)?
        );
        (
            metadata.dev(),
            metadata.ino(),
            fs_err::read_link(executable.path())?,
        )
    };
    #[cfg(windows)]
    assert!(metadata.is_file() && !metadata.is_symlink());

    uv_snapshot!(context.filters(), Command::new(executable.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hi from the simple launcher!
    ");

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("simple-launcher"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed environment for `simple-launcher`
    ");

    tool_dir
        .child("simple-launcher")
        .assert(predicate::path::missing());
    assert_eq!(fs_err::read(receipt.path())?, receipt_contents);
    assert_eq!(fs_err::read(executable.path())?, contents);
    assert_eq!(fs_err::read(&source)?, contents);
    let current_metadata = fs_err::symlink_metadata(executable.path())?;
    assert_eq!(current_metadata.file_type(), metadata.file_type());
    #[cfg(unix)]
    assert_eq!(
        (
            current_metadata.dev(),
            current_metadata.ino(),
            fs_err::read_link(executable.path())?
        ),
        identity
    );
    uv_snapshot!(context.filters(), Command::new(executable.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hi from the simple launcher!
    ");

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("basic-app"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 2 executables: basic-app, simple_launcher
    ");
    executable.assert(predicate::path::missing());
    tool_dir
        .child("basic-app")
        .assert(predicate::path::missing());
    Ok(())
}

#[test]
fn tool_uninstall_removes_current_replaced_executable() -> Result<()> {
    let context = uv_test::test_context!("3.13")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let executable = context
        .temp_dir
        .child("bin")
        .child(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX));
    install_replaced_executable(&context);
    let receipt = tool_dir.child("simple-launcher").child("uv-receipt.toml");
    let receipt_contents = fs_err::read(receipt.path())?;

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("basic-app"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 2 executables: basic-app, simple_launcher
    ");
    executable.assert(predicate::path::missing());
    assert_eq!(fs_err::read(receipt.path())?, receipt_contents);
    uv_snapshot!(context.filters(), context.tool_uninstall().arg("simple-launcher"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed environment for `simple-launcher`
    ");
    executable.assert(predicate::path::missing());
    Ok(())
}

#[test]
fn tool_uninstall_validates_other_tools_before_removing_environment() -> Result<()> {
    let context = uv_test::test_context!("3.13")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");
    let launcher = context
        .workspace_root
        .join("test/links/simple_launcher-0.1.0-py3-none-any.whl");

    context
        .tool_install()
        .arg(&launcher)
        .arg("--no-index")
        .assert()
        .success();

    let receipt = tool_dir.child("simple-launcher").child("uv-receipt.toml");
    let receipt_contents = fs_err::read(receipt.path())?;
    let executable = bin_dir.child(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX));
    let contents = fs_err::read(executable.path())?;
    tool_dir.child("broken-tool").create_dir_all()?;

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("simple-launcher"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to find a receipt for tool `broken-tool` at [TEMP_DIR]/tools/broken-tool/uv-receipt.toml
    ");

    assert_eq!(fs_err::read(receipt.path())?, receipt_contents);
    assert_eq!(fs_err::read(executable.path())?, contents);
    uv_snapshot!(context.filters(), Command::new(executable.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hi from the simple launcher!
    ");

    Ok(())
}

#[test]
fn tool_uninstall_rejects_malformed_other_receipt() -> Result<()> {
    let context = uv_test::test_context!("3.13")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");
    let executable = context
        .temp_dir
        .child("bin")
        .child(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX));
    install_replaced_executable(&context);
    let receipt = tool_dir.child("simple-launcher").child("uv-receipt.toml");
    let receipt_contents = fs_err::read(receipt.path())?;
    let contents = fs_err::read(executable.path())?;
    fs_err::write(
        tool_dir.child("basic-app").child("uv-receipt.toml"),
        "not valid toml",
    )?;

    context
        .tool_uninstall()
        .arg("simple-launcher")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("Failed to read `uv-receipt.toml`"));
    assert_eq!(fs_err::read(receipt.path())?, receipt_contents);
    assert_eq!(fs_err::read(executable.path())?, contents);
    uv_snapshot!(context.filters(), Command::new(executable.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hi from the simple launcher!
    ");
    Ok(())
}

#[test]
fn tool_uninstall_preserves_unrelated_replacement() -> Result<()> {
    let context = uv_test::test_context!("3.13")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let executable = context
        .temp_dir
        .child("bin")
        .child(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX));
    install_replaced_executable(&context);
    fs_err::remove_file(executable.path())?;
    fs_err::write(executable.path(), b"not an installed tool executable")?;

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("simple-launcher"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed environment for `simple-launcher`
    ");
    assert_eq!(
        fs_err::read(executable.path())?,
        b"not an installed tool executable"
    );
    uv_snapshot!(context.filters(), context.tool_uninstall().arg("basic-app"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 executable: basic-app
    ");
    assert_eq!(
        fs_err::read(executable.path())?,
        b"not an installed tool executable"
    );
    assert!(!fs_err::symlink_metadata(executable.path())?.is_symlink());
    Ok(())
}

#[test]
fn tool_uninstall_after_same_tool_reinstall() -> Result<()> {
    let context = uv_test::test_context!("3.13")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let links = context.workspace_root.join("test/links");
    for reinstall in [false, true] {
        let mut install = context.tool_install();
        install
            .arg("simple-launcher==0.1.0")
            .arg("--no-index")
            .arg("--find-links")
            .arg(&links);
        if reinstall {
            install.arg("--force").arg("--reinstall");
        }
        install.assert().success();
    }
    let executable = context
        .temp_dir
        .child("bin")
        .child(format!("simple_launcher{}", std::env::consts::EXE_SUFFIX));
    uv_snapshot!(context.filters(), Command::new(executable.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hi from the simple launcher!
    ");
    uv_snapshot!(context.filters(), context.tool_uninstall().arg("simple-launcher"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 1 executable: simple_launcher
    ");
    executable.assert(predicate::path::missing());
    Ok(())
}

#[cfg(windows)]
#[test]
fn tool_uninstall_rejects_ambiguous_copied_executable() -> Result<()> {
    let context = uv_test::test_context!("3.13")
        .with_filtered_exe_suffix()
        .with_filter((r"[\\/]BIN[\\/]shared\.cmd", "/bin/shared.cmd"))
        .with_tool_dirs();
    let links = context.temp_dir.child("links");
    let tool_dir = context.temp_dir.child("tools");
    links.create_dir_all()?;
    let script = "@echo off\r\necho shared native-script fixture\r\n";
    for name in ["first-native", "second-native"] {
        let normalized = name.replace('-', "_");
        let script_path = format!("{normalized}-0.1.0.data/scripts/shared.cmd");
        let (filename, wheel) = generate_wheel_with_files(
            &name.parse()?,
            &"0.1.0".parse()?,
            &[],
            &Default::default(),
            None,
            "py3-none-any",
            &[(script_path.as_str(), script)],
        );
        fs_err::write(links.child(filename), wheel)?;
        let mut install = context.tool_install();
        install
            .arg(format!("{name}==0.1.0"))
            .arg("--no-index")
            .arg("--find-links")
            .arg(links.path())
            .arg("--force");
        if name == "second-native" {
            install.env(
                EnvVars::UV_TOOL_BIN_DIR,
                context.temp_dir.child("BIN").path(),
            );
        }
        install.assert().success();
    }
    let executable = context.temp_dir.child("bin").child("shared.cmd");
    let first_receipt = tool_dir.child("first-native").child("uv-receipt.toml");
    let second_receipt = tool_dir.child("second-native").child("uv-receipt.toml");
    let first_contents = fs_err::read(first_receipt.path())?;
    let second_contents = fs_err::read(second_receipt.path())?;
    assert_eq!(
        uv_fs::is_same_file_allow_missing(
            executable.path(),
            context.temp_dir.child("BIN").child("shared.cmd").path()
        ),
        Some(true)
    );
    assert_eq!(fs_err::read(executable.path())?, script.as_bytes());
    assert_eq!(
        fs_err::read(venv_bin_path(tool_dir.child("first-native").path()).join("shared.cmd"))?,
        script.as_bytes()
    );
    assert_eq!(
        fs_err::read(venv_bin_path(tool_dir.child("second-native").path()).join("shared.cmd"))?,
        script.as_bytes()
    );

    for name in ["first-native", "second-native", "--all"] {
        uv_snapshot!(context.filters(), context.tool_uninstall().arg(name), @"
        exit_code: 2 (failure)
        ----- stderr -----
        error: Cannot determine whether executable `[TEMP_DIR]/bin/shared.cmd` belongs to `first-native` or `second-native`; no tools were removed
        ");
        assert_eq!(fs_err::read(first_receipt.path())?, first_contents);
        assert_eq!(fs_err::read(second_receipt.path())?, second_contents);
        assert_eq!(fs_err::read(executable.path())?, script.as_bytes());
    }
    uv_snapshot!(context.filters(), Command::new("cmd").arg("/D").arg("/C").arg(executable.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    shared native-script fixture
    ");
    Ok(())
}

#[cfg(windows)]
#[test]
fn tool_uninstall_rejects_different_basename_aliases() -> Result<()> {
    let context = uv_test::test_context!("3.13")
        .with_filter((r"[\\/](?:first|same-owner|second)\.cmd", "/[ALIAS].cmd"))
        .with_tool_dirs();
    let links = context.temp_dir.child("links");
    let tool_dir = context.temp_dir.child("tools");
    let bin_dir = context.temp_dir.child("bin");
    links.create_dir_all()?;
    let script = "@echo off\r\necho shared command alias fixture\r\n";
    for (name, scripts) in [
        ("aaa-unrelated", &["unrelated.cmd"][..]),
        ("first-command", &["first.cmd", "same-owner.cmd"][..]),
        ("second-command", &["second.cmd"][..]),
    ] {
        let normalized = name.replace('-', "_");
        let files = scripts
            .iter()
            .map(|filename| {
                (
                    format!("{normalized}-0.1.0.data/scripts/{filename}"),
                    script,
                )
            })
            .collect::<Vec<_>>();
        let files = files
            .iter()
            .map(|(path, contents)| (path.as_str(), *contents))
            .collect::<Vec<_>>();
        let (filename, wheel) = generate_wheel_with_files(
            &name.parse()?,
            &"0.1.0".parse()?,
            &[],
            &Default::default(),
            None,
            "py3-none-any",
            &files,
        );
        fs_err::write(links.child(filename), wheel)?;
        context
            .tool_install()
            .arg(format!("{name}==0.1.0"))
            .arg("--no-index")
            .arg("--find-links")
            .arg(links.path())
            .assert()
            .success();
    }

    let first = bin_dir.child("first.cmd");
    let same_owner = bin_dir.child("same-owner.cmd");
    let second = bin_dir.child("second.cmd");
    // Different directory entries can alias the same copied executable on Windows.
    for alias in [&same_owner, &second] {
        fs_err::remove_file(alias.path())?;
        fs_err::hard_link(first.path(), alias.path())?;
        assert_eq!(
            uv_fs::is_same_file_allow_missing(first.path(), alias.path()),
            Some(true)
        );
    }
    let receipts = ["aaa-unrelated", "first-command", "second-command"]
        .map(|name| tool_dir.child(name).child("uv-receipt.toml"));
    let receipt_contents = receipts
        .iter()
        .map(|receipt| fs_err::read(receipt.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    let sources = [
        ("aaa-unrelated", "unrelated.cmd"),
        ("first-command", "first.cmd"),
        ("first-command", "same-owner.cmd"),
        ("second-command", "second.cmd"),
    ]
    .map(|(name, filename)| venv_bin_path(tool_dir.child(name).path()).join(filename));
    for source in &sources {
        assert_eq!(fs_err::read(source)?, script.as_bytes());
    }

    for name in ["first-command", "second-command", "--all"] {
        uv_snapshot!(context.filters(), context.tool_uninstall().arg(name), @"
        exit_code: 2 (failure)
        ----- stderr -----
        error: Cannot determine whether executable `[TEMP_DIR]/bin/[ALIAS].cmd` belongs to `first-command` or `second-command`; no tools were removed
        ");
        for (receipt, contents) in receipts.iter().zip(&receipt_contents) {
            assert_eq!(fs_err::read(receipt.path())?, *contents);
        }
        for source in &sources {
            assert_eq!(fs_err::read(source)?, script.as_bytes());
        }
        for export in [&first, &same_owner, &second] {
            assert_eq!(fs_err::read(export.path())?, script.as_bytes());
            assert_eq!(
                uv_fs::is_same_file_allow_missing(first.path(), export.path()),
                Some(true)
            );
        }
        assert_eq!(
            fs_err::read(bin_dir.child("unrelated.cmd").path())?,
            script.as_bytes()
        );
    }
    for export in [&first, &second] {
        uv_snapshot!(context.filters(), Command::new("cmd").arg("/D").arg("/C").arg(export.path()), @"
        exit_code: 0 (success)
        ----- stdout -----
        shared command alias fixture
        ");
    }
    Ok(())
}

#[test]
fn tool_uninstall_multiple_names() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // Install `black`
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    context.tool_install().arg("ruff==0.3.4").assert().success();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("black").arg("ruff"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Uninstalled 3 executables: black, blackd, ruff
    ");

    // After uninstalling the tool, it shouldn't be listed.
    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stderr -----
    No tools installed
    ");
}

#[test]
fn tool_uninstall_not_installed() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("black"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `black` is not installed
    ");
}

#[test]
fn tool_uninstall_missing_receipt() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");

    // Install `black`
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    fs_err::remove_file(tool_dir.join("black").join("uv-receipt.toml")).unwrap();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("black"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed dangling environment for `black`
    ");
}

#[test]
fn tool_uninstall_multiple_names_with_missing_receipt() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");

    // Install `black`
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    context.tool_install().arg("ruff==0.3.4").assert().success();

    fs_err::remove_file(tool_dir.join("black").join("uv-receipt.toml")).unwrap();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("black").arg("ruff"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed dangling environment for `black`
    Uninstalled 1 executable: ruff
    ");

    // After uninstalling both tools, neither should be listed.
    uv_snapshot!(context.filters(), context.tool_list(), @"
    exit_code: 0 (success)
    ----- stderr -----
    No tools installed
    ");
}

#[test]
fn tool_uninstall_all_missing_receipt() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let tool_dir = context.temp_dir.child("tools");

    // Install `black`
    context
        .tool_install()
        .arg("black==24.2.0")
        .assert()
        .success();

    fs_err::remove_file(tool_dir.join("black").join("uv-receipt.toml")).unwrap();

    uv_snapshot!(context.filters(), context.tool_uninstall().arg("--all"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed dangling environment for `black`
    ");
}
