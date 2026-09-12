use std::collections::BTreeMap;
use std::env::consts::EXE_SUFFIX;

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use async_zip::base::read::mem::ZipFileReader;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use fs_err as fs;
use futures::executor::block_on;
use futures::io::AsyncReadExt;
use indoc::{formatdoc, indoc};
use predicates::Predicate;
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_extract::hash::Hasher;
use uv_fs::{Simplified, copy_dir_all};
use uv_pypi_types::{HashAlgorithm, HashDigest};
use uv_static::EnvVars;
use uv_test::find_links::FindLinksServer;
use uv_test::packse::{PackseServer, generate_wheel};
use uv_test::{download_to_disk, site_packages_path, uv_snapshot};

fn artifact_hash(
    server: &PackseServer,
    filename: &str,
    algorithm: HashAlgorithm,
) -> Result<String> {
    let mut hasher = Hasher::from(algorithm);
    hasher.update(&server.file_bytes(filename)?);
    Ok(HashDigest::from(hasher).digest.to_string())
}

#[test]
fn missing_requirements_txt() {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: File not found: `requirements.txt`
    ");

    requirements_txt.assert(predicates::path::missing());
}

/// `--cert` is forwarded to the HTTP client rather than silently ignored.
#[test]
fn cert() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filtered_missing_file_error();
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("iniconfig==2.0.0")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--cert")
        .arg("ca-bundle.pem"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to read certificate file `ca-bundle.pem`
      cause: [OS ERROR 2]
    ");

    Ok(())
}

#[test]
fn missing_venv() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_virtualenv_bin()
        .with_filtered_python_names();

    let requirements = context.temp_dir.child("requirements.txt");
    requirements.write_str("anyio")?;
    fs::remove_dir_all(&context.venv)?;

    uv_snapshot!(context.filters(), context.pip_sync().arg("requirements.txt"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to inspect Python interpreter from active virtual environment at `.venv/[BIN]/[PYTHON]`
      cause: Python interpreter not found at `[VENV]/[BIN]/[PYTHON]`
    ");

    assert!(predicates::path::missing().eval(&context.venv));

    // If not "active", we hint to create one
    uv_snapshot!(context.filters(), context.pip_sync().arg("requirements.txt").env_remove(EnvVars::VIRTUAL_ENV), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No virtual environment found; run `uv venv` to create an environment, or pass `--system` to install into a non-virtual environment
    ");

    assert!(predicates::path::missing().eval(&context.venv));

    Ok(())
}

#[test]
fn missing_system() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let requirements = context.temp_dir.child("requirements.txt");
    requirements.write_str("anyio")?;

    uv_snapshot!(context.filters(), context.pip_sync().arg("requirements.txt").arg("--system"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No system Python installation found
    ");

    Ok(())
}

/// Install a package into a virtual environment using the default link semantics. (On macOS,
/// this using `clone` semantics.)
#[test]
fn install() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("simple-package==2.1.3")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + simple-package==2.1.3
    "
    );

    // Counterpart for the `compile()` test.
    assert!(
        !context
            .site_packages()
            .join("simple_package")
            .join("__pycache__")
            .join("__init__.cpython-312.pyc")
            .exists()
    );

    context
        .assert_command("from simple_package import __version__")
        .success();

    // Removing the cache shouldn't invalidate the virtual environment.
    fs::remove_dir_all(context.cache_dir.path())?;

    context.assert_command("import simple_package").success();

    Ok(())
}

/// Install a package into a virtual environment using copy semantics.
#[test]
fn install_copy() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("simple-package==2.1.3")?;

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
     + simple-package==2.1.3
    "
    );

    context.assert_command("import simple_package").success();

    // Removing the cache shouldn't invalidate the virtual environment.
    fs::remove_dir_all(context.cache_dir.path())?;

    context.assert_command("import simple_package").success();

    Ok(())
}

/// Install a package into a virtual environment using hardlink semantics.
#[test]
fn install_hardlink() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("simple-package==2.1.3")?;

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
     + simple-package==2.1.3
    "
    );

    context.assert_command("import simple_package").success();

    // Removing the cache shouldn't invalidate the virtual environment.
    fs::remove_dir_all(context.cache_dir.path())?;

    context.assert_command("import simple_package").success();

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

    let server = PackseServer::new("packages/pip-commands.toml");
    let Some(context) = uv_test::test_context!("3.12").with_cache_on_lowlinks_fs()? else {
        return Ok(());
    };
    let context = context.with_default_index(&server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("simple-package==2.1.3")?;

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
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("simple-package==2.1.3")?;

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
     + simple-package==2.1.3
    "
    );

    context.assert_command("import simple_package").success();

    // Removing the cache _should_ invalidate the virtual environment.
    fs::remove_dir_all(context.cache_dir.path())?;

    context
        .assert_command("from simple_package import __version__")
        .failure();

    Ok(())
}

/// Reject attempts to use symlink semantics with `--no-cache`.
#[test]
fn install_symlink_no_cache() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("simple-package==2.1.3")?;

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
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("simple-package==2.1.3\nother-package==2.0.1")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + other-package==2.0.1
     + simple-package==2.1.3
    "
    );

    context
        .assert_command("import other_package; import simple_package")
        .success();

    Ok(())
}

/// Attempt to install an already-installed package into a virtual environment.
#[test]
fn noop() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("simple-package==2.1.3")?;

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

    context.assert_command("import simple_package").success();

    Ok(())
}

/// Attempt to sync an empty set of requirements.
#[test]
fn pip_sync_empty() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.touch()?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Requirements file `requirements.txt` does not contain any dependencies
    No requirements found (hint: use `--allow-empty-requirements` to clear the environment)
    "
    );

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
    requirements_txt.write_str("simple-package==2.1.3")?;
    context
        .pip_sync()
        .arg("requirements.txt")
        .assert()
        .success();

    // Now, syncing should remove the package.
    requirements_txt.write_str("")?;
    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--allow-empty-requirements"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Requirements file `requirements.txt` does not contain any dependencies
    Resolved in [TIME]
    Uninstalled 1 package in [TIME]
     - simple-package==2.1.3
    "
    );

    Ok(())
}

/// Install a package into a virtual environment, then install the same package into a different
/// virtual environment.
#[test]
fn link() -> Result<()> {
    let context1 = uv_test::test_context!("3.12").with_packse_index("packages/pip-commands.toml");

    let requirements_txt = context1.temp_dir.child("requirements.txt");
    requirements_txt.write_str("simple-package==2.1.3")?;

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
     + simple-package==2.1.3
    "
    );

    context2
        .python_command()
        .arg("-c")
        .arg("import simple_package")
        .current_dir(&context2.temp_dir)
        .assert()
        .success();

    Ok(())
}

/// Install a package into a virtual environment, then sync the virtual environment with a
/// different requirements file.
#[test]
fn add_remove() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("simple-package==2.1.3")?;

    context
        .pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        .assert()
        .success();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("other-package==2.0.1")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     + other-package==2.0.1
     - simple-package==2.1.3
    "
    );

    context.assert_command("import other_package").success();
    context.assert_command("import simple_package").failure();

    Ok(())
}

/// Install a package into a virtual environment, then install a second package into the same
/// virtual environment.
#[test]
fn install_sequential() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("simple-package==2.1.3")?;

    context
        .pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        .assert()
        .success();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("simple-package==2.1.3\nother-package==2.0.1")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + other-package==2.0.1
    "
    );

    context
        .assert_command("import other_package; import simple_package")
        .success();

    Ok(())
}

/// Install a package into a virtual environment, then install a second package into the same
/// virtual environment.
#[test]
fn upgrade() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/pip-commands.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("simple-package==2.0.0")?;

    context
        .pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        .assert()
        .success();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("simple-package==2.1.3")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - simple-package==2.0.0
     + simple-package==2.1.3
    "
    );

    context.assert_command("import simple_package").success();

    Ok(())
}

/// Install a package into a virtual environment from a URL.
#[test]
fn install_url() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "werkzeug @ {artifact_url_0}",
        artifact_url_0 = registry_artifacts.file_url("werkzeug-2.0.0-py3-none-any.whl")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + werkzeug==2.0.0 (from http://[LOCALHOST]/files/werkzeug-2.0.0-py3-none-any.whl)
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
    requirements_txt.write_str("example-pkg-a @ git+https://github.com/pypa/sample-namespace-packages.git@df7530eeb8fa0cb7dbb8ecb28363e8e36bfa2f45#subdirectory=pkg_resources/pkg_a\nexample-pkg-b @ git+https://github.com/pypa/sample-namespace-packages.git@df7530eeb8fa0cb7dbb8ecb28363e8e36bfa2f45#subdirectory=pkg_resources/pkg_b")?;

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
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "source-distribution @ {artifact_url_0}",
        artifact_url_0 = registry_artifacts.file_url("source_distribution-0.0.1.tar.gz")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.1 (from http://[LOCALHOST]/files/source_distribution-0.0.1.tar.gz)
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
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "werkzeug @ {artifact_url_0}",
        artifact_url_0 = registry_artifacts.file_url("werkzeug-2.0.0-py3-none-any.whl")
    ))?;

    context
        .pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_sync()
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
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "werkzeug @ {artifact_url_0}",
        artifact_url_0 = registry_artifacts.file_url("werkzeug-2.0.0-py3-none-any.whl")
    ))?;

    context
        .pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        .assert()
        .success();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("werkzeug==2.0.0")?;

    uv_snapshot!(context.filters(), context.pip_sync()
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

/// Install a package via a registry version, then via a direct URL version. The second install
/// should remove the registry-based version.
#[test]
fn install_version_then_install_url() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("werkzeug==2.0.0")?;

    context
        .pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        .assert()
        .success();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "werkzeug @ {artifact_url_0}",
        artifact_url_0 = registry_artifacts.file_url("werkzeug-2.0.0-py3-none-any.whl")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - werkzeug==2.0.0
     + werkzeug==2.0.0 (from http://[LOCALHOST]/files/werkzeug-2.0.0-py3-none-any.whl)
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

/// Attempt to install a package without using a remote index.
#[test]
fn install_no_index() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("iniconfig==2.0.0")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--no-index")
        .arg("--strict"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because iniconfig was not found in the provided package locations and you require iniconfig==2.0.0, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because index lookups were disabled and no additional package locations were provided (try: `--find-links <uri>`)
    "
    );

    context.assert_command("import iniconfig").failure();

    Ok(())
}

/// Attempt to install a package without using a remote index
/// after a previous successful installation.
#[test]
fn install_no_index_cached() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("iniconfig==2.0.0")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    "
    );

    context.assert_command("import iniconfig").success();

    context.pip_uninstall().arg("iniconfig").assert().success();

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--no-index")
        .arg("--strict"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because iniconfig was not found in the provided package locations and you require iniconfig==2.0.0, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because index lookups were disabled and no additional package locations were provided (try: `--find-links <uri>`)
    "
    );

    context.assert_command("import iniconfig").failure();

    Ok(())
}

