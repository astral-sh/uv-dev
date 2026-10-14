use std::env::consts::EXE_SUFFIX;
use std::path::Path;

use anyhow::{Context, Result};
use assert_cmd::prelude::*;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use fs_err as fs;
use indoc::{formatdoc, indoc};
use sha2::{Digest, Sha256};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_fs::Simplified;
use uv_redacted::SafeUrl;
use uv_static::EnvVars;
use uv_test::archive::generate_source_archive;
use uv_test::find_links::FindLinksServer;
use uv_test::packse::PackseServer;
use uv_test::{TestContext, copy_dir_ignore, download_to_disk, uv_snapshot, venv_bin_path};

/// Install a package into a virtual environment using the default link semantics. (On macOS,
/// this using `clone` semantics.)
#[test]
fn install() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("MarkupSafe==2.1.3")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + markupsafe==2.1.3
    "
    );

    // Counterpart for the `compile()` test.
    assert!(
        !context
            .site_packages()
            .join("markupsafe")
            .join("__pycache__")
            .join("__init__.cpython-312.pyc")
            .exists()
    );

    context
        .assert_command("from markupsafe import Markup")
        .success();

    // Removing the cache shouldn't invalidate the virtual environment.
    fs::remove_dir_all(context.cache_dir.path())?;

    context
        .assert_command("from markupsafe import Markup")
        .success();

    Ok(())
}

/// Compare the imported module with the corresponding unpacked cache entry.
fn assert_markupsafe_link_mode(
    context: &uv_test::TestContext,
    expected: &'static str,
) -> Result<()> {
    let cached = context
        .cache_files(uv_cache::CacheBucket::Archive)?
        .into_iter()
        .find(|path| path.ends_with("markupsafe/__init__.py"))
        .context("missing cached MarkupSafe module")?;
    context
        .python_command()
        .arg("-c")
        .arg(indoc! {r#"
            import sys
            from pathlib import Path
            import markupsafe
            from markupsafe import Markup

            installed = Path(markupsafe.__file__)
            same_file = installed.samefile(sys.argv[1])
            if installed.is_symlink():
                assert same_file, f"symlink does not target the cached module: {installed}"
                mode = "symlink"
            else:
                mode = "hardlink" if same_file else "copy"
            print(mode, end="")
        "#})
        .arg(cached)
        .assert()
        .success()
        .stdout(expected);
    Ok(())
}

/// Install a package into a virtual environment using copy semantics.
#[test]
fn install_copy() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("MarkupSafe==2.1.3")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--link-mode")
        .arg("copy")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + markupsafe==2.1.3
    "
    );

    assert_markupsafe_link_mode(&context, "copy")?;

    // Removing the cache shouldn't invalidate the virtual environment.
    fs::remove_dir_all(context.cache_dir.path())?;

    context
        .assert_command("from markupsafe import Markup")
        .success();

    Ok(())
}

/// Install a package into a virtual environment using hardlink semantics.
#[test]
fn install_hardlink() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("MarkupSafe==2.1.3")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--link-mode")
        .arg("hardlink")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + markupsafe==2.1.3
    "
    );

    assert_markupsafe_link_mode(&context, "hardlink")?;

    // Removing the cache shouldn't invalidate the virtual environment.
    fs::remove_dir_all(context.cache_dir.path())?;

    context
        .assert_command("from markupsafe import Markup")
        .success();

    Ok(())
}

/// Test that EMLINK (too many hardlinks) is handled gracefully.
///
/// This test exhausts the hardlink limit on a cached file, then verifies that
/// a subsequent install still succeeds by resetting the file's inode.
///
/// Requires `UV_INTERNAL__TEST_LOWLINKS_FS` pointing to a filesystem with a
/// low hardlink limit (e.g., minix with ~250).
#[test]
fn install_hardlink_after_emlink() -> anyhow::Result<()> {
    use walkdir::WalkDir;

    let Some(context) = uv_test::test_context!("3.12").with_cache_on_lowlinks_fs()? else {
        return Ok(());
    };

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("iniconfig==2.0.0")?;

    // First install to populate the cache.
    context
        .pip_sync()
        .arg("requirements.txt")
        .arg("--link-mode")
        .arg("hardlink")
        .assert()
        .success();

    // Find a cached .py file from the package.
    let cached_file = WalkDir::new(context.cache_dir.path())
        .into_iter()
        .filter_map(Result::ok)
        .find(|e| e.file_type().is_file() && e.path().extension().is_some_and(|ext| ext == "py"))
        .expect("should find a cached file")
        .into_path();

    // Create a temp directory to hold hardlinks on the same filesystem but outside the
    // cache tree (so the installer doesn't try to install the link files).
    let hardlink_dir = tempfile::tempdir_in(
        context
            .cache_dir
            .parent()
            .expect("cache dir should have a parent"),
    )?;

    // Create hardlinks until we hit EMLINK or reach 66000 (minix limit is 250, ext4 is 65000).
    let mut hit_emlink = false;
    for i in 0..66000 {
        let link_path = hardlink_dir.path().join(format!("link_{i}"));
        match fs::hard_link(&cached_file, &link_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::TooManyLinks => {
                hit_emlink = true;
                break;
            }
            Err(err) => {
                return Err(err.into());
            }
        }
    }

    assert!(
        hit_emlink,
        "Expected to hit TooManyLinks while creating hardlinks"
    );

    // Now try to install into a new venv on the same filesystem so the
    // hardlink stays same-device and actually hits EMLINK (then recovers).
    let venv2_dir = tempfile::tempdir_in(
        context
            .cache_dir
            .parent()
            .expect("cache dir should have a parent"),
    )?;
    let venv2 = venv2_dir.path().join("venv2");
    context.venv().arg(&venv2).assert().success();

    context
        .pip_sync()
        .arg("requirements.txt")
        .arg("--link-mode")
        .arg("hardlink")
        .env(EnvVars::VIRTUAL_ENV, &venv2)
        .assert()
        .success();

    // Verify that another hardlink can be created after recovery.
    let extra_link = hardlink_dir.path().join("post_recovery_link");
    fs::hard_link(&cached_file, &extra_link)?;

    Ok(())
}

/// Install a package into a virtual environment using symlink semantics.
#[test]
#[cfg(unix)] // Windows does not allow symlinks by default
fn install_symlink() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("MarkupSafe==2.1.3")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--link-mode")
        .arg("symlink")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + markupsafe==2.1.3
    "
    );

    assert_markupsafe_link_mode(&context, "symlink")?;

    // Removing the cache _should_ invalidate the virtual environment.
    fs::remove_dir_all(context.cache_dir.path())?;

    context
        .assert_command("from markupsafe import Markup")
        .failure();

    Ok(())
}

/// Reject attempts to use symlink semantics with `--no-cache`.
#[test]
fn install_symlink_no_cache() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("MarkupSafe==2.1.3")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--link-mode")
        .arg("symlink")
        .arg("--no-cache")
        .arg("--strict"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    error: Symlink-based installation is not supported with `--no-cache`. The created environment will be rendered unusable by the removal of the cache.
    "
    );

    Ok(())
}

/// Install multiple packages into a virtual environment.
#[test]
fn install_many() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {"
        MarkupSafe==2.1.3
        tomli==2.0.1
    "})?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + markupsafe==2.1.3
     + tomli==2.0.1
    "
    );

    context
        .assert_command("from markupsafe import Markup; import tomli")
        .success();

    Ok(())
}

/// Attempt to install an already-installed package into a virtual environment.
#[test]
fn noop() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("MarkupSafe==2.1.3")?;

    context
        .pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        .assert()
        .success();

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    "
    );

    context
        .assert_command("from markupsafe import Markup")
        .success();

    Ok(())
}

/// Attempt to sync an empty set of requirements.
#[test]
fn pip_sync_empty() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.touch()?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--allow-empty-requirements"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Requirements file `requirements.txt` does not contain any dependencies
    Resolved in [TIME]
    Checked in [TIME]
    "
    );

    // Install a package.
    requirements_txt.write_str("iniconfig==2.0.0")?;
    context
        .pip_sync()
        .arg("requirements.txt")
        .assert()
        .success();

    // An empty requirements file must not clear the environment without explicit opt-in.
    requirements_txt.write_str("")?;
    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Requirements file `requirements.txt` does not contain any dependencies
    No requirements found (hint: use `--allow-empty-requirements` to clear the environment)
    "
    );
    context.assert_command("import iniconfig").success();

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--allow-empty-requirements"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Requirements file `requirements.txt` does not contain any dependencies
    Resolved in [TIME]
    Uninstalled 1 package in [TIME]
     - iniconfig==2.0.0
    "
    );
    context.assert_not_installed("iniconfig");

    Ok(())
}

/// Install a package into a virtual environment, then install the same package into a different
/// virtual environment.
#[test]
fn link() -> Result<()> {
    let context1 = uv_test::test_context!("3.12");

    let requirements_txt = context1.temp_dir.child("requirements.txt");
    requirements_txt.write_str("iniconfig==2.0.0")?;

    context1
        .pip_sync()
        .arg(requirements_txt.path())
        .arg("--strict")
        .assert()
        .success();

    // Create a separate virtual environment, but reuse the same cache.
    let context2 = uv_test::test_context!("3.12");
    let mut cmd = context1.pip_sync();
    cmd.env(EnvVars::VIRTUAL_ENV, context2.venv.as_os_str())
        .current_dir(&context2.temp_dir);

    uv_snapshot!(cmd
        .arg(requirements_txt.path())
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    "
    );

    context2
        .python_command()
        .arg("-c")
        .arg("import iniconfig")
        .current_dir(&context2.temp_dir)
        .assert()
        .success();

    Ok(())
}

/// Install a package into a virtual environment, then sync the virtual environment with a
/// different requirements file.
#[test]
fn add_remove() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("iniconfig==2.0.0")?;

    context
        .pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        .assert()
        .success();
    context.assert_command("import iniconfig").success();

    requirements_txt.write_str("tomli==2.0.1")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - iniconfig==2.0.0
     + tomli==2.0.1
    "
    );

    context.assert_command("import tomli").success();
    context.assert_not_installed("iniconfig");

    Ok(())
}

/// Install a package into a virtual environment, then install a second package into the same
/// virtual environment.
#[test]
fn install_sequential() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("iniconfig==2.0.0")?;

    context
        .pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        .assert()
        .success();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {"
        iniconfig==2.0.0
        tomli==2.0.1
    "})?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tomli==2.0.1
    "
    );

    context
        .assert_command("import iniconfig; import tomli")
        .success();

    Ok(())
}

/// Install a package into a virtual environment, then install a second package into the same
/// virtual environment.
#[test]
fn upgrade() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("tomli==2.0.0")?;

    context
        .pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        .assert()
        .success();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("tomli==2.0.1")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - tomli==2.0.0
     + tomli==2.0.1
    "
    );

    context.assert_command("import tomli").success();

    Ok(())
}

/// Install a package into a virtual environment from a URL.
#[test]
fn install_url() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("werkzeug @ https://files.pythonhosted.org/packages/ff/1d/960bb4017c68674a1cb099534840f18d3def3ce44aed12b5ed8b78e0153e/Werkzeug-2.0.0-py3-none-any.whl")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + werkzeug==2.0.0 (from https://files.pythonhosted.org/packages/ff/1d/960bb4017c68674a1cb099534840f18d3def3ce44aed12b5ed8b78e0153e/Werkzeug-2.0.0-py3-none-any.whl)
    "
    );

    context.assert_command("import werkzeug").success();

    Ok(())
}

/// Install a package into a virtual environment from a Git repository.
#[test]
#[cfg(feature = "test-git")]
fn install_git_commit() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("uv-public-pypackage @ git+https://github.com/astral-test/uv-public-pypackage@b270df1a2fb5d012294e9aaf05e7e0bab1e6a389")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage@b270df1a2fb5d012294e9aaf05e7e0bab1e6a389)
    "
    );

    context
        .assert_command("import uv_public_pypackage")
        .success();

    Ok(())
}

/// Install a package into a virtual environment from a Git repository.
#[test]
#[cfg(feature = "test-git")]
fn install_git_tag() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(
        "uv-public-pypackage @ git+https://github.com/astral-test/uv-public-pypackage@test-tag",
    )?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage@0dacfd662c64cb4ceb16e6cf65a157a8b715b979)
    "
    );

    context
        .assert_command("import uv_public_pypackage")
        .success();

    Ok(())
}

/// Install two packages from the same Git repository.
#[test]
#[cfg(feature = "test-git")]
fn install_git_subdirectories() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {"
        example-pkg-a @ git+https://github.com/pypa/sample-namespace-packages.git@df7530eeb8fa0cb7dbb8ecb28363e8e36bfa2f45#subdirectory=pkg_resources/pkg_a
        example-pkg-b @ git+https://github.com/pypa/sample-namespace-packages.git@df7530eeb8fa0cb7dbb8ecb28363e8e36bfa2f45#subdirectory=pkg_resources/pkg_b
    "})?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + example-pkg-a==1 (from git+https://github.com/pypa/sample-namespace-packages.git@df7530eeb8fa0cb7dbb8ecb28363e8e36bfa2f45#subdirectory=pkg_resources/pkg_a)
     + example-pkg-b==1 (from git+https://github.com/pypa/sample-namespace-packages.git@df7530eeb8fa0cb7dbb8ecb28363e8e36bfa2f45#subdirectory=pkg_resources/pkg_b)
    "
    );

    context.assert_command("import example_pkg").success();
    context.assert_command("import example_pkg.a").success();
    context.assert_command("import example_pkg.b").success();

    Ok(())
}

