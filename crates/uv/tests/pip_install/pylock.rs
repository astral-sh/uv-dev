//! Installation from PEP 751 lockfiles (`pylock.toml`).

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
use fs_err as fs;
use indoc::indoc;
use sha2::{Digest, Sha256};

#[cfg(feature = "test-pypi")]
use uv_test::download_to_disk;
use uv_test::uv_snapshot;

#[test]
fn invalid_toml_filename() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let test_toml = context.temp_dir.child("test.toml");
    test_toml.touch()?;

    uv_snapshot!(context.pip_install()
        .arg("-r")
        .arg("test.toml"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `test.toml` is not a valid PEP 751 filename: expected `pylock.toml` or `pylock.<name>.toml`, where `<name>` is non-empty and contains no dots
    "
    );

    Ok(())
}

#[test]
fn invalid_pylock_toml_filename() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("pylock..toml").touch()?;
    context.temp_dir.child("pylock.foo.bar.toml").touch()?;

    uv_snapshot!(context.pip_install()
        .arg("-r")
        .arg("pylock..toml"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `pylock..toml` is not a valid PEP 751 filename: expected `pylock.toml` or `pylock.<name>.toml`, where `<name>` is non-empty and contains no dots
    "
    );

    uv_snapshot!(context.pip_install()
        .arg("-r")
        .arg("pylock.foo.bar.toml"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `pylock.foo.bar.toml` is not a valid PEP 751 filename: expected `pylock.toml` or `pylock.<name>.toml`, where `<name>` is non-empty and contains no dots
    "
    );

    Ok(())
}

#[cfg(feature = "test-pypi")]
#[test]
fn pep_751_install_registry_wheel() -> Result<()> {
    let context = uv_test::test_context!("3.12");

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

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    "
    );

    // An unrelated distribution should not be inspected when installing from a `pylock.toml`.
    let unrelated = context.site_packages().join("unrelated-1.0.0.dist-info");
    fs::create_dir_all(&unrelated)?;
    fs::write(unrelated.join("direct_url.json"), "invalid")?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    "
    );

    Ok(())
}

#[cfg(feature = "test-pypi")]
#[test]
fn pep_751_install_registry_sdist() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_exclude_newer("2025-01-29T00:00:00Z");

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

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + source-distribution==0.0.3
    "
    );

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    "
    );

    Ok(())
}

#[test]
fn pep_751_install_invalid_artifact_urls() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pylock_toml = context.temp_dir.child("pylock.toml");
    pylock_toml.write_str(
        r#"
        lock-version = "1.0"
        created-by = "uv"
        requires-python = ">=3.12"

        [[packages]]
        name = "foo"
        version = "1.0.0"
        wheels = [{ name = "foo-1.0.0-py3-none-any.whl", url = "data:application/octet-stream,ignored", hashes = { sha256 = "0000000000000000000000000000000000000000000000000000000000000000" } }]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Invalid artifact URL: data:application/octet-stream,ignored
    "
    );

    pylock_toml.write_str(
        r#"
        lock-version = "1.0"
        created-by = "uv"
        requires-python = ">=3.12"

        [[packages]]
        name = "foo"
        version = "1.0.0"
        sdist = { name = "foo-1.0.0.tar.gz", url = "data:application/octet-stream,ignored", hashes = { sha256 = "0000000000000000000000000000000000000000000000000000000000000000" } }
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Invalid artifact URL: data:application/octet-stream,ignored
    "
    );

    Ok(())
}

#[test]
fn pep_751_install_invalid_hashes() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let pylock_toml = context.temp_dir.child("pylock.toml");

    pylock_toml.write_str(
        r#"
        lock-version = "1.0"
        created-by = "uv"
        requires-python = ">=3.12"

        [[packages]]
        name = "foo"
        version = "1.0.0"
        wheels = [{ name = "foo-1.0.0-py3-none-any.whl", url = "https://example.com/foo-1.0.0-py3-none-any.whl", hashes = { sha256 = "short" } }]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("-r")
        .arg("pylock.toml"), @r###"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Not a valid `pylock.toml` file: pylock.toml
      cause: TOML parse error at line 9, column 123
               |
             9 |         wheels = [{ name = "foo-1.0.0-py3-none-any.whl", url = "https://example.com/foo-1.0.0-py3-none-any.whl", hashes = { sha256 = "short" } }]
               |                                                                                                                           ^^^^^^^^^^^^^^^^^^^^
             Invalid hash digest length (expected 64 hexadecimal characters, found 5)
             in `sha256`
    "###);

    Ok(())
}