#[test]
fn warn_on_yanked() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // This version is yanked.
    let requirements_in = context.temp_dir.child("requirements.txt");
    requirements_in.write_str("colorama==0.4.2")?;

    uv_snapshot!(context.filters(), windows_filters=false, context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + colorama==0.4.2
    warning: `colorama==0.4.2` is yanked (reason: "Bad build, missing files, will not install")
    "#
    );

    Ok(())
}

#[test]
fn warn_on_yanked_dry_run() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // This version is yanked.
    let requirements_in = context.temp_dir.child("requirements.txt");
    requirements_in.write_str("colorama==0.4.2")?;

    uv_snapshot!(context.filters(), windows_filters=false, context.pip_sync()
        .arg("requirements.txt")
        .arg("--dry-run")
        .arg("--strict"), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Would download 1 package
    Would install 1 package
     + colorama==0.4.2
    warning: `colorama==0.4.2` is yanked (reason: "Bad build, missing files, will not install")
    "#
    );

    Ok(())
}

/// Resolve a local wheel.
#[test]
fn install_local_wheel() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    // Download a wheel.
    let archive = context.temp_dir.child("tomli-2.0.1-py3-none-any.whl");
    download_to_disk(
        &registry_artifacts.file_url("tomli-2.0.1-py3-none-any.whl"),
        &archive,
    );

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "tomli @ {}",
        Url::from_file_path(archive.path()).unwrap()
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tomli==2.0.1 (from file://[TEMP_DIR]/tomli-2.0.1-py3-none-any.whl)
    "
    );

    context.assert_command("import tomli").success();

    // Create a new virtual environment.
    context.reset_venv();

    // Reinstall. The wheel should come from the cache, so there shouldn't be a "download".
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + tomli==2.0.1 (from file://[TEMP_DIR]/tomli-2.0.1-py3-none-any.whl)
    "
    );

    context.assert_command("import tomli").success();

    // Create a new virtual environment.
    context.reset_venv();

    // "Modify" the wheel.
    // The `filetime` crate works on Windows unlike the std.
    filetime::set_file_mtime(&archive, filetime::FileTime::now()).unwrap();

    // Reinstall. The wheel should be "downloaded" again.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tomli==2.0.1 (from file://[TEMP_DIR]/tomli-2.0.1-py3-none-any.whl)
    "
    );

    context.assert_command("import tomli").success();

    // "Modify" the wheel.
    filetime::set_file_mtime(&archive, filetime::FileTime::now()).unwrap();

    // Reinstall into the same virtual environment. The wheel should be reinstalled.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ tomli==2.0.1 (from file://[TEMP_DIR]/tomli-2.0.1-py3-none-any.whl)
    "
    );

    // Reinstall into the same virtual environment. The wheel should _not_ be reinstalled.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    "
    );

    context.assert_command("import tomli").success();

    // Reinstall without the package name.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!("{}", Url::from_file_path(archive.path()).unwrap()))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Checked 1 package in [TIME]
    "
    );

    context.assert_command("import tomli").success();

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
    error: The wheel filename "pkg-1.0-py3-none-../../../target.whl" is invalid: Tag components must contain only ASCII letters, digits, underscores, and periods
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
    error: The wheel filename "pkg-1.0-py3-none-target:stream.whl" is invalid: Tag components must contain only ASCII letters, digits, underscores, and periods
    "#
    );

    Ok(())
}

/// Install a wheel whose actual version doesn't match the version encoded in the filename.
#[test]
fn mismatched_version() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    // Download a wheel.
    let archive = context.temp_dir.child("tomli-3.7.2-py3-none-any.whl");
    download_to_disk(
        &registry_artifacts.file_url("tomli-2.0.1-py3-none-any.whl"),
        &archive,
    );

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "tomli @ {}",
        Url::from_file_path(archive.path()).unwrap()
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    error: Failed to install: tomli-3.7.2-py3-none-any.whl (tomli==3.7.2 (from file://[TEMP_DIR]/tomli-3.7.2-py3-none-any.whl))
      cause: Wheel version does not match filename (2.0.1 != 3.7.2), which indicates a malformed wheel. If this is intentional, set `UV_SKIP_WHEEL_FILENAME_CHECK=1`.
    "
    );

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        .env(EnvVars::UV_SKIP_WHEEL_FILENAME_CHECK, "1"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + tomli==3.7.2 (from file://[TEMP_DIR]/tomli-3.7.2-py3-none-any.whl)
    "
    );

    Ok(())
}

/// Install a wheel whose actual name doesn't match the name encoded in the filename.
#[test]
fn mismatched_name() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    // Download a wheel.
    let archive = context.temp_dir.child("foo-2.0.1-py3-none-any.whl");
    download_to_disk(
        &registry_artifacts.file_url("tomli-2.0.1-py3-none-any.whl"),
        &archive,
    );

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "foo @ {}",
        Url::from_file_path(archive.path()).unwrap()
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because foo has an invalid package format and you require foo, we can conclude that your requirements are unsatisfiable.

    hint: The structure of `foo` was invalid
      Caused by: The .dist-info directory tomli-2.0.1 does not start with the normalized package name: foo
    "
    );

    Ok(())
}

/// Install a local source distribution.
#[test]
fn install_local_source_distribution() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    // Download a source distribution.
    let archive = context.temp_dir.child("wheel-0.42.0.tar.gz");
    download_to_disk(
        &registry_artifacts.file_url("wheel-0.42.0.tar.gz"),
        &archive,
    );

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "wheel @ {}",
        Url::from_file_path(archive.path()).unwrap()
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
/// Like `pip` and `build`, we should use PEP 517 here and respect the `requires`, but use the
/// default build backend.
#[test]
fn install_build_system_no_backend() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_packse_index("packages/pip-sync-build.toml");
    let links = context.temp_dir.child("links");
    links.create_dir_all()?;
    uv_test::archive::write_tar_gz(
        fs::File::create(links.child("build_system_no_backend-0.1.0.tar.gz").path())?,
        &[
            (
                "build_system_no_backend-0.1.0/pyproject.toml",
                indoc! {r#"
                [build-system]
                requires = ["setuptools", "wheel", "build-requirement==1.0.0"]
            "#},
            ),
            (
                "build_system_no_backend-0.1.0/setup.py",
                indoc! {r#"
                from build_requirement import VALUE
                from setuptools import setup

                assert VALUE == "available during the build"
                setup(name="build-system-no-backend", version="0.1.0", py_modules=["build_system_no_backend"])
            "#},
            ),
            (
                "build_system_no_backend-0.1.0/build_system_no_backend.py",
                "__version__ = '0.1.0'\n",
            ),
        ],
    )?;
    let artifacts = FindLinksServer::new(links.path());

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "build-system-no-backend @ {}",
        artifacts.file_url("build_system_no_backend-0.1.0.tar.gz")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + build-system-no-backend==0.1.0 (from http://[LOCALHOST]/build_system_no_backend-0.1.0.tar.gz)
    "
    );

    context
        .assert_command("import build_system_no_backend")
        .success();

    Ok(())
}

/// Check that we show the right messages on cached, direct URL source distribution installs.
#[test]
fn install_url_source_dist_cached() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "source_distribution @ {artifact_url_0}",
        artifact_url_0 = registry_artifacts.file_url("source_distribution-0.0.1.tar.gz")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.1 (from http://[LOCALHOST]/files/source_distribution-0.0.1.tar.gz)
    "
    );

    context
        .assert_command("import source_distribution")
        .success();

    // Re-run the installation in a new virtual environment.
    context.reset_venv();

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.1 (from http://[LOCALHOST]/files/source_distribution-0.0.1.tar.gz)
    "
    );

    context
        .assert_command("import source_distribution")
        .success();

    // Clear the cache, then re-run the installation in a new virtual environment.
    context.reset_venv();

    uv_snapshot!(
        context.filters(),
        context.clean().arg("source_distribution"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed [N] files ([SIZE])
    "
    );

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.1 (from http://[LOCALHOST]/files/source_distribution-0.0.1.tar.gz)
    "
    );

    context
        .assert_command("import source_distribution")
        .success();

    Ok(())
}

/// Check that we show the right messages on cached, Git source distribution installs.
#[test]
#[cfg(feature = "test-git")]
fn install_git_source_dist_cached() -> Result<()> {
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

    // Clear the cache, then re-run the installation in a new virtual environment.
    context.reset_venv();

    let filters = if cfg!(windows) {
        [("Removed 2 files", "Removed 3 files")]
            .into_iter()
            .chain(context.filters())
            .collect()
    } else {
        context.filters()
    };
    uv_snapshot!(filters, context.clean()
        .arg("werkzeug"), @"
    exit_code: 0 (success)
    ----- stderr -----
    No cache entries found
    "
    );

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

    Ok(())
}

/// Check that we show the right messages on cached, registry source distribution installs.
#[test]
fn install_registry_source_dist_cached() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_exclude_newer("2025-01-29T00:00:00Z")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("source_distribution==0.0.1")?;

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
     + source-distribution==0.0.1
    "
    );

    context
        .assert_command("import source_distribution")
        .success();

    // Clear the cache, then re-run the installation in a new virtual environment.
    context.reset_venv();

    uv_snapshot!(context.filters(), context.clean()
        .arg("source_distribution"), @"
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
     + source-distribution==0.0.1
    "
    );

    context
        .assert_command("import source_distribution")
        .success();

    Ok(())
}

/// Check that we show the right messages on cached, local source distribution installs.
#[test]
fn install_path_source_dist_cached() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units();

    // Download a source distribution.
    let archive = context.temp_dir.child("source_distribution-0.0.1.tar.gz");
    download_to_disk(
        &registry_artifacts.file_url("source_distribution-0.0.1.tar.gz"),
        &archive,
    );

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "source-distribution @ {}",
        Url::from_file_path(archive.path()).unwrap()
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.1 (from file://[TEMP_DIR]/source_distribution-0.0.1.tar.gz)
    "
    );

    context
        .assert_command("import source_distribution")
        .success();

    // Re-run the installation in a new virtual environment.
    context.reset_venv();

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.1 (from file://[TEMP_DIR]/source_distribution-0.0.1.tar.gz)
    "
    );

    context
        .assert_command("import source_distribution")
        .success();

    // Clear the cache, then re-run the installation in a new virtual environment.
    context.reset_venv();

    uv_snapshot!(
        context.filters(),
        context.clean().arg("source-distribution"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed [N] files ([SIZE])
    "
    );

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.1 (from file://[TEMP_DIR]/source_distribution-0.0.1.tar.gz)
    "
    );

    context
        .assert_command("import source_distribution")
        .success();

    Ok(())
}