/// Install a source distribution into a virtual environment.
#[test]
fn install_sdist() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_exclude_newer("2025-01-29T00:00:00Z");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("source-distribution==0.0.1")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.1
    "
    );

    context
        .assert_command("import source_distribution")
        .success();

    Ok(())
}

/// Install a source distribution into a virtual environment.
#[test]
fn install_sdist_url() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("source-distribution @ https://files.pythonhosted.org/packages/10/1f/57aa4cce1b1abf6b433106676e15f9fa2c92ed2bd4cf77c3b50a9e9ac773/source_distribution-0.0.1.tar.gz")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.1 (from https://files.pythonhosted.org/packages/10/1f/57aa4cce1b1abf6b433106676e15f9fa2c92ed2bd4cf77c3b50a9e9ac773/source_distribution-0.0.1.tar.gz)
    "
    );

    context
        .assert_command("import source_distribution")
        .success();

    Ok(())
}

/// Attempt to install a direct URL source distribution with a non-PEP 625-compliant
/// archive format (e.g., `.tar.bz2`). This should hard-error.
#[test]
fn reject_sdist_archive_type_bz2() -> Result<()> {
    let context = uv_test::test_context!("3.9");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "bz2 @ {}",
        context
            .workspace_root
            .join("test/links/bz2-1.0.0.tar.bz2")
            .display()
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Source distribution `[WORKSPACE]/test/links/bz2-1.0.0.tar.bz2` has a non-PEP 625-compliant filename; only `.tar.gz` and `.zip` archives are accepted
    "
    );

    Ok(())
}

/// Attempt to re-install a package into a virtual environment from a URL. The second install
/// should be a no-op.
#[test]
fn install_url_then_install_url() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("werkzeug @ https://files.pythonhosted.org/packages/ff/1d/960bb4017c68674a1cb099534840f18d3def3ce44aed12b5ed8b78e0153e/Werkzeug-2.0.0-py3-none-any.whl")?;

    context
        .pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        .assert()
        .success();

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    "
    );

    context.assert_command("import werkzeug").success();

    Ok(())
}

/// Install a package via a URL, then via a registry version. The second install _should_ remove the
/// URL-based version, but doesn't right now.
#[test]
fn install_url_then_install_version() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("werkzeug @ https://files.pythonhosted.org/packages/ff/1d/960bb4017c68674a1cb099534840f18d3def3ce44aed12b5ed8b78e0153e/Werkzeug-2.0.0-py3-none-any.whl")?;

    context
        .pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        .assert()
        .success();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("werkzeug==2.0.0")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    "
    );

    context.assert_command("import werkzeug").success();

    Ok(())
}

/// Test that we select the last 3.8 compatible numpy version instead of trying to compile an
/// incompatible sdist <https://github.com/astral-sh/uv/issues/388>
#[cfg(feature = "test-python-eol")]
#[test]
fn install_numpy_py38() -> Result<()> {
    let context = uv_test::test_context!("3.8");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("numpy")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + numpy==1.24.4
    "
    );

    context.assert_command("import numpy").success();

    Ok(())
}

#[test]
fn json() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("iniconfig==2.0.0")?;

    context.pip_install().arg("idna==3.6").assert().success();

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--check")
        .arg("--output-format=json")
        .arg("--quiet"), @r#"
    exit_code: 1 (failure)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "changes": [
        {
          "name": "idna",
          "version": "3.6",
          "action": "uninstalled"
        },
        {
          "name": "iniconfig",
          "version": "2.0.0",
          "action": "installed"
        }
      ],
      "dry_run": true
    }
    "#);

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--dry-run")
        .arg("--output-format=json"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "changes": [
        {
          "name": "idna",
          "version": "3.6",
          "action": "uninstalled"
        },
        {
          "name": "iniconfig",
          "version": "2.0.0",
          "action": "installed"
        }
      ],
      "dry_run": true
    }

    ----- stderr -----
    Resolved 1 package in [TIME]
    Would download 1 package
    Would uninstall 1 package
    Would install 1 package
     - idna==3.6
     + iniconfig==2.0.0
    "#);

    context
        .pip_freeze()
        .assert()
        .success()
        .stdout("idna==3.6\n");

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--output-format=json")
        .arg("--quiet"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "changes": [
        {
          "name": "idna",
          "version": "3.6",
          "action": "uninstalled"
        },
        {
          "name": "iniconfig",
          "version": "2.0.0",
          "action": "installed"
        }
      ],
      "dry_run": false
    }
    "#);

    context
        .pip_freeze()
        .assert()
        .success()
        .stdout("iniconfig==2.0.0\n");

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--output-format=json")
        .arg("--quiet"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "changes": [],
      "dry_run": false
    }
    "#);

    requirements_txt.write_str("")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--output-format=json"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    {
      "schema": {
        "version": "preview"
      },
      "changes": [],
      "dry_run": false
    }

    ----- stderr -----
    warning: Requirements file `requirements.txt` does not contain any dependencies
    No requirements found (hint: use `--allow-empty-requirements` to clear the environment)
    "#);

    Ok(())
}

#[test]
fn check_sync() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("iniconfig==2.0.0")?;

    uv_snapshot!(context.pip_sync().arg("requirements.txt").arg("--check"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Would download 1 package
    Would install 1 package
     + iniconfig==2.0.0
    ");

    // Checking must leave the environment unchanged.
    uv_snapshot!(context.pip_sync().arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    uv_snapshot!(context.pip_sync().arg("requirements.txt").arg("--check").arg("--offline"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    Would make no changes
    ");

    // Sync also checks for packages that would be removed.
    requirements_txt.write_str("")?;
    uv_snapshot!(context.pip_sync().arg("requirements.txt").arg("--allow-empty-requirements").arg("--check"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    warning: Requirements file `requirements.txt` does not contain any dependencies
    Resolved in [TIME]
    Would uninstall 1 package
     - iniconfig==2.0.0
    ");

    context.assert_command("import iniconfig").success();

    Ok(())
}

/// Reject decoded path separators in an unnamed wheel URL before using the filename in cache paths.
#[test]
fn install_unnamed_wheel_url_rejects_path_traversal() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("https://example.com/packages/pkg-1.0-py3-none-..%2F..%2F..%2Ftarget.whl")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    error: The wheel filename `pkg-1.0-py3-none-../../../target.whl` is invalid: Tag components must contain only ASCII letters, digits, underscores, and periods
    "#
    );

    Ok(())
}

/// Reject decoded stream separators in an unnamed wheel URL before using the filename in cache paths.
#[test]
fn install_unnamed_wheel_url_rejects_stream_separator() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("https://example.com/packages/pkg-1.0-py3-none-target%3Astream.whl")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @r#"
    exit_code: 1 (failure)
    ----- stderr -----
    error: The wheel filename `pkg-1.0-py3-none-target:stream.whl` is invalid: Tag components must contain only ASCII letters, digits, underscores, and periods
    "#
    );

    Ok(())
}

/// Install a local source distribution.
#[test]
fn install_local_source_distribution() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Download a source distribution.
    let archive = context.temp_dir.child("wheel-0.42.0.tar.gz");
    download_to_disk(
        "https://files.pythonhosted.org/packages/b0/b4/bc2baae3970c282fae6c2cb8e0f179923dceb7eaffb0e76170628f9af97b/wheel-0.42.0.tar.gz",
        &archive,
    );

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "wheel @ {}",
        SafeUrl::from_file_path(archive.path()).unwrap()
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + wheel==0.42.0 (from file://[TEMP_DIR]/wheel-0.42.0.tar.gz)
    "
    );

    context.assert_command("import wheel").success();

    Ok(())
}

/// This package includes a `[build-system]`, but no `build-backend`.
///
/// It lists some explicit build requirements that are necessary to build the distribution:
/// ```toml
/// [build-system]
/// requires = ["Cython<3", "setuptools", "wheel"]
/// ```
///
/// Like `pip` and `build`, we should use PEP 517 here and respect the `requires`, but use the
/// default build backend.
///
/// The example is based `DTLSSocket==0.1.16`
#[test]
fn install_build_system_no_backend() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("build-system-no-backend @ https://files.pythonhosted.org/packages/ec/25/1e531108ca027dc3a3b37d351f4b86d811df4884c6a81cd99e73b8b589f5/build-system-no-backend-0.1.0.tar.gz")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + build-system-no-backend==0.1.0 (from https://files.pythonhosted.org/packages/ec/25/1e531108ca027dc3a3b37d351f4b86d811df4884c6a81cd99e73b8b589f5/build-system-no-backend-0.1.0.tar.gz)
    "
    );

    context
        .assert_command("import build_system_no_backend")
        .success();

    Ok(())
}

/// Check that we show the right messages on cached, Git source distribution installs.
#[test]
#[cfg(feature = "test-git")]
fn install_git_source_dist_cached() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("uv-public-pypackage @ git+https://github.com/astral-test/uv-public-pypackage@b270df1a2fb5d012294e9aaf05e7e0bab1e6a389")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage@b270df1a2fb5d012294e9aaf05e7e0bab1e6a389)
    "
    );

    context
        .assert_command("import uv_public_pypackage")
        .success();

    // Re-run the installation in a new virtual environment.
    context.reset_venv();

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage@b270df1a2fb5d012294e9aaf05e7e0bab1e6a389)
    "
    );

    context
        .assert_command("import uv_public_pypackage")
        .success();

    // Remove the package's cached build, then re-run the installation in a new environment.
    context.reset_venv();

    uv_snapshot!(context.filters(), context.clean()
        .arg("uv-public-pypackage"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed [N] files ([SIZE])
    "
    );

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage@b270df1a2fb5d012294e9aaf05e7e0bab1e6a389)
    "
    );

    context
        .assert_command("import uv_public_pypackage")
        .success();

    Ok(())
}

/// Verify that fail with an appropriate error when a package is repeated.
#[test]
fn duplicate_package_overlap() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {"
        MarkupSafe==2.1.3
        MarkupSafe==2.1.2
    "})?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because you require markupsafe==2.1.3 and markupsafe==2.1.2, we can conclude that your requirements are unsatisfiable.
    "
    );

    Ok(())
}

/// Verify that allow duplicate packages when they are disjoint.
#[test]
fn duplicate_package_disjoint() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {"
        MarkupSafe==2.1.3
        MarkupSafe==2.1.2 ; python_version < '3.6'
    "})?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + markupsafe==2.1.3
    "
    );

    Ok(())
}

/// Verify that we can force reinstall of Git dependencies.
#[test]
#[cfg(feature = "test-git")]
fn reinstall_git() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("uv-public-pypackage @ git+https://github.com/astral-test/uv-public-pypackage@b270df1a2fb5d012294e9aaf05e7e0bab1e6a389")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage@b270df1a2fb5d012294e9aaf05e7e0bab1e6a389)
    "
    );

    context
        .assert_command("import uv_public_pypackage")
        .success();

    // Re-run the installation with `--reinstall`.
    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall-package")
        .arg("uv-public-pypackage")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage@b270df1a2fb5d012294e9aaf05e7e0bab1e6a389)
    "
    );

    context
        .assert_command("import uv_public_pypackage")
        .success();

    Ok(())
}

/// Verify that we can force refresh of cached data.
#[test]
fn refresh() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {"
        MarkupSafe==2.1.3
        tomli==2.0.1
    "})?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + markupsafe==2.1.3
     + tomli==2.0.1
    "
    );

    context.assert_command("import markupsafe").success();
    context.assert_command("import tomli").success();

    // Re-run the installation into with `--refresh`. Ensure that we resolve and download the
    // latest versions of the packages.
    context.reset_venv();

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--refresh")
        .arg("--strict")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + markupsafe==2.1.3
     + tomli==2.0.1
    "
    );

    context.assert_command("import markupsafe").success();
    context.assert_command("import tomli").success();

    Ok(())
}

/// Verify that we can force refresh of selective packages.
#[test]
fn refresh_package() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {"
        MarkupSafe==2.1.3
        tomli==2.0.1
    "})?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + markupsafe==2.1.3
     + tomli==2.0.1
    "
    );

    context.assert_command("import markupsafe").success();
    context.assert_command("import tomli").success();

    // Re-run the installation into with `--refresh`. Ensure that we resolve and download the
    // latest versions of the packages.
    context.reset_venv();

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--refresh-package")
        .arg("tomli")
        .arg("--strict")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 2 packages in [TIME]
     + markupsafe==2.1.3
     + tomli==2.0.1
    "
    );

    context.assert_command("import markupsafe").success();
    context.assert_command("import tomli").success();

    Ok(())
}

