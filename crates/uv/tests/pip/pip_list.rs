use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::fixture::ChildPath;
use assert_fs::fixture::FileWriteStr;
use assert_fs::fixture::PathChild;
use assert_fs::prelude::*;
use insta::allow_duplicates;

use uv_static::EnvVars;
use uv_test::uv_snapshot;

const INVALID_INSTALLER_METADATA: &str =
    r#""https://user:sidecar-secret@example.invalid/a?sig=sidecar-signature""#;

/// Populate enough wheel records to exercise batched installed-package indexing.
fn create_many_installed_distributions(site_packages: &ChildPath) -> Result<Vec<String>> {
    create_installed_distributions(site_packages, 1_024)
}

fn create_installed_distributions(site_packages: &ChildPath, count: usize) -> Result<Vec<String>> {
    let mut packages = Vec::with_capacity(count);
    for index in 0..count {
        let package = format!("filler{index:04}");
        site_packages
            .child(format!("{package}-1.0.0.dist-info"))
            .create_dir_all()?;
        packages.push(package);
    }
    Ok(packages)
}

#[test]
fn list_empty_columns() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.pip_list()
        .arg("--format")
        .arg("columns"), @"
    exit_code: 0 (success)
    "
    );
}

#[test]
fn list_empty_freeze() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.pip_list()
        .arg("--format")
        .arg("freeze"), @"
    exit_code: 0 (success)
    "
    );
}

#[test]
fn list_empty_json() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.pip_list()
        .arg("--format")
        .arg("json"), @"
    exit_code: 0 (success)
    ----- stdout -----
    []
    "
    );
}

#[test]
fn list_many_distributions_keeps_metadata_lazy() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_concurrent_installs("4");
    let site_packages = ChildPath::new(context.site_packages());
    let packages = create_many_installed_distributions(&site_packages)?;

    for (package, version) in [("omega", "2.0.0"), ("alpha", "1.0.0")] {
        let dist_info = site_packages.child(format!("{package}-{version}.dist-info"));
        dist_info.create_dir_all()?;
        dist_info.child("METADATA").write_str("invalid")?;
        dist_info.child("WHEEL").write_str("invalid")?;
    }

    let mut command = context.pip_list();
    command.arg("--format=json");
    for package in packages {
        command.arg("--exclude").arg(package);
    }

    uv_snapshot!(context.filters(), command, @r#"
    exit_code: 0 (success)
    ----- stdout -----
    [{"name":"alpha","version":"1.0.0"},{"name":"omega","version":"2.0.0"}]
    "#);

    Ok(())
}