#[cfg(feature = "test-pypi")]
#[test]
fn pep_751_install_directory() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Create a local dependency in a subdirectory.
    let pyproject_toml = context.temp_dir.child("foo").child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "foo"
        version = "1.0.0"
        dependencies = ["anyio"]

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

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["foo"]

        [tool.uv.sources]
        foo = { path = "foo" }
        "#,
    )?;

    context
        .export()
        .arg("-o")
        .arg("pylock.toml")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + foo==1.0.0 (from file://[TEMP_DIR]/foo)
     + idna==3.6
     + sniffio==1.3.1
    "
    );

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 4 packages in [TIME]
    "
    );

    Ok(())
}

#[test]
fn pep_751_install_require_hashes_directory() -> Result<()> {
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

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("-r")
        .arg("pylock.toml")
        .arg("--require-hashes"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have a hash, but none were provided for: foo
    "
    );

    Ok(())
}

#[test]
#[cfg(all(feature = "test-universal", feature = "test-git"))]
fn pep_751_install_git_rejects_mismatched_exact_revision() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    context.temp_dir.child("pylock.toml").write_str(indoc! {r#"
        lock-version = "1.0"
        created-by = "uv"
        requires-python = ">=3.12"

        [[packages]]
        name = "example"
        version = "1.0.0"
        vcs = { type = "git", url = "https://git:secret-token@example.com/pkg.git", requested-revision = "0dacfd662c64cb4ceb16e6cf65a157a8b715b979", commit-id = "b270df1a2fb5d012294e9aaf05e7e0bab1e6a389" }
    "#})?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Exact Git revision `0dacfd662c64cb4ceb16e6cf65a157a8b715b979` does not match precise commit `b270df1a2fb5d012294e9aaf05e7e0bab1e6a389` for `https://git:****@example.com/pkg.git`
    ");

    context.temp_dir.child("requirements.in").write_str(
        "example @ git+https://git:secret-token@example.com/pkg.git@0dacfd662c64cb4ceb16e6cf65a157a8b715b979\n",
    )?;

    uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in")
        .arg("--universal")
        .arg("--offline")
        .arg("-o")
        .arg("pylock.toml"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Exact Git revision `0dacfd662c64cb4ceb16e6cf65a157a8b715b979` does not match precise commit `b270df1a2fb5d012294e9aaf05e7e0bab1e6a389` for `https://git:****@example.com/pkg.git`
    ");

    Ok(())
}

#[test]
#[cfg(all(feature = "test-git", feature = "test-pypi"))]
fn pep_751_install_git() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["uv-public-pypackage @ git+https://github.com/astral-test/uv-public-pypackage.git@0.0.1"]
        "#,
    )?;

    context
        .export()
        .arg("-o")
        .arg("pylock.toml")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage.git@0dacfd662c64cb4ceb16e6cf65a157a8b715b979)
    "
    );

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    "
    );

    Ok(())
}

#[cfg(feature = "test-pypi")]
#[test]
fn pep_751_install_url_wheel() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["anyio @ https://files.pythonhosted.org/packages/14/fd/2f20c40b45e4fb4324834aea24bd4afdf1143390242c0b33774da0e2e34f/anyio-4.3.0-py3-none-any.whl"]
        "#,
    )?;

    context
        .export()
        .arg("-o")
        .arg("pylock.toml")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 2 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.3.0 (from https://files.pythonhosted.org/packages/14/fd/2f20c40b45e4fb4324834aea24bd4afdf1143390242c0b33774da0e2e34f/anyio-4.3.0-py3-none-any.whl)
     + idna==3.6
     + sniffio==1.3.1
    "
    );

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 3 packages in [TIME]
    "
    );

    Ok(())
}

#[cfg(feature = "test-pypi")]
#[test]
fn pep_751_install_url_sdist() -> Result<()> {
    let server = uv_test::packse::PackseServer::new("simple/single-package.toml");
    let context = uv_test::test_context!("3.12");

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(&indoc::formatdoc! {
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["a @ {sdist_url}"]
        "#,
        sdist_url = server.file_url("a-1.0.0.tar.gz"),
    })?;

    context
        .export()
        .arg("--index-url")
        .arg(server.index_url())
        .arg("-o")
        .arg("pylock.toml")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--index-url")
        .arg(server.index_url())
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + a==1.0.0 (from http://[LOCALHOST]/files/a-1.0.0.tar.gz)
    "
    );

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--index-url")
        .arg(server.index_url())
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    "
    );

    Ok(())
}