#[test]
fn sync_editable() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let poetry_editable = context.temp_dir.child("poetry_editable");

    // Copy into the temporary directory so we can mutate it.
    copy_dir_ignore(
        context.workspace_root.join("test/packages/poetry_editable"),
        &poetry_editable,
    )?;

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&indoc::formatdoc! {r"
        anyio==3.7.0
        -e file://{poetry_editable}
        ",
        poetry_editable = poetry_editable.display()
    })?;

    // Install the editable package.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg(requirements_txt.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + anyio==3.7.0
     + poetry-editable==0.1.0 (from file://[TEMP_DIR]/poetry_editable)
    "
    );

    // Re-install the editable package. This is a no-op.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg(requirements_txt.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Checked 2 packages in [TIME]
    "
    );

    // Reinstall the editable package. This won't trigger a rebuild, but it will trigger an install.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg(requirements_txt.path())
        .arg("--reinstall-package")
        .arg("poetry-editable"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ poetry-editable==0.1.0 (from file://[TEMP_DIR]/poetry_editable)
    "
    );

    let python_source_file = poetry_editable.path().join("poetry_editable/__init__.py");
    let check_installed = indoc::indoc! {r#"
        from poetry_editable import a

        assert a() == "a", a()
   "#};
    context.assert_command(check_installed).success();

    // Edit the sources and make sure the changes are respected without syncing again.
    let python_version_1 = indoc::indoc! {r"
        version = 1
   "};
    fs_err::write(&python_source_file, python_version_1)?;

    let check_installed = indoc::indoc! {r"
        from poetry_editable import version

        assert version == 1, version
   "};
    context.assert_command(check_installed).success();

    let python_version_2 = indoc::indoc! {r"
        version = 2
   "};
    fs_err::write(&python_source_file, python_version_2)?;

    let check_installed = indoc::indoc! {r"
        from poetry_editable import version

        assert version == 2, version
   "};
    context.assert_command(check_installed).success();

    // Reinstall the editable package. This won't trigger a rebuild or reinstall, since we only
    // detect changes to metadata files (like `pyproject.toml`).
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg(requirements_txt.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Checked 2 packages in [TIME]
    "
    );

    // Modify the `pyproject.toml` file.
    let pyproject_toml = poetry_editable.path().join("pyproject.toml");
    let pyproject_toml_contents = context.read("poetry_editable/pyproject.toml");
    fs_err::write(
        &pyproject_toml,
        pyproject_toml_contents.replace("0.1.0", "0.1.1"),
    )?;

    // Reinstall the editable package. This will trigger a rebuild and reinstall.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg(requirements_txt.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - poetry-editable==0.1.0 (from file://[TEMP_DIR]/poetry_editable)
     + poetry-editable==0.1.1 (from file://[TEMP_DIR]/poetry_editable)
    "
    );

    // Updating metadata without changing the version must also trigger a rebuild.
    let pyproject_toml_contents = context.read("poetry_editable/pyproject.toml");
    fs_err::write(
        &pyproject_toml,
        pyproject_toml_contents.replace(
            "description = \"\"",
            "description = \"Updated editable metadata\"",
        ),
    )?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg(requirements_txt.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ poetry-editable==0.1.1 (from file://[TEMP_DIR]/poetry_editable)
    "
    );
    context
        .assert_command(
            "from importlib.metadata import metadata; print(metadata('poetry-editable')['Summary'])",
        )
        .success()
        .stdout(predicates::str::contains("Updated editable metadata"));

    Ok(())
}

#[test]
fn sync_editable_and_registry() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Copy the black test editable into the "current" directory
    copy_dir_ignore(
        context.workspace_root.join("test/packages/black_editable"),
        context.temp_dir.join("black_editable"),
    )?;

    // Install the registry-based version of Black.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc::indoc! {r"
        black==24.1.0
        "
    })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg(requirements_txt.path())
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + black==24.1.0
    warning: The package `black` requires `click>=8.0.0`, but it's not installed
    warning: The package `black` requires `mypy-extensions>=0.4.3`, but it's not installed
    warning: The package `black` requires `packaging>=22.0`, but it's not installed
    warning: The package `black` requires `pathspec>=0.9.0`, but it's not installed
    warning: The package `black` requires `platformdirs>=2`, but it's not installed
    "
    );

    // Install the editable version of Black. This should remove the registry-based version.
    // Use the `file:` syntax for extra coverage.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc::indoc! {r"
        -e file:./black_editable
        "
    })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg(requirements_txt.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - black==24.1.0
     + black==0.1.0 (from file://[TEMP_DIR]/black_editable)
    "
    );

    // Re-install the registry-based version of Black. This should be a no-op, since we have a
    // version of Black installed (the editable version) that satisfies the requirements.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc::indoc! {r"
        black
        "
    })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg(requirements_txt.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    "
    );

    // Re-install Black at a specific version. This should replace the editable version.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc::indoc! {r"
        black==23.10.0
        "
    })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg(requirements_txt.path())
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - black==0.1.0 (from file://[TEMP_DIR]/black_editable)
     + black==23.10.0
    warning: The package `black` requires `click>=8.0.0`, but it's not installed
    warning: The package `black` requires `mypy-extensions>=0.4.3`, but it's not installed
    warning: The package `black` requires `packaging>=22.0`, but it's not installed
    warning: The package `black` requires `pathspec>=0.9.0`, but it's not installed
    warning: The package `black` requires `platformdirs>=2`, but it's not installed
    "
    );

    Ok(())
}

/// Switching installation modes must change where Python imports the package from.
#[test]
fn sync_editable_and_local() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Copy the black test editable into the "current" directory
    copy_dir_ignore(
        context.workspace_root.join("test/packages/black_editable"),
        context.temp_dir.join("black_editable"),
    )?;
    let source_module = context.temp_dir.join("black_editable/black/__init__.py");
    let installed_module = context.site_packages().join("black/__init__.py");
    let assert_import_origin = |expected: &Path| {
        context
            .python_command()
            .args([
                "-I",
                "-c",
                "import black, os, sys; assert os.path.samefile(black.__file__, sys.argv[1]), black.__file__",
            ])
            .arg(expected)
            .assert()
            .success();
    };

    // Install the editable version of Black.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc::indoc! {r"
        -e file:./black_editable
        "
    })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg(requirements_txt.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + black==0.1.0 (from file://[TEMP_DIR]/black_editable)
    "
    );
    assert_import_origin(&source_module);

    // Install the non-editable version of Black. This should replace the editable version.
    requirements_txt.write_str(indoc::indoc! {r"
        black @ file:./black_editable
        "
    })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg(requirements_txt.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ black==0.1.0 (from file://[TEMP_DIR]/black_editable)
    "
    );
    assert_import_origin(&installed_module);

    // Reinstall the editable version of Black. This should replace the non-editable version.
    requirements_txt.write_str(indoc::indoc! {r"
        -e file:./black_editable
        "
    })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg(requirements_txt.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ black==0.1.0 (from file://[TEMP_DIR]/black_editable)
    "
    );
    assert_import_origin(&source_module);

    Ok(())
}

#[test]
fn incompatible_wheel() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = context.temp_dir.child("foo-1.2.3-py3-none-any.whl");
    wheel.touch()?;

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!("foo @ {}", wheel.path().simplified_display()))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because foo has an invalid package format and you require foo, we can conclude that your requirements are unsatisfiable.

    hint: The structure of `foo` was invalid
      Caused by: Failed to read from zip file
      Caused by: unable to locate the end of central directory record
    "
    );

    Ok(())
}

/// Install a project without a `pyproject.toml`, using the PEP 517 build backend.
#[test]
fn sync_legacy_sdist_pep_517() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_in = context.temp_dir.child("requirements.in");
    requirements_in.write_str("flake8 @ https://files.pythonhosted.org/packages/66/53/3ad4a3b74d609b3b9008a10075c40e7c8909eae60af53623c3888f7a529a/flake8-6.0.0.tar.gz")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.in"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + flake8==6.0.0 (from https://files.pythonhosted.org/packages/66/53/3ad4a3b74d609b3b9008a10075c40e7c8909eae60af53623c3888f7a529a/flake8-6.0.0.tar.gz)
    "
    );

    Ok(())
}

/// Sync using `--find-links` with a local directory.
#[test]
fn find_links() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {r"
        markupsafe==2.1.3
        numpy==1.26.3
        tqdm==1000.0.0
        werkzeug @ https://files.pythonhosted.org/packages/c3/fc/254c3e9b5feb89ff5b9076a23218dafbc99c96ac5941e900b71206e6313b/werkzeug-3.0.1-py3-none-any.whl
    "})?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--find-links")
        .arg(context.workspace_root.join("test/links/")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + markupsafe==2.1.3
     + numpy==1.26.3
     + tqdm==1000.0.0
     + werkzeug==3.0.1 (from https://files.pythonhosted.org/packages/c3/fc/254c3e9b5feb89ff5b9076a23218dafbc99c96ac5941e900b71206e6313b/werkzeug-3.0.1-py3-none-any.whl)
    "
    );

    Ok(())
}

/// A second installation reuses the cached `--find-links` wheel without downloading it again.
#[tokio::test]
async fn find_links_wheel_cache() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let filename = "ok-1.0.0-py3-none-any.whl";
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            format!(r#"<a href="/{filename}">{filename}</a>"#),
            "text/html",
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{filename}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(fs::read(
            context.workspace_root.join("test/links").join(filename),
        )?))
        .expect(1)
        .mount(&server)
        .await;

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("ok==1.0.0")?;

    // Populate the wheel cache.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--no-index")
        .arg("--find-links")
        .arg(server.uri()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==1.0.0
    "
    );

    context.pip_uninstall().arg("ok").assert().success();

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--no-index")
        .arg("--find-links")
        .arg(server.uri()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==1.0.0
    "
    );

    context.assert_installed("ok", "1.0.0");
    Ok(())
}

/// Reuse a wheel built from a local `--find-links` source distribution.
#[test]
fn find_links_source_cache() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let build_index = PackseServer::empty();
    let links = context.temp_dir.child("links");
    links.create_dir_all()?;
    let sdist = "tqdm-999.0.0.tar.gz";
    fs::copy(
        context.workspace_root.join("test/links").join(sdist),
        links.child(sdist),
    )?;

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("tqdm==999.0.0")?;

    // Build and cache the source distribution's wheel.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--find-links")
        .arg(links.path())
        .arg("--index-url")
        .arg(build_index.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==999.0.0
    "
    );

    context.pip_uninstall().arg("tqdm").assert().success();
    context.assert_not_installed("hatchling");

    // Building without isolation would require `hatchling` in the target environment. With only
    // the source archive available in `links`, installation therefore requires the cached wheel.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--no-build-isolation")
        .arg("--no-index")
        .arg("--find-links")
        .arg(links.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==999.0.0
    "
    );

    Ok(())
}

/// Install without network access via the `--offline` flag.
#[test]
fn offline() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let requirements_in = context.temp_dir.child("requirements.in");
    requirements_in.write_str("black==23.10.1")?;

    // Install with `--offline` with an empty cache.
    uv_snapshot!(context.pip_sync()
        .arg("requirements.in")
        .arg("--offline"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because black was not found in the cache and you require black==23.10.1, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because the network was disabled. When the network is disabled, registry packages may only be read from the cache.
    "
    );

    // Populate the cache.
    uv_snapshot!(context.pip_sync()
        .arg("requirements.in"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + black==23.10.1
    "
    );

    // Install with `--offline` with a populated cache.
    context.reset_venv();

    uv_snapshot!(context.pip_sync()
        .arg("requirements.in")
        .arg("--offline")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + black==23.10.1
    "
    );

    Ok(())
}

/// Include a `constraints.txt` file with a compatible constraint.
#[test]
fn compatible_constraint() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("anyio==3.7.0")?;

    let constraints_txt = context.temp_dir.child("constraints.txt");
    constraints_txt.write_str("anyio==3.7.0")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--constraint")
        .arg("constraints.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==3.7.0
    "
    );

    Ok(())
}

/// Include a `constraints.txt` file with an incompatible constraint.
#[test]
fn incompatible_constraint() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("anyio==3.7.0")?;

    let constraints_txt = context.temp_dir.child("constraints.txt");
    constraints_txt.write_str("anyio==3.6.0")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--constraint")
        .arg("constraints.txt"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because you require anyio==3.7.0 and anyio==3.6.0, we can conclude that your requirements are unsatisfiable.
    "
    );

    Ok(())
}

/// Include a `constraints.txt` file with an irrelevant constraint.
#[test]
fn irrelevant_constraint() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("anyio==3.7.0")?;

    let constraints_txt = context.temp_dir.child("constraints.txt");
    constraints_txt.write_str("black==23.10.1")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--constraint")
        .arg("constraints.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==3.7.0
    "
    );

    Ok(())
}