#[test]
fn list_many_distributions_reports_first_error() -> Result<()> {
    for concurrent_installs in ["1", "4"] {
        let context = uv_test::test_context!("3.12").with_concurrent_installs(concurrent_installs);
        let site_packages = ChildPath::new(context.site_packages());
        create_many_installed_distributions(&site_packages)?;

        for package in ["a_warning", "c_warning"] {
            let dist_info = site_packages.child(format!("{package}-1.0.0.dist-info"));
            dist_info.create_dir_all()?;
            dist_info
                .child("direct_url.json")
                .write_str(r#"{"url":"relative","dir_info":{}}"#)?;
        }
        for package in ["b_error", "z_error"] {
            let dist_info = site_packages.child(format!("{package}-1.0.0.dist-info"));
            dist_info.create_dir_all()?;
            dist_info.child("direct_url.json").write_str("invalid")?;
        }
        site_packages
            .child("~dangling-1.0.0.dist-info")
            .create_dir_all()?;

        allow_duplicates! {
            uv_snapshot!(context.filters(), context.pip_list()
                .env(EnvVars::RUST_LOG, "uv_distribution_types=warn"), @"
            exit_code: 2 (failure)
            ----- stderr -----
            WARN Failed to parse direct URL: relative URL without a base
            error: Failed to read metadata from: `[SITE_PACKAGES]/b_error-1.0.0.dist-info`
              Caused by: expected value at line 1 column 1
            ");
        }
    }

    Ok(())
}

#[test]
fn list_orders_optional_sidecar_warnings_at_parallel_threshold() -> Result<()> {
    for concurrent_installs in ["1", "4"] {
        for distribution_count in [1_023, 1_024, 1_025] {
            let context =
                uv_test::test_context!("3.12").with_concurrent_installs(concurrent_installs);
            let site_packages = ChildPath::new(context.site_packages());
            create_installed_distributions(&site_packages, distribution_count - 2)?;
            for package in ["z_warning", "a_warning"] {
                let dist_info = site_packages.child(format!("{package}-1.0.0.dist-info"));
                dist_info.create_dir_all()?;
                for sidecar in ["uv_cache.json", "uv_build.json"] {
                    dist_info
                        .child(sidecar)
                        .write_str(INVALID_INSTALLER_METADATA)?;
                }
            }

            allow_duplicates! {
                uv_snapshot!(context.filters(), context.pip_list().arg("--editable"), @"
                exit_code: 0 (success)
                ----- stderr -----
                warning: Ignoring invalid installer metadata at `[SITE_PACKAGES]/a_warning-1.0.0.dist-info/uv_cache.json`: invalid JSON data
                warning: Ignoring invalid installer metadata at `[SITE_PACKAGES]/a_warning-1.0.0.dist-info/uv_build.json`: invalid JSON data
                warning: Ignoring invalid installer metadata at `[SITE_PACKAGES]/z_warning-1.0.0.dist-info/uv_cache.json`: invalid JSON data
                warning: Ignoring invalid installer metadata at `[SITE_PACKAGES]/z_warning-1.0.0.dist-info/uv_build.json`: invalid JSON data
                ");
            }
        }
    }
    Ok(())
}

#[test]
fn list_many_distributions_reports_sidecar_warnings_before_error() -> Result<()> {
    for concurrent_installs in ["1", "4"] {
        for error_index in [30, 32] {
            let before = format!("filler{:04}", error_index - 1);
            let error = format!("filler{error_index:04}");
            let context = uv_test::test_context!("3.12")
                .with_concurrent_installs(concurrent_installs)
                .with_filter((before.clone(), "before"))
                .with_filter((error.clone(), "error"));
            let site_packages = ChildPath::new(context.site_packages());
            create_many_installed_distributions(&site_packages)?;

            site_packages
                .child(format!("{before}-1.0.0.dist-info/uv_cache.json"))
                .write_str(INVALID_INSTALLER_METADATA)?;
            let dist_info = site_packages.child(format!("{error}-1.0.0.dist-info"));
            for sidecar in ["uv_cache.json", "uv_build.json"] {
                dist_info
                    .child(sidecar)
                    .write_str(INVALID_INSTALLER_METADATA)?;
            }
            dist_info.child("direct_url.json").write_str("invalid")?;

            for later_index in [error_index + 1, error_index + 32] {
                for sidecar in ["uv_cache.json", "uv_build.json"] {
                    site_packages
                        .child(format!("filler{later_index:04}-1.0.0.dist-info/{sidecar}"))
                        .write_str(INVALID_INSTALLER_METADATA)?;
                }
            }

            allow_duplicates! {
                uv_snapshot!(context.filters(), context.pip_list(), @"
                exit_code: 2 (failure)
                ----- stderr -----
                warning: Ignoring invalid installer metadata at `[SITE_PACKAGES]/before-1.0.0.dist-info/uv_cache.json`: invalid JSON data
                warning: Ignoring invalid installer metadata at `[SITE_PACKAGES]/error-1.0.0.dist-info/uv_cache.json`: invalid JSON data
                warning: Ignoring invalid installer metadata at `[SITE_PACKAGES]/error-1.0.0.dist-info/uv_build.json`: invalid JSON data
                error: Failed to read metadata from: `[SITE_PACKAGES]/error-1.0.0.dist-info`
                  Caused by: expected value at line 1 column 1
                ");
            }
        }
    }
    Ok(())
}

#[test]
fn list_many_distributions_orders_optional_sidecar_read_errors() -> Result<()> {
    for concurrent_installs in ["1", "4"] {
        for error_sidecar in ["uv_cache.json", "uv_build.json"] {
            let context = uv_test::test_context!("3.12")
                .with_concurrent_installs(concurrent_installs)
                .with_filter((
                    r"failed to (?:read from|open) file (`[^`]+`): [^\n]+",
                    "failed to read file $1: [IO_ERROR]",
                ));
            let site_packages = ChildPath::new(context.site_packages());
            create_many_installed_distributions(&site_packages)?;
            let dist_info = site_packages.child("filler0031-1.0.0.dist-info");
            for sidecar in ["uv_cache.json", "uv_build.json"] {
                if sidecar == error_sidecar {
                    dist_info.child(sidecar).create_dir_all()?;
                } else {
                    dist_info
                        .child(sidecar)
                        .write_str(INVALID_INSTALLER_METADATA)?;
                }
                site_packages
                    .child(format!("filler0032-1.0.0.dist-info/{sidecar}"))
                    .write_str(INVALID_INSTALLER_METADATA)?;
            }

            if error_sidecar == "uv_cache.json" {
                allow_duplicates! {
                    uv_snapshot!(context.filters(), context.pip_list(), @"
                    exit_code: 2 (failure)
                    ----- stderr -----
                    error: Failed to read metadata from: `[SITE_PACKAGES]/filler0031-1.0.0.dist-info`
                      Caused by: failed to read file `[SITE_PACKAGES]/filler0031-1.0.0.dist-info/uv_cache.json`: [IO_ERROR]
                    ");
                }
            } else {
                allow_duplicates! {
                    uv_snapshot!(context.filters(), context.pip_list(), @"
                    exit_code: 2 (failure)
                    ----- stderr -----
                    warning: Ignoring invalid installer metadata at `[SITE_PACKAGES]/filler0031-1.0.0.dist-info/uv_cache.json`: invalid JSON data
                    error: Failed to read metadata from: `[SITE_PACKAGES]/filler0031-1.0.0.dist-info`
                      Caused by: failed to read file `[SITE_PACKAGES]/filler0031-1.0.0.dist-info/uv_build.json`: [IO_ERROR]
                    ");
                }
            }
        }
    }
    Ok(())
}

#[test]
fn list_many_distributions_orders_dangling_warnings() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_concurrent_installs("4");
    let site_packages = ChildPath::new(context.site_packages());
    create_many_installed_distributions(&site_packages)?;
    for package in ["~z", "~a"] {
        site_packages
            .child(format!("{package}-1.0.0.dist-info"))
            .create_dir_all()?;
    }

    uv_snapshot!(context.filters(), context.pip_list().arg("--editable"), @"
    exit_code: 0 (success)
    ----- stderr -----
    warning: Ignoring dangling temporary directory: `[SITE_PACKAGES]/~a-1.0.0.dist-info`
    warning: Ignoring dangling temporary directory: `[SITE_PACKAGES]/~z-1.0.0.dist-info`
    ");

    Ok(())
}

#[test]
fn list_editable_non_file_url() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let dist_info = ChildPath::new(context.site_packages()).child("project-1.0.0.dist-info");
    dist_info.create_dir_all()?;
    dist_info
        .child("METADATA")
        .write_str("Metadata-Version: 2.1\nName: project\nVersion: 1.0.0\n")?;
    dist_info
        .child("direct_url.json")
        .write_str(r#"{"url":"https://example.com/project","dir_info":{"editable":true}}"#)?;

    uv_snapshot!(context.pip_list(), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package Version
    ------- -------
    project 1.0.0
    ");

    uv_snapshot!(context.pip_list().arg("--format=json"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    [{"name":"project","version":"1.0.0"}]
    "#);

    Ok(())
}

/// Invalid optional installer metadata must not prevent installed distributions from being listed.
#[test]
fn list_invalid_installer_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let dist_info = ChildPath::new(context.site_packages()).child("project-1.0.0.dist-info");
    dist_info.create_dir_all()?;
    dist_info
        .child("METADATA")
        .write_str("Metadata-Version: 2.1\nName: project\nVersion: 1.0.0\n")?;
    dist_info.child("uv_cache.json").write_str("{")?;
    dist_info.child("uv_build.json").write_str("{")?;

    uv_snapshot!(context.filters(), context.pip_list(), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package Version
    ------- -------
    project 1.0.0

    ----- stderr -----
    warning: Ignoring invalid installer metadata at `[SITE_PACKAGES]/project-1.0.0.dist-info/uv_cache.json`: invalid JSON data
    warning: Ignoring invalid installer metadata at `[SITE_PACKAGES]/project-1.0.0.dist-info/uv_build.json`: invalid JSON data
    ");

    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn list_single_no_editable() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("MarkupSafe==2.1.3")?;

    uv_snapshot!(context.pip_install()
        .arg("-r")
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

    context.assert_command("import markupsafe").success();

    uv_snapshot!(context.pip_list(), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package    Version
    ---------- -------
    markupsafe 2.1.3
    "
    );

    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn list_outdated_columns() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("anyio==3.0.0")?;

    uv_snapshot!(context.pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.0.0
     + idna==3.6
     + sniffio==1.3.1
    "
    );

    uv_snapshot!(context.pip_list().arg("--outdated"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package Version Latest Type
    ------- ------- ------ -----
    anyio   3.0.0   4.3.0  wheel
    "
    );

    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn list_outdated_json() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("anyio==3.0.0")?;

    uv_snapshot!(context.pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.0.0
     + idna==3.6
     + sniffio==1.3.1
    "
    );

    uv_snapshot!(context.pip_list().arg("--outdated").arg("--format").arg("json"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    [{"name":"anyio","version":"3.0.0","latest_version":"4.3.0","latest_filetype":"wheel"}]
    "#
    );

    Ok(())
}

#[test]
fn list_outdated_find_links() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let links_dir = context.workspace_root.join("test/links");
    let first_links_dir = context.temp_dir.child("first-links");
    first_links_dir.create_dir_all()?;
    fs_err::copy(
        links_dir.join("validation-2.0.0-py3-none-any.whl"),
        first_links_dir
            .child("validation-2.0.0-py3-none-any.whl")
            .path(),
    )?;
    let second_links_dir = context.temp_dir.child("second-links");
    second_links_dir.create_dir_all()?;
    fs_err::copy(
        links_dir.join("validation-3.0.0-py3-none-any.whl"),
        second_links_dir
            .child("validation-3.0.0-py3-none-any.whl")
            .path(),
    )?;

    uv_snapshot!(context.filters(), context.pip_install()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("validation==1.0.0")
        .arg("--find-links")
        .arg(&links_dir)
        .arg("--no-index"), @r###"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + validation==1.0.0
    "###
    );

    uv_snapshot!(context.filters(), context.pip_list()
        .env_remove(EnvVars::UV_EXCLUDE_NEWER)
        .arg("--outdated")
        .arg("--find-links")
        .arg(first_links_dir.path())
        .arg("--find-links")
        .arg(second_links_dir.path())
        .arg("--no-index"), @r###"
    exit_code: 0 (success)
    ----- stdout -----
    Package    Version Latest Type
    ---------- ------- ------ -----
    validation 1.0.0   3.0.0  wheel
    "###
    );

    Ok(())
}

#[test]
fn list_outdated_freeze() {
    let context = uv_test::test_context!("3.12");

    uv_snapshot!(context.pip_list().arg("--outdated").arg("--format").arg("freeze"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `--outdated` cannot be used with `--format freeze`
    "
    );
}

#[test]
#[cfg(feature = "test-git")]
fn list_outdated_git() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str(indoc::indoc! {r"
        iniconfig==1.0.0
        uv-public-pypackage @ git+https://github.com/astral-test/uv-public-pypackage@0.0.1
    "})?;

    uv_snapshot!(context.pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + iniconfig==1.0.0
     + uv-public-pypackage==0.1.0 (from git+https://github.com/astral-test/uv-public-pypackage@0dacfd662c64cb4ceb16e6cf65a157a8b715b979)
    "
    );

    uv_snapshot!(context.pip_list().arg("--outdated"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package   Version Latest Type
    --------- ------- ------ -----
    iniconfig 1.0.0   2.0.0  wheel
    "
    );

    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn list_outdated_index() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("anyio==3.0.0")?;

    uv_snapshot!(context.pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--strict"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + anyio==3.0.0
     + idna==3.6
     + sniffio==1.3.1
    "
    );

    uv_snapshot!(context.pip_list()
        .arg("--outdated")
        .arg("--index-url")
        .arg("https://test.pypi.org/simple"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package Version Latest Type
    ------- ------- ------ -----
    anyio   3.0.0   3.5.0  wheel
    "
    );

    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn list_editable() {
    let context = uv_test::test_context!("3.12")
        .with_filter((r"\-\-\-\-\-\-+.*", "[UNDERLINE]"))
        .with_filter(("  +", " "));

    // Install the editable package.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("-e")
        .arg(context.workspace_root.join("test/packages/poetry_editable")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + poetry-editable==0.1.0 (from file://[WORKSPACE]/test/packages/poetry_editable)
     + sniffio==1.3.1
    "
    );

    uv_snapshot!(context.filters(), context.pip_list(), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package Version Editable project location
    [UNDERLINE]
    anyio 4.3.0
    idna 3.6
    poetry-editable 0.1.0 [WORKSPACE]/test/packages/poetry_editable
    sniffio 1.3.1
    "
    );
}

#[test]
#[cfg(feature = "test-pypi")]
fn list_editable_only() {
    let context = uv_test::test_context!("3.12")
        .with_filter((r"\-\-\-\-\-\-+.*", "[UNDERLINE]"))
        .with_filter(("  +", " "));

    // Install the editable package.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("-e")
        .arg(context.workspace_root.join("test/packages/poetry_editable")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + poetry-editable==0.1.0 (from file://[WORKSPACE]/test/packages/poetry_editable)
     + sniffio==1.3.1
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
        .arg("--editable"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package Version Editable project location
    [UNDERLINE]
    poetry-editable 0.1.0 [WORKSPACE]/test/packages/poetry_editable
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
        .arg("--exclude-editable"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package Version
    [UNDERLINE]
    anyio 4.3.0
    idna 3.6
    sniffio 1.3.1
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
        .arg("--editable")
        .arg("--exclude-editable"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: the argument '--editable' cannot be used with '--exclude-editable'

    Usage: uv pip list --cache-dir [CACHE_DIR] --editable --exclude-newer <EXCLUDE_NEWER>

    For more information, try '--help'.
    "
    );
}

#[test]
#[cfg(feature = "test-pypi")]
fn list_exclude() {
    let context = uv_test::test_context!("3.12")
        .with_filter((r"\-\-\-\-\-\-+.*", "[UNDERLINE]"))
        .with_filter(("  +", " "));

    // Install the editable package.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("-e")
        .arg(context.workspace_root.join("test/packages/poetry_editable")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + poetry-editable==0.1.0 (from file://[WORKSPACE]/test/packages/poetry_editable)
     + sniffio==1.3.1
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--exclude")
    .arg("numpy"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package Version Editable project location
    [UNDERLINE]
    anyio 4.3.0
    idna 3.6
    poetry-editable 0.1.0 [WORKSPACE]/test/packages/poetry_editable
    sniffio 1.3.1
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--exclude")
    .arg("poetry-editable"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package Version
    [UNDERLINE]
    anyio 4.3.0
    idna 3.6
    sniffio 1.3.1
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--exclude")
    .arg("numpy")
    .arg("--exclude")
    .arg("poetry-editable"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package Version
    [UNDERLINE]
    anyio 4.3.0
    idna 3.6
    sniffio 1.3.1
    "
    );
}

#[test]
#[cfg(feature = "test-pypi")]
#[cfg(not(windows))]
fn list_format_json() {
    let context = uv_test::test_context!("3.12");

    // Install the editable package.
    uv_snapshot!(context.filters(), context.pip_install()
        .arg("-e")
        .arg(context.workspace_root.join("test/packages/poetry_editable")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + poetry-editable==0.1.0 (from file://[WORKSPACE]/test/packages/poetry_editable)
     + sniffio==1.3.1
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--format=json"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    [{"name":"anyio","version":"4.3.0"},{"name":"idna","version":"3.6"},{"name":"poetry-editable","version":"0.1.0","editable_project_location":"[WORKSPACE]/test/packages/poetry_editable"},{"name":"sniffio","version":"1.3.1"}]
    "#
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--format=json")
    .arg("--editable"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    [{"name":"poetry-editable","version":"0.1.0","editable_project_location":"[WORKSPACE]/test/packages/poetry_editable"}]
    "#
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--format=json")
    .arg("--exclude-editable"), @r#"
    exit_code: 0 (success)
    ----- stdout -----
    [{"name":"anyio","version":"4.3.0"},{"name":"idna","version":"3.6"},{"name":"sniffio","version":"1.3.1"}]
    "#
    );
}

#[test]
#[cfg(feature = "test-pypi")]
fn list_format_freeze() {
    let context = uv_test::test_context!("3.12");

    // Install the editable package.
    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("-e")
        .arg(context.workspace_root.join("test/packages/poetry_editable")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + poetry-editable==0.1.0 (from file://[WORKSPACE]/test/packages/poetry_editable)
     + sniffio==1.3.1
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--format=freeze"), @"
    exit_code: 0 (success)
    ----- stdout -----
    anyio==4.3.0
    idna==3.6
    poetry-editable==0.1.0
    sniffio==1.3.1
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--format=freeze")
    .arg("--editable"), @"
    exit_code: 0 (success)
    ----- stdout -----
    poetry-editable==0.1.0
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--format=freeze")
    .arg("--exclude-editable"), @"
    exit_code: 0 (success)
    ----- stdout -----
    anyio==4.3.0
    idna==3.6
    sniffio==1.3.1
    "
    );
}

#[test]
fn list_legacy_editable() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_concurrent_installs("4")
        .with_filter((r"\-\-\-\-\-\-+.*", "[UNDERLINE]"))
        .with_filter(("  +", " "));

    let site_packages = ChildPath::new(context.site_packages());
    create_many_installed_distributions(&site_packages)?;

    let target = context.temp_dir.child("zstandard_project");
    target.child("zstd").create_dir_all()?;
    target.child("zstd").child("__init__.py").write_str("")?;

    target.child("zstandard.egg-info").create_dir_all()?;
    target
        .child("zstandard.egg-info")
        .child("PKG-INFO")
        .write_str(
            "Metadata-Version: 2.1
Name: zstandard
Version: 0.22.0
",
        )?;

    site_packages
        .child("zstandard.egg-link")
        .write_str(target.path().to_str().unwrap())?;

    site_packages.child("easy-install.pth").write_str(&format!(
        "something\n{}\nanother thing\n",
        target.path().to_str().unwrap()
    ))?;

    uv_snapshot!(context.filters(), context.pip_list()
        .arg("--editable"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package Version Editable project location
    [UNDERLINE]
    zstandard 0.22.0 [TEMP_DIR]/zstandard_project
    "
    );

    Ok(())
}

#[test]
fn list_legacy_editable_invalid_version() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_filter(("  +", " "));

    let site_packages = ChildPath::new(context.site_packages());

    let target = context.temp_dir.child("paramiko_project");
    target.child("paramiko.egg-info").create_dir_all()?;
    target
        .child("paramiko.egg-info")
        .child("PKG-INFO")
        .write_str(
            "Metadata-Version: 1.0
Name: paramiko
Version: 0.1-bulbasaur
",
        )?;
    site_packages
        .child("paramiko.egg-link")
        .write_str(target.path().to_str().unwrap())?;

    uv_snapshot!(context.filters(), context.pip_list()
        .arg("--editable"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to read metadata from: `[SITE_PACKAGES]/paramiko.egg-link`
     cause: after parsing `0.1-b`, found `ulbasaur`, which is not part of a valid version
    "
    );

    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn list_ignores_quiet_flag_format_freeze() {
    let context = uv_test::test_context!("3.12");

    // Install the editable package.
    uv_snapshot!(context.filters(), context
        .pip_install()
        .arg("-e")
        .arg(context.workspace_root.join("test/packages/poetry_editable")), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 4 packages in [TIME]
    Installed 4 packages in [TIME]
     + anyio==4.3.0
     + idna==3.6
     + poetry-editable==0.1.0 (from file://[WORKSPACE]/test/packages/poetry_editable)
     + sniffio==1.3.1
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--format=freeze")
    .arg("--quiet"), @"
    exit_code: 0 (success)
    ----- stdout -----
    anyio==4.3.0
    idna==3.6
    poetry-editable==0.1.0
    sniffio==1.3.1
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--format=freeze")
    .arg("--editable")
    .arg("--quiet"), @"
    exit_code: 0 (success)
    ----- stdout -----
    poetry-editable==0.1.0
    "
    );

    uv_snapshot!(context.filters(), context.pip_list()
    .arg("--format=freeze")
    .arg("--exclude-editable")
    .arg("--quiet"), @"
    exit_code: 0 (success)
    ----- stdout -----
    anyio==4.3.0
    idna==3.6
    sniffio==1.3.1
    "
    );
}

#[test]
#[cfg(feature = "test-pypi")]
fn list_target() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("MarkupSafe==2.1.3\ntomli==2.0.1")?;

    let target = context.temp_dir.child("target");

    // Install packages to a target directory.
    context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--target")
        .arg(target.path())
        .assert()
        .success();

    // List packages in the target directory.
    uv_snapshot!(context.filters(), context.pip_list()
        .arg("--target")
        .arg(target.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package    Version
    ---------- -------
    markupsafe 2.1.3
    tomli      2.0.1
    "
    );

    // Without --target, the packages should not be visible.
    uv_snapshot!(context.pip_list(), @"
    exit_code: 0 (success)
    "
    );

    Ok(())
}

#[test]
#[cfg(feature = "test-pypi")]
fn list_prefix() -> Result<()> {
    let context = uv_test::test_context!("3.12");

    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("MarkupSafe==2.1.3\ntomli==2.0.1")?;

    let prefix = context.temp_dir.child("prefix");

    // Install packages to a prefix directory.
    context
        .pip_install()
        .arg("-r")
        .arg("requirements.txt")
        .arg("--prefix")
        .arg(prefix.path())
        .assert()
        .success();

    // List packages in the prefix directory.
    uv_snapshot!(context.filters(), context.pip_list()
        .arg("--prefix")
        .arg(prefix.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    Package    Version
    ---------- -------
    markupsafe 2.1.3
    tomli      2.0.1
    "
    );

    // Without --prefix, the packages should not be visible.
    uv_snapshot!(context.pip_list(), @"
    exit_code: 0 (success)
    "
    );

    Ok(())
}