/// Check that we show the right messages on cached, local source distribution installs.
#[test]
fn install_path_built_dist_cached() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units();

    // Download a wheel.
    let archive = context.temp_dir.child("tomli-2.0.1-py3-none-any.whl");
    download_to_disk(
        &registry_artifacts.file_url("tomli-2.0.1-py3-none-any.whl"),
        &archive,
    );

    let requirements_txt = context.temp_dir.child("requirements.txt");
    let url = Url::from_file_path(archive.path()).unwrap();
    requirements_txt.write_str(&format!("tomli @ {url}"))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tomli==2.0.1 (from file://[TEMP_DIR]/tomli-2.0.1-py3-none-any.whl)
    "
    );

    context.assert_command("import tomli").success();

    // Re-run the installation in a new virtual environment.
    context.reset_venv();

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + tomli==2.0.1 (from file://[TEMP_DIR]/tomli-2.0.1-py3-none-any.whl)
    "
    );

    context.assert_command("import tomli").success();

    // Clear the cache, then re-run the installation in a new virtual environment.
    context.reset_venv();

    uv_snapshot!(
        context.filters(),
        context.clean().arg("tomli"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed [N] files ([SIZE])
    "
    );

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tomli==2.0.1 (from file://[TEMP_DIR]/tomli-2.0.1-py3-none-any.whl)
    "
    );

    context.assert_command("import tomli").success();

    Ok(())
}

/// Check that we show the right messages on cached, direct URL built distribution installs.
#[test]
fn install_url_built_dist_cached() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12")
        .with_filtered_file_counts()
        .with_filtered_sizes_and_units();

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "tqdm @ {artifact_url_0}",
        artifact_url_0 = registry_artifacts.file_url("tqdm-4.66.1-py3-none-any.whl")
    ))?;

    let context_filters = if cfg!(windows) {
        [("warning: The package `tqdm` requires `colorama ; sys_platform == 'win32'`, but it's not installed\n", "")]
            .into_iter()
            .chain(context.filters())
            .collect()
    } else {
        context.filters()
    };
    uv_snapshot!(context_filters, context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==4.66.1 (from http://[LOCALHOST]/files/tqdm-4.66.1-py3-none-any.whl)
    "
    );

    context.assert_command("import tqdm").success();

    // Re-run the installation in a new virtual environment.
    context.reset_venv();

    uv_snapshot!(context_filters, context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==4.66.1 (from http://[LOCALHOST]/files/tqdm-4.66.1-py3-none-any.whl)
    "
    );

    context.assert_command("import tqdm").success();

    // Clear the cache, then re-run the installation in a new virtual environment.
    context.reset_venv();

    uv_snapshot!(
        context_filters,
        context.clean().arg("tqdm"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Removed [N] files ([SIZE])
    "
    );

    uv_snapshot!(context_filters, context.pip_sync()
        .arg("requirements.txt")
        .arg("--strict")
        , @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==4.66.1 (from http://[LOCALHOST]/files/tqdm-4.66.1-py3-none-any.whl)
    "
    );

    context.assert_command("import tqdm").success();

    Ok(())
}

/// Verify that fail with an appropriate error when a package is repeated.
#[test]
fn duplicate_package_overlap() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("MarkupSafe==2.1.3\nMarkupSafe==2.1.2")?;

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
    requirements_txt.write_str("MarkupSafe==2.1.3\nMarkupSafe==2.1.2 ; python_version < '3.6'")?;

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

/// Verify that we can force reinstall of packages.
#[test]
fn reinstall() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("MarkupSafe==2.1.3\ntomli==2.0.1")?;

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

    // Re-run the installation with `--reinstall`.
    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Uninstalled 2 packages in [TIME]
    Installed 2 packages in [TIME]
     ~ markupsafe==2.1.3
     ~ tomli==2.0.1
    "
    );

    context.assert_command("import markupsafe").success();
    context.assert_command("import tomli").success();

    Ok(())
}

/// Verify that we can force reinstall of selective packages.
#[test]
fn reinstall_package() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("MarkupSafe==2.1.3\ntomli==2.0.1")?;

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

    // Re-run the installation with `--reinstall`.
    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall-package")
        .arg("tomli")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ tomli==2.0.1
    "
    );

    context.assert_command("import markupsafe").success();
    context.assert_command("import tomli").success();

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
    requirements_txt.write_str("MarkupSafe==2.1.3\ntomli==2.0.1")?;

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
    requirements_txt.write_str("MarkupSafe==2.1.3\ntomli==2.0.1")?;

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
    copy_dir_all(
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
    let pyproject_toml_contents = fs_err::read_to_string(&pyproject_toml)?;
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

    // Modify the `pyproject.toml` file.
    let pyproject_toml = poetry_editable.path().join("pyproject.toml");
    let pyproject_toml_contents = fs_err::read_to_string(&pyproject_toml)?;
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
     ~ poetry-editable==0.1.1 (from file://[TEMP_DIR]/poetry_editable)
    "
    );

    Ok(())
}

#[test]
fn sync_editable_and_registry() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Copy the black test editable into the "current" directory
    copy_dir_all(
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

#[test]
fn sync_editable_and_local() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Copy the black test editable into the "current" directory
    copy_dir_all(
        context.workspace_root.join("test/packages/black_editable"),
        context.temp_dir.join("black_editable"),
    )?;

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

    // Install the non-editable version of Black. This should replace the editable version.
    let requirements_txt = context.temp_dir.child("requirements.txt");
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

    // Reinstall the editable version of Black. This should replace the non-editable version.
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
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ black==0.1.0 (from file://[TEMP_DIR]/black_editable)
    "
    );

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
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_in = context.temp_dir.child("requirements.in");
    requirements_in.write_str(&format!(
        "flake8 @ {artifact_url_0}",
        artifact_url_0 = registry_artifacts.file_url("flake8-6.0.0.tar.gz")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.in"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + flake8==6.0.0 (from http://[LOCALHOST]/files/flake8-6.0.0.tar.gz)
    "
    );

    Ok(())
}

/// Sync using `--find-links` with a local directory.
#[test]
fn find_links() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&indoc::formatdoc! {r"
        markupsafe==2.1.3
        numpy==1.26.3
        tqdm==1000.0.0
        werkzeug @ {artifact_url_0}
    ", artifact_url_0 = registry_artifacts.file_url("werkzeug-3.0.1-py3-none-any.whl") })?;

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
     + werkzeug==3.0.1 (from http://[LOCALHOST]/files/werkzeug-3.0.1-py3-none-any.whl)
    "
    );

    Ok(())
}

/// Sync using `--find-links` with `--no-index`, which should accept the local wheel.
#[test]
fn find_links_no_index_match() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {r"
        tqdm==1000.0.0
    "})?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--no-index")
        .arg("--find-links")
        .arg(context.workspace_root.join("test/links/")), @"
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

/// Sync using `--find-links` with `--offline`, which should accept the local wheel.
#[test]
fn find_links_offline_match() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {r"
        tqdm==1000.0.0
    "})?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--offline")
        .arg("--find-links")
        .arg(context.workspace_root.join("test/links/")), @"
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

/// Sync using `--find-links` with `--offline`, which should fail to find `numpy`.
#[test]
fn find_links_offline_no_match() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {r"
        numpy
        tqdm==1000.0.0
    "})?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--offline")
        .arg("--find-links")
        .arg(context.workspace_root.join("test/links/")), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because numpy was not found in the cache and you require numpy, we can conclude that your requirements are unsatisfiable.

    hint: Packages were unavailable because the network was disabled. When the network is disabled, registry packages may only be read from the cache.
    "
    );

    Ok(())
}

/// Sync using `--find-links` with a local directory. Ensure that cached wheels are reused.
#[test]
fn find_links_wheel_cache() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {r"
        tqdm==1000.0.0
    "})?;

    // Install `tqdm`.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--find-links")
        .arg(context.workspace_root.join("test/links/")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==1000.0.0
    "
    );

    // Reinstall `tqdm` with `--reinstall`. Ensure that the wheel is reused.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--find-links")
        .arg(context.workspace_root.join("test/links/")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ tqdm==1000.0.0
    "
    );

    Ok(())
}