/// Sync with a repeated `anyio` requirement.
#[test]
fn repeat_requirement_identical() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let requirements_in = context.temp_dir.child("requirements.in");
    requirements_in.write_str(indoc! {"
        anyio
        anyio
    "})?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.in"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.3.0
    ");

    Ok(())
}

/// Sync with a repeated `anyio` requirement, with compatible versions.
#[test]
fn repeat_requirement_compatible() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let requirements_in = context.temp_dir.child("requirements.in");
    requirements_in.write_str(indoc! {"
        anyio
        anyio==4.0.0
    "})?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.in"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.0.0
    ");

    Ok(())
}

/// Sync with a repeated, but conflicting `anyio` requirement.
#[test]
fn repeat_requirement_incompatible() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let requirements_in = context.temp_dir.child("requirements.in");
    requirements_in.write_str(indoc! {"
        anyio<4.0.0
        anyio==4.0.0
    "})?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.in"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because you require anyio<4.0.0 and anyio==4.0.0, we can conclude that your requirements are unsatisfiable.
    ");

    Ok(())
}

/// Installed files must remain owner-readable even if the archive records restrictive modes.
#[test]
fn set_read_permissions() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let requirements_in = context.temp_dir.child("requirements.in");
    requirements_in.write_str("databricks==0.2")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.in"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + databricks==0.2
    ");

    context
        .assert_command(indoc! {r#"
            import stat
            from importlib.metadata import distribution

            files = [entry.locate() for entry in distribution("databricks").files or ()]
            files = [path for path in files if path.is_file()]
            assert files, "no installed files were found"
            unreadable = [str(path) for path in files if not path.stat().st_mode & stat.S_IRUSR]
            assert not unreadable, f"installed files without owner read permission: {unreadable}"
        "#})
        .success();

    Ok(())
}

/// Generate the versioned pip launcher for the target interpreter. See astral-sh/uv#1593.
#[test]
fn pip_entrypoints() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("pip==24.0")?;

    uv_snapshot!(context.pip_sync().arg("requirements.txt").arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + pip==24.0
    ");

    let bin_dir = ChildPath::new(venv_bin_path(&context.venv));
    bin_dir
        .child(format!("pip3.10{EXE_SUFFIX}"))
        .assert(predicates::path::missing());
    bin_dir
        .child(format!("pip3.12{EXE_SUFFIX}"))
        .assert(predicates::path::is_file());

    Ok(())
}

/// pip changed its launcher entry points in 24.1b1. See astral-sh/uv#1982.
#[test]
fn pip_entrypoints_24_1() -> Result<()> {
    // The default test cutoff predates pip's changed entry-point format.
    let context = uv_test::test_context!("3.12").with_exclude_newer("2025-01-01T00:00:00Z");
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("pip==24.1b1")?;

    uv_snapshot!(context.pip_sync().arg("requirements.txt").arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + pip==24.1b1
    ");

    let bin_dir = ChildPath::new(venv_bin_path(&context.venv));
    bin_dir
        .child(format!("pip3.10{EXE_SUFFIX}"))
        .assert(predicates::path::missing());
    bin_dir
        .child(format!("pip3.12{EXE_SUFFIX}"))
        .assert(predicates::path::is_file());

    Ok(())
}

#[test]
fn invalidate_on_change() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let inspect_requirements = "from importlib.metadata import requires; print([''.join(r.split()) for r in requires('example')])";

    // Create an editable package.
    let editable_dir = context.temp_dir.child("editable");
    editable_dir.create_dir_all()?;
    let pyproject_toml = editable_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"[project]
name = "example"
version = "0.0.0"
dependencies = [
  "anyio==4.0.0"
]
requires-python = ">=3.8"
"#,
    )?;

    // Write to a requirements file.
    let requirements_in = context.temp_dir.child("requirements.in");
    requirements_in.write_str(&format!("-e {}", editable_dir.path().display()))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.in"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + example==0.0.0 (from file://[TEMP_DIR]/editable)
    "
    );
    context
        .assert_command(inspect_requirements)
        .success()
        .stdout(predicates::str::contains("['anyio==4.0.0']"));

    // Installing again should be a no-op.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.in"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    "
    );

    // Modify the editable package.
    pyproject_toml.write_str(
        r#"[project]
name = "example"
version = "0.0.0"
dependencies = [
  "anyio==3.7.1"
]
requires-python = ">=3.8"
"#,
    )?;

    // Re-installing should update the package.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.in"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ example==0.0.0 (from file://[TEMP_DIR]/editable)
    "
    );
    context
        .assert_command(inspect_requirements)
        .success()
        .stdout(predicates::str::contains("['anyio==3.7.1']"));

    Ok(())
}

/// Install with bytecode compilation.
#[test]
fn compile() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("MarkupSafe==2.1.3")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--compile")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
    Bytecode compiled 2 files in [TIME]
     + markupsafe==2.1.3
    "
    );

    ChildPath::new(context.site_packages())
        .child("markupsafe/__pycache__/__init__.cpython-312.pyc")
        .assert(predicates::path::is_file());

    context.assert_command("import markupsafe").success();

    Ok(())
}

/// Re-install with bytecode compilation.
#[test]
fn recompile() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("MarkupSafe==2.1.3")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + markupsafe==2.1.3
    "
    );

    let bytecode = ChildPath::new(context.site_packages())
        .child("markupsafe/__pycache__/__init__.cpython-312.pyc");
    bytecode.assert(predicates::path::missing());

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--compile")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Bytecode compiled 2 files in [TIME]
    "
    );

    bytecode.assert(predicates::path::is_file());

    context.assert_command("import markupsafe").success();

    Ok(())
}

/// Raise an error when an editable's `Requires-Python` constraint is not met.
#[test]
fn requires_python_editable() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Create an editable package with a `Requires-Python` constraint that is not met.
    let editable_dir = context.temp_dir.child("editable");
    editable_dir.create_dir_all()?;
    let pyproject_toml = editable_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"[project]
name = "example"
version = "0.0.0"
dependencies = [
  "anyio==4.0.0"
]
requires-python = ">=3.13"
"#,
    )?;

    // Write to a requirements file.
    let requirements_in = context.temp_dir.child("requirements.in");
    requirements_in.write_str(&format!("-e {}", editable_dir.path().display()))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.in"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because the current Python version (3.12.[X]) does not satisfy Python>=3.13 and example==0.0.0 depends on Python>=3.13, we can conclude that example==0.0.0 cannot be used.
             And because only example==0.0.0 is available and you require example, we can conclude that your requirements are unsatisfiable.
    "
    );

    Ok(())
}

/// Raise an error when a direct URL dependency's `Requires-Python` constraint is not met.
#[test]
fn requires_python_direct_url() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Create an editable package with a `Requires-Python` constraint that is not met.
    let editable_dir = context.temp_dir.child("editable");
    editable_dir.create_dir_all()?;
    let pyproject_toml = editable_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"[project]
name = "example"
version = "0.0.0"
dependencies = [
  "anyio==4.0.0"
]
requires-python = ">=3.13"
"#,
    )?;

    // Write to a requirements file.
    let requirements_in = context.temp_dir.child("requirements.in");
    requirements_in.write_str(&format!("example @ {}", editable_dir.path().display()))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.in"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because the current Python version (3.12.[X]) does not satisfy Python>=3.13 and example==0.0.0 depends on Python>=3.13, we can conclude that example==0.0.0 cannot be used.
             And because only example==0.0.0 is available and you require example, we can conclude that your requirements are unsatisfiable.
    "
    );

    Ok(())
}

/// Use an unknown hash algorithm with `--require-hashes`.
#[test]
fn require_hashes_unknown_algorithm() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(
        "anyio==4.0.0 --hash=foo:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f",
    )?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Unsupported hash algorithm (expected one of: `md5`, `sha256`, `sha384`, `sha512`, or `blake2b`) on: `foo`
    "
    );

    Ok(())
}

/// Omit the hash with `--require-hashes`.
#[test]
fn require_hashes_missing_hash() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("anyio==4.0.0")?;

    // Install without error when `--require-hashes` is omitted.
    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.0.0
    "
    );

    // Error when `--require-hashes` is provided.
    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have a hash, but none were provided for: anyio==4.0.0
    "
    );

    Ok(())
}

/// Enable `--require-hashes` from the `requirements.txt`.
#[test]
fn require_hashes_in_requirements_txt() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {r"
        --require-hashes
        anyio
    "})?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have their versions pinned with `==`, but found: anyio
    "
    );

    requirements_txt.write_str(indoc! {r"
        --require-hashes
        iniconfig==2.0.0
    "})?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--no-require-hashes"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have a hash, but none were provided for: iniconfig==2.0.0
    "
    );

    Ok(())
}

/// Omit the version with `--require-hashes`.
#[test]
fn require_hashes_missing_version() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(
        "anyio --hash=sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f",
    )?;

    // Install without error when `--require-hashes` is omitted.
    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.3.0
    "
    );

    // Error when `--require-hashes` is provided.
    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have their versions pinned with `==`, but found: anyio
    "
    );

    Ok(())
}

/// Use a non-`==` operator with `--require-hashes`.
#[test]
fn require_hashes_invalid_operator() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(
        "anyio>4.0.0 --hash=sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f",
    )?;

    // Install without error when `--require-hashes` is omitted.
    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.3.0
    "
    );

    // Error when `--require-hashes` is provided.
    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have their versions pinned with `==`, but found: anyio>4.0.0
    "
    );

    Ok(())
}

/// Include the hash for _just_ the wheel with `--no-binary`.
#[test]
fn require_hashes_wheel_no_binary() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_http_retries();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("anyio==4.0.0 --hash=sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--no-binary")
        .arg(":all:")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download and build `anyio==4.0.0`
      cause: Hash mismatch for `anyio==4.0.0`

             Expected:
               sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f

             Computed:
               sha256:f7ed51751b2c2add651e5747c891b47e26d2a21be5d32d9311dfe9692f3e5d7a
    "
    );

    Ok(())
}

/// Include the hash for _just_ the wheel with `--only-binary`.
#[test]
fn require_hashes_wheel_only_binary() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("anyio==4.0.0 --hash=sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--only-binary")
        .arg(":all:")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.0.0
    "
    );

    Ok(())
}

/// Include the hash for _just_ the source distribution with `--no-binary`.
#[test]
fn require_hashes_source_no_binary() -> Result<()> {
    let server = PackseServer::new("simple/single-package.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(
        "a==1.0.0 --hash=sha256:957f99ff1d65ce0d7883d50f4e67ed8d4b42e76d2c2b5e62384ff0ba538647b5",
    )?;

    uv_snapshot!(context.pip_sync()
        .arg("--index-url").arg(server.index_url())
        .arg("requirements.txt")
        .arg("--no-binary")
        .arg("a")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + a==1.0.0
    "
    );

    Ok(())
}

/// Include the hash for _just_ the source distribution, with `--binary-only`.
#[test]
fn require_hashes_source_only_binary() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_http_retries();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("anyio==4.0.0 --hash=sha256:f7ed51751b2c2add651e5747c891b47e26d2a21be5d32d9311dfe9692f3e5d7a")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--only-binary")
        .arg(":all:")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `anyio==4.0.0`
      cause: Hash mismatch for `anyio==4.0.0`

             Expected:
               sha256:f7ed51751b2c2add651e5747c891b47e26d2a21be5d32d9311dfe9692f3e5d7a

             Computed:
               sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f
    "
    );

    Ok(())
}

/// Include the correct hash algorithm, but the wrong digest.
#[test]
fn require_hashes_wrong_digest() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_http_retries();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("anyio==4.0.0 --hash=sha256:afdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `anyio==4.0.0`
      cause: Hash mismatch for `anyio==4.0.0`

             Expected:
               sha256:afdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f

             Computed:
               sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f
    "
    );

    Ok(())
}

/// Include the correct hash, but the wrong algorithm.
#[test]
fn require_hashes_wrong_algorithm() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_http_retries();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("anyio==4.0.0 --hash=sha512:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5fcfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `anyio==4.0.0`
      cause: Hash mismatch for `anyio==4.0.0`

             Expected:
               sha512:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5fcfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f

             Computed:
               sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f
               sha512:f30761c1e8725b49c498273b90dba4b05c0fd157811994c806183062cb6647e773364ce45f0e1ff0b10e32fe6d0232ea5ad39476ccf37109d6b49603a09c11c2
    "
    );

    Ok(())
}

/// Include the hash for a source distribution specified as a direct URL dependency.
#[test]
fn require_hashes_source_url() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_http_retries();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("source-distribution @ https://files.pythonhosted.org/packages/10/1f/57aa4cce1b1abf6b433106676e15f9fa2c92ed2bd4cf77c3b50a9e9ac773/source_distribution-0.0.1.tar.gz --hash=sha256:1f83ed7498336c7f2ab9b002cf22583d91115ebc624053dc4eb3a45694490106")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.1 (from https://files.pythonhosted.org/packages/10/1f/57aa4cce1b1abf6b433106676e15f9fa2c92ed2bd4cf77c3b50a9e9ac773/source_distribution-0.0.1.tar.gz)
    "
    );

    // Reinstall with the right hash, and verify that it's reused.
    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ source-distribution==0.0.1 (from https://files.pythonhosted.org/packages/10/1f/57aa4cce1b1abf6b433106676e15f9fa2c92ed2bd4cf77c3b50a9e9ac773/source_distribution-0.0.1.tar.gz)
    "
    );

    // Reinstall with the wrong hash, and verify that it's rejected despite being cached.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("source-distribution @ https://files.pythonhosted.org/packages/10/1f/57aa4cce1b1abf6b433106676e15f9fa2c92ed2bd4cf77c3b50a9e9ac773/source_distribution-0.0.1.tar.gz --hash=sha256:a7ed51751b2c2add651e5747c891b47e26d2a21be5d32d9311dfe9692f3e5d7a")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download and build `source-distribution @ https://files.pythonhosted.org/packages/10/1f/57aa4cce1b1abf6b433106676e15f9fa2c92ed2bd4cf77c3b50a9e9ac773/source_distribution-0.0.1.tar.gz`
      cause: Hash mismatch for `source-distribution @ https://files.pythonhosted.org/packages/10/1f/57aa4cce1b1abf6b433106676e15f9fa2c92ed2bd4cf77c3b50a9e9ac773/source_distribution-0.0.1.tar.gz`

             Expected:
               sha256:a7ed51751b2c2add651e5747c891b47e26d2a21be5d32d9311dfe9692f3e5d7a

             Computed:
               sha256:1f83ed7498336c7f2ab9b002cf22583d91115ebc624053dc4eb3a45694490106
    "
    );

    Ok(())
}