#[test]
fn pep_751_install_path_wheel() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let filename = "ok-1.0.0-py3-none-any.whl";
    let wheel = context.read_bytes(context.workspace_root.join("test/links").join(filename));
    let sha256 = hex::encode(Sha256::digest(&wheel));
    context.temp_dir.child(filename).write_binary(&wheel)?;
    let context = context.with_filter((sha256, "[WHEEL_SHA256]"));

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["ok"]

        [tool.uv.sources]
        ok = { path = "ok-1.0.0-py3-none-any.whl" }
        "#,
    )?;

    context
        .export()
        .arg("--offline")
        .arg("-o")
        .arg("pylock.toml")
        .assert()
        .success();

    let lock = context.read("pylock.toml");

    insta::with_settings!({
        filters => context.filters(),
    }, {
        insta::assert_snapshot!(
            lock, @r#"
        # This file was autogenerated by uv via the following command:
        #    uv export --cache-dir [CACHE_DIR] --offline -o pylock.toml
        lock-version = "1.0"
        created-by = "uv"
        requires-python = ">=3.12"

        [[packages]]
        name = "ok"
        version = "1.0.0"
        archive = { path = "ok-1.0.0-py3-none-any.whl", hashes = { sha256 = "[WHEEL_SHA256]" } }
        "#
        );
    });

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Installed 1 package in [TIME]
     + ok==1.0.0 (from file://[TEMP_DIR]/ok-1.0.0-py3-none-any.whl)
    "
    );

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    "
    );

    Ok(())
}

#[test]
fn pep_751_prefers_path_over_url() -> Result<()> {
    let context = uv_test::test_context!("3.13");

    for wheel in [
        "ok-1.0.0-py3-none-any.whl",
        "basic_package-0.1.0-py3-none-any.whl",
    ] {
        fs::copy(
            context.workspace_root.join("test/links").join(wheel),
            context.temp_dir.child(wheel),
        )?;
    }

    context.temp_dir.child("pylock.toml").write_str(
        r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "ok"
        version = "1.0.0"
        archive = { path = "ok-1.0.0-py3-none-any.whl", url = "https://example.invalid/ok-1.0.0.tar.gz", hashes = { sha256 = "79f0b33e6ce1e09eaa1784c8eee275dfe84d215d9c65c652f07c18e85fdaac5f" } }

        [[packages]]
        name = "basic-package"
        version = "0.1.0"
        wheels = [{ path = "basic_package-0.1.0-py3-none-any.whl", url = "https://example.invalid/basic_package-0.1.0-py3-none-any.whl", hashes = { sha256 = "7b6229db79b5800e4e98a351b5628c1c8a944533a2d428aeeaa7275a30d4ea82" } }]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("--no-build")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + basic-package==0.1.0
     + ok==1.0.0 (from file://[TEMP_DIR]/ok-1.0.0-py3-none-any.whl)
    ");

    Ok(())
}

#[cfg(feature = "test-pypi")]
#[test]
fn pep_751_install_path_sdist() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Download the source.
    let archive = context.temp_dir.child("iniconfig-2.0.0.tar.gz");
    download_to_disk(
        "https://files.pythonhosted.org/packages/d7/4b/cbd8e699e64a6f16ca3a8220661b5f83792b3017d0f79807cb8708d33913/iniconfig-2.0.0.tar.gz",
        &archive,
    );

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(
        r#"
        [project]
        name = "project"
        version = "0.1.0"
        requires-python = ">=3.12"
        dependencies = ["iniconfig"]

        [tool.uv.sources]
        iniconfig = { path = "iniconfig-2.0.0.tar.gz" }
        "#,
    )?;

    context
        .export()
        .arg("-o")
        .arg("pylock.toml")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0 (from file://[TEMP_DIR]/iniconfig-2.0.0.tar.gz)
    "
    );

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    "
    );

    Ok(())
}