/// Sync using `--find-links` with a local directory. Ensure that cached source distributions are
/// reused.
#[test]
fn find_links_source_cache() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc! {r"
        tqdm==999.0.0
    "})?;

    // Install `tqdm`.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--find-links")
        .arg(context.workspace_root.join("test/links/")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==999.0.0
    "
    );

    // Reinstall `tqdm` with `--reinstall`. Ensure that the wheel is reused.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--find-links")
        .arg(context.workspace_root.join("test/links/")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ tqdm==999.0.0
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
    requirements_in.write_str("anyio\nanyio")?;

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
    requirements_in.write_str("anyio\nanyio==4.0.0")?;

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
    requirements_in.write_str("anyio<4.0.0\nanyio==4.0.0")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.in"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because you require anyio<4.0.0 and anyio==4.0.0, we can conclude that your requirements are unsatisfiable.
    ");

    Ok(())
}

/// Don't preserve the mtime from .tar.gz files, it may be the unix epoch (1970-01-01), while Python's zip
/// implementation can't handle files with an mtime older than 1980.
/// See also <https://github.com/alexcrichton/tar-rs/issues/349>.
#[test]
fn tar_dont_preserve_mtime() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "tomli @ {artifact_url_0}",
        artifact_url_0 = registry_artifacts.file_url("tomli-2.0.1.tar.gz")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tomli==2.0.1 (from http://[LOCALHOST]/files/tomli-2.0.1.tar.gz)
    ");

    Ok(())
}

/// Avoid creating a file with 000 permissions
#[test]
fn set_read_permissions() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let links = context.temp_dir.child("links");
    links.create_dir_all()?;
    let (filename, bytes) = generate_wheel(
        &"read-permissions".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::default(),
        None,
        "py3-none-any",
    );
    let bytes = block_on(async {
        let reader = ZipFileReader::new(bytes).await?;
        let mut writer = ZipFileWriter::new(Vec::new());
        for (index, entry) in reader.file().entries().iter().enumerate() {
            let mut contents = Vec::new();
            reader
                .reader_without_entry(index)
                .await?
                .read_to_end(&mut contents)
                .await?;
            let entry = ZipEntryBuilder::new(
                entry.filename().as_str()?.to_owned().into(),
                Compression::Stored,
            )
            .unix_permissions(0);
            writer.write_entry_whole(entry, &contents).await?;
        }
        Ok::<_, anyhow::Error>(writer.close().await?)
    })?;
    fs::write(links.child(&filename), bytes)?;
    let artifacts = FindLinksServer::new(links.path());
    let requirements_in = context.temp_dir.child("requirements.in");
    requirements_in.write_str(&format!(
        "read-permissions @ {}",
        artifacts.file_url(&filename)
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.in"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + read-permissions==1.0.0 (from http://[LOCALHOST]/read_permissions-1.0.0-py3-none-any.whl)
    ");

    context.assert_command("import read_permissions").success();

    Ok(())
}

/// Test special case to generate versioned pip launchers.
/// <https://github.com/pypa/pip/blob/3898741e29b7279e7bffe044ecfbe20f6a438b1e/src/pip/_internal/operations/install/wheel.py#L283>
/// <https://github.com/astral-sh/uv/issues/1593>
#[test]
fn pip_entrypoints() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_exclude_newer("2024-06-01T00:00:00Z");

    for pip_requirement in [
        // Test compatibility with launchers in 24.0
        // https://inspector.pypi.io/project/pip/24.0/packages/8a/6a/19e9fe04fca059ccf770861c7d5721ab4c2aebc539889e97c7977528a53b/pip-24.0-py3-none-any.whl/pip-24.0.dist-info/entry_points.txt
        "pip==24.0",
        // Test compatibility with launcher changes from https://github.com/pypa/pip/pull/12536 released in 24.1b1
        // See https://github.com/astral-sh/uv/pull/1982
        "pip==24.1b1",
    ] {
        let requirements_txt = context.temp_dir.child("requirements.txt");
        requirements_txt.write_str(pip_requirement)?;

        context
            .pip_sync()
            .arg("requirements.txt")
            .arg("--strict")
            .assert()
            .success();
        context
            .assert_command(&format!(
                "import pip; assert pip.__version__ == {:?}",
                pip_requirement.trim_start_matches("pip==")
            ))
            .success();

        let bin_dir = context.venv.join(if cfg!(unix) {
            "bin"
        } else if cfg!(windows) {
            "Scripts"
        } else {
            unimplemented!("Only Windows and Unix are supported")
        });
        ChildPath::new(bin_dir.join(format!("pip3.10{EXE_SUFFIX}")))
            .assert(predicates::path::missing());
        ChildPath::new(bin_dir.join(format!("pip3.12{EXE_SUFFIX}")))
            .assert(predicates::path::exists());
    }

    Ok(())
}

#[test]
fn invalidate_on_change() -> Result<()> {
    let context = uv_test::test_context!("3.12");

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

    assert!(
        context
            .site_packages()
            .join("markupsafe")
            .join("__pycache__")
            .join("__init__.cpython-312.pyc")
            .exists()
    );

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

    assert!(
        context
            .site_packages()
            .join("markupsafe")
            .join("__pycache__")
            .join("__init__.cpython-312.pyc")
            .exists()
    );

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
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "anyio==4.0.0 --hash=foo:{artifact_hash_0}",
        artifact_hash_0 = registry_artifacts
            .file_hash("anyio-4.0.0-py3-none-any.whl")
            .expect("fixture distribution should exist")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
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
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "anyio==4.0.0 --hash=sha256:{artifact_hash_0}",
        artifact_hash_0 = registry_artifacts
            .file_hash("anyio-4.0.0-py3-none-any.whl")
            .expect("fixture distribution should exist")
    ))?;

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
               sha256:199e461df405c68762d1b9ec6185a32bbb28f0bf3a14deab9f42c630743dfeae

             Computed:
               sha256:9d6958e8e40836504967c30143d1df606b8d64f2459c759afcea867e7b642c7a
    "
    );

    Ok(())
}

/// Include the hash for _just_ the wheel with `--only-binary`.
#[test]
fn require_hashes_wheel_only_binary() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "anyio==4.0.0 --hash=sha256:{artifact_hash_0}",
        artifact_hash_0 = registry_artifacts
            .file_hash("anyio-4.0.0-py3-none-any.whl")
            .expect("fixture distribution should exist")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
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
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "anyio==4.0.0 --hash=sha256:{artifact_hash_0}",
        artifact_hash_0 = registry_artifacts
            .file_hash("anyio-4.0.0.tar.gz")
            .expect("fixture distribution should exist")
    ))?;

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
               sha256:9d6958e8e40836504967c30143d1df606b8d64f2459c759afcea867e7b642c7a

             Computed:
               sha256:199e461df405c68762d1b9ec6185a32bbb28f0bf3a14deab9f42c630743dfeae
    "
    );

    Ok(())
}

/// Include the correct hash algorithm, but the wrong digest.
#[test]
fn require_hashes_wrong_digest() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("anyio==4.0.0 --hash=sha256:afdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f")?;

    uv_snapshot!(context.pip_sync()
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
               sha256:199e461df405c68762d1b9ec6185a32bbb28f0bf3a14deab9f42c630743dfeae
    "
    );

    Ok(())
}

/// Include the correct hash, but the wrong algorithm.
#[test]
fn require_hashes_wrong_algorithm() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "anyio==4.0.0 --hash=sha512:{artifact_hash_0}",
        artifact_hash_0 = registry_artifacts
            .file_hash("anyio-4.0.0-py3-none-any.whl")
            .expect("fixture distribution should exist")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `anyio==4.0.0`
      cause: Hash mismatch for `anyio==4.0.0`

             Expected:
               sha512:199e461df405c68762d1b9ec6185a32bbb28f0bf3a14deab9f42c630743dfeae

             Computed:
               sha256:199e461df405c68762d1b9ec6185a32bbb28f0bf3a14deab9f42c630743dfeae
               sha512:c3205a36615f29b02bbedbe5f1549952bd25b7a2d631db9fbb485fe95b9a673993f4a71fbc68e0902ba91f3930ccaa5ad0cbd19524876b50dc3dc3b39695003f
    "
    );

    Ok(())
}

/// Include the hash for a source distribution specified as a direct URL dependency.
#[test]
fn require_hashes_source_url() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "source-distribution @ {artifact_url_0} --hash=sha256:{artifact_hash_1}",
        artifact_url_0 = registry_artifacts.file_url("source_distribution-0.0.1.tar.gz"),
        artifact_hash_1 = registry_artifacts
            .file_hash("source_distribution-0.0.1.tar.gz")
            .expect("fixture distribution should exist")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.1 (from http://[LOCALHOST]/files/source_distribution-0.0.1.tar.gz)
    "
    );

    // Reinstall with the right hash, and verify that it's reused.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ source-distribution==0.0.1 (from http://[LOCALHOST]/files/source_distribution-0.0.1.tar.gz)
    "
    );

    // Reinstall with the wrong hash, and verify that it's rejected despite being cached.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(&format!("source-distribution @ {artifact_url_0} --hash=sha256:a7ed51751b2c2add651e5747c891b47e26d2a21be5d32d9311dfe9692f3e5d7a", artifact_url_0 = registry_artifacts.file_url("source_distribution-0.0.1.tar.gz")))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download and build `source-distribution @ http://[LOCALHOST]/files/source_distribution-0.0.1.tar.gz`
      cause: Hash mismatch for `source-distribution @ http://[LOCALHOST]/files/source_distribution-0.0.1.tar.gz`

             Expected:
               sha256:a7ed51751b2c2add651e5747c891b47e26d2a21be5d32d9311dfe9692f3e5d7a

             Computed:
               sha256:92ddfc533c533fb4b5160171a8d31a3c0c3edd290d28e8434908ea42d4445c74
    "
    );

    Ok(())
}

/// Include the _wrong_ hash for a source distribution specified as a direct URL dependency.
#[test]
fn require_hashes_source_url_mismatch() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(&format!("source-distribution @ {artifact_url_0} --hash=sha256:a7ed51751b2c2add651e5747c891b47e26d2a21be5d32d9311dfe9692f3e5d7a", artifact_url_0 = registry_artifacts.file_url("source_distribution-0.0.1.tar.gz")))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download and build `source-distribution @ http://[LOCALHOST]/files/source_distribution-0.0.1.tar.gz`
      cause: Hash mismatch for `source-distribution @ http://[LOCALHOST]/files/source_distribution-0.0.1.tar.gz`

             Expected:
               sha256:a7ed51751b2c2add651e5747c891b47e26d2a21be5d32d9311dfe9692f3e5d7a

             Computed:
               sha256:92ddfc533c533fb4b5160171a8d31a3c0c3edd290d28e8434908ea42d4445c74
    "
    );

    Ok(())
}

/// Include the hash for a built distribution specified as a direct URL dependency.
#[test]
fn require_hashes_wheel_url() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "anyio @ {artifact_url_0} --hash=sha256:{artifact_hash_1}",
        artifact_url_0 = registry_artifacts.file_url("anyio-4.0.0-py3-none-any.whl"),
        artifact_hash_1 = registry_artifacts
            .file_hash("anyio-4.0.0-py3-none-any.whl")
            .expect("fixture distribution should exist")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.0.0 (from http://[LOCALHOST]/files/anyio-4.0.0-py3-none-any.whl)
    "
    );

    // Reinstall with the right hash, and verify that it's reused.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ anyio==4.0.0 (from http://[LOCALHOST]/files/anyio-4.0.0-py3-none-any.whl)
    "
    );

    // Reinstall with the wrong hash, and verify that it's rejected despite being cached.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(&format!("anyio @ {artifact_url_0} --hash=sha256:afdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f", artifact_url_0 = registry_artifacts.file_url("anyio-4.0.0-py3-none-any.whl")))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `anyio @ http://[LOCALHOST]/files/anyio-4.0.0-py3-none-any.whl`
      cause: Hash mismatch for `anyio @ http://[LOCALHOST]/files/anyio-4.0.0-py3-none-any.whl`

             Expected:
               sha256:afdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f

             Computed:
               sha256:199e461df405c68762d1b9ec6185a32bbb28f0bf3a14deab9f42c630743dfeae
    "
    );

    // Sync a new dependency and include the wrong hash for anyio. Verify that we reuse anyio
    // despite the wrong hash, like pip, since we don't validate hashes for already-installed
    // distributions.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(&format!("anyio==4.0.0 --hash=sha256:afdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f\niniconfig==2.0.0 --hash=sha256:{artifact_hash_0}", artifact_hash_0 = registry_artifacts.file_hash("iniconfig-2.0.0-py3-none-any.whl").expect("fixture distribution should exist")))?;

    uv_snapshot!(context.filters(), context.pip_sync()
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

/// Include the _wrong_ hash for a built distribution specified as a direct URL dependency.
#[test]
fn require_hashes_wheel_url_mismatch() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(&format!("anyio @ {artifact_url_0} --hash=sha256:afdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f", artifact_url_0 = registry_artifacts.file_url("anyio-4.0.0-py3-none-any.whl")))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `anyio @ http://[LOCALHOST]/files/anyio-4.0.0-py3-none-any.whl`
      cause: Hash mismatch for `anyio @ http://[LOCALHOST]/files/anyio-4.0.0-py3-none-any.whl`

             Expected:
               sha256:afdb2b588b9fc25ede96d8db56ed50848b0b649dca3dd1df0b11f683bb9e0b5f

             Computed:
               sha256:199e461df405c68762d1b9ec6185a32bbb28f0bf3a14deab9f42c630743dfeae
    "
    );

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
      cause: Hash-checking is not supported for Git repositories: `anyio @ git+https://github.com/agronholm/anyio@4a23745badf5bf5ef7928f1e346e9986bd696d82`
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
      cause: Hash-checking is not supported for local directories: `black @ file://[WORKSPACE]/test/packages/black_editable`
    "
    );

    Ok(())
}