/// Include the _wrong_ hash for a source distribution specified as a direct URL dependency.
#[test]
fn require_hashes_source_url_mismatch() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_http_retries();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("source-distribution @ https://files.pythonhosted.org/packages/10/1f/57aa4cce1b1abf6b433106676e15f9fa2c92ed2bd4cf77c3b50a9e9ac773/source_distribution-0.0.1.tar.gz --hash=sha256:a7ed51751b2c2add651e5747c891b47e26d2a21be5d32d9311dfe9692f3e5d7a")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download and build `source-distribution @ https://files.pythonhosted.org/packages/10/1f/57aa4cce1b1abf6b433106676e15f9fa2c92ed2bd4cf77c3b50a9e9ac773/source_distribution-0.0.1.tar.gz`
      cause: Hash mismatch for `source-distribution @ https://files.pythonhosted.org/packages/10/1f/57aa4cce1b1abf6b433106676e15f9fa2c92ed2bd4cf77c3b50a9e9ac773/source_distribution-0.0.1.tar.gz`

             Expected:
               sha256:a7ed51751b2c2add651e5747c891b47e26d2a21be5d32d9311dfe9692f3e5d7a

             Computed:
               sha256:1f83ed7498336c7f2ab9b002cf22583d91115ebc624053dc4eb3a45694490106
    "
    );

    Ok(())
}

/// Include the hash for a built distribution specified as a direct URL dependency.
#[test]
fn require_hashes_wheel_url() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_http_retries();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl --hash=sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.0.0 (from https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl)
    "
    );

    // Reinstall with the right hash, and verify that it's reused.
    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ anyio==4.0.0 (from https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl)
    "
    );

    // Reinstall with the wrong hash, and verify that it's rejected despite being cached.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl --hash=sha256:afdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl`
      cause: Hash mismatch for `anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl`

             Expected:
               sha256:afdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f

             Computed:
               sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f
    "
    );

    // Sync a new dependency and include the wrong hash for anyio. Verify that we reuse anyio
    // despite the wrong hash, like pip, since we don't validate hashes for already-installed
    // distributions.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {"
        anyio==4.0.0 --hash=sha256:afdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f
        iniconfig==2.0.0 --hash=sha256:b6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374
    "})?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    "
    );

    Ok(())
}

fn assert_archive_bucket_missing(context: &uv_test::TestContext) {
    let cache = uv_cache::Cache::from_path(context.cache_dir.path());
    uv_test::assert_path_missing(cache.bucket(uv_cache::CacheBucket::Archive));
}

/// Include the _wrong_ hash for a built distribution specified as a direct URL dependency.
#[test]
fn require_hashes_wheel_url_mismatch() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_http_retries();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl --hash=sha256:afdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl`
      cause: Hash mismatch for `anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl`

             Expected:
               sha256:afdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f

             Computed:
               sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f
    "
    );

    // A rejected wheel must not be published to the archive cache.
    assert_archive_bucket_missing(&context);

    Ok(())
}

/// Reject Git dependencies when `--require-hashes` is provided.
#[test]
#[cfg(feature = "test-git")]
fn require_hashes_git() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("anyio @ git+https://github.com/agronholm/anyio@4a23745badf5bf5ef7928f1e346e9986bd696d82 --hash=sha256:f7ed51751b2c2add651e5747c891b47e26d2a21be5d32d9311dfe9692f3e5d7a")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download and build `anyio @ git+https://github.com/agronholm/anyio@4a23745badf5bf5ef7928f1e346e9986bd696d82`
      cause: Hash-checking is not supported for Git repositories: anyio @ git+https://github.com/agronholm/anyio@4a23745badf5bf5ef7928f1e346e9986bd696d82
    "
    );

    Ok(())
}

/// Reject local directory dependencies when `--require-hashes` is provided.
#[test]
fn require_hashes_source_tree() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "black @ {} --hash=sha256:f7ed51751b2c2add651e5747c891b47e26d2a21be5d32d9311dfe9692f3e5d7a",
        context
            .workspace_root
            .join("test/packages/black_editable")
            .display()
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to build `black @ file://[WORKSPACE]/test/packages/black_editable`
      cause: Hash-checking is not supported for local directories: black @ file://[WORKSPACE]/test/packages/black_editable
    "
    );

    Ok(())
}

/// Include the hash for _just_ the wheel with `--only-binary`.
#[test]
fn require_hashes_re_download() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_http_retries();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("anyio==4.0.0")?;

    // Install without `--require-hashes`.
    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.0.0
    "
    );

    // Reinstall with `--require-hashes`, and the wrong hash.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("anyio==4.0.0 --hash=sha256:afdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `anyio==4.0.0`
      cause: Hash mismatch for `anyio==4.0.0`

             Expected:
               sha256:afdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f

             Computed:
               sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f
    "
    );

    // Reinstall with `--require-hashes`, and the right hash.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("anyio==4.0.0 --hash=sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ anyio==4.0.0
    "
    );

    Ok(())
}

/// Include the hash for a built distribution specified as a local path dependency.
#[test]
fn require_hashes_wheel_path() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "tqdm @ {} --hash=sha256:a34996d4bd5abb2336e14ff0a2d22b92cfd0f0ed344e6883041ce01953276a13",
        context
            .workspace_root
            .join("test/links/tqdm-1000.0.0-py3-none-any.whl")
            .display()
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==1000.0.0 (from file://[WORKSPACE]/test/links/tqdm-1000.0.0-py3-none-any.whl)
    "
    );

    Ok(())
}

/// Include a `BLAKE2b` hash for a built distribution specified as a local path dependency.
#[test]
fn require_hashes_wheel_path_blake2b() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "tqdm @ {} --hash=blake2b:fd611597f5e771ac942d300426f16a38f1579ab572bf4bca968a53709db0a292",
        context
            .workspace_root
            .join("test/links/tqdm-1000.0.0-py3-none-any.whl")
            .display()
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==1000.0.0 (from file://[WORKSPACE]/test/links/tqdm-1000.0.0-py3-none-any.whl)
    "
    );

    Ok(())
}

/// Include the wrong `BLAKE2b` hash for a built distribution specified as a local path dependency.
#[test]
fn require_hashes_wheel_path_blake2b_mismatch() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "tqdm @ {} --hash=blake2b:ad611597f5e771ac942d300426f16a38f1579ab572bf4bca968a53709db0a292",
        context
            .workspace_root
            .join("test/links/tqdm-1000.0.0-py3-none-any.whl")
            .display()
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to read `tqdm @ file://[WORKSPACE]/test/links/tqdm-1000.0.0-py3-none-any.whl`
      cause: Hash mismatch for `tqdm @ file://[WORKSPACE]/test/links/tqdm-1000.0.0-py3-none-any.whl`

             Expected:
               blake2b:ad611597f5e771ac942d300426f16a38f1579ab572bf4bca968a53709db0a292

             Computed:
               blake2b:fd611597f5e771ac942d300426f16a38f1579ab572bf4bca968a53709db0a292
    "
    );

    Ok(())
}

/// Include the _wrong_ hash for a built distribution specified as a local path dependency.
#[test]
fn require_hashes_wheel_path_mismatch() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "tqdm @ {} --hash=sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f",
        context
            .workspace_root
            .join("test/links/tqdm-1000.0.0-py3-none-any.whl")
            .display()
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to read `tqdm @ file://[WORKSPACE]/test/links/tqdm-1000.0.0-py3-none-any.whl`
      cause: Hash mismatch for `tqdm @ file://[WORKSPACE]/test/links/tqdm-1000.0.0-py3-none-any.whl`

             Expected:
               sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f

             Computed:
               sha256:a34996d4bd5abb2336e14ff0a2d22b92cfd0f0ed344e6883041ce01953276a13
    "
    );

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--no-require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to read `tqdm @ file://[WORKSPACE]/test/links/tqdm-1000.0.0-py3-none-any.whl`
      cause: Hash mismatch for `tqdm @ file://[WORKSPACE]/test/links/tqdm-1000.0.0-py3-none-any.whl`

             Expected:
               sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f

             Computed:
               sha256:a34996d4bd5abb2336e14ff0a2d22b92cfd0f0ed344e6883041ce01953276a13
    "
    );

    // A rejected wheel must not be published to the archive cache.
    assert_archive_bucket_missing(&context);

    Ok(())
}

/// Include the hash for a source distribution specified as a local path dependency.
#[test]
fn require_hashes_source_path() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "tqdm @ {} --hash=sha256:89fa05cffa7f457658373b85de302d24d0c205ceda2819a8739e324b75e9430b",
        context
            .workspace_root
            .join("test/links/tqdm-999.0.0.tar.gz")
            .display()
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==999.0.0 (from file://[WORKSPACE]/test/links/tqdm-999.0.0.tar.gz)
    "
    );

    Ok(())
}

/// Include the _wrong_ hash for a source distribution specified as a local path dependency.
#[test]
fn require_hashes_source_path_mismatch() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "tqdm @ {} --hash=sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f",
        context
            .workspace_root
            .join("test/links/tqdm-999.0.0.tar.gz")
            .display()
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to build `tqdm @ file://[WORKSPACE]/test/links/tqdm-999.0.0.tar.gz`
      cause: Hash mismatch for `tqdm @ file://[WORKSPACE]/test/links/tqdm-999.0.0.tar.gz`

             Expected:
               sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f

             Computed:
               sha256:89fa05cffa7f457658373b85de302d24d0c205ceda2819a8739e324b75e9430b
    "
    );

    Ok(())
}

/// We allow `--require-hashes` for direct URL dependencies.
#[test]
fn require_hashes_unnamed() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(indoc::indoc! {r"
            https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl --hash=sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f
        "} )?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.0.0 (from https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl)
    "
    );

    Ok(())
}

/// We disallow `--require-hashes` for editables.
#[test]
fn require_hashes_editable() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&indoc::formatdoc! {r"
        -e file://{workspace_root}/test/packages/black_editable[d]
        ",
        workspace_root = context.workspace_root.simplified_display(),
    })?;

    // Install the editable packages.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg(requirements_txt.path())
        .arg("--require-hashes"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have a hash, but none were provided for: file://[WORKSPACE]/test/packages/black_editable[d]
    "
    );

    Ok(())
}

/// If a dependency is repeated, the hash should be required for both instances.
#[test]
fn require_hashes_repeated_dependency() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {"
        anyio==4.0.0 --hash=sha256:f7ed51751b2c2add651e5747c891b47e26d2a21be5d32d9311dfe9692f3e5d7a
        anyio
    "})?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have their versions pinned with `==`, but found: anyio
    "
    );

    // Reverse the order.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {"
        anyio
        anyio==4.0.0 --hash=sha256:f7ed51751b2c2add651e5747c891b47e26d2a21be5d32d9311dfe9692f3e5d7a
    "})?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have their versions pinned with `==`, but found: anyio
    "
    );

    Ok(())
}

/// Repeated direct URL requirements merge compatible hashes instead of overwriting them.
#[test]
fn require_hashes_repeated_hash() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_http_retries();

    // Use the same hash in both cases.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(indoc::indoc! { r"
            anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl --hash=sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f
            anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl --hash=sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f
    " })?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.0.0 (from https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl)
    "
    );

    // Use a different hash, but both are correct.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(indoc::indoc! { r"
            anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl --hash=sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f
            anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl --hash=sha512:f30761c1e8725b49c498273b90dba4b05c0fd157811994c806183062cb6647e773364ce45f0e1ff0b10e32fe6d0232ea5ad39476ccf37109d6b49603a09c11c2
    " })?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes")
        .arg("--reinstall"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ anyio==4.0.0 (from https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl)
    "
    );

    // Use a different hash. The `sha512` is wrong, so validation should fail even though the
    // `sha256` is still correct.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(indoc::indoc! { r"
            anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl --hash=sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f
            anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl --hash=sha512:e30761c1e8725b49c498273b90dba4b05c0fd157811994c806183062cb6647e773364ce45f0e1ff0b10e32fe6d0232ea5ad39476ccf37109d6b49603a09c11c2
    " })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes")
        .arg("--reinstall"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl`
      cause: Hash mismatch for `anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl`

             Expected:
               sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f
               sha512:e30761c1e8725b49c498273b90dba4b05c0fd157811994c806183062cb6647e773364ce45f0e1ff0b10e32fe6d0232ea5ad39476ccf37109d6b49603a09c11c2

             Computed:
               sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f
               sha512:f30761c1e8725b49c498273b90dba4b05c0fd157811994c806183062cb6647e773364ce45f0e1ff0b10e32fe6d0232ea5ad39476ccf37109d6b49603a09c11c2
    "
    );

    // Use different hashes, but both are wrong. This should fail because none of the merged
    // hashes match.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(indoc::indoc! { r"
            anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl --hash=sha256:f7ed51751b2c2add651e5747c891b47e26d2a21be5d32d9311dfe9692f3e5d7a
            anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl --hash=sha512:e30761c1e8725b49c498273b90dba4b05c0fd157811994c806183062cb6647e773364ce45f0e1ff0b10e32fe6d0232ea5ad39476ccf37109d6b49603a09c11c2
    " })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes")
        .arg("--reinstall"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl`
      cause: Hash mismatch for `anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl`

             Expected:
               sha256:f7ed51751b2c2add651e5747c891b47e26d2a21be5d32d9311dfe9692f3e5d7a
               sha512:e30761c1e8725b49c498273b90dba4b05c0fd157811994c806183062cb6647e773364ce45f0e1ff0b10e32fe6d0232ea5ad39476ccf37109d6b49603a09c11c2

             Computed:
               sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f
               sha512:f30761c1e8725b49c498273b90dba4b05c0fd157811994c806183062cb6647e773364ce45f0e1ff0b10e32fe6d0232ea5ad39476ccf37109d6b49603a09c11c2
    "
    );

    Ok(())
}

