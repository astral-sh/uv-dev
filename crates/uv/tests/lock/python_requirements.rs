use anyhow::Result;
use assert_fs::prelude::*;
use indoc::{formatdoc, indoc};

use uv_lock::Lock;
use uv_pep440::Version;
use uv_test::uv_snapshot;

/// Interior exclusions are part of the persisted Python contract.
#[test]
fn project_python_exclusions_invalidate_lock() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let pyproject = context.temp_dir.child("pyproject.toml");
    let write_project = |requires_python: &str| {
        pyproject.write_str(&formatdoc! {r#"
            [project]
            name = "project"
            version = "0.1.0"
            requires-python = "{requires_python}"
        "#})
    };
    write_project(">=3.9,<3.14")?;
    uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");

    for (requires_python, allows_zero, allows_five) in [
        (">=3.9,<3.14,!=3.10.*", false, false),
        (">=3.9,<3.14,!=3.10.0", false, true),
        (">=3.9,<3.14", true, true),
    ] {
        let before = context.read("uv.lock");
        write_project(requires_python)?;
        insta::allow_duplicates! {
            uv_snapshot!(context.filters(), context.lock().args(["--offline", "--locked"]), @"
            exit_code: 1 (failure)
            ----- stderr -----
            Resolved 1 package in [TIME]
            error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

            hint: To update the lockfile, run `uv lock`.
            ");
        }
        assert_eq!(before, context.read("uv.lock"));
        insta::allow_duplicates! {
            uv_snapshot!(context.filters(), context.lock().arg("--offline"), @"
            exit_code: 0 (success)
            ----- stderr -----
            Resolved 1 package in [TIME]
            ");
        }
        let updated = context.read("uv.lock");
        let lock = Lock::from_toml(&updated)?;
        assert_eq!(
            lock.requires_python().contains(&Version::new([3, 10, 0])),
            allows_zero,
            "{requires_python}: {updated}"
        );
        assert_eq!(
            lock.requires_python().contains(&Version::new([3, 10, 5])),
            allows_five,
            "{requires_python}: {updated}"
        );

        // A redundant bound changes the spelling, but not the supported Python versions.
        write_project(&format!(">=3.8,{requires_python}"))?;
        insta::allow_duplicates! {
            uv_snapshot!(context.filters(), context.lock().args(["--offline", "--locked"]), @"
            exit_code: 0 (success)
            ----- stderr -----
            Resolved 1 package in [TIME]
            ");
        }
        assert_eq!(updated, context.read("uv.lock"));
    }
    Ok(())
}

/// Script locks use the same complete Python requirement comparison as project locks.
#[test]
fn script_python_exclusions_invalidate_lock() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let script = context.temp_dir.child("script.py");
    let write_script = |requires_python: &str| {
        script.write_str(&formatdoc! {r#"
            # /// script
            # requires-python = "{requires_python}"
            # dependencies = []
            # ///
        "#})
    };
    write_script(">=3.9")?;
    uv_snapshot!(context.filters(), context.lock().args(["--script", "script.py", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved in [TIME]
    ");
    for (requires_python, allows_zero, allows_five) in [
        (">=3.9,!=3.10.*", false, false),
        (">=3.9,!=3.10.0.*", false, true),
        (">=3.9", true, true),
    ] {
        let before = context.read("script.py.lock");
        write_script(requires_python)?;
        insta::allow_duplicates! {
            uv_snapshot!(context.filters(), context.lock().args(["--script", "script.py", "--offline", "--locked"]), @"
            exit_code: 1 (failure)
            ----- stderr -----
            Resolved in [TIME]
            error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

            hint: To update the lockfile, run `uv lock`.
            ");
        }
        assert_eq!(before, context.read("script.py.lock"));
        insta::allow_duplicates! {
            uv_snapshot!(context.filters(), context.lock().args(["--script", "script.py", "--offline"]), @"
            exit_code: 0 (success)
            ----- stderr -----
            Resolved in [TIME]
            ");
        }
        let updated = context.read("script.py.lock");
        let lock = Lock::from_toml(&updated)?;
        assert_eq!(
            lock.requires_python().contains(&Version::new([3, 10, 0])),
            allows_zero
        );
        assert_eq!(
            lock.requires_python().contains(&Version::new([3, 10, 5])),
            allows_five
        );
        write_script(&format!(">=3.8,{requires_python}"))?;
        insta::allow_duplicates! {
            uv_snapshot!(context.filters(), context.lock().args(["--script", "script.py", "--offline", "--locked"]), @"
            exit_code: 0 (success)
            ----- stderr -----
            Resolved in [TIME]
            ");
        }
        assert_eq!(updated, context.read("script.py.lock"));
    }
    Ok(())
}

/// A member's interior exclusion is recorded in the intersection used by frozen Python selection.
#[test]
fn workspace_python_exclusion_reaches_frozen_lock() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.13"]);
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "root"
        version = "0.1.0"
        requires-python = ">=3.9"

        [tool.uv.workspace]
        members = ["member"]
    "#})?;
    let member = context.temp_dir.child("member/pyproject.toml");
    let write_member = |requires_python: &str| {
        member.write_str(&formatdoc! {r#"
            [project]
            name = "member"
            version = "0.1.0"
            requires-python = "{requires_python}"
        "#})
    };
    write_member(">=3.10")?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--python", "3.13"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Resolved 2 packages in [TIME]
    ");
    let before = context.read("uv.lock");
    write_member(">=3.10,!=3.12.*")?;
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--python", "3.13", "--locked"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Resolved 2 packages in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(before, context.read("uv.lock"));
    uv_snapshot!(context.filters(), context.lock().args(["--offline", "--python", "3.13"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.13.[X] interpreter at: [PYTHON-3.13]
    Resolved 2 packages in [TIME]
    ");
    let updated = context.read("uv.lock");
    let lock = Lock::from_toml(&updated)?;
    assert!(!lock.requires_python().contains(&Version::new([3, 12, 1])));
    assert!(lock.requires_python().contains(&Version::new([3, 13, 1])));

    fs_err::remove_file(context.temp_dir.child("pyproject.toml"))?;
    uv_snapshot!(context.filters(), context.sync().args([
        "--offline", "--frozen", "--preview-features", "frozen-lockfile",
        "--no-default-groups", "--python", "3.12",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    error: The requested interpreter resolved to Python 3.12.[X], which is incompatible with the project's Python requirement: `>=3.10, !=3.12.*` (from `requires-python` in `uv.lock`).
    ");
    assert_eq!(updated, context.read("uv.lock"));
    Ok(())
}