/// Include the hash for _just_ the wheel with `--only-binary`.
#[test]
fn require_hashes_re_download() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("anyio==4.0.0")?;

    // Install without `--require-hashes`.
    uv_snapshot!(context.filters(), context.pip_sync()
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
               sha256:199e461df405c68762d1b9ec6185a32bbb28f0bf3a14deab9f42c630743dfeae
    "
    );

    // Reinstall with `--require-hashes`, and the right hash.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "anyio==4.0.0 --hash=sha256:{artifact_hash_0}",
        artifact_hash_0 = registry_artifacts
            .file_hash("anyio-4.0.0-py3-none-any.whl")
            .expect("fixture distribution should exist")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
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
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(&indoc::formatdoc!{r"
            {artifact_url_0} --hash=sha256:{artifact_hash_1}
        ", artifact_url_0 = registry_artifacts.file_url("anyio-4.0.0-py3-none-any.whl"), artifact_hash_1 = registry_artifacts.file_hash("anyio-4.0.0-py3-none-any.whl").expect("fixture distribution should exist") } )?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.0.0 (from http://[LOCALHOST]/files/anyio-4.0.0-py3-none-any.whl)
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
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "anyio==4.0.0 --hash=sha256:{artifact_hash_0}\nanyio",
        artifact_hash_0 = registry_artifacts
            .file_hash("anyio-4.0.0.tar.gz")
            .expect("fixture distribution should exist")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have their versions pinned with `==`, but found: anyio
    "
    );

    // Reverse the order.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "anyio\nanyio==4.0.0 --hash=sha256:{artifact_hash_0}",
        artifact_hash_0 = registry_artifacts
            .file_hash("anyio-4.0.0.tar.gz")
            .expect("fixture distribution should exist")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
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
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    // Use the same hash in both cases.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(&indoc::formatdoc!{ r"
            anyio @ {artifact_url_0} --hash=sha256:{artifact_hash_2}
            anyio @ {artifact_url_1} --hash=sha256:{artifact_hash_3}
    ", artifact_url_0 = registry_artifacts.file_url("anyio-4.0.0-py3-none-any.whl"), artifact_url_1 = registry_artifacts.file_url("anyio-4.0.0-py3-none-any.whl"), artifact_hash_2 = registry_artifacts.file_hash("anyio-4.0.0-py3-none-any.whl").expect("fixture distribution should exist"), artifact_hash_3 = registry_artifacts.file_hash("anyio-4.0.0-py3-none-any.whl").expect("fixture distribution should exist") })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.0.0 (from http://[LOCALHOST]/files/anyio-4.0.0-py3-none-any.whl)
    "
    );

    // Use a different hash, but both are correct.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(&indoc::formatdoc!{ r"
            anyio @ {artifact_url_0} --hash=sha256:{artifact_hash_2}
            anyio @ {artifact_url_1} --hash=sha512:{sha512}
    ", artifact_url_0 = registry_artifacts.file_url("anyio-4.0.0-py3-none-any.whl"), artifact_url_1 = registry_artifacts.file_url("anyio-4.0.0-py3-none-any.whl"), artifact_hash_2 = registry_artifacts.file_hash("anyio-4.0.0-py3-none-any.whl").expect("fixture distribution should exist"), sha512 = artifact_hash(&registry_artifacts, "anyio-4.0.0-py3-none-any.whl", HashAlgorithm::Sha512)? })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes")
        .arg("--reinstall"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ anyio==4.0.0 (from http://[LOCALHOST]/files/anyio-4.0.0-py3-none-any.whl)
    "
    );

    // Use a different hash. The `sha512` is wrong, so validation should fail even though the
    // `sha256` is still correct.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(&indoc::formatdoc!{ r"
            anyio @ {artifact_url_0} --hash=sha256:{artifact_hash_2}
            anyio @ {artifact_url_1} --hash=sha512:e30761c1e8725b49c498273b90dba4b05c0fd157811994c806183062cb6647e773364ce45f0e1ff0b10e32fe6d0232ea5ad39476ccf37109d6b49603a09c11c2
    ", artifact_url_0 = registry_artifacts.file_url("anyio-4.0.0-py3-none-any.whl"), artifact_url_1 = registry_artifacts.file_url("anyio-4.0.0-py3-none-any.whl"), artifact_hash_2 = registry_artifacts.file_hash("anyio-4.0.0-py3-none-any.whl").expect("fixture distribution should exist") })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes")
        .arg("--reinstall"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `anyio @ http://[LOCALHOST]/files/anyio-4.0.0-py3-none-any.whl`
      cause: Hash mismatch for `anyio @ http://[LOCALHOST]/files/anyio-4.0.0-py3-none-any.whl`

             Expected:
               sha256:199e461df405c68762d1b9ec6185a32bbb28f0bf3a14deab9f42c630743dfeae
               sha512:e30761c1e8725b49c498273b90dba4b05c0fd157811994c806183062cb6647e773364ce45f0e1ff0b10e32fe6d0232ea5ad39476ccf37109d6b49603a09c11c2

             Computed:
               sha256:199e461df405c68762d1b9ec6185a32bbb28f0bf3a14deab9f42c630743dfeae
               sha512:c3205a36615f29b02bbedbe5f1549952bd25b7a2d631db9fbb485fe95b9a673993f4a71fbc68e0902ba91f3930ccaa5ad0cbd19524876b50dc3dc3b39695003f
    "
    );

    // Use different hashes, but both are wrong. This should fail because none of the merged
    // hashes match.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(&indoc::formatdoc!{ r"
            anyio @ {artifact_url_0} --hash=sha256:{artifact_hash_2}
            anyio @ {artifact_url_1} --hash=sha512:e30761c1e8725b49c498273b90dba4b05c0fd157811994c806183062cb6647e773364ce45f0e1ff0b10e32fe6d0232ea5ad39476ccf37109d6b49603a09c11c2
    ", artifact_url_0 = registry_artifacts.file_url("anyio-4.0.0-py3-none-any.whl"), artifact_url_1 = registry_artifacts.file_url("anyio-4.0.0-py3-none-any.whl"), artifact_hash_2 = registry_artifacts.file_hash("anyio-4.0.0.tar.gz").expect("fixture distribution should exist") })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes")
        .arg("--reinstall"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `anyio @ http://[LOCALHOST]/files/anyio-4.0.0-py3-none-any.whl`
      cause: Hash mismatch for `anyio @ http://[LOCALHOST]/files/anyio-4.0.0-py3-none-any.whl`

             Expected:
               sha256:9d6958e8e40836504967c30143d1df606b8d64f2459c759afcea867e7b642c7a
               sha512:e30761c1e8725b49c498273b90dba4b05c0fd157811994c806183062cb6647e773364ce45f0e1ff0b10e32fe6d0232ea5ad39476ccf37109d6b49603a09c11c2

             Computed:
               sha256:199e461df405c68762d1b9ec6185a32bbb28f0bf3a14deab9f42c630743dfeae
               sha512:c3205a36615f29b02bbedbe5f1549952bd25b7a2d631db9fbb485fe95b9a673993f4a71fbc68e0902ba91f3930ccaa5ad0cbd19524876b50dc3dc3b39695003f
    "
    );

    Ok(())
}