/// Repeated direct URL requirements merge hashes across nested requirements files.
#[test]
fn require_hashes_repeated_hash_multiple_files() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_a = context.temp_dir.child("requirements-a.txt");
    requirements_a.write_str(indoc::indoc! { r"
        anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl --hash=sha256:cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f
    " })?;

    let requirements_b = context.temp_dir.child("requirements-b.txt");
    requirements_b.write_str(indoc::indoc! { r"
        anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl --hash=sha512:f30761c1e8725b49c498273b90dba4b05c0fd157811994c806183062cb6647e773364ce45f0e1ff0b10e32fe6d0232ea5ad39476ccf37109d6b49603a09c11c2
    " })?;

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc::indoc! { r"
        -r requirements-a.txt
        -r requirements-b.txt
    " })?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.0.0 (from https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl)
    "
    );

    Ok(())
}

/// If a dependency is repeated, the hash should be required for both instances.
#[test]
fn require_hashes_at_least_one() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // An MD5 digest alone must not satisfy integrity-enforced installs.
    let md5_requirements_txt = context.temp_dir.child("requirements-md5.txt");
    md5_requirements_txt.write_str("anyio==4.0.0 --hash=md5:420d85e19168705cdf0223621b18831a")?;

    uv_snapshot!(context.pip_sync()
        .arg(md5_requirements_txt.path())
        .arg("--require-hashes"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `md5` hashes are insecure and cannot be used with `--require-hashes` but no other hashes are available for: anyio==4.0.0
    ");

    // Request `anyio` with an uppercase `sha256` hash.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("anyio==4.0.0 --hash=sha256:F7ED51751B2C2ADD651E5747C891B47E26D2A21BE5D32D9311DFE9692F3E5D7A")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.0.0
    "
    );

    // An MD5 requirement can still use a secure hash supplied by its constraint.
    let constraints_txt = context.temp_dir.child("constraints.txt");
    constraints_txt
        .write_str("anyio==4.0.0 --hash=sha256:f7ed51751b2c2add651e5747c891b47e26d2a21be5d32d9311dfe9692f3e5d7a")?;

    uv_snapshot!(context.pip_sync()
        .arg(md5_requirements_txt.path())
        .arg("--constraint")
        .arg(constraints_txt.path())
        .arg("--reinstall")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ anyio==4.0.0
    "
    );

    // Reinstall, requesting both `sha256` and `sha512`. We should reinstall from the cache, since
    // at least one hash matches.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("anyio==4.0.0 --hash=sha256:f7ed51751b2c2add651e5747c891b47e26d2a21be5d32d9311dfe9692f3e5d7a --hash=md5:420d85e19168705cdf0223621b18831a")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ anyio==4.0.0
    "
    );

    // This should be true even if the second hash is wrong.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("anyio==4.0.0 --hash=sha256:f7ed51751b2c2add651e5747c891b47e26d2a21be5d32d9311dfe9692f3e5d7a --hash=md5:12341234123412341234123412341234")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ anyio==4.0.0
    "
    );

    // MD5 remains supported when hash checking is not required.
    uv_snapshot!(context.pip_sync()
        .arg(md5_requirements_txt.path())
        .arg("--reinstall"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ anyio==4.0.0
    ");

    Ok(())
}

/// Using `--find-links`, but the registry doesn't provide us with a hash.
#[test]
fn require_hashes_find_links_no_hash() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_http_retries();
    let links = context.temp_dir.child("links");
    links.create_dir_all()?;
    let wheel = "basic_package-0.1.0-py3-none-any.whl";
    fs::copy(
        context.workspace_root.join("test/links").join(wheel),
        links.child(wheel),
    )?;
    let source_built = context.temp_dir.child("source-built");
    let source = generate_source_archive(
        &"basic-package".parse()?,
        &"0.1.0".parse()?,
        "",
        Some(source_built.path()),
    )?;
    let source_hash = hex::encode(Sha256::digest(&source));
    links
        .child("basic_package-0.1.0.tar.gz")
        .write_binary(&source)?;
    let server = FindLinksServer::new(links.path());
    let context = context.with_filter((source_hash.clone(), "[SOURCE_SHA256]"));

    // First, use the correct hash.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(
        "basic-package==0.1.0 --hash=sha256:7b6229db79b5800e4e98a351b5628c1c8a944533a2d428aeeaa7275a30d4ea82",
    )?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes")
        .arg("--no-index")
        .arg("--find-links")
        .arg(server.url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + basic-package==0.1.0
    "
    );

    // Second, use an incorrect hash.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(
        "basic-package==0.1.0 --hash=sha256:0000000000000000000000000000000000000000000000000000000000000000",
    )?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes")
        .arg("--no-index")
        .arg("--find-links")
        .arg(server.url()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `basic-package==0.1.0`
      cause: Hash mismatch for `basic-package==0.1.0`

             Expected:
               sha256:0000000000000000000000000000000000000000000000000000000000000000

             Computed:
               sha256:7b6229db79b5800e4e98a351b5628c1c8a944533a2d428aeeaa7275a30d4ea82
    "
    );

    // Third, use the hash from the source distribution. This will actually fail, when it _could_
    // succeed, but pip has the same behavior.
    requirements_txt.write_str(&format!("basic-package==0.1.0 --hash=sha256:{source_hash}"))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes")
        .arg("--no-index")
        .arg("--find-links")
        .arg(server.url()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `basic-package==0.1.0`
      cause: Hash mismatch for `basic-package==0.1.0`

             Expected:
               sha256:[SOURCE_SHA256]

             Computed:
               sha256:7b6229db79b5800e4e98a351b5628c1c8a944533a2d428aeeaa7275a30d4ea82
    "
    );

    // Disabling wheels makes the same source hash usable.
    source_built.assert(predicates::path::missing());

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--no-binary")
        .arg(":all:")
        .arg("--reinstall")
        .arg("--require-hashes")
        .arg("--no-index")
        .arg("--find-links")
        .arg(server.url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ basic-package==0.1.0
    "
    );
    source_built.assert(predicates::path::exists());

    Ok(())
}

/// A flat index advertises the same hash as the requirement and the downloaded wheel.
#[tokio::test]
async fn require_hashes_find_links_valid_hash() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let fixture = RegistryHashFixture::new(&context);
    let requirement = fixture.hashed_requirement();
    let hash = fixture.sha256.clone();
    let server = fixture.serve(Some(&hash)).await;

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&requirement)?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes")
        .arg("--no-index")
        .arg("--find-links")
        .arg(format!("{}{}", server.uri(), RegistryHashFixture::PAGE_PATH)), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==1000.0.0
    "
    );

    Ok(())
}

/// A wheel-backed HTML index with a configurable SHA-256 fragment.
struct RegistryHashFixture {
    wheel: Vec<u8>,
    sha256: String,
}

impl RegistryHashFixture {
    const FILENAME: &str = "tqdm-1000.0.0-py3-none-any.whl";
    const PAGE_PATH: &str = "/simple/tqdm/";
    const REQUIREMENT: &str = "tqdm==1000.0.0";

    fn new(context: &uv_test::TestContext) -> Self {
        let wheel = context.read_bytes(
            context
                .workspace_root
                .join("test/links")
                .join(Self::FILENAME),
        );
        let sha256 = hex::encode(Sha256::digest(&wheel));
        Self { wheel, sha256 }
    }

    fn hashed_requirement(&self) -> String {
        format!("{} --hash=sha256:{}", Self::REQUIREMENT, self.sha256)
    }

    async fn serve(self, advertised_hash: Option<&str>) -> MockServer {
        let server = MockServer::start().await;
        let fragment = advertised_hash.map_or_else(String::new, |hash| format!("#sha256={hash}"));
        let filename = Self::FILENAME;
        Mock::given(method("GET"))
            .and(path(Self::PAGE_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                format!(r#"<a href="/{filename}{fragment}">{filename}</a>"#),
                "text/html",
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/{filename}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(self.wheel))
            .mount(&server)
            .await;
        server
    }
}

/// Using `--index-url`, but the registry doesn't provide us with a hash.
#[tokio::test]
async fn require_hashes_registry_no_hash() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let fixture = RegistryHashFixture::new(&context);
    let requirement = fixture.hashed_requirement();
    let server = fixture.serve(None).await;

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&requirement)?;

    uv_snapshot!(context.pip_sync()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("requirements.txt")
        .arg("--require-hashes")
        .arg("--index-url")
        .arg(format!("{}/simple/", server.uri())), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==1000.0.0
    "
    );

    Ok(())
}

/// Using `--index-url`, and the registry serves us a correct hash.
#[tokio::test]
async fn require_hashes_registry_valid_hash() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let fixture = RegistryHashFixture::new(&context);
    let requirement = fixture.hashed_requirement();
    let hash = fixture.sha256.clone();
    let server = fixture.serve(Some(&hash)).await;

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&requirement)?;

    uv_snapshot!(context.pip_sync()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("requirements.txt")
        .arg("--require-hashes")
        .arg("--index-url")
        .arg(format!("{}/simple/", server.uri())), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==1000.0.0
    "
    );

    Ok(())
}

/// Verify the index's wheel hash even when no explicit hash policy is requested.
#[tokio::test]
async fn index_wheel_hash_mismatch() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_http_retries();
    let fixture = RegistryHashFixture::new(&context);
    let computed_hash = fixture.sha256.clone();
    let index_hash = "0".repeat(64);
    assert_ne!(
        index_hash, computed_hash,
        "the index must advertise a wrong hash"
    );
    let context = context
        .with_filter((computed_hash, "[WHEEL_SHA256]"))
        .with_filter((index_hash.clone(), "[INDEX_SHA256]"));

    let server = fixture.serve(Some(&index_hash)).await;

    context
        .temp_dir
        .child("requirements.txt")
        .write_str(RegistryHashFixture::REQUIREMENT)?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("requirements.txt")
        .arg("--only-binary=:all:")
        .arg("--index-url")
        .arg(format!("{}/simple/", server.uri())), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `tqdm==1000.0.0`
      cause: Hash mismatch for `tqdm==1000.0.0`

             Expected:
               sha256:[INDEX_SHA256]

             Computed:
               sha256:[WHEEL_SHA256]
    ");

    assert_archive_bucket_missing(&context);
    Ok(())
}

/// Include the hash in the URL directly.
#[test]
fn require_hashes_url() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_exclude_newer("2025-01-29T00:00:00Z");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("iniconfig @ https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl#sha256=b6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0 (from https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl#sha256=b6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374)
    "
    );

    Ok(())
}

/// Include an irrelevant fragment in the URL.
#[test]
fn require_hashes_url_other_fragment() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_exclude_newer("2025-01-29T00:00:00Z");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("iniconfig @ https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl#foo=bar")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have a hash, but none were provided for: iniconfig @ https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl#foo=bar
    "
    );

    Ok(())
}

/// Include an invalid hash in the URL directly.
#[test]
fn require_hashes_url_invalid() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_exclude_newer("2025-01-29T00:00:00Z")
        .with_filtered_http_retries();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("iniconfig @ https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl#sha256=c6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `iniconfig @ https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl#sha256=c6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374`
      cause: Hash mismatch for `iniconfig @ https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl#sha256=c6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374`

             Expected:
               sha256:c6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374

             Computed:
               sha256:b6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374
    "
    );

    Ok(())
}

/// Reject malformed URL hashes without downloading the distribution.
#[test]
fn require_hashes_url_malformed() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(
        "iniconfig @ https://files.pythonhosted.org/packages/iniconfig-2.0.0-py3-none-any.whl#subdirectory=src&sha256=123",
    )?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes")
        .arg("--offline"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Invalid hash digest length (expected 64 hexadecimal characters, found 3)
    ");

    Ok(())
}

/// Merge the hash on the fragment with hashes provided directly.
#[test]
fn require_hashes_url_merge() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_exclude_newer("2025-01-29T00:00:00Z");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("anyio @ https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl#sha256=cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f --hash sha512:f30761c1e8725b49c498273b90dba4b05c0fd157811994c806183062cb6647e773364ce45f0e1ff0b10e32fe6d0232ea5ad39476ccf37109d6b49603a09c11c2")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.0.0 (from https://files.pythonhosted.org/packages/36/55/ad4de788d84a630656ece71059665e01ca793c04294c463fd84132f40fe6/anyio-4.0.0-py3-none-any.whl#sha256=cfdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f)
    "
    );

    Ok(())
}