#[test]
fn pep_751_empty_archive_hashes() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("pylock.toml").write_str(indoc! {r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "iniconfig"
        version = "2.0.0"
        [packages.archive]
        url = "https://example.com/iniconfig-2.0.0.tar.gz"
        hashes = {}
    "#})?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("--require-hashes")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: Empty hash tables in `pylock.toml` will be rejected in a future uv version. Rerun the original `uv export` or `uv pip compile` command to regenerate the file.
    error: In `--require-hashes` mode, all requirements must have a hash, but none were provided for: iniconfig
    ");

    Ok(())
}

#[test]
fn pep_751_empty_sdist_hashes() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("pylock.toml").write_str(indoc! {r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "iniconfig"
        version = "2.0.0"
        [packages.sdist]
        url = "https://example.com/iniconfig-2.0.0.tar.gz"
        hashes = {}
    "#})?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("--require-hashes")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: Empty hash tables in `pylock.toml` will be rejected in a future uv version. Rerun the original `uv export` or `uv pip compile` command to regenerate the file.
    error: In `--require-hashes` mode, all requirements must have a hash, but none were provided for: iniconfig
    ");

    Ok(())
}

#[test]
fn pep_751_empty_wheel_hashes() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("pylock.toml").write_str(indoc! {r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "iniconfig"
        version = "2.0.0"
        [[packages.wheels]]
        url = "https://example.com/iniconfig-2.0.0-py3-none-any.whl"
        hashes = {}
    "#})?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("--require-hashes")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    warning: Empty hash tables in `pylock.toml` will be rejected in a future uv version. Rerun the original `uv export` or `uv pip compile` command to regenerate the file.
    error: In `--require-hashes` mode, all requirements must have a hash, but none were provided for: iniconfig
    ");

    Ok(())
}

#[test]
fn pep_751_missing_hashes() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("pylock.toml").write_str(indoc! {r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "iniconfig"
        version = "2.0.0"
        wheels = [{ url = "https://example.com/iniconfig-2.0.0-py3-none-any.whl" }]
    "#})?;

    // Omitting the required hashes field should fail even with verification disabled.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("--no-verify-hashes")
        .arg("-r")
        .arg("pylock.toml"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Not a valid `pylock.toml` file: pylock.toml
      cause: TOML parse error at line 7, column 11
               |
             7 | wheels = [{ url = "https://example.com/iniconfig-2.0.0-py3-none-any.whl" }]
               |           ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
             missing field `hashes`
    "#);

    Ok(())
}

#[test]
fn pep_751_empty_hashes_unselected_artifacts() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    fs::copy(
        context
            .workspace_root
            .join("test/links/ok-1.0.0-py3-none-any.whl"),
        context.temp_dir.child("ok-1.0.0-py3-none-any.whl"),
    )?;
    context.temp_dir.child("pylock.toml").write_str(indoc! {r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "ok"
        version = "1.0.0"
        sdist = { url = "https://example.com/ok-1.0.0.tar.gz", hashes = {} }
        wheels = [
            { url = "https://example.com/ok-1.0.0-cp311-cp311-win32.whl", hashes = { sha3_256 = "0000000000000000000000000000000000000000000000000000000000000000" } },
            { path = "ok-1.0.0-py3-none-any.whl", hashes = { sha256 = "79f0b33e6ce1e09eaa1784c8eee275dfe84d215d9c65c652f07c18e85fdaac5f" } },
        ]

        [[packages]]
        name = "unused"
        version = "1.0.0"
        marker = "python_version < '3'"
        archive = { url = "https://example.com/unused-1.0.0.tar.gz", hashes = {} }
    "#})?;

    // Empty tables on unselected artifacts should warn once without preventing installation.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Empty hash tables in `pylock.toml` will be rejected in a future uv version. Rerun the original `uv export` or `uv pip compile` command to regenerate the file.
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==1.0.0
    ");

    Ok(())
}

#[test]
fn pep_751_unsupported_hashes() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("pylock.toml").write_str(indoc! {r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "iniconfig"
        version = "2.0.0"
        wheels = [{ url = "https://example.com/iniconfig-2.0.0-py3-none-any.whl", hashes = { sha3_256 = "0000000000000000000000000000000000000000000000000000000000000000" } }]
    "#})?;

    // Unsupported algorithms should remain valid when parsing a pylock file.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("--no-verify-hashes")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Would download 1 package
    Would install 1 package
     + iniconfig==2.0.0
    ");

    // Requiring hashes should reject artifacts with only unsupported algorithms.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("--require-hashes")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: In `--require-hashes` mode, all requirements must have a hash, but none were provided for: iniconfig
    ");

    Ok(())
}

#[cfg(feature = "test-pypi")]
#[test]
fn pep_751_hash_mismatch() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    // Download the source.
    let archive = context.temp_dir.child("iniconfig-2.0.0-py3-none-any.whl");
    download_to_disk(
        "https://files.pythonhosted.org/packages/ef/a6/62565a6e1cf69e10f5727360368e451d4b7f58beeac6173dc9db836a5b46/iniconfig-2.0.0-py3-none-any.whl",
        &archive,
    );

    let pylock_toml = context.temp_dir.child("pylock.toml");
    pylock_toml.write_str(r#"
        # This file was autogenerated by uv via the following command:
        #    uv export --cache-dir [CACHE_DIR] -o pylock.toml
        lock-version = "1.0"
        created-by = "uv"
        requires-python = ">=3.12"

        [[packages]]
        name = "iniconfig"
        version = "2.0.0"
        archive = { path = "iniconfig-2.0.0-py3-none-any.whl", hashes = { sha256 = "c5185871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374", sha3_256 = "0000000000000000000000000000000000000000000000000000000000000000" } }
    "#)?;

    // An unsupported algorithm should not prevent verification of a supported hash.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to read `iniconfig @ file://[TEMP_DIR]/iniconfig-2.0.0-py3-none-any.whl`
      cause: Hash mismatch for `iniconfig @ file://[TEMP_DIR]/iniconfig-2.0.0-py3-none-any.whl`

             Expected:
               sha256:c5185871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374

             Computed:
               sha256:b6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374
    "
    );

    pylock_toml.write_str(&context.read("pylock.toml").replace(
        "c5185871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374",
        "b6a85871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374",
    ))?;

    // A matching supported hash should permit installation alongside an unsupported algorithm.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0 (from file://[TEMP_DIR]/iniconfig-2.0.0-py3-none-any.whl)
    ");

    Ok(())
}

#[test]
fn pep_751_rejects_mismatched_wheel_identity() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pylock_toml = context.temp_dir.child("pylock.toml");
    pylock_toml.write_str(
        r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "different"
        version = "2.0.0"
        wheels = [{ url = "https://example.com/iniconfig-2.0.0-py3-none-any.whl", hashes = { sha256 = "0000000000000000000000000000000000000000000000000000000000000000" } }]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("-r")
        .arg("pylock.toml"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Wheel filename `iniconfig-2.0.0-py3-none-any.whl` does not match package name `different`
    "#);

    pylock_toml.write_str(
        r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "iniconfig"
        version = "1.0.0"
        archive = { url = "https://example.com/iniconfig-2.0.0-py3-none-any.whl", hashes = { sha256 = "0000000000000000000000000000000000000000000000000000000000000000" } }
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("-r")
        .arg("pylock.toml"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Wheel filename `iniconfig-2.0.0-py3-none-any.whl` does not match package version `1.0.0`
    "#);

    // An incompatible wheel must still be validated before falling back to the sdist.
    pylock_toml.write_str(
        r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "iniconfig"
        version = "2.0.0"
        sdist = { url = "https://example.com/iniconfig-2.0.0.tar.gz", hashes = { sha256 = "0000000000000000000000000000000000000000000000000000000000000000" } }
        wheels = [{ url = "https://example.com/different-2.0.0-cp39-cp39-any.whl", hashes = { sha256 = "0000000000000000000000000000000000000000000000000000000000000000" } }]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("-r")
        .arg("pylock.toml"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Wheel filename `different-2.0.0-cp39-cp39-any.whl` does not match package name `iniconfig`
    "#);

    // A malformed wheel must not be ignored merely because an sdist is available.
    pylock_toml.write_str(
        r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "iniconfig"
        version = "2.0.0"
        sdist = { url = "https://example.com/iniconfig-2.0.0.tar.gz", hashes = { sha256 = "0000000000000000000000000000000000000000000000000000000000000000" } }
        wheels = [{ url = "https://example.com/invalid.whl", hashes = { sha256 = "0000000000000000000000000000000000000000000000000000000000000000" } }]
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("-r")
        .arg("pylock.toml"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The wheel filename `invalid.whl` is invalid: Must have a version
    "#);

    Ok(())
}

#[test]
fn pep_751_mix() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let lockfile = indoc! {r#"
        lock-version = "1.0"
        created-by = "uv"
        packages = []
    "#};
    for filename in ["pylock.toml", "pylock.dev.toml"] {
        context.temp_dir.child(filename).write_str(lockfile)?;
    }

    context.temp_dir.child("requirements.txt").touch()?;
    context.temp_dir.child("constraints.txt").touch()?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml")
        .arg("-r")
        .arg("pylock.dev.toml"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Multiple `pylock.toml` files specified: `pylock.toml` vs. `pylock.dev.toml`
    "
    );

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml")
        .arg("-r")
        .arg("requirements.txt"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Cannot specify additional requirements alongside a `pylock.toml` file
    "
    );

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml")
        .arg("-c")
        .arg("constraints.txt"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Cannot specify constraints with a `pylock.toml` file
    "
    );

    Ok(())
}

#[test]
fn pep_751_multiple_sources() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pylock_toml = context.temp_dir.child("pylock.toml");
    pylock_toml.write_str(r#"
        # This file was autogenerated by uv via the following command:
        #    uv export --cache-dir [CACHE_DIR] -o pylock.toml
        lock-version = "1.0"
        created-by = "uv"
        requires-python = ">=3.12"

        [[packages]]
        name = "typing-extensions"
        version = "4.10.0"
        index = "https://pypi.org/simple"
        sdist = { url = "https://files.pythonhosted.org/packages/16/3a/0d26ce356c7465a19c9ea8814b960f8a36c3b0d07c323176620b7b483e44/typing_extensions-4.10.0.tar.gz", size = 77558, hashes = { sha256 = "b0abd7c89e8fb96f98db18d86106ff1d90ab692004eb746cf6eda2682f91b3cb" } }
        wheels = [{ url = "https://files.pythonhosted.org/packages/f9/de/dc04a3ea60b22624b51c703a84bbe0184abcd1d0b9bc8074b5d6b7ab90bb/typing_extensions-4.10.0-py3-none-any.whl", size = 33926, hashes = { sha256 = "69b1a937c3a517342112fb4c6df7e72fc39a38e7891a5730ed4985b5214b5475" } }]
        archive = { path = "iniconfig-2.0.0-py3-none-any.whl", hashes = { sha256 = "c5185871a79d2e3b22d2d1b94ac2824226a63c6b741c88f7ae975f18b6778374" } }
    "#)?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package `typing-extensions` includes both a registry (`packages.wheels`) and an archive source (`packages.archive`)
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

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("-r")
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

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("-r")
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

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("-r")
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

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("-r")
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

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("-r")
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

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("--offline")
        .arg("--dry-run")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    Would make no changes
    ");

    Ok(())
}

#[cfg(feature = "test-pypi")]
#[test]
fn pep_751_groups() -> Result<()> {
    let context = uv_test::test_context!("3.13");

    let pylock_toml = context.temp_dir.child("pylock.toml");
    pylock_toml.write_str(
        r#"
lock-version = "1.0"
requires-python = "==3.13.*"
environments = [
    "python_version == \"3.13\"",
]
extras = ["async", "dev"]
dependency-groups = ["default", "test"]
default-groups = ["default"]
created-by = "pdm"
[[packages]]
name = "anyio"
version = "4.9.0"
requires-python = ">=3.9"
sdist = {name = "anyio-4.9.0.tar.gz", url = "https://files.pythonhosted.org/packages/95/7d/4c1bd541d4dffa1b52bd83fb8527089e097a106fc90b467a7313b105f840/anyio-4.9.0.tar.gz", hashes = {sha256 = "673c0c244e15788651a4ff38710fea9675823028a6f08a5eda409e0c9840a028"}}
wheels = [
    {name = "anyio-4.9.0-py3-none-any.whl",url = "https://files.pythonhosted.org/packages/a1/ee/48ca1a7c89ffec8b6a0c5d02b89c305671d5ffd8d3c94acf8b8c408575bb/anyio-4.9.0-py3-none-any.whl",hashes = {sha256 = "9f76d541cad6e36af7beb62e978876f3b41e3e04f2c1fbf0884604c0a9c4d93c"}},
]
marker = "\"async\" in extras"

[packages.tool.pdm]
dependencies = [
    "exceptiongroup>=1.0.2; python_version < \"3.11\"",
    "idna>=2.8",
    "sniffio>=1.1",
    "typing-extensions>=4.5; python_version < \"3.13\"",
]

[[packages]]
name = "blinker"
version = "1.9.0"
requires-python = ">=3.9"
sdist = {name = "blinker-1.9.0.tar.gz", url = "https://files.pythonhosted.org/packages/21/28/9b3f50ce0e048515135495f198351908d99540d69bfdc8c1d15b73dc55ce/blinker-1.9.0.tar.gz", hashes = {sha256 = "b4ce2265a7abece45e7cc896e98dbebe6cead56bcf805a3d23136d145f5445bf"}}
wheels = [
    {name = "blinker-1.9.0-py3-none-any.whl",url = "https://files.pythonhosted.org/packages/10/cb/f2ad4230dc2eb1a74edf38f1a38b9b52277f75bef262d8908e60d957e13c/blinker-1.9.0-py3-none-any.whl",hashes = {sha256 = "ba0efaa9080b619ff2f3459d1d500c57bddea4a6b424b60a91141db6fd2f08bc"}},
]
marker = "\"dev\" in extras"

[packages.tool.pdm]
dependencies = []

[[packages]]
name = "idna"
version = "3.10"
requires-python = ">=3.6"
sdist = {name = "idna-3.10.tar.gz", url = "https://files.pythonhosted.org/packages/f1/70/7703c29685631f5a7590aa73f1f1d3fa9a380e654b86af429e0934a32f7d/idna-3.10.tar.gz", hashes = {sha256 = "12f65c9b470abda6dc35cf8e63cc574b1c52b11df2c86030af0ac09b01b13ea9"}}
wheels = [
    {name = "idna-3.10-py3-none-any.whl",url = "https://files.pythonhosted.org/packages/76/c6/c88e154df9c4e1a2a66ccf0005a88dfb2650c1dffb6f5ce603dfbd452ce3/idna-3.10-py3-none-any.whl",hashes = {sha256 = "946d195a0d259cbba61165e88e65941f16e9b36ea6ddb97f00452bae8b1287d3"}},
]
marker = "\"async\" in extras"

[packages.tool.pdm]
dependencies = []

[[packages]]
name = "iniconfig"
version = "2.1.0"
requires-python = ">=3.8"
sdist = {name = "iniconfig-2.1.0.tar.gz", url = "https://files.pythonhosted.org/packages/f2/97/ebf4da567aa6827c909642694d71c9fcf53e5b504f2d96afea02718862f3/iniconfig-2.1.0.tar.gz", hashes = {sha256 = "3abbd2e30b36733fee78f9c7f7308f2d0050e88f0087fd25c2645f63c773e1c7"}}
wheels = [
    {name = "iniconfig-2.1.0-py3-none-any.whl",url = "https://files.pythonhosted.org/packages/2c/e1/e6716421ea10d38022b952c159d5161ca1193197fb744506875fbb87ea7b/iniconfig-2.1.0-py3-none-any.whl",hashes = {sha256 = "9deba5723312380e77435581c6bf4935c94cbfab9b1ed33ef8d238ea168eb760"}},
]
marker = "\"default\" in dependency_groups"

[packages.tool.pdm]
dependencies = []

[[packages]]
name = "pygments"
version = "2.19.2"
requires-python = ">=3.8"
sdist = {name = "pygments-2.19.2.tar.gz", url = "https://files.pythonhosted.org/packages/b0/77/a5b8c569bf593b0140bde72ea885a803b82086995367bf2037de0159d924/pygments-2.19.2.tar.gz", hashes = {sha256 = "636cb2477cec7f8952536970bc533bc43743542f70392ae026374600add5b887"}}
wheels = [
    {name = "pygments-2.19.2-py3-none-any.whl",url = "https://files.pythonhosted.org/packages/c7/21/705964c7812476f378728bdf590ca4b771ec72385c533964653c68e86bdc/pygments-2.19.2-py3-none-any.whl",hashes = {sha256 = "86540386c03d588bb81d44bc3928634ff26449851e99741617ecb9037ee5ec0b"}},
]
marker = "\"test\" in dependency_groups"

[packages.tool.pdm]
dependencies = []

[[packages]]
name = "sniffio"
version = "1.3.1"
requires-python = ">=3.7"
sdist = {name = "sniffio-1.3.1.tar.gz", url = "https://files.pythonhosted.org/packages/a2/87/a6771e1546d97e7e041b6ae58d80074f81b7d5121207425c964ddf5cfdbd/sniffio-1.3.1.tar.gz", hashes = {sha256 = "f4324edc670a0f49750a81b895f35c3adb843cca46f0530f79fc1babb23789dc"}}
wheels = [
    {name = "sniffio-1.3.1-py3-none-any.whl",url = "https://files.pythonhosted.org/packages/e9/44/75a9c9421471a6c4805dbf2356f7c181a29c1879239abab1ea2cc8f38b40/sniffio-1.3.1-py3-none-any.whl",hashes = {sha256 = "2f6da418d1f1e0fddd844478f41680e794e6051915791a034ff65e5f100525a2"}},
]
marker = "\"async\" in extras"

[packages.tool.pdm]
dependencies = []

[tool.pdm]
hashes = {sha256 = "51795362d337720c28bd6c3a26eb33751f2b69590261f599ffb4172ee2c441c6"}

[[tool.pdm.targets]]
requires_python = "==3.13.*"
        "#,
    )?;

    // By default, only `iniconfig` should be installed, since it's in the default group.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.1.0
    "
    );

    // With `--extra async`, `anyio` should be installed.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml")
        .arg("--extra")
        .arg("async"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==4.9.0
     + idna==3.10
     + sniffio==1.3.1
    "
    );

    // With `--group test`, `pygments` should be installed.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml")
        .arg("--group")
        .arg("test"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + pygments==2.19.2
    "
    );

    // With `--all-extras`, `blinker` should be installed.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml")
        .arg("--all-extras"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + blinker==1.9.0
    "
    );

    // `--group pylock.toml:test` should be rejected.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml")
        .arg("--group")
        .arg("pylock.toml:test"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: invalid value 'pylock.toml:test' for '--group <GROUP>': The `--group` path is required to end in `pyproject.toml` for compatibility with pip; got: pylock.toml

    For more information, try '--help'.
    "
    );

    Ok(())
}

#[test]
fn pep_751_lock_version() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pylock_toml = context.temp_dir.child("pylock.toml");
    pylock_toml.write_str(
        r#"
        lock-version = "2.0"
        created-by = "uv"
        packages = []
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Not a valid `pylock.toml` file: pylock.toml
      cause: TOML parse error at line 2, column 24
               |
             2 |         lock-version = "2.0"
               |                        ^^^^^
             unsupported lock version (`2.0`, but only major version 1 is supported)
    "#
    );

    // Later minor versions are forwards-compatible with the supported major version.
    pylock_toml.write_str(
        r#"
        lock-version = "1.1"
        created-by = "uv"
        packages = []
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    "#
    );

    Ok(())
}

#[test]
fn pep_751_incompatible_environment() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("pylock.toml").write_str(indoc! {r#"
        lock-version = "1.0"
        created-by = "uv"
        environments = ["python_version < '3.12'"]
        packages = []
    "#})?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The target environment is not supported by the lockfile
    ");

    Ok(())
}

#[test]
fn pep_751_package_requires_python() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pylock_toml = context.temp_dir.child("pylock.toml");
    pylock_toml.write_str(
        r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "example"
        marker = "python_version < '3.0'"
        requires-python = ">=99"
        directory = { path = "." }
        "#,
    )?;

    // Skip the Python requirement for packages excluded by their marker.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Checked in [TIME]
    "#
    );

    pylock_toml.write_str(
        r#"
        lock-version = "1.0"
        created-by = "uv"

        [[packages]]
        name = "example"
        requires-python = ">=99"
        directory = { path = "." }
        "#,
    )?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @r#"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Package `example` requires Python >=99, but the target Python version is 3.12.[X]
    "#
    );

    Ok(())
}

#[test]
fn pep_751_requires_python() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context.temp_dir.child("pylock.toml").write_str(indoc! {r#"
        lock-version = "1.0"
        created-by = "uv"
        requires-python = ">=3.13"
        packages = []
    "#})?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
        .arg("pylock.toml"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: The requested interpreter resolved to Python 3.12.[X], which is incompatible with the `pylock.toml`'s Python requirement: `>=3.13`
    "
    );

    Ok(())
}

#[cfg(feature = "test-pypi")]
#[test]
fn pep_751_dependency() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let pylock_toml = context.temp_dir.child("pylock.toml");
    pylock_toml.write_str(r#"
        # This file was autogenerated by uv via the following command:
        #    uv export --cache-dir [CACHE_DIR] -o pylock.toml
        lock-version = "1.0"
        created-by = "uv"
        requires-python = ">=3.12"

        [[packages]]
        name = "anyio"
        version = "4.3.0"
        sdist = { url = "https://files.pythonhosted.org/packages/db/4d/3970183622f0330d3c23d9b8a5f52e365e50381fd484d08e3285104333d3/anyio-4.3.0.tar.gz", upload-time = 2024-02-19T08:36:28Z, size = 159642, hashes = { sha256 = "f75253795a87df48568485fd18cdd2a3fa5c4f7c5be8e5e36637733fce06fed6" } }
        wheels = [{ url = "https://files.pythonhosted.org/packages/14/fd/2f20c40b45e4fb4324834aea24bd4afdf1143390242c0b33774da0e2e34f/anyio-4.3.0-py3-none-any.whl", upload-time = 2024-02-19T08:36:26Z, size = 85584, hashes = { sha256 = "048e05d0f6caeed70d731f3db756d35dcc1f35747c8c403364a8332c630441b8" } }]
        dependencies = [
            { name = "idna" },
            { name = "sniffio" },
        ]

        [[packages]]
        name = "idna"
        version = "3.6"
        sdist = { url = "https://files.pythonhosted.org/packages/bf/3f/ea4b9117521a1e9c50344b909be7886dd00a519552724809bb1f486986c2/idna-3.6.tar.gz", upload-time = 2023-11-25T15:40:54Z, size = 175426, hashes = { sha256 = "9ecdbbd083b06798ae1e86adcbfe8ab1479cf864e4ee30fe4e46a003d12491ca" } }
        wheels = [{ url = "https://files.pythonhosted.org/packages/c2/e7/a82b05cf63a603df6e68d59ae6a68bf5064484a0718ea5033660af4b54a9/idna-3.6-py3-none-any.whl", upload-time = 2023-11-25T15:40:52Z, size = 61567, hashes = { sha256 = "c05567e9c24a6b9faaa835c4821bad0590fbb9d5779e7caa6e1cc4978e7eb24f" } }]

        [[packages]]
        name = "sniffio"
        version = "1.3.1"
        sdist = { url = "https://files.pythonhosted.org/packages/a2/87/a6771e1546d97e7e041b6ae58d80074f81b7d5121207425c964ddf5cfdbd/sniffio-1.3.1.tar.gz", upload-time = 2024-02-25T23:20:04Z, size = 20372, hashes = { sha256 = "f4324edc670a0f49750a81b895f35c3adb843cca46f0530f79fc1babb23789dc" } }
        wheels = [{ url = "https://files.pythonhosted.org/packages/e9/44/75a9c9421471a6c4805dbf2356f7c181a29c1879239abab1ea2cc8f38b40/sniffio-1.3.1-py3-none-any.whl", upload-time = 2024-02-25T23:20:01Z, size = 10235, hashes = { sha256 = "2f6da418d1f1e0fddd844478f41680e794e6051915791a034ff65e5f100525a2" } }]
    "#)?;

    uv_snapshot!(context.filters(), context.pip_install()
        .arg("--preview")
        .arg("-r")
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

    Ok(())
}