/// Repeated direct URL requirements merge hashes across nested requirements files.
#[test]
fn require_hashes_repeated_hash_multiple_files() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_a = context.temp_dir.child("requirements-a.txt");
    requirements_a.write_str(&indoc::formatdoc!{ r"
        anyio @ {artifact_url_0} --hash=sha256:{artifact_hash_1}
    ", artifact_url_0 = registry_artifacts.file_url("anyio-4.0.0-py3-none-any.whl"), artifact_hash_1 = registry_artifacts.file_hash("anyio-4.0.0-py3-none-any.whl").expect("fixture distribution should exist") })?;

    let requirements_b = context.temp_dir.child("requirements-b.txt");
    requirements_b.write_str(&indoc::formatdoc!{ r"
        anyio @ {artifact_url_0} --hash=sha512:{sha512}
    ", artifact_url_0 = registry_artifacts.file_url("anyio-4.0.0-py3-none-any.whl"), sha512 = artifact_hash(&registry_artifacts, "anyio-4.0.0-py3-none-any.whl", HashAlgorithm::Sha512)? })?;

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc::indoc! { r"
        -r requirements-a.txt
        -r requirements-b.txt
    " })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.0.0 (from http://[LOCALHOST]/files/anyio-4.0.0-py3-none-any.whl)
    "
    );

    Ok(())
}

/// If a dependency is repeated, the hash should be required for both instances.
#[test]
fn require_hashes_at_least_one() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    // An MD5 digest alone must not satisfy integrity-enforced installs.
    let md5_requirements_txt = context.temp_dir.child("requirements-md5.txt");
    let md5 = artifact_hash(
        &registry_artifacts,
        "anyio-4.0.0-py3-none-any.whl",
        HashAlgorithm::Md5,
    )?;
    md5_requirements_txt.write_str(&format!("anyio==4.0.0 --hash=md5:{md5}"))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg(md5_requirements_txt.path())
        .arg("--require-hashes"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `md5` hashes are insecure and cannot be used with `--require-hashes` but no other hashes are available for: anyio==4.0.0
    ");

    // Request `anyio` with a `sha256` hash.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "anyio==4.0.0 --hash=sha256:{artifact_hash_0}",
        artifact_hash_0 = registry_artifacts
            .file_hash("anyio-4.0.0.tar.gz")
            .expect("fixture distribution should exist")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
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
    constraints_txt.write_str(&format!(
        "anyio==4.0.0 --hash=sha256:{artifact_hash_0}",
        artifact_hash_0 = registry_artifacts
            .file_hash("anyio-4.0.0.tar.gz")
            .expect("fixture distribution should exist")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
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

    // Reinstall, requesting both `sha256` and `md5`. We should reinstall from the cache, since
    // at least one hash matches.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "anyio==4.0.0 --hash=sha256:{artifact_hash_0} --hash=md5:{md5}",
        artifact_hash_0 = registry_artifacts
            .file_hash("anyio-4.0.0.tar.gz")
            .expect("fixture distribution should exist")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
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
    requirements_txt.write_str(&format!(
        "anyio==4.0.0 --hash=sha256:{artifact_hash_0} --hash=md5:1234",
        artifact_hash_0 = registry_artifacts
            .file_hash("anyio-4.0.0.tar.gz")
            .expect("fixture distribution should exist")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
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
    uv_snapshot!(context.filters(), context.pip_sync()
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
    let context = uv_test::test_context!("3.12");
    let server = FindLinksServer::new(&context.workspace_root.join("test/links"));
    let index = PackseServer::empty();

    // First, use the correct hash.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(
        "basic-package==0.1.0 --hash=sha256:7b6229db79b5800e4e98a351b5628c1c8a944533a2d428aeeaa7275a30d4ea82",
    )?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes")
        .arg("--index-url")
        .arg(index.index_url())
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
    requirements_txt.write_str("basic-package==0.1.0 --hash=sha256:123")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes")
        .arg("--index-url")
        .arg(index.index_url())
        .arg("--find-links")
        .arg(server.url()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `basic-package==0.1.0`
      cause: Hash mismatch for `basic-package==0.1.0`

             Expected:
               sha256:123

             Computed:
               sha256:7b6229db79b5800e4e98a351b5628c1c8a944533a2d428aeeaa7275a30d4ea82
    "
    );

    // Third, use the hash from the source distribution. This will actually fail, when it _could_
    // succeed, but pip has the same behavior.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(
        "basic-package==0.1.0 --hash=sha256:af478ff91ec60856c99a540b8df13d756513bebb65bc301fb27e0d1f974532b4",
    )?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes")
        .arg("--index-url")
        .arg(index.index_url())
        .arg("--find-links")
        .arg(server.url()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `basic-package==0.1.0`
      cause: Hash mismatch for `basic-package==0.1.0`

             Expected:
               sha256:af478ff91ec60856c99a540b8df13d756513bebb65bc301fb27e0d1f974532b4

             Computed:
               sha256:7b6229db79b5800e4e98a351b5628c1c8a944533a2d428aeeaa7275a30d4ea82
    "
    );

    // Fourth, use the hash from the source distribution, and disable wheels. This should succeed.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(
        "basic-package==0.1.0 --hash=sha256:af478ff91ec60856c99a540b8df13d756513bebb65bc301fb27e0d1f974532b4",
    )?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--no-binary")
        .arg(":all:")
        .arg("--reinstall")
        .arg("--require-hashes")
        .arg("--index-url")
        .arg(index.index_url())
        .arg("--find-links")
        .arg(server.url()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download and build `basic-package==0.1.0`
      cause: Failed to resolve requirements from `build-system.requires`
      cause: No solution found when resolving: `uv-build>=0.8.3, <0.9.0`
      cause: Because uv-build was not found in the package registry and you require uv-build>=0.8.3,<0.9.0, we can conclude that your requirements are unsatisfiable.
    "
    );

    Ok(())
}

/// Using `--find-links`, and the registry serves us a correct hash.
#[test]
fn require_hashes_find_links_valid_hash() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("example-a-961b4c22==1.0.0 --hash=sha256:5d69f0b590514103234f0c3526563856f04d044d8d0ea1073a843ae429b3187e")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes")
        .arg("--find-links")
        .arg("https://raw.githubusercontent.com/astral-test/astral-test-hash/main/valid-hash/simple-html/example-a-961b4c22/index.html"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + example-a-961b4c22==1.0.0
    "
    );

    Ok(())
}

/// Using `--find-links`, and the registry serves us an incorrect hash.
#[test]
fn require_hashes_find_links_invalid_hash() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // First, request some other hash.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("example-a-961b4c22==1.0.0 --hash=sha256:123")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes")
        .arg("--find-links")
        .arg("https://raw.githubusercontent.com/astral-test/astral-test-hash/main/invalid-hash/simple-html/example-a-961b4c22/index.html"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `example-a-961b4c22==1.0.0`
      cause: Hash mismatch for `example-a-961b4c22==1.0.0`

             Expected:
               sha256:123

             Computed:
               sha256:5d69f0b590514103234f0c3526563856f04d044d8d0ea1073a843ae429b3187e
    "
    );

    // Second, request the invalid hash, that the registry _thinks_ is correct. We should reject it.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("example-a-961b4c22==1.0.0 --hash=sha256:8838f9d005ff0432b258ba648d9cabb1cbdf06ac29d14f788b02edae544032ea")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes")
        .arg("--find-links")
        .arg("https://raw.githubusercontent.com/astral-test/astral-test-hash/main/invalid-hash/simple-html/example-a-961b4c22/index.html"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `example-a-961b4c22==1.0.0`
      cause: Hash mismatch for `example-a-961b4c22==1.0.0`

             Expected:
               sha256:8838f9d005ff0432b258ba648d9cabb1cbdf06ac29d14f788b02edae544032ea

             Computed:
               sha256:5d69f0b590514103234f0c3526563856f04d044d8d0ea1073a843ae429b3187e
    "
    );

    // Third, request the correct hash, that the registry _thinks_ is correct. We should accept
    // it, since it's already cached under this hash.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("example-a-961b4c22==1.0.0 --hash=sha256:5d69f0b590514103234f0c3526563856f04d044d8d0ea1073a843ae429b3187e")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes")
        .arg("--find-links")
        .arg("https://raw.githubusercontent.com/astral-test/astral-test-hash/main/invalid-hash/simple-html/example-a-961b4c22/index.html"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + example-a-961b4c22==1.0.0
    "
    );

    // Fourth, request the correct hash, that the registry _thinks_ is correct, but without the
    // cache. We _should_ accept it, but we currently don't.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("example-a-961b4c22==1.0.0 --hash=sha256:5d69f0b590514103234f0c3526563856f04d044d8d0ea1073a843ae429b3187e")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--refresh")
        .arg("--reinstall")
        .arg("--require-hashes")
        .arg("--find-links")
        .arg("https://raw.githubusercontent.com/astral-test/astral-test-hash/main/invalid-hash/simple-html/example-a-961b4c22/index.html"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ example-a-961b4c22==1.0.0
    "
    );

    // Finally, request the correct hash, along with the incorrect hash for the source distribution.
    // Resolution will fail, since the incorrect hash matches the registry's hash.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("example-a-961b4c22==1.0.0 --hash=sha256:5d69f0b590514103234f0c3526563856f04d044d8d0ea1073a843ae429b3187e --hash=sha256:a3cf07a05aac526131a2e8b6e4375ee6c6eaac8add05b88035e960ac6cd999ee")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
        .arg("--refresh")
        .arg("--reinstall")
        .arg("--require-hashes")
        .arg("--find-links")
        .arg("https://raw.githubusercontent.com/astral-test/astral-test-hash/main/invalid-hash/simple-html/example-a-961b4c22/index.html"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download and build `example-a-961b4c22==1.0.0`
      cause: Hash mismatch for `example-a-961b4c22==1.0.0`

             Expected:
               sha256:5d69f0b590514103234f0c3526563856f04d044d8d0ea1073a843ae429b3187e
               sha256:a3cf07a05aac526131a2e8b6e4375ee6c6eaac8add05b88035e960ac6cd999ee

             Computed:
               sha256:294e788dbe500fdc39e8b88e82652ab67409a1dc9dd06543d0fe0ae31b713eb3
    "
    );

    Ok(())
}

/// Using `--index-url`, but the registry doesn't provide us with a hash.
#[test]
fn require_hashes_registry_no_hash() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let scenario = uv_test::packse::scenario::Scenario::from_path(
        &context
            .workspace_root
            .join("test/scenarios/packages/pip-sync-hashes.toml"),
    )?;
    let server = PackseServer::from_scenario_without_hashes(&scenario);
    let wheel_hash = artifact_hash(
        &server,
        "hash_package-1.0.0-py3-none-any.whl",
        HashAlgorithm::Sha256,
    )?;

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!("hash-package==1.0.0 --hash=sha256:{wheel_hash}"))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("requirements.txt")
        .arg("--require-hashes")
        .arg("--index-url")
        .arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + hash-package==1.0.0
    "
    );

    Ok(())
}

/// Using `--index-url`, and the registry serves us a correct hash.
#[test]
fn require_hashes_registry_valid_hash() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let scenario = uv_test::packse::scenario::Scenario::from_path(
        &context
            .workspace_root
            .join("test/scenarios/packages/pip-sync-hashes.toml"),
    )?;
    let server = PackseServer::from_scenario(&scenario);
    let wheel_hash = artifact_hash(
        &server,
        "hash_package-1.0.0-py3-none-any.whl",
        HashAlgorithm::Sha256,
    )?;

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!("hash-package==1.0.0 --hash=sha256:{wheel_hash}"))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("requirements.txt")
        .arg("--require-hashes")
        .arg("--index-url")
        .arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + hash-package==1.0.0
    "
    );

    Ok(())
}

/// A `--find-links` page without distribution links does not act as a package index.
#[test]
fn require_hashes_empty_find_links() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let links = context.temp_dir.child("links");
    links.create_dir_all()?;
    let server = FindLinksServer::new(links.path());
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("hash-package==1.0.0 --hash=sha256:123")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes")
        .arg("--find-links")
        .arg(server.url()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving dependencies
      cause: Because hash-package was not found in the package registry and you require hash-package==1.0.0, we can conclude that your requirements are unsatisfiable.
    ");

    Ok(())
}