/// Include the hash in the URL directly.
#[test]
fn require_hashes_url_unnamed() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_exclude_newer("2025-01-29T00:00:00Z");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl#sha256=b6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0 (from https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl#sha256=b6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374)
    "
    );

    Ok(())
}

/// Sync to a `--target` directory with a built distribution.
#[test]
fn target_built_distribution() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin()
        .with_filtered_exe_suffix();

    // Install `iniconfig` to the target directory.
    let requirements_in = context.temp_dir.child("requirements.in");
    requirements_in.write_str("iniconfig==2.0.0")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.in")
        .arg("--target")
        .arg("target"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: .venv/[BIN]/[PYTHON]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    let target = context.temp_dir.child("target");
    target.child("iniconfig").assert(predicates::path::is_dir());

    context.assert_not_installed("iniconfig");

    // Ensure that we can import the package by augmenting the `PYTHONPATH`.
    context
        .python_command()
        .arg("-c")
        .arg("import iniconfig")
        .env(EnvVars::PYTHONPATH, target.path())
        .assert()
        .success();

    // Replace it with an older version.
    requirements_in.write_str("iniconfig==1.1.1")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.in")
        .arg("--target")
        .arg("target"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: .venv/[BIN]/[PYTHON]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - iniconfig==2.0.0
     + iniconfig==1.1.1
    ");
    target
        .child("iniconfig-2.0.0.dist-info")
        .assert(predicates::path::missing());
    target
        .child("iniconfig-1.1.1.dist-info")
        .assert(predicates::path::is_dir());

    // Remove it, and replace with `flask`, which includes a binary.
    requirements_in.write_str("flask")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.in")
        .arg("--target")
        .arg("target"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: .venv/[BIN]/[PYTHON]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     + flask==3.0.2
     - iniconfig==1.1.1
    ");
    target
        .child("iniconfig")
        .assert(predicates::path::missing());
    target
        .child("iniconfig-1.1.1.dist-info")
        .assert(predicates::path::missing());
    target
        .child("bin")
        .child(format!("flask{EXE_SUFFIX}"))
        .assert(predicates::path::is_file());

    Ok(())
}

/// Sync to a `--target` directory with a package that requires building from source.
#[test]
fn target_source_distribution() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin()
        .with_filtered_exe_suffix();

    // Install `iniconfig` to the target directory.
    let requirements_in = context.temp_dir.child("requirements.in");
    requirements_in.write_str("iniconfig==2.0.0")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.in")
        .arg("--no-binary")
        .arg("iniconfig")
        .arg("--target")
        .arg("target"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: .venv/[BIN]/[PYTHON]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    let target = context.temp_dir.child("target");
    assert_target_distributions(&context, target.path(), "iniconfig==2.0.0\n");
    target
        .child("hatchling")
        .assert(predicates::path::missing());
    context.assert_not_installed("iniconfig");

    // Ensure that we can import the package by augmenting the `PYTHONPATH`.
    context
        .python_command()
        .arg("-c")
        .arg("import iniconfig")
        .env(EnvVars::PYTHONPATH, target.path())
        .assert()
        .success();

    Ok(())
}

/// Sync to a `--target` directory with a package that requires building from source, along with
/// `--no-build-isolation`.
#[test]
fn target_no_build_isolation() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin()
        .with_filtered_exe_suffix();

    // Install `flit_core` into the current environment.
    let requirements_in = context.temp_dir.child("requirements.in");
    requirements_in.write_str("flit_core")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.in"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + flit-core==3.9.0
    ");

    // Install `wheel` to the target directory.
    let requirements_in = context.temp_dir.child("requirements.in");
    requirements_in.write_str("wheel")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.in")
        .arg("--no-build-isolation")
        .arg("--no-binary")
        .arg("wheel")
        .arg("--target")
        .arg("target"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: .venv/[BIN]/[PYTHON]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + wheel==0.43.0
    ");

    let target = context.temp_dir.child("target");
    assert_target_distributions(&context, target.path(), "wheel==0.43.0\n");
    target
        .child("flit_core")
        .assert(predicates::path::missing());
    context.assert_not_installed("wheel");
    context
        .assert_command(
            "from importlib.metadata import version; print(version('flit-core'), end='')",
        )
        .success()
        .stdout("3.9.0");

    // Ensure that we can import the package by augmenting the `PYTHONPATH`.
    context
        .python_command()
        .arg("-c")
        .arg("import wheel")
        .env(EnvVars::PYTHONPATH, target.path())
        .assert()
        .success();

    Ok(())
}

fn assert_target_distributions(context: &TestContext, target: &Path, expected: &'static str) {
    context
        .python_command()
        .arg("-c")
        .arg(indoc! {r#"
            from importlib.metadata import distributions
            import sys

            installed = sorted(f'{dist.metadata["Name"]}=={dist.version}' for dist in distributions(path=[sys.argv[1]]))
            print('\n'.join(installed))
        "#})
        .arg(target)
        .assert()
        .success()
        .stdout(expected);
}

/// Include a `build_constraints.txt` file with an incompatible constraint.
#[test]
fn incompatible_build_constraint() -> Result<()> {
    let context = uv_test::test_context!("3.9");
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("requests==1.2")?;

    let constraints_txt = context.temp_dir.child("build_constraints.txt");
    constraints_txt.write_str("setuptools==1")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--build-constraint")
        .arg("build_constraints.txt"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download and build `requests==1.2.0`
      cause: Failed to resolve requirements from `setup.py` build
      cause: No solution found when resolving: `setuptools>=40.8.0`
      cause: Because you require setuptools>=40.8.0 and setuptools==1, we can conclude that your requirements are unsatisfiable.
    "
    );

    Ok(())
}

/// Include a `build_constraints.txt` file with a compatible constraint.
#[test]
fn compatible_build_constraint() -> Result<()> {
    let context = uv_test::test_context!("3.9");
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("requests==1.2")?;

    let constraints_txt = context.temp_dir.child("build_constraints.txt");
    // Verify mode ignores hashes on unpinned constraints and does not activate extras.
    constraints_txt.write_str("setuptools[foo]>=40 --hash=sha256:incorrect")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--verify-hashes")
        .arg("--build-constraint")
        .arg("build_constraints.txt")
        .env(EnvVars::UV_REQUIRE_BUILD_HASHES, "true"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: The `--require-build-hashes` option is experimental and may change without warning. Pass `--preview-features build-dependency-hashes` to disable this warning.
    error: In `--require-hashes` mode, all requirements must have their versions pinned with `==`, but found: setuptools[foo]>=40
    ");

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--verify-hashes")
        .arg("--build-constraint")
        .arg("build_constraints.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + requests==1.2.0
    "
    );

    Ok(())
}

#[test]
fn sync_seed() -> Result<()> {
    let context = uv_test::test_context!("3.9");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("requests==1.2")?;

    // Add `pip` to the environment.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("pip"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + pip==24.0
    "
    );
    context.assert_installed("pip", "24.0");

    // A manually installed `pip` is not a seed package and must be removed.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - pip==24.0
     + requests==1.2.0
    "
    );
    context.assert_not_installed("pip");

    // Re-create the environment with seed packages.
    uv_snapshot!(context.filters(), context.venv().arg("--clear")
        .arg("--seed"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.9.[X] interpreter at: [PYTHON-3.9]
    Creating virtual environment with seed packages at: .venv
     + pip==24.0
     + setuptools==69.2.0
     + wheel==0.43.0
    Activate with: source .venv/[BIN]/activate
    "
    );

    // Packages recorded as seeds when the environment was created must be retained.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + requests==1.2.0
    "
    );
    context
        .assert_command(indoc! {r#"
            from importlib.metadata import version

            for name in ("pip", "setuptools", "wheel"):
                print(f"{name}=={version(name)}")
        "#})
        .success()
        .stdout("pip==24.0\nsetuptools==69.2.0\nwheel==0.43.0\n");

    Ok(())
}

#[test]
fn pep_751() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio"]
        "#,
    )?;

    context
        .export()
        .arg("-o")
        .arg("pylock.toml")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    "
    );

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 3 packages in [TIME]
    "
    );

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig"]
        "#,
    )?;

    context
        .export()
        .arg("-o")
        .arg("pylock.toml")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Uninstalled 3 packages in [TIME]
    Installed 1 package in [TIME]
     - anyio==4.3.0
     - idna==3.6
     + iniconfig==2.0.0
     - sniffio==1.3.1
    "
    );

    Ok(())
}

#[test]
fn pep_751_default_groups() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pylock.toml").write_str(
        r#"
        lock-version = "1.0"
        created-by = "uv"
        default-groups = ["default"]

        [[packages]]
        name = "iniconfig"
        version = "2.0.0"
        marker = "'default' in dependency_groups"
        wheels = [{ url = "https://example.com/iniconfig-2.0.0-py3-none-any.whl", hashes = { sha256 = "0000000000000000000000000000000000000000000000000000000000000000" } }]

        [[packages]]
        name = "typing-extensions"
        version = "4.10.0"
        marker = "'test' in dependency_groups"
        wheels = [{ url = "https://example.com/typing_extensions-4.10.0-py3-none-any.whl", hashes = { sha256 = "1111111111111111111111111111111111111111111111111111111111111111" } }]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would download 1 package
    Would install 1 package
     + iniconfig==2.0.0
    ");

    Ok(())
}

#[test]
fn pep_751_default_groups_empty_public_groups() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pylock.toml").write_str(
        r#"
        lock-version = "1.0"
        created-by = "uv"
        dependency-groups = []
        default-groups = ["default"]

        [[packages]]
        name = "iniconfig"
        version = "2.0.0"
        marker = "'default' in dependency_groups"
        wheels = [{ url = "https://example.com/iniconfig-2.0.0-py3-none-any.whl", hashes = { sha256 = "0000000000000000000000000000000000000000000000000000000000000000" } }]

        [[packages]]
        name = "typing-extensions"
        version = "4.10.0"
        marker = "'test' in dependency_groups"
        wheels = [{ url = "https://example.com/typing_extensions-4.10.0-py3-none-any.whl", hashes = { sha256 = "1111111111111111111111111111111111111111111111111111111111111111" } }]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would download 1 package
    Would install 1 package
     + iniconfig==2.0.0
    ");

    Ok(())
}

#[test]
fn pep_751_default_groups_unrelated_public_groups() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pylock.toml").write_str(
        r#"
        lock-version = "1.0"
        created-by = "uv"
        dependency-groups = ["test"]
        default-groups = ["default"]

        [[packages]]
        name = "iniconfig"
        version = "2.0.0"
        marker = "'default' in dependency_groups"
        wheels = [{ url = "https://example.com/iniconfig-2.0.0-py3-none-any.whl", hashes = { sha256 = "0000000000000000000000000000000000000000000000000000000000000000" } }]

        [[packages]]
        name = "typing-extensions"
        version = "4.10.0"
        marker = "'test' in dependency_groups"
        wheels = [{ url = "https://example.com/typing_extensions-4.10.0-py3-none-any.whl", hashes = { sha256 = "1111111111111111111111111111111111111111111111111111111111111111" } }]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would download 1 package
    Would install 1 package
     + iniconfig==2.0.0
    ");

    Ok(())
}

#[test]
fn pep_751_default_groups_overlapping_public_groups() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pylock.toml").write_str(
        r#"
        lock-version = "1.0"
        created-by = "uv"
        dependency-groups = ["default", "test"]
        default-groups = ["default"]

        [[packages]]
        name = "iniconfig"
        version = "2.0.0"
        marker = "'default' in dependency_groups"
        wheels = [{ url = "https://example.com/iniconfig-2.0.0-py3-none-any.whl", hashes = { sha256 = "0000000000000000000000000000000000000000000000000000000000000000" } }]

        [[packages]]
        name = "typing-extensions"
        version = "4.10.0"
        marker = "'test' in dependency_groups"
        wheels = [{ url = "https://example.com/typing_extensions-4.10.0-py3-none-any.whl", hashes = { sha256 = "1111111111111111111111111111111111111111111111111111111111111111" } }]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would download 1 package
    Would install 1 package
     + iniconfig==2.0.0
    ");

    Ok(())
}

#[test]
fn pep_751_default_groups_explicit_group() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pylock.toml").write_str(
        r#"
        lock-version = "1.0"
        created-by = "uv"
        dependency-groups = ["test"]
        default-groups = ["default"]

        [[packages]]
        name = "iniconfig"
        version = "2.0.0"
        marker = "'default' in dependency_groups"
        wheels = [{ url = "https://example.com/iniconfig-2.0.0-py3-none-any.whl", hashes = { sha256 = "0000000000000000000000000000000000000000000000000000000000000000" } }]

        [[packages]]
        name = "typing-extensions"
        version = "4.10.0"
        marker = "'test' in dependency_groups"
        wheels = [{ url = "https://example.com/typing_extensions-4.10.0-py3-none-any.whl", hashes = { sha256 = "1111111111111111111111111111111111111111111111111111111111111111" } }]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("pylock.toml")
        .arg("--group")
        .arg("test"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would download 1 package
    Would install 1 package
     + typing-extensions==4.10.0
    ");

    Ok(())
}

