use anyhow::Result;
use assert_fs::prelude::{FileWriteStr, PathChild};
use indoc::indoc;

use uv_test::uv_snapshot;

/// Requirements can continue between the package name and version specifier.
#[test]
fn pip_install_continued_version_specifier() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {r"
        idna \
            >1.0.0
    "})?;

    uv_snapshot!(context.pip_install()
        .arg("--dry-run")
        .arg("-r")
        .arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Would download 1 package
    Would install 1 package
     + idna==3.6
    "
    );

    Ok(())
}

/// Continuations may occur within requirement components, but never extend comments.
#[test]
fn pip_install_continued_requirement_components() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {r"
        # Comments do not continue. \
        id\
        na \
            >=3.\
        6, \
            <4  # The requirement can still have a trailing comment.
    "})?;

    uv_snapshot!(context.pip_install()
        .arg("--dry-run")
        .arg("-r")
        .arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Would download 1 package
    Would install 1 package
     + idna==3.6
    "
    );

    Ok(())
}

/// Requirements-file continuations also recognize Windows line endings.
#[test]
fn pip_install_continued_version_specifier_crlf() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(
        &indoc! {r"
            idna \
                >1.0.0
        "}
        .replace('\n', "\r\n"),
    )?;

    uv_snapshot!(context.pip_install()
        .arg("--dry-run")
        .arg("-r")
        .arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Would download 1 package
    Would install 1 package
     + idna==3.6
    "
    );

    Ok(())
}