/// Using `--index-url`, and the registry serves us an incorrect hash.
#[test]
fn require_hashes_registry_invalid_hash() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let scenario = uv_test::packse::scenario::Scenario::from_path(
        &context
            .workspace_root
            .join("test/scenarios/packages/pip-sync-hashes.toml"),
    )?;
    let server = PackseServer::from_scenario_with_hash_overrides(
        &scenario,
        &[
            (
                "hash_package-1.0.0-py3-none-any.whl",
                "8838f9d005ff0432b258ba648d9cabb1cbdf06ac29d14f788b02edae544032ea",
            ),
            (
                "hash_package-1.0.0.tar.gz",
                "a3cf07a05aac526131a2e8b6e4375ee6c6eaac8add05b88035e960ac6cd999ee",
            ),
        ],
    )?;
    let wheel_hash = artifact_hash(
        &server,
        "hash_package-1.0.0-py3-none-any.whl",
        HashAlgorithm::Sha256,
    )?;

    // First, request some other hash.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("hash-package==1.0.0 --hash=sha256:123")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes")
        .arg("--index-url")
        .arg(server.index_url()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `hash-package==1.0.0`
      cause: Hash mismatch for `hash-package==1.0.0`

             Expected:
               sha256:123

             Computed:
               sha256:3602f1781d7745b35f66987a26ef6164809800c5fbf208740b47ad3179b88386
    "
    );

    // Second, request the invalid hash, that the registry _thinks_ is correct. We should reject it.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str("hash-package==1.0.0 --hash=sha256:8838f9d005ff0432b258ba648d9cabb1cbdf06ac29d14f788b02edae544032ea")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes")
        .arg("--index-url")
        .arg(server.index_url()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `hash-package==1.0.0`
      cause: Hash mismatch for `hash-package==1.0.0`

             Expected:
               sha256:8838f9d005ff0432b258ba648d9cabb1cbdf06ac29d14f788b02edae544032ea

             Computed:
               sha256:3602f1781d7745b35f66987a26ef6164809800c5fbf208740b47ad3179b88386
    "
    );

    // Third, request the correct hash, that the registry _thinks_ is correct. We should accept
    // it, since it's already cached under this hash.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!("hash-package==1.0.0 --hash=sha256:{wheel_hash}"))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("requirements.txt")
        .arg("--reinstall")
        .arg("--require-hashes")
        .arg("--index-url")
        .arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + hash-package==1.0.0
    "
    );

    // Fourth, request the correct hash, that the registry _thinks_ is correct, but without the
    // cache. We _should_ accept it, but we currently don't.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!("hash-package==1.0.0 --hash=sha256:{wheel_hash}"))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("requirements.txt")
        .arg("--refresh")
        .arg("--reinstall")
        .arg("--require-hashes")
        .arg("--index-url")
        .arg(server.index_url()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ hash-package==1.0.0
    "
    );

    // Finally, request the correct hash, along with the incorrect hash for the source distribution.
    // Resolution will fail, since the incorrect hash matches the registry's hash.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(&format!("hash-package==1.0.0 --hash=sha256:{wheel_hash} --hash=sha256:a3cf07a05aac526131a2e8b6e4375ee6c6eaac8add05b88035e960ac6cd999ee"))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("requirements.txt")
        .arg("--refresh")
        .arg("--reinstall")
        .arg("--require-hashes")
        .arg("--index-url")
        .arg(server.index_url()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download and build `hash-package==1.0.0`
      cause: Hash mismatch for `hash-package==1.0.0`

             Expected:
               sha256:3602f1781d7745b35f66987a26ef6164809800c5fbf208740b47ad3179b88386
               sha256:a3cf07a05aac526131a2e8b6e4375ee6c6eaac8add05b88035e960ac6cd999ee

             Computed:
               sha256:e1f4ab699f853c66e1dbb62d4fcbe5d437706850a19b4ef10e19d50fa3a2fff9
    "
    );

    Ok(())
}

/// Include the hash in the URL directly.
#[test]
fn require_hashes_url() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12").with_exclude_newer("2025-01-29T00:00:00Z");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "iniconfig @ {artifact_url_0}#sha256={artifact_hash_1}",
        artifact_url_0 = registry_artifacts.file_url("iniconfig-2.0.0-py3-none-any.whl"),
        artifact_hash_1 = registry_artifacts
            .file_hash("iniconfig-2.0.0-py3-none-any.whl")
            .expect("fixture distribution should exist")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0 (from http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl#sha256=8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c)
    "
    );

    Ok(())
}

/// Include an irrelevant fragment in the URL.
#[test]
fn require_hashes_url_other_fragment() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12").with_exclude_newer("2025-01-29T00:00:00Z");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "iniconfig @ {artifact_url_0}#foo=bar",
        artifact_url_0 = registry_artifacts.file_url("iniconfig-2.0.0-py3-none-any.whl")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have a hash, but none were provided for: iniconfig @ http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl#foo=bar
    "
    );

    Ok(())
}

/// Include an invalid hash in the URL directly.
#[test]
fn require_hashes_url_invalid() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12").with_exclude_newer("2025-01-29T00:00:00Z");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(&format!("iniconfig @ {artifact_url_0}#sha256=c6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374", artifact_url_0 = registry_artifacts.file_url("iniconfig-2.0.0-py3-none-any.whl")))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to download `iniconfig @ http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl#sha256=c6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374`
      cause: Hash mismatch for `iniconfig @ http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl#sha256=c6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374`

             Expected:
               sha256:c6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374

             Computed:
               sha256:8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c
    "
    );

    Ok(())
}

/// Merge the hash on the fragment with hashes provided directly.
#[test]
fn require_hashes_url_merge() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12").with_exclude_newer("2025-01-29T00:00:00Z");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "anyio @ {artifact_url_0}#sha256={artifact_hash_1} --hash sha512:{sha512}",
        artifact_url_0 = registry_artifacts.file_url("anyio-4.0.0-py3-none-any.whl"),
        artifact_hash_1 = registry_artifacts
            .file_hash("anyio-4.0.0-py3-none-any.whl")
            .expect("fixture distribution should exist"),
        sha512 = artifact_hash(
            &registry_artifacts,
            "anyio-4.0.0-py3-none-any.whl",
            HashAlgorithm::Sha512
        )?
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + anyio==4.0.0 (from http://[LOCALHOST]/files/anyio-4.0.0-py3-none-any.whl#sha256=199e461df405c68762d1b9ec6185a32bbb28f0bf3a14deab9f42c630743dfeae)
    "
    );

    Ok(())
}

/// Include the hash in the URL directly.
#[test]
fn require_hashes_url_unnamed() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12").with_exclude_newer("2025-01-29T00:00:00Z");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "{artifact_url_0}#sha256={artifact_hash_1}",
        artifact_url_0 = registry_artifacts.file_url("iniconfig-2.0.0-py3-none-any.whl"),
        artifact_hash_1 = registry_artifacts
            .file_hash("iniconfig-2.0.0-py3-none-any.whl")
            .expect("fixture distribution should exist")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--require-hashes"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0 (from http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl#sha256=8a0fc44e516906bdecc91af1c3bc12134c9d1647a482446edc62f2f72191416c)
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

    // Ensure that the package is present in the target directory.
    assert!(context.temp_dir.child("target").child("iniconfig").is_dir());

    // Ensure that we can't import the package.
    context.assert_command("import iniconfig").failure();

    // Ensure that we can import the package by augmenting the `PYTHONPATH`.
    context
        .python_command()
        .arg("-c")
        .arg("import iniconfig")
        .env(EnvVars::PYTHONPATH, context.temp_dir.child("target").path())
        .assert()
        .success();

    // Upgrade it.
    let requirements_in = context.temp_dir.child("requirements.in");
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

    // Remove it, and replace with `flask`, which includes a binary.
    let requirements_in = context.temp_dir.child("requirements.in");
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
    // Ensure that the binary is present in the target directory.
    assert!(
        context
            .temp_dir
            .child("target")
            .child("bin")
            .child(format!("flask{EXE_SUFFIX}"))
            .is_file()
    );

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

    // Ensure that the build requirements are not present in the target directory.
    assert!(!context.temp_dir.child("target").child("hatchling").is_dir());

    // Ensure that the package is present in the target directory.
    assert!(context.temp_dir.child("target").child("iniconfig").is_dir());

    // Ensure that we can't import the package.
    context.assert_command("import iniconfig").failure();

    // Ensure that we can import the package by augmenting the `PYTHONPATH`.
    context
        .python_command()
        .arg("-c")
        .arg("import iniconfig")
        .env(EnvVars::PYTHONPATH, context.temp_dir.child("target").path())
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
     + wheel==0.42.0
    ");

    // Ensure that the build requirements are not present in the target directory.
    assert!(!context.temp_dir.child("target").child("flit_core").is_dir());

    // Ensure that the package is present in the target directory.
    assert!(context.temp_dir.child("target").child("wheel").is_dir());

    // Ensure that we can't import the package.
    context.assert_command("import wheel").failure();

    // Ensure that we can import the package by augmenting the `PYTHONPATH`.
    context
        .python_command()
        .arg("-c")
        .arg("import wheel")
        .env(EnvVars::PYTHONPATH, context.temp_dir.child("target").path())
        .assert()
        .success();

    Ok(())
}

/// Sync to a `--target` directory without a virtual environment.
#[test]
fn target_system() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&["3.12"]);

    // Install `iniconfig` to the target directory.
    let requirements_in = context.temp_dir.child("requirements.in");
    requirements_in.write_str("iniconfig==2.0.0")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.in")
        .arg("--target")
        .arg("target"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: [PYTHON-3.12]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    // Ensure that the package is present in the target directory.
    assert!(context.temp_dir.child("target").child("iniconfig").is_dir());

    Ok(())
}

/// Sync to a `--prefix` directory.
#[test]
fn prefix() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_filtered_python_names()
        .with_filtered_virtualenv_bin()
        .with_filtered_exe_suffix();

    // Install `iniconfig` to the prefix directory.
    let requirements_in = context.temp_dir.child("requirements.in");
    requirements_in.write_str("iniconfig==2.0.0")?;

    let prefix = context.temp_dir.child("prefix");

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.in")
        .arg("--prefix")
        .arg(prefix.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.12.[X] interpreter at: .venv/[BIN]/[PYTHON]
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    // Ensure that we can't import the package.
    context.assert_command("import iniconfig").failure();

    // Ensure that we can import the package by augmenting the `PYTHONPATH`.
    context
        .python_command()
        .arg("-c")
        .arg("import iniconfig")
        .env(
            EnvVars::PYTHONPATH,
            site_packages_path(&context.temp_dir.join("prefix"), "python3.12"),
        )
        .assert()
        .success();

    // Upgrade it.
    let requirements_in = context.temp_dir.child("requirements.in");
    requirements_in.write_str("iniconfig==1.1.1")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.in")
        .arg("--prefix")
        .arg(prefix.path()), @"
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

    Ok(())
}