#[test]
fn pep_751_default_groups_absent() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pylock.toml").write_str(
        r#"
        lock-version = "1.0"
        created-by = "uv"
        dependency-groups = ["default", "test"]

        [[packages]]
        name = "iniconfig"
        version = "2.0.0"
        marker = "'default' in dependency_groups"
        wheels = [{ url = "https://example.com/iniconfig-2.0.0-py3-none-any.whl", hashes = { sha256 = "0000000000000000000000000000000000000000000000000000000000000000" } }]

        [[packages]]
        name = "typing-extensions"
        version = "4.10.0"
        marker = "'test' in dependency_groups"
        wheels = [{ url = "https://example.com/typing_extensions-4.10.0-py3-none-any.whl", hashes = { sha256 = "1111111111111111111111111111111111111111111111111111111111111111" } }]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    Would make no changes
    ");

    Ok(())
}

#[test]
fn pep_751_rejects_duplicate_active_packages() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pylock.toml").write_str(
        r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "iniconfig"
        version = "2.0.0"
        wheels = [{ url = "https://example.com/iniconfig-2.0.0-py3-none-any.whl", hashes = { sha256 = "0000000000000000000000000000000000000000000000000000000000000000" } }]

        [[packages]]
        name = "iniconfig"
        version = "2.1.0"
        wheels = [{ url = "https://example.com/iniconfig-2.1.0-py3-none-any.whl", hashes = { sha256 = "1111111111111111111111111111111111111111111111111111111111111111" } }]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Multiple active package entries found for `iniconfig`
    "#);

    Ok(())
}

#[test]
fn pep_751_requires_packages() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pylock.toml").write_str(
        r#"
        lock-version = "1.0"
        created-by = "uv"
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Not a valid `pylock.toml` file: pylock.toml
      cause: TOML parse error at line 1, column 1
               |
             1 |
               | ^
             missing field `packages`
    "#);

    Ok(())
}

#[test]
fn pep_751_empty_hashes() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("simple/single-package.toml");
    context
        .temp_dir
        .child("pylock.toml")
        .write_str(&formatdoc! {r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "a"
        version = "1.0.0"
        wheels = [{{ url = "{wheel_url}", hashes = {{}} }}]
    "#,
            wheel_url = server.file_url("a-1.0.0-py3-none-any.whl"),
        })?;

    // Empty hash tables should warn without preventing installation by default.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Empty hash tables in `pylock.toml` will be rejected in a future uv version. Rerun the original `uv export` or `uv pip compile` command to regenerate the file.
    Would download 1 package
    Would install 1 package
     + a==1.0.0
    ");

    // Empty hash tables should warn even when verification is disabled.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("--no-verify-hashes")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Empty hash tables in `pylock.toml` will be rejected in a future uv version. Rerun the original `uv export` or `uv pip compile` command to regenerate the file.
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + a==1.0.0
    ");

    Ok(())
}

#[test]
fn pep_751_validates_archive_size() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("iniconfig-2.0.0-py3-none-any.whl")
        .touch()?;

    context.temp_dir.child("pylock.toml").write_str(
        r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "iniconfig"
        version = "2.0.0"
        archive = { path = "iniconfig-2.0.0-py3-none-any.whl", size = 1, hashes = { sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855" } }
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Archive `[TEMP_DIR]/iniconfig-2.0.0-py3-none-any.whl` has size 0, but the lockfile records 1
    "#);

    Ok(())
}

#[test]
fn pep_751_require_hashes_directory() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("foo").child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "foo"
        version = "1.0.0"

        [build-system]
        requires = ["hatchling"]
        build-backend = "hatchling.build"
        "#,
    )?;
    context
        .temp_dir
        .child("foo")
        .child("src")
        .child("foo")
        .child("__init__.py")
        .touch()?;

    let pylock_toml = context.temp_dir.child("pylock.toml");
    pylock_toml.write_str(
        r#"
        lock-version = "1.0"
        created-by = "uv"
        requires-python = ">=3.12"

        [[packages]]
        name = "foo"
        version = "1.0.0"
        directory = { path = "foo" }
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml")
        .arg("--require-hashes"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have a hash, but none were provided for: foo
    "
    );

    Ok(())
}

#[tokio::test]
async fn pep_751_remote() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = MockServer::start().await;
    let filename = "ok-1.0.0-py3-none-any.whl";
    let wheel = context.read_bytes(context.workspace_root.join("test/links").join(filename));
    let sha256 = hex::encode(Sha256::digest(&wheel));

    Mock::given(method("GET"))
        .and(path(format!("/{filename}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(wheel))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/pylock.toml"))
        .respond_with(ResponseTemplate::new(200).set_body_string(formatdoc! {r#"
            lock-version = "1.0"
            created-by = "uv"

            [[packages]]
            name = "ok"
            version = "1.0.0"
            wheels = [{{ url = "{server_url}/{filename}", hashes = {{ sha256 = "{sha256}" }} }}]
            "#,
            server_url = server.uri(),
        }))
        .expect(1)
        .mount(&server)
        .await;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("--no-index")
        .arg(format!("{}/pylock.toml", server.uri())), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==1.0.0
    "
    );
    context.assert_command("import ok").success();

    Ok(())
}

/// Avoid erroring for packages that only include wheels, and _don't_ include a wheel for the
/// current platform, but are omitted by markers anyway.
///
/// See: <https://github.com/astral-sh/uv/issues/13127>
#[test]
fn pep_751_wheel_only() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12.0"
        dependencies = ["torch"]
        "#,
    )?;

    context
        .export()
        .arg("-o")
        .arg("pylock.toml")
        .assert()
        .success();

    // If there's no compatible wheel for a package we _don't_ need to install (e.g., anything
    // CUDA-related), succeed.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml")
        .arg("--dry-run")
        .arg("--python-platform")
        .arg("macos"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would download 9 packages
    Would install 9 packages
     + filelock==3.13.1
     + fsspec==2024.3.1
     + jinja2==3.1.3
     + markupsafe==2.1.5
     + mpmath==1.3.0
     + networkx==3.2.1
     + sympy==1.12
     + torch==2.2.1
     + typing-extensions==4.10.0
    "
    );

    // However, if there's no compatible wheel for a package that we _do_ need to install, we should
    // error
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml")
        .arg("--dry-run")
        .arg("--python-platform")
        .arg("macos")
        .arg("--python-version")
        .arg("3.8"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package `torch` can't be installed because it doesn't have a source distribution or wheel for the current platform

    hint: You're using CPython 3.8 (`cp38`), but `torch` (v2.2.1) only has wheels with the following Python implementation tag: `cp312`
    "
    );

    Ok(())
}

/// Respect `--no-binary` et al when installing from a `pylock.toml`.
#[test]
fn pep_751_build_options() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_exclude_newer("2025-01-29T00:00:00Z");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio"]
        "#,
    )?;

    context
        .export()
        .arg("-o")
        .arg("pylock.toml")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml")
        .arg("--no-binary")
        .arg("anyio"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.8.0
     + idna==3.10
     + sniffio==1.3.1
     + typing-extensions==4.12.2
    "
    );

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["odrive"]
        "#,
    )?;

    context
        .export()
        .arg("-o")
        .arg("pylock.toml")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml")
        .arg("--no-binary")
        .arg("odrive"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package `odrive` can't be installed because it is marked as `--no-binary` but has no source distribution
    "
    );

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["source-distribution"]
        "#,
    )?;

    context
        .export()
        .arg("-o")
        .arg("pylock.toml")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml")
        .arg("--only-binary")
        .arg("source-distribution"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package `source-distribution` can't be installed because it is marked as `--no-build` but has no binary distribution
    "
    );

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml")
        .arg("--no-binary")
        .arg("source-distribution"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Uninstalled 4 packages in [TIME]
    Installed 1 package in [TIME]
     - anyio==4.8.0
     - idna==3.10
     - sniffio==1.3.1
     + source-distribution==0.0.3
     - typing-extensions==4.12.2
    "
    );

    Ok(())
}

#[test]
fn pep_751_direct_url_tags() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["MarkupSafe @ https://files.pythonhosted.org/packages/6b/b0/18f76bba336fa5aecf79d45dcd6c806c280ec44538b3c13671d49099fdd0/MarkupSafe-3.0.2-cp312-cp312-macosx_11_0_arm64.whl"]
        "#,
    )?;

    context
        .export()
        .arg("-o")
        .arg("pylock.toml")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml")
        .arg("--python-platform")
        .arg("linux"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to determine installation plan
      cause: A URL (https://files.pythonhosted.org/packages/6b/b0/18f76bba336fa5aecf79d45dcd6c806c280ec44538b3c13671d49099fdd0/MarkupSafe-3.0.2-cp312-cp312-macosx_11_0_arm64.whl) dependency is incompatible with the current platform

    hint: The wheel is compatible with macOS (`macosx_11_0_arm64`), but you're on Linux (`manylinux_2_28_x86_64`)
    "
    );

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml")
        .arg("--python-platform")
        .arg("macos"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed 1 package in [TIME]
     + markupsafe==3.0.2 (from https://files.pythonhosted.org/packages/6b/b0/18f76bba336fa5aecf79d45dcd6c806c280ec44538b3c13671d49099fdd0/MarkupSafe-3.0.2-cp312-cp312-macosx_11_0_arm64.whl)
    "
    );

    Ok(())
}

#[test]
fn incompatible_python_version_direct_url() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("numpy @ https://files.pythonhosted.org/packages/ae/11/7c546fcf42145f29b71e4d6f429e96d8d68e5a7ba1830b2e68d7418f0bbd/numpy-2.3.2-cp313-cp313-win32.whl")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--python-platform")
        .arg("windows"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to determine installation plan
      cause: A URL (https://files.pythonhosted.org/packages/ae/11/7c546fcf42145f29b71e4d6f429e96d8d68e5a7ba1830b2e68d7418f0bbd/numpy-2.3.2-cp313-cp313-win32.whl) dependency is incompatible with the current platform

    hint: The wheel is compatible with CPython 3.13 (`cp313`), but you're using CPython 3.12 (`cp312`)
    "
    );

    Ok(())
}

#[test]
fn incompatible_direct_url_redacts_credentials() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("numpy @ https://user:secret@files.pythonhosted.org/packages/ae/11/7c546fcf42145f29b71e4d6f429e96d8d68e5a7ba1830b2e68d7418f0bbd/numpy-2.3.2-cp313-cp313-win32.whl?X-Amz-Signature=signing-secret")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--python-platform")
        .arg("windows"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to determine installation plan
      cause: A URL (https://user:****@files.pythonhosted.org/packages/ae/11/7c546fcf42145f29b71e4d6f429e96d8d68e5a7ba1830b2e68d7418f0bbd/numpy-2.3.2-cp313-cp313-win32.whl?X-Amz-Signature=****) dependency is incompatible with the current platform

    hint: The wheel is compatible with CPython 3.13 (`cp313`), but you're using CPython 3.12 (`cp312`)
    "
    );

    Ok(())
}

#[test]
fn incompatible_platform_direct_url() -> Result<()> {
    let context = uv_test::test_context!("3.13");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("numpy @ https://files.pythonhosted.org/packages/ae/11/7c546fcf42145f29b71e4d6f429e96d8d68e5a7ba1830b2e68d7418f0bbd/numpy-2.3.2-cp313-cp313-win32.whl")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--python-platform")
        .arg("linux"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to determine installation plan
      cause: A URL (https://files.pythonhosted.org/packages/ae/11/7c546fcf42145f29b71e4d6f429e96d8d68e5a7ba1830b2e68d7418f0bbd/numpy-2.3.2-cp313-cp313-win32.whl) dependency is incompatible with the current platform

    hint: The wheel is compatible with Windows (`win32`), but you're on Linux (`manylinux_2_28_x86_64`)
    "
    );

    Ok(())
}

/// Test that a missing Python version is not installed when not using `--target` or `--prefix`.
#[cfg(feature = "test-python-managed")]
#[test]
fn sync_missing_python_no_target() -> Result<()> {
    // Create a context that only has Python 3.11 available.
    let context = uv_test::test_context!("3.11").with_managed_python_dirs();

    let requirements = context.temp_dir.child("requirements.txt");
    requirements.write_str("anyio")?;

    // Request Python 3.12; which should fail
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--python").arg("3.12")
        .arg("requirements.txt"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No virtual environment found for Python 3.12; run `uv venv` to create an environment, or pass `--system` to install into a non-virtual environment
    "
    );
    Ok(())
}

#[cfg(feature = "test-python-managed")]
#[test]
fn sync_with_target_installs_missing_python() -> Result<()> {
    // Create a context that only has Python 3.11 available.
    let context = uv_test::test_context!("3.11")
        .with_managed_python_dirs()
        .with_filtered_latest_python_versions();

    let target_dir = context.temp_dir.child("target-dir");
    let requirements = context.temp_dir.child("requirements.txt");
    requirements.write_str("anyio")?;

    // Request Python 3.12 which is not installed in this context.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--python").arg("3.12")
        .arg("--target").arg(target_dir.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[LATEST]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.3.0
    "
    );
    Ok(())
}
