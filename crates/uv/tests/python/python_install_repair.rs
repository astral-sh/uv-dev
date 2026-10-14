//! Offline repairs of simulated managed Python installations.

use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

use assert_cmd::assert::OutputAssertExt;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::{FileTouch, FileWriteStr, PathChild, PathCreateDir};
use indoc::indoc;
use uv_test::TestContext;

// Repair still resolves the installation's download key, so use an entry independent of the host.
const INSTALLATION_KEY: &str = "cpython-3.12.9-linux-x86_64-gnu";

/// Create the minimal on-disk shape needed for `uv python install` to repair an installation.
fn mock_cpython_installation(context: &TestContext) -> anyhow::Result<ChildPath> {
    let installation = context.temp_dir.child("managed").child(INSTALLATION_KEY);
    installation
        .child("lib")
        .child("python3.12")
        .create_dir_all()?;
    let bin = installation.child("bin");
    bin.create_dir_all()?;
    bin.child("python3.12").touch()?;
    Ok(installation)
}

/// Regression test for a panic when `/install` in a sysconfig value is followed by a non-ASCII
/// character.
#[test]
fn python_install_sysconfig_prefix() -> anyhow::Result<()> {
    let context = uv_test::test_context_with_versions!(&[])
        .with_filtered_python_keys()
        .with_managed_python_dirs();
    let installation = mock_cpython_installation(&context)?;
    let lib = installation.child("lib").child("python3.12");
    let sysconfig = lib.child("_sysconfigdata__test.py");
    sysconfig.write_str(indoc! {r#"
        # system configuration generated and used by the sysconfig module
        build_time_vars = {
            "PREFIX": "/install",
            "SUBDIRECTORY": "/install/é",
            "UNICODE_SUFFIX": "/installé",
            "OTHER_PREFIX": "/installation"
        }
    "#})?;
    fs_err::set_permissions(sysconfig.path(), std::fs::Permissions::from_mode(0o640))?;
    let original = fs_err::read_to_string(sysconfig.path())?;
    let mut reader = fs_err::File::open(sysconfig.path())?;

    context
        .python_install()
        .arg(INSTALLATION_KEY)
        .arg("--no-bin")
        .arg("--offline")
        .assert()
        .success();

    let contents = context.read(sysconfig.path());
    insta::with_settings!({ filters => context.filters() }, {
        insta::assert_snapshot!(contents, @r#"
        # system configuration generated and used by the sysconfig module
        build_time_vars = {
            "OTHER_PREFIX": "/installation",
            "PREFIX": "[TEMP_DIR]/managed/cpython-3.12.9-[PLATFORM]",
            "PYTHON_BUILD_STANDALONE": 1,
            "SUBDIRECTORY": "[TEMP_DIR]/managed/cpython-3.12.9-[PLATFORM]/é",
            "UNICODE_SUFFIX": "/installé"
        }
        "#);
    });

    // A Python process which already opened the module must see the complete original.
    let mut previous_contents = String::new();
    reader.read_to_string(&mut previous_contents)?;
    assert_eq!(previous_contents, original);
    let metadata = fs_err::metadata(sysconfig.path())?;
    assert_eq!(metadata.permissions().mode() & 0o777, 0o640);

    context
        .python_install()
        .arg(INSTALLATION_KEY)
        .arg("--no-bin")
        .arg("--offline")
        .assert()
        .success();
    // Already-patched sysconfig data does not need a new file.
    assert_eq!(fs_err::metadata(sysconfig.path())?.ino(), metadata.ino());

    Ok(())
}

/// Pkgconfig repairs must leave existing readers on the complete original file.
#[test]
fn python_install_pkgconfig() -> anyhow::Result<()> {
    let context = uv_test::test_context_with_versions!(&[]).with_managed_python_dirs();
    let installation = mock_cpython_installation(&context)?;
    let lib = installation.child("lib").child("python3.12");
    lib.child("_sysconfigdata__test.py").write_str(indoc! {r"
        # system configuration generated and used by the sysconfig module
        build_time_vars = {}
    "})?;

    let pkgconfigs = installation.child("lib").child("pkgconfig");
    pkgconfigs.create_dir_all()?;
    let pkgconfig = pkgconfigs.child("python.pc");
    pkgconfig.write_str("prefix=/install\nlibdir=${prefix}/lib\n")?;
    fs_err::set_permissions(pkgconfig.path(), std::fs::Permissions::from_mode(0o640))?;
    let original = fs_err::read_to_string(pkgconfig.path())?;
    let mut reader = fs_err::File::open(pkgconfig.path())?;
    let ignored = pkgconfigs.child("ignored.txt");
    ignored.write_str("prefix=/install\n")?;
    pkgconfigs.child("directory.pc").create_dir_all()?;
    let linked = installation.child("linked.pc");
    linked.write_str("prefix=/install\n")?;
    fs_err::os::unix::fs::symlink(linked.path(), pkgconfigs.child("linked.pc").path())?;

    context
        .python_install()
        .arg(INSTALLATION_KEY)
        .arg("--no-bin")
        .arg("--offline")
        .assert()
        .success();

    insta::assert_snapshot!(context.read(pkgconfig.path()), @r"
    prefix=${pcfiledir}/../..
    libdir=${prefix}/lib
    ");
    let mut previous_contents = String::new();
    reader.read_to_string(&mut previous_contents)?;
    assert_eq!(previous_contents, original);
    assert_eq!(
        fs_err::metadata(pkgconfig.path())?.permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(context.read(ignored.path()), "prefix=/install\n");
    assert_eq!(context.read(linked.path()), "prefix=/install\n");

    Ok(())
}