/// Ensure that we install packages with markers on them.
#[test]
fn preserve_markers() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("anyio ; python_version > '3.7'")?;

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

    Ok(())
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
    constraints_txt.write_str("setuptools>=40")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt")
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

    // Syncing should remove the seed packages.
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

    // Re-create the environment with seed packages.
    uv_snapshot!(context.filters(), context.venv().arg("--clear")
        .arg("--seed"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Using CPython 3.9.[X] interpreter at: [PYTHON-3.9]
    Creating virtual environment with seed packages at: .venv
     + pip==24.0
     + setuptools==69.2.0
     + wheel==0.42.0
    Activate with: source .venv/[BIN]/activate
    "
    );

    // Syncing should retain the seed packages.
    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Installed 1 package in [TIME]
     + requests==1.2.0
    "
    );

    Ok(())
}

/// Sanitize zip files during extraction.
#[test]
fn sanitize() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Install a zip file that includes a path that extends outside the parent.
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("payload-package @ https://github.com/astral-sh/sanitize-wheel-test/raw/bc59283d5b4b136a191792e32baa51b477fdf65e/payload_package-0.1.0-py3-none-any.whl")?;

    uv_snapshot!(context.pip_sync()
        .arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + payload-package==0.1.0 (from https://github.com/astral-sh/sanitize-wheel-test/raw/bc59283d5b4b136a191792e32baa51b477fdf65e/payload_package-0.1.0-py3-none-any.whl)
    "
    );

    // There should be no `payload` file in the root.
    if let Some(parent) = context.temp_dir.parent() {
        assert!(!parent.join("payload").exists());
    }

    Ok(())
}

/// Allow semicolons attached to markers, as long as they're preceded by a space.
#[test]
fn semicolon_trailing_space() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements = context.temp_dir.child("requirements.txt");
    requirements.write_str(&format!(
        "iniconfig @ {artifact_url_0}; python_version > '3.10'",
        artifact_url_0 = registry_artifacts.file_url("iniconfig-2.0.0-py3-none-any.whl")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0 (from http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl)
    "
    );

    Ok(())
}

/// Treat a semicolon that's not whitespace-separated as a part of the URL.
#[test]
fn semicolon_no_space() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let requirements = context.temp_dir.child("requirements.txt");
    requirements.write_str(&format!(
        "iniconfig @ {artifact_url_0};python_version > '3.10'",
        artifact_url_0 = registry_artifacts.file_url("iniconfig-2.0.0-py3-none-any.whl")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Couldn't parse requirement in `requirements.txt` at position 0
      cause: Expected direct URL (`http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl;python_version%20%3E%20'3.10'`) to end in a supported file extension: `.whl`, `.tar.gz`, `.zip`, `.tar.bz2`, `.tar.lz`, `.tar.lzma`, `.tar.xz`, `.tar.zst`, `.tar`, `.tbz`, `.tgz`, `.tlz`, or `.txz`
             iniconfig @ http://[LOCALHOST]/files/iniconfig-2.0.0-py3-none-any.whl;python_version > '3.10'
                         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    "
    );

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
fn pep_751_validates_remote_archive_size() -> Result<()> {
    let server = PackseServer::new("simple/single-package.toml");
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("pylock.toml").write_str(&formatdoc! {
        r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "a"
        version = "1.0.0"
        archive = {{ url = "{wheel_url}", size = 1, hashes = {{ sha256 = "f936eedc194aa91ca01a4c6c9981136ca6c75ce6df47e3951b12522881dce809" }} }}
        "#,
        wheel_url = server.file_url("a-1.0.0-py3-none-any.whl"),
    })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`
      cause: Size mismatch for `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`: expected 1 bytes, but downloaded 921 bytes
    ");

    context.temp_dir.child("pylock.toml").write_str(&formatdoc! {
        r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "a"
        version = "1.0.0"
        wheels = [{{ url = "{wheel_url}", size = 1, hashes = {{ sha256 = "f936eedc194aa91ca01a4c6c9981136ca6c75ce6df47e3951b12522881dce809" }} }}]
        "#,
        wheel_url = server.file_url("a-1.0.0-py3-none-any.whl"),
    })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `a==1.0.0`
      cause: Size mismatch for `a==1.0.0`: expected 1 bytes, but downloaded 921 bytes
    ");

    context.temp_dir.child("pylock.toml").write_str(&formatdoc! {
        r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "a"
        version = "1.0.0"
        sdist = {{ url = "{sdist_url}", size = 1, hashes = {{ sha256 = "3d2b4c28a4e112f3a1cef1db4dc5efa33fcbbcc38bc11ccc80321097db86c097" }} }}
        "#,
        sdist_url = server.file_url("a-1.0.0.tar.gz"),
    })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("pylock.toml"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download and build `a==1.0.0`
      cause: Size mismatch for `a==1.0.0`: expected 1 bytes, but downloaded 607 bytes
    ");

    Ok(())
}

#[test]
fn pep_751_validates_cached_remote_archive_size() -> Result<()> {
    let server = PackseServer::new("simple/single-package.toml");
    let context = uv_test::test_context!("3.12");
    let wheel_url = server.file_url("a-1.0.0-py3-none-any.whl");

    context
        .pip_install()
        .arg(format!("a @ {wheel_url}"))
        .assert()
        .success();

    context.temp_dir.child("pylock.toml").write_str(&formatdoc! {
        r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "a"
        version = "1.0.0"
        archive = {{ url = "{wheel_url}", size = 1, hashes = {{ sha256 = "f936eedc194aa91ca01a4c6c9981136ca6c75ce6df47e3951b12522881dce809" }} }}
        "#,
    })?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg("--offline")
        .arg("--reinstall")
        .arg("pylock.toml"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`
      cause: Size mismatch for `a @ http://[LOCALHOST]/files/a-1.0.0-py3-none-any.whl`: expected 1 bytes, but downloaded 921 bytes
    ");

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

    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio==4.3.0"]
    "#})?;
    context
        .export()
        .arg("-o")
        .arg("pylock.toml")
        .assert()
        .success();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/pylock.toml"))
        .respond_with(ResponseTemplate::new(200).set_body_string(context.read("pylock.toml")))
        .mount(&server)
        .await;

    let pylock_url = format!("{}/pylock.toml", server.uri());

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("--preview")
        .arg(&pylock_url), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
    "
    );

    Ok(())
}

/// Avoid erroring for packages that only include wheels, and _don't_ include a wheel for the
/// current platform, but are omitted by markers anyway.
///
/// See: <https://github.com/astral-sh/uv/issues/13127>
#[test]
fn pep_751_wheel_only() -> Result<()> {
    let context =
        uv_test::test_context!("3.12").with_packse_index("packages/pip-sync-wheel-only.toml");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12.0"
        dependencies = ["platform-wheel"]
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
    Would download 1 package
    Would install 1 package
     + platform-wheel==1.0.0
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
    error: Package `platform-wheel` can't be installed because it doesn't have a source distribution or wheel for the current platform

    hint: You're using CPython 3.8 (`cp38`), but `platform-wheel` (v1.0.0) only has wheels with the following Python implementation tag: `cp312`
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
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + sniffio==1.3.1
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
    Uninstalled 3 packages in [TIME]
    Installed 1 package in [TIME]
     - anyio==4.3.0
     - idna==3.6
     - sniffio==1.3.1
     + source-distribution==0.0.3
    "
    );

    Ok(())
}

#[test]
fn pep_751_direct_url_tags() -> Result<()> {
    let registry_artifacts = PackseServer::new("packages/pip-install.toml");
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&format!(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["MarkupSafe @ {artifact_url_0}"]
        "#,
        artifact_url_0 =
            registry_artifacts.file_url("markupsafe-3.0.2-cp312-cp312-macosx_11_0_arm64.whl")
    ))?;

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
      cause: A URL (http://[LOCALHOST]/files/markupsafe-3.0.2-cp312-cp312-macosx_11_0_arm64.whl) dependency is incompatible with the current platform

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
     + markupsafe==3.0.2 (from http://[LOCALHOST]/files/markupsafe-3.0.2-cp312-cp312-macosx_11_0_arm64.whl)
    "
    );

    Ok(())
}

#[test]
fn incompatible_python_version_direct_url() -> Result<()> {
    let server = PackseServer::new("packages/pip-sync-wheel-only.toml");
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "windows-wheel @ {}",
        server.file_url("windows_wheel-1.0.0-cp313-cp313-win32.whl")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--python-platform")
        .arg("windows"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to determine installation plan
      cause: A URL (http://[LOCALHOST]/files/windows_wheel-1.0.0-cp313-cp313-win32.whl) dependency is incompatible with the current platform

    hint: The wheel is compatible with CPython 3.13 (`cp313`), but you're using CPython 3.12 (`cp312`)
    "
    );

    Ok(())
}

#[test]
fn incompatible_direct_url_redacts_credentials() -> Result<()> {
    let server = PackseServer::new("packages/pip-sync-wheel-only.toml");
    let context = uv_test::test_context!("3.12");

    let mut wheel_url = Url::parse(&server.file_url("windows_wheel-1.0.0-cp313-cp313-win32.whl"))?;
    wheel_url
        .set_username("user")
        .expect("HTTP URL accepts a username");
    wheel_url
        .set_password(Some("secret"))
        .expect("HTTP URL accepts a password");
    wheel_url
        .query_pairs_mut()
        .append_pair("X-Amz-Signature", "signing-secret");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!("windows-wheel @ {wheel_url}"))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--python-platform")
        .arg("windows"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to determine installation plan
      cause: A URL (http://user:****@[LOCALHOST]/files/windows_wheel-1.0.0-cp313-cp313-win32.whl?X-Amz-Signature=****) dependency is incompatible with the current platform

    hint: The wheel is compatible with CPython 3.13 (`cp313`), but you're using CPython 3.12 (`cp312`)
    "
    );

    Ok(())
}

#[test]
fn incompatible_platform_direct_url() -> Result<()> {
    let server = PackseServer::new("packages/pip-sync-wheel-only.toml");
    let context = uv_test::test_context!("3.13");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(&format!(
        "windows-wheel @ {}",
        server.file_url("windows_wheel-1.0.0-cp313-cp313-win32.whl")
    ))?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .arg("requirements.txt")
        .arg("--python-platform")
        .arg("linux"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: Failed to determine installation plan
      cause: A URL (http://[LOCALHOST]/files/windows_wheel-1.0.0-cp313-cp313-win32.whl) dependency is incompatible with the current platform

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
