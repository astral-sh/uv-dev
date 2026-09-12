#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::prelude::*;
#[cfg(unix)]
use fs_err::{metadata, set_permissions};
use indoc::indoc;
use uv_fs::copy_dir_all;
use uv_static::EnvVars;
use uv_test::{uv_snapshot, venv_bin_path};

#[test]
fn tool_run_args() {
    let context = uv_test::test_context!("3.12")
        .with_packse_index("packages/tool-run.toml")
        .with_filtered_counts()
        .with_tool_dirs();
    let context = context
        .with_filter((
            r"Usage: uv(?:\.exe)? tool run \[OPTIONS\] (?s:.*?)(\n----- stderr -----|$)",
            "[UV TOOL RUN HELP]$1",
        ))
        .with_filter((
            r"usage: pytest \[options\] (?s:.*?)(\n----- stderr -----|$)",
            "[PYTEST HELP]$1",
        ));

    // We treat arguments before the command as uv tool run arguments
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--help")
        .arg("run-tool"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Run a command provided by a Python package

    [UV TOOL RUN HELP]
    ");

    // We don't treat arguments after the command as uv tool run arguments
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("run-tool")
        .arg("--help"), @"
    exit_code: 0 (success)
    ----- stdout -----
    run-tool 8.1.1

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + run-helper==1.4.0
     + run-tool==8.1.1
    ");

    // Can use `--` to separate uv arguments from the command arguments.
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--")
        .arg("run-tool")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    run-tool 8.1.1

    ----- stderr -----
    Resolved [N] packages in [TIME]
    ");
}

#[test]
fn tool_run_at_version() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_exe_suffix().with_tool_dirs();
    uv_snapshot!(
        context.filters(),
        context.tool_run().arg("run-tool@8.0.0").arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    run-tool 8.0.0

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + run-helper==1.4.0
     + run-tool==8.0.0
    "
    ); // Empty versions are just treated as package and command names
    uv_snapshot!(
        context.filters(),
        context.tool_run().arg("run-tool@").arg("--version"),
        @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Failed to parse: `run-tool@`
      cause: Expected URL
             run-tool@
                      ^
    "
    ); // Invalid versions are just treated as package and command names
    uv_snapshot!(
        context.filters(),
        context.tool_run().arg("run-tool@invalid").arg("--version"),
        @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to resolve tool requirement
      cause: Distribution not found at: file://[TEMP_DIR]/invalid
    "
    );
    let filters = context
        .filters()
        .into_iter()
        .chain([(
            // The error message is different on Windows
            "cause: program not found",
            "cause: No such file or directory (os error 2)",
        )])
        .collect::<Vec<_>>(); // When `--from` is used, `@` is not treated as a version request
    uv_snapshot!(
        filters,
        context
            .tool_run()
            .arg("--from")
            .arg("run-tool")
            .arg("run-tool@8.0.0")
            .arg("--version"),
        @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 2 packages in [TIME]
     + run-helper==1.4.0
     + run-tool==8.1.1
    An executable named `run-tool@8.0.0` is not provided by package `run-tool`.
    The following executables are available:
    - run-tool
    "
    );
}

#[test]
fn tool_run_no_binary_package_env_var() {
    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    context
        .tool_install()
        .arg("pytest")
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("pytest")
        .arg("--version")
        .env(EnvVars::UV_NO_BINARY_PACKAGE, "iniconfig"), @"
    exit_code: 0 (success)
    ----- stdout -----
    pytest 8.1.1

    ----- stderr -----
    Resolved 4 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 4 packages in [TIME]
     + iniconfig==2.0.0
     + packaging==24.0
     + pluggy==1.4.0
     + pytest==8.1.1
    ");
}

#[test]
fn tool_run_from_version() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg("run-tool==8.0.0")
        .arg("run-tool")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    run-tool 8.0.0

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + run-helper==1.4.0
     + run-tool==8.0.0
    ");
}

#[test]
fn tool_run_constraints() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_tool_dirs();

    let constraints_txt = context.temp_dir.child("constraints.txt");
    constraints_txt.write_str("run-helper<1.4.0").unwrap();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--constraints")
        .arg("constraints.txt")
        .arg("run-tool")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    run-tool 8.0.2

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + run-helper==1.3.0
     + run-tool==8.0.2
    ");
}

#[test]
fn tool_run_overrides() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&_server.index_url())
        .with_tool_dirs();

    let overrides_txt = context.temp_dir.child("overrides.txt");
    overrides_txt.write_str("run-helper<1.4.0").unwrap();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--overrides")
        .arg("overrides.txt")
        .arg("run-tool")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    run-tool 8.1.1

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + run-helper==1.3.0
     + run-tool==8.1.1
    ");
}

#[test]
fn tool_run_suggest_valid_commands() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_exe_suffix().with_tool_dirs();
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--from")
            .arg("format-tool")
            .arg("orange"),
        @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + format-support==1.0.0
     + format-tool==24.3.0
    An executable named `orange` is not provided by package `format-tool`.
    The following executables are available:
    - format-tool
    - format-tool-daemon
    "
    );
    uv_snapshot!(
        context.filters(),
        context.tool_run().arg("no-script"),
        @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + no-script==2.31.0
     + no-script-leaf==1.0.0
    Package `no-script` does not provide any executables.
    "
    );
}

#[test]
fn tool_run_warn_executable_not_in_from() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_exe_suffix().with_tool_dirs();
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--from")
            .arg("cli-provider")
            .arg("cli-provider"),
        @"
    exit_code: 2 (failure)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + cli-provider==0.1.0
     + provider-cli==0.1.0
    warning: An executable named `cli-provider` is not provided by package `cli-provider` but is available via the dependency `provider-cli`. Consider using `uv tool run --from provider-cli cli-provider` instead.
    "
    );
}

#[test]
fn tool_run_from_install() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts().with_tool_dirs(); // Install `format-tool` at a specific version.
    context
        .tool_install()
        .arg("format-tool==24.1.0")
        .assert()
        .success(); // Verify that `tool run format-tool` uses the already-installed version.
    uv_snapshot!(
        context.filters(),
        context.tool_run().arg("format-tool").arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    format-tool 24.1.0
    "
    ); // Verify that `--isolated` uses an isolated environment.
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--isolated")
            .arg("format-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    format-tool 24.3.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + format-support==1.0.0
     + format-tool==24.3.0
    "
    ); // Verify that `tool run format-tool` at a different version installs the new version.
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("format-tool@24.1.1")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    format-tool 24.1.1

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + format-support==1.0.0
     + format-tool==24.1.1
    "
    ); // Verify that `--with` installs a new version.
    // TODO(charlie): This could (in theory) layer the `--with` requirements on top of the existing
    // environment.
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--with")
            .arg("extra-requirement")
            .arg("format-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    format-tool 24.3.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + extra-requirement==2.0.0
     + format-support==1.0.0
     + format-tool==24.3.0
    "
    ); // Verify that `tool run format-tool` at a different version (via `--from`) installs the new version.
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--from")
            .arg("format-tool==24.2.0")
            .arg("format-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    format-tool 24.2.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + format-support==1.0.0
     + format-tool==24.2.0
    "
    );
}

#[test]
fn tool_run_from_install_constraints() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts().with_tool_dirs(); // Install `web-tool` at a specific version.
    context
        .tool_install()
        .arg("web-tool==3.0.0")
        .assert()
        .success(); // Verify that `tool run web-tool` uses the already-installed version.
    uv_snapshot!(
        context.filters(),
        context.tool_run().arg("web-tool").arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    web-tool 3.0.0
    "
    ); // Verify that `tool run web-tool` with a compatible constraint uses the already-installed version.
    context
        .temp_dir
        .child("constraints.txt")
        .write_str("web-runtime<4.0.0")
        .unwrap();
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--constraints")
            .arg("constraints.txt")
            .arg("web-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    web-tool 3.0.0
    "
    ); // Verify that `tool run web-tool` with an incompatible constraint installs a new version.
    context
        .temp_dir
        .child("constraints.txt")
        .write_str("web-runtime<3.0.0")
        .unwrap();
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--constraints")
            .arg("constraints.txt")
            .arg("web-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    web-tool 2.3.3

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + web-runtime==2.3.8
     + web-tool==2.3.3
    "
    ); // Verify that `tool run web-tool` with a compatible override uses the already-installed version.
    context
        .temp_dir
        .child("override.txt")
        .write_str("web-runtime==3.0.1")
        .unwrap();
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--override")
            .arg("override.txt")
            .arg("web-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    web-tool 3.0.0
    "
    ); // Verify that `tool run web-tool` with an incompatible override installs a new version.
    context
        .temp_dir
        .child("override.txt")
        .write_str("web-runtime==3.0.0")
        .unwrap();
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--override")
            .arg("override.txt")
            .arg("web-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    web-tool 3.0.2

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + web-runtime==3.0.0
     + web-tool==3.0.2
    "
    ); // Verify that an override that enables a new extra also invalidates the environment.
    context
        .temp_dir
        .child("override.txt")
        .write_str("web-tool[dotenv]")
        .unwrap();
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--override")
            .arg("override.txt")
            .arg("web-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    web-tool 3.0.0
    "
    );
}

#[test]
fn tool_run_cache() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_default_index(&_server.index_url());
    let context = context.with_filtered_counts().with_tool_dirs(); // Verify that `tool run format-tool` installs the latest version.
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("-p")
            .arg("3.12")
            .arg("format-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    format-tool 24.3.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + format-support==1.0.0
     + format-tool==24.3.0
    "
    ); // Verify that `tool run format-tool` uses the cached version.
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("-p")
            .arg("3.12")
            .arg("format-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    format-tool 24.3.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    "
    ); // Verify that `--refresh` allows cache reuse.
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("-p")
            .arg("3.12")
            .arg("--refresh")
            .arg("format-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    format-tool 24.3.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    "
    ); // Verify that `--refresh-package` allows cache reuse.
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("-p")
            .arg("3.12")
            .arg("--refresh-package")
            .arg("format-support")
            .arg("format-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    format-tool 24.3.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    "
    ); // Verify that varying the interpreter leads to a fresh environment.
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("-p")
            .arg("3.11")
            .arg("format-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    format-tool 24.3.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + format-support==1.0.0
     + format-tool==24.3.0
    "
    ); // But that re-invoking with the previous interpreter retains the cached version.
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("-p")
            .arg("3.12")
            .arg("format-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    format-tool 24.3.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    "
    ); // Verify that `--with` leads to a fresh environment.
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("-p")
            .arg("3.12")
            .arg("--with")
            .arg("extra-requirement")
            .arg("format-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    format-tool 24.3.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + extra-requirement==2.0.0
     + format-support==1.0.0
     + format-tool==24.3.0
    "
    );
}

#[test]
fn tool_run_url() {
    let registry_artifacts = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12")
        .with_default_index(&registry_artifacts.index_url())
        .with_filtered_counts()
        .with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg(format!("run-tool @ {}", registry_artifacts.file_url("run_tool-8.1.1-py3-none-any.whl")))
        .arg("run-tool")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    run-tool 8.1.1

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + run-helper==1.4.0
     + run-tool==8.1.1 (from http://[LOCALHOST]/files/run_tool-8.1.1-py3-none-any.whl)
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg(registry_artifacts.file_url("run_tool-8.1.1-py3-none-any.whl"))
        .arg("run-tool")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    run-tool 8.1.1

    ----- stderr -----
    Resolved [N] packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg(format!("run-tool @ {}", registry_artifacts.file_url("run_tool-8.1.1-py3-none-any.whl")))
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    run-tool 8.1.1

    ----- stderr -----
    Resolved [N] packages in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg(registry_artifacts.file_url("run_tool-8.1.1-py3-none-any.whl"))
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    run-tool 8.1.1

    ----- stderr -----
    Resolved [N] packages in [TIME]
    ");
}

/// Test running a tool with a Git requirement.
#[test]
#[cfg(feature = "test-git")]
fn tool_run_git() -> Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_packse_index("packages/tool-run.toml")
        .with_filter((r"@[0-9a-f]{40}", "@[COMMIT]"))
        .with_filtered_counts()
        .with_tool_dirs();
    let repository_url = crate::git::tool_repository(&context)?;

    uv_snapshot!(context.filters(), context.tool_run()
        .arg(&repository_url)
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    git-tool 1.0.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + extra-requirement==2.0.0
     + git-tool==1.0.0 (from git+file://[TEMP_DIR]/repository@[COMMIT])
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg(format!("git-tool @ {repository_url}"))
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    git-tool 1.0.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    ");

    // Clear the cache.
    fs_err::remove_dir_all(&context.cache_dir).expect("Failed to remove cache dir.");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg(&repository_url)
        .arg("git-tool")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    git-tool 1.0.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + extra-requirement==2.0.0
     + git-tool==1.0.0 (from git+file://[TEMP_DIR]/repository@[COMMIT])
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg(format!("git-tool @ {repository_url}"))
        .arg("git-tool")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    git-tool 1.0.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    ");

    Ok(())
}

/// Test that running a tool from Git uses statically available `requires-python` metadata before
/// selecting a global Python pin.
#[test]
#[cfg(feature = "test-git")]
fn tool_run_git_infers_static_requires_python() {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.11"])
        .with_filtered_counts()
        .with_tool_dirs();

    context
        .python_pin()
        .arg("3.11")
        .arg("--global")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg("git+https://github.com/astral-sh/uv-dynamic-requires-python-test@75a612dc87fc215e999a25a0efc376cbf9831afa#subdirectory=static")
        .arg("static-requires-python-tool"), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.12

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + static-requires-python-tool==0.1.0 (from git+https://github.com/astral-sh/uv-dynamic-requires-python-test@75a612dc87fc215e999a25a0efc376cbf9831afa#subdirectory=static)
    ");
}

/// Test that running a tool from Git does not infer dynamic `requires-python` metadata.
#[test]
#[cfg(feature = "test-git")]
fn tool_run_git_does_not_infer_dynamic_requires_python() {
    let context = uv_test::test_context_with_versions!(&["3.12", "3.11"])
        .with_filtered_counts()
        .with_tool_dirs();

    context
        .python_pin()
        .arg("3.11")
        .arg("--global")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg("git+https://github.com/astral-sh/uv-dynamic-requires-python-test@75a612dc87fc215e999a25a0efc376cbf9831afa#subdirectory=dynamic")
        .arg("dynamic-requires-python-tool"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving tool dependencies
      cause: Because the current Python version (3.11.[X]) does not satisfy Python>=3.12,<3.13 and dynamic-requires-python-tool==0.1.0 depends on Python>=3.12,<3.13, we can conclude that dynamic-requires-python-tool==0.1.0 cannot be used.
             And because only dynamic-requires-python-tool==0.1.0 is available and you require dynamic-requires-python-tool, we can conclude that your requirements are unsatisfiable.
    ");
}

/// Test running a tool with a Git LFS enabled requirement.
#[test]
#[cfg(feature = "test-git-lfs")]
fn tool_run_git_lfs() {
    let context = uv_test::test_context!("3.13")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_git_lfs_config()
        .with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--lfs")
        .arg("git+https://github.com/astral-sh/test-lfs-repo@e282f5be233e3f1d44934164895a043fc534b8aa"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello from test-lfs-repo!

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + test-lfs-repo==0.1.0 (from git+https://github.com/astral-sh/test-lfs-repo@e282f5be233e3f1d44934164895a043fc534b8aa#lfs=true)
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--lfs")
        .arg("test-lfs-repo @ git+https://github.com/astral-sh/test-lfs-repo@e282f5be233e3f1d44934164895a043fc534b8aa"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello from test-lfs-repo!

    ----- stderr -----
    Resolved [N] packages in [TIME]
    ");

    // Clear the cache.
    fs_err::remove_dir_all(&context.cache_dir).expect("Failed to remove cache dir.");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg("git+https://github.com/astral-sh/test-lfs-repo@e282f5be233e3f1d44934164895a043fc534b8aa")
        .arg("--lfs")
        .arg("test-lfs-repo-assets"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello from test-lfs-repo! LFS_TEST=True ANOTHER_LFS_TEST=True

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + test-lfs-repo==0.1.0 (from git+https://github.com/astral-sh/test-lfs-repo@e282f5be233e3f1d44934164895a043fc534b8aa#lfs=true)
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg("test-lfs-repo @ git+https://github.com/astral-sh/test-lfs-repo@e282f5be233e3f1d44934164895a043fc534b8aa")
        .arg("--lfs")
        .arg("test-lfs-repo-assets"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello from test-lfs-repo! LFS_TEST=True ANOTHER_LFS_TEST=True

    ----- stderr -----
    Resolved [N] packages in [TIME]
    ");

    // Clear the cache.
    fs_err::remove_dir_all(&context.cache_dir).expect("Failed to remove cache dir.");

    // Attempt to run when LFS artifacts are missing and LFS is requested.

    // The filters below will remove any boilerplate before what we actually want to match.
    // They help handle slightly different output in uv-distribution/src/source/mod.rs between
    // calls to `git` and `git_metadata` functions which don't have guaranteed execution order.
    // In addition, we can get different error codes depending on where the failure occurs,
    // although we know the error code cannot be 0.
    let context = context
        .with_filter((r"exit_code: -?[1-9]\d*", "exit_code: [ERROR_CODE]"))
        .with_filter((
            "(?s)(----- stderr -----).*?The source distribution `[^`]+` is missing Git LFS artifacts.*",
            "$1\n[PREFIX]The source distribution `[DISTRIBUTION]` is missing Git LFS artifacts",
        ));

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--lfs")
        .arg("test-lfs-repo @ git+https://github.com/astral-sh/test-lfs-repo@e282f5be233e3f1d44934164895a043fc534b8aa")
        .env(EnvVars::UV_INTERNAL__TEST_LFS_DISABLED, "1"), @"
    exit_code: [ERROR_CODE] (failure)
    ----- stderr -----
    [PREFIX]The source distribution `[DISTRIBUTION]` is missing Git LFS artifacts
    ");

    // Attempt to run when LFS artifacts are missing but LFS was not requested.
    #[cfg(not(windows))]
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg("test-lfs-repo @ git+https://github.com/astral-sh/test-lfs-repo@e282f5be233e3f1d44934164895a043fc534b8aa")
        .arg("test-lfs-repo-assets"), @r#"
    exit_code: [ERROR_CODE] (failure)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + test-lfs-repo==0.1.0 (from git+https://github.com/astral-sh/test-lfs-repo@e282f5be233e3f1d44934164895a043fc534b8aa)
    Traceback (most recent call last):
      File "[CACHE_DIR]/archive-v0/[HASH]/bin/test-lfs-repo-assets", line 12, in <module>
        sys.exit(main_lfs())
                 ~~~~~~~~^^
      File "[CACHE_DIR]/archive-v0/[HASH]/[PYTHON-LIB]/site-packages/test_lfs_repo/__init__.py", line 5, in main_lfs
        from .lfs_module import LFS_TEST
      File "[CACHE_DIR]/archive-v0/[HASH]/[PYTHON-LIB]/site-packages/test_lfs_repo/lfs_module.py", line 1
        version https://git-lfs.github.com/spec/v1
                ^^^^^
    SyntaxError: invalid syntax
    "#);

    #[cfg(windows)]
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg("test-lfs-repo @ git+https://github.com/astral-sh/test-lfs-repo@e282f5be233e3f1d44934164895a043fc534b8aa")
        .arg("test-lfs-repo-assets"), @r#"
    exit_code: [ERROR_CODE] (failure)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + test-lfs-repo==0.1.0 (from git+https://github.com/astral-sh/test-lfs-repo@e282f5be233e3f1d44934164895a043fc534b8aa)
    Traceback (most recent call last):
      File "<frozen runpy>", line 198, in _run_module_as_main
      File "<frozen runpy>", line 88, in _run_code
      File "[CACHE_DIR]/archive-v0/[HASH]/Scripts/test-lfs-repo-assets/__main__.py", line 10, in <module>
        sys.exit(main_lfs())
                 ~~~~~~~~^^
      File "[CACHE_DIR]/archive-v0/[HASH]/[PYTHON-LIB]/site-packages/test_lfs_repo/__init__.py", line 5, in main_lfs
        from .lfs_module import LFS_TEST
      File "[CACHE_DIR]/archive-v0/[HASH]/[PYTHON-LIB]/site-packages/test_lfs_repo/lfs_module.py", line 1
        version https://git-lfs.github.com/spec/v1
                ^^^^^
    SyntaxError: invalid syntax
    "#);
}

/// Read requirements from a `requirements.txt` file.
#[test]
fn tool_run_requirements_txt() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts().with_tool_dirs();
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("extra-requirement").unwrap();
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--with-requirements")
            .arg("requirements.txt")
            .arg("--with")
            .arg("other-requirement")
            .arg("web-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    web-tool 3.0.2

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + extra-requirement==2.0.0
     + other-requirement==4.10.0
     + web-runtime==3.0.1
     + web-tool==3.0.2
    "
    );
}

/// Ignore and warn when (e.g.) the `--index-url` argument is a provided `requirements.txt`.
#[test]
fn tool_run_requirements_txt_arguments() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts().with_tool_dirs();
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt
        .write_str(indoc! { r"
        --index-url https://test.pypi.org/simple
        no-script-leaf
        " })
        .unwrap();
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--with-requirements")
            .arg("requirements.txt")
            .arg("web-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    web-tool 3.0.2

    ----- stderr -----
    warning: Ignoring `--index-url` from requirements file: `https://test.pypi.org/simple`. Instead, use the `--index-url` command-line argument, or set `index-url` in a `uv.toml` or `pyproject.toml` file.
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + no-script-leaf==1.0.0
     + web-runtime==3.0.1
     + web-tool==3.0.2
    "
    );
}

/// List installed tools when no command arg is given (e.g. `uv tool run`).
#[test]
fn tool_run_list_installed() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_exe_suffix().with_tool_dirs(); // No tools installed.
    uv_snapshot!(context.filters(), context.tool_run(), @"
    exit_code: 2 (failure)
    ----- stdout -----
    Provide a command to run with `uv tool run <command>`.

    See `uv tool run --help` for more information.
    "); // Install `format-tool`.
    context
        .tool_install()
        .arg("format-tool==24.2.0")
        .assert()
        .success(); // List installed tools.
    uv_snapshot!(context.filters(), context.tool_run(), @"
    exit_code: 2 (failure)
    ----- stdout -----
    Provide a command to run with `uv tool run <command>`.

    The following tools are installed:

    - format-tool v24.2.0

    See `uv tool run --help` for more information.
    ");
}

/// By default, omit resolver and installer output.
#[test]
fn tool_run_without_output() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts().with_tool_dirs(); // On the first run, only show the summary line.
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .env_remove(EnvVars::UV_SHOW_RESOLUTION)
            .arg("--")
            .arg("run-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    run-tool 8.1.1

    ----- stderr -----
    Installed [N] packages in [TIME]
    "
    ); // Subsequent runs are quiet.
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .env_remove(EnvVars::UV_SHOW_RESOLUTION)
            .arg("--")
            .arg("run-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    run-tool 8.1.1
    "
    );
}

#[test]
#[cfg(not(windows))]
fn tool_run_csv_with_shorthand() -> anyhow::Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts().with_tool_dirs();
    let anyio_local = context.temp_dir.child("src").child("anyio_local");
    copy_dir_all(
        context.workspace_root.join("test/packages/anyio_local"),
        &anyio_local,
    )?;
    let black_editable = context.temp_dir.child("src").child("black_editable");
    copy_dir_all(
        context.workspace_root.join("test/packages/black_editable"),
        &black_editable,
    )?;
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["no-script", "no-script-leaf==1.0.0"]
        "# })?;
    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r"
        import no_script_leaf
       " })?; // Performs a tool run with a comma-separated `--with` flag.
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("-w")
            .arg("extra-requirement,other-requirement")
            .arg("run-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    run-tool 8.1.1

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + extra-requirement==2.0.0
     + other-requirement==4.10.0
     + run-helper==1.4.0
     + run-tool==8.1.1
    "
    );
    Ok(())
}

#[test]
#[cfg(not(windows))]
fn tool_run_csv_with() -> anyhow::Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts().with_tool_dirs();
    let anyio_local = context.temp_dir.child("src").child("anyio_local");
    copy_dir_all(
        context.workspace_root.join("test/packages/anyio_local"),
        &anyio_local,
    )?;
    let black_editable = context.temp_dir.child("src").child("black_editable");
    copy_dir_all(
        context.workspace_root.join("test/packages/black_editable"),
        &black_editable,
    )?;
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["no-script", "no-script-leaf==1.0.0"]
        "# })?;
    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r"
        import no_script_leaf
       " })?; // Performs a tool run with a comma-separated `--with` flag.
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--with")
            .arg("extra-requirement,other-requirement")
            .arg("run-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    run-tool 8.1.1

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + extra-requirement==2.0.0
     + other-requirement==4.10.0
     + run-helper==1.4.0
     + run-tool==8.1.1
    "
    );
    Ok(())
}

#[test]
#[cfg(windows)]
fn tool_run_csv_with() -> anyhow::Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts().with_tool_dirs();
    let anyio_local = context.temp_dir.child("src").child("anyio_local");
    copy_dir_all(
        context.workspace_root.join("test/packages/anyio_local"),
        &anyio_local,
    )?;
    let black_editable = context.temp_dir.child("src").child("black_editable");
    copy_dir_all(
        context.workspace_root.join("test/packages/black_editable"),
        &black_editable,
    )?;
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["no-script", "no-script-leaf==1.0.0"]
        "# })?;
    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r"
        import no_script_leaf
       " })?; // Performs a tool run with a comma-separated `--with` flag.
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--with")
            .arg("extra-requirement,other-requirement")
            .arg("run-tool")
            .arg("--version"),
        @""
    );
    Ok(())
}

#[test]
#[cfg(not(windows))]
fn tool_run_repeated_with() -> anyhow::Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts().with_tool_dirs();
    let anyio_local = context.temp_dir.child("src").child("anyio_local");
    copy_dir_all(
        context.workspace_root.join("test/packages/anyio_local"),
        &anyio_local,
    )?;
    let black_editable = context.temp_dir.child("src").child("black_editable");
    copy_dir_all(
        context.workspace_root.join("test/packages/black_editable"),
        &black_editable,
    )?;
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["no-script", "no-script-leaf==1.0.0"]
        "# })?;
    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r"
        import no_script_leaf
       " })?; // Performs a tool run with a repeated `--with` flag.
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--with")
            .arg("extra-requirement")
            .arg("--with")
            .arg("other-requirement")
            .arg("run-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    run-tool 8.1.1

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + extra-requirement==2.0.0
     + other-requirement==4.10.0
     + run-helper==1.4.0
     + run-tool==8.1.1
    "
    );
    Ok(())
}

#[test]
#[cfg(windows)]
fn tool_run_repeated_with() -> anyhow::Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts().with_tool_dirs();
    let anyio_local = context.temp_dir.child("src").child("anyio_local");
    copy_dir_all(
        context.workspace_root.join("test/packages/anyio_local"),
        &anyio_local,
    )?;
    let black_editable = context.temp_dir.child("src").child("black_editable");
    copy_dir_all(
        context.workspace_root.join("test/packages/black_editable"),
        &black_editable,
    )?;
    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["no-script", "no-script-leaf==1.0.0"]
        "# })?;
    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r"
        import no_script_leaf
       " })?; // Performs a tool run with a repeated `--with` flag.
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--with")
            .arg("extra-requirement")
            .arg("--with")
            .arg("other-requirement")
            .arg("run-tool")
            .arg("--version"),
        @""
    );
    Ok(())
}

#[test]
fn tool_run_with_editable() -> anyhow::Result<()> {
    let context = uv_test::test_context!("3.12")
        .with_packse_index("packages/tool-run.toml")
        .with_filtered_counts()
        .with_tool_dirs();

    let anyio_local = context.temp_dir.child("src").child("anyio_local");
    copy_dir_all(
        context.workspace_root.join("test/packages/anyio_local"),
        &anyio_local,
    )?;

    let black_editable = context.temp_dir.child("src").child("black_editable");
    copy_dir_all(
        context.workspace_root.join("test/packages/black_editable"),
        &black_editable,
    )?;

    let pyproject_toml = context.temp_dir.child("pyproject.toml");
    pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = ["no-script", "no-script-leaf==1.0.0"]
        "#
    })?;

    let test_script = context.temp_dir.child("main.py");
    test_script.write_str(indoc! { r"
        import no_script_leaf
       "
    })?;

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--with-editable")
        .arg("./src/black_editable")
        .arg("--with")
        .arg("extra-requirement")
        .arg("web-tool")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    web-tool 3.0.2

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + black==0.1.0 (from file://[TEMP_DIR]/src/black_editable)
     + extra-requirement==2.0.0
     + web-runtime==3.0.1
     + web-tool==3.0.2
    ");

    // Requesting an editable requirement should install it in a layer, even if it satisfied
    uv_snapshot!(context.filters(), context.tool_run().arg("--with-editable").arg("./src/anyio_local").arg("web-tool").arg("--version"),
    @"
    exit_code: 0 (success)
    ----- stdout -----
    web-tool 3.0.2

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + anyio==4.3.0+foo (from file://[TEMP_DIR]/src/anyio_local)
     + web-runtime==3.0.1
     + web-tool==3.0.2
    ");

    // Requesting the project itself should use a new environment.
    uv_snapshot!(context.filters(), context.tool_run().arg("--with-editable").arg(".").arg("web-tool").arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    web-tool 3.0.2

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/)
     + no-script==2.31.0
     + no-script-leaf==1.0.0
     + web-runtime==3.0.1
     + web-tool==3.0.2
    ");

    Ok(())
}

/// Invalid `--with` requirements should use the standard user-error renderer.
#[test]
fn tool_run_invalid_with() {
    let context = uv_test::test_context!("3.12").with_tool_dirs();

    uv_snapshot!(context.filters(), context
        .tool_run()
        .arg("--with")
        .arg("./foo")
        .arg("flask")
        .arg("--version")
        , @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to resolve `--with` requirement
      cause: Distribution not found at: file://[TEMP_DIR]/foo
    ");
}

#[test]
fn warn_no_executables_found() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_exe_suffix().with_tool_dirs();
    uv_snapshot!(
        context.filters(),
        context.tool_run().arg("no-script"),
        @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + no-script==2.31.0
     + no-script-leaf==1.0.0
    Package `no-script` does not provide any executables.
    "
    );
}

/// Warn when a user passes `--upgrade` to `uv tool run`.
#[test]
fn tool_run_upgrade_warn() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts().with_tool_dirs();
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--upgrade")
            .arg("run-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    run-tool 8.1.1

    ----- stderr -----
    warning: Tools cannot be upgraded via `uv tool run`; use `uv tool upgrade --all` to upgrade all installed tools, or `uv tool run package@latest` to run the latest version of a tool.
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + run-helper==1.4.0
     + run-tool==8.1.1
    "
    );
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--upgrade")
            .arg("--with")
            .arg("other-requirement")
            .arg("run-tool")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    run-tool 8.1.1

    ----- stderr -----
    warning: Tools cannot be upgraded via `uv tool run`; use `uv tool upgrade --all` to upgrade all installed tools, `uv tool run package@latest` to run the latest version of a tool, or `uv tool run --refresh package` to upgrade any `--with` dependencies.
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + other-requirement==4.10.0
     + run-helper==1.4.0
     + run-tool==8.1.1
    "
    );
}

/// If we fail to resolve the tool, we should include "tool" in the error message.
#[test]
fn tool_run_resolution_error() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts().with_tool_dirs();
    uv_snapshot!(
        context.filters(),
        context.tool_run().arg("add"),
        @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving tool dependencies
      cause: Because add was not found in the package registry and you require add, we can conclude that your requirements are unsatisfiable.
    "
    );
}

#[test]
fn tool_run_latest() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_exe_suffix().with_tool_dirs(); // Install `run-tool` at a specific version.
    context
        .tool_install()
        .arg("run-tool==7.0.0")
        .assert()
        .success(); // Run `run-tool`, which should use the installed version.
    uv_snapshot!(
        context.filters(),
        context.tool_run().arg("run-tool").arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    run-tool 7.0.0
    "
    ); // Run `run-tool@latest`, which should use the latest version.
    uv_snapshot!(
        context.filters(),
        context.tool_run().arg("run-tool@latest").arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    run-tool 8.1.1

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + run-helper==1.4.0
     + run-tool==8.1.1
    "
    ); // Run `run-tool`, which should use the installed version.
    uv_snapshot!(
        context.filters(),
        context.tool_run().arg("run-tool").arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    run-tool 7.0.0
    "
    );
}

#[test]
fn tool_run_latest_extra() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_exe_suffix().with_tool_dirs();
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("web-tool[dotenv]@latest")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    web-tool 3.0.2

    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + web-extra==1.0.1
     + web-runtime==3.0.1
     + web-tool==3.0.2
    "
    );
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("web-tool[dotenv]@3.0.0")
            .arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    web-tool 3.0.0

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 2 packages in [TIME]
     + web-runtime==3.0.1
     + web-tool==3.0.0
    warning: The package `web-tool==3.0.0` does not have an extra named `dotenv`
    "
    );
}

#[test]
fn tool_run_extra() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_exe_suffix().with_tool_dirs();
    uv_snapshot!(
        context.filters(),
        context.tool_run().arg("web-tool[dotenv]").arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    web-tool 3.0.2

    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 3 packages in [TIME]
    Installed 3 packages in [TIME]
     + web-extra==1.0.1
     + web-runtime==3.0.1
     + web-tool==3.0.2
    "
    );
}

#[test]
fn tool_run_specifier() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_exe_suffix().with_tool_dirs();
    uv_snapshot!(
        context.filters(),
        context.tool_run().arg("web-tool<3.0.0").arg("--version"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    web-tool 2.3.3

    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Installed 2 packages in [TIME]
     + web-runtime==2.3.8
     + web-tool==2.3.3
    "
    );
}

#[test]
fn tool_run_python() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts();
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("python")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Resolved in [TIME]
    Checked in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("python")
        .arg("-c")
        .arg("print('Hello, world!')"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Hello, world!

    ----- stderr -----
    Resolved in [TIME]
    ");
}

#[test]
fn tool_run_python_at_version() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context_with_versions!(&["3.12", "3.11"])
        .with_default_index(&_server.index_url());
    let context = context
        .with_filtered_counts()
        .with_filtered_python_sources();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("python")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Resolved in [TIME]
    Checked in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tool_run()
            .arg("python@3.12")
            .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Resolved in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("python@3.11")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Resolved in [TIME]
    Checked in [TIME]
    ");

    // The @ is optional.
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("python3.11")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Resolved in [TIME]
    ");

    // Dotless syntax also works.
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("python311")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Resolved in [TIME]
    ");

    // Other implementations like PyPy also work. PyPy isn't currently in the test suite, so
    // specify CPython and rely on the fact that they go through the same codepath.
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("cpython311")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Resolved in [TIME]
    ");

    // But short names don't work in the executable position (as opposed to with -p/--python). We
    // interpret those as package names.
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("cp311")
        .arg("--version"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving tool dependencies
      cause: Because cp311 was not found in the package registry and you require cp311, we can conclude that your requirements are unsatisfiable.
    ");

    // Bare versions don't work either. Again we interpret them as package names.
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("311")
        .arg("--version"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving tool dependencies
      cause: Because 311 was not found in the package registry and you require 311, we can conclude that your requirements are unsatisfiable.
    ");

    // Request a version via `-p`
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("-p")
        .arg("3.11")
        .arg("python")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Resolved in [TIME]
    ");

    // @ syntax is also allowed here.
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("-p")
        .arg("python@311")
        .arg("python")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Resolved in [TIME]
    ");

    // But @ with nothing in front of it is not.
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("-p")
        .arg("@311")
        .arg("python")
        .arg("--version"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for executable name `@311` in [PYTHON SOURCES]
    ");

    // Request a version in the tool and `-p`
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("-p")
        .arg("3.12")
        .arg("python@3.11")
        .arg("--version"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Received multiple Python version requests: `3.12` and `3.11`
    ");

    // Request a version that does not exist
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("python@3.12.99"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.12.[X] in [PYTHON SOURCES]
    ");

    // Request an invalid version
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("python@3.300"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Invalid version request: 3.300
    ");

    // Request `@latest` (not yet supported)
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("python@latest")
        .arg("--version"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Requesting the 'latest' Python version is not yet supported
    ");
}

#[test]
fn tool_run_hint_version_not_available() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context =
        uv_test::test_context_with_versions!(&[]).with_default_index(&_server.index_url());
    let context = context
        .with_filtered_counts()
        .with_filtered_python_sources();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("python@3.12")
        .env(EnvVars::UV_PYTHON_DOWNLOADS, "never"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.12 in [PYTHON SOURCES]

    hint: A managed Python download is available for Python 3.12, but Python downloads are set to 'never'
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("python@3.12")
        .env(EnvVars::UV_PYTHON_DOWNLOADS, "auto")
        .env(EnvVars::UV_OFFLINE, "true"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.12 in [PYTHON SOURCES]

    hint: A managed Python download is available for Python 3.12, but uv is set to offline mode
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("python@3.12")
        .env(EnvVars::UV_PYTHON_DOWNLOADS, "auto")
        .env(EnvVars::UV_NO_MANAGED_PYTHON, "true"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.12 in [PYTHON SOURCES]

    hint: A managed Python download is available for Python 3.12, but the Python preference is set to 'only system'
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--no-managed-python")
        .arg("python@3.12")
        .env(EnvVars::UV_PYTHON_DOWNLOADS, "auto")
        .env(EnvVars::UV_OFFLINE, "true")
        .env(EnvVars::UV_MANAGED_PYTHON, "true"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: No interpreter found for Python 3.12 in [PYTHON SOURCES]

    hint: A managed Python download is available for Python 3.12, but the Python preference is set to 'only system'
    ");
}

#[test]
fn tool_run_python_from_global_version_file() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context_with_versions!(&["3.12", "3.11"])
        .with_default_index(&_server.index_url());
    let context = context
        .with_filtered_counts()
        .with_filtered_python_sources();

    context
        .python_pin()
        .arg("3.11")
        .arg("--global")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("python")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Resolved in [TIME]
    Checked in [TIME]
    ");
}

#[test]
fn tool_run_python_version_overrides_global_pin() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context_with_versions!(&["3.12", "3.11"])
        .with_default_index(&_server.index_url());
    let context = context
        .with_filtered_counts()
        .with_filtered_python_sources();

    // Set global pin to 3.11
    context
        .python_pin()
        .arg("3.11")
        .arg("--global")
        .assert()
        .success();

    // Explicitly request python3.12, should override global pin
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("python3.12")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Resolved in [TIME]
    Checked in [TIME]
    ");
}

#[test]
fn tool_run_python_with_explicit_default_bypasses_global_pin() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context_with_versions!(&["3.12", "3.11"])
        .with_default_index(&_server.index_url());
    let context = context
        .with_filtered_counts()
        .with_filtered_python_sources();

    // Set global pin to 3.11
    context
        .python_pin()
        .arg("3.11")
        .arg("--global")
        .assert()
        .success();

    // Explicitly request --python default, should bypass global pin and use system default (3.12)
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--python")
        .arg("default")
        .arg("python")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Resolved in [TIME]
    Checked in [TIME]
    ");
}

#[test]
fn tool_run_python_from() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context_with_versions!(&["3.12", "3.11"])
        .with_default_index(&_server.index_url());
    let context = context
        .with_filtered_counts()
        .with_filtered_python_sources();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg("python")
        .arg("python")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Resolved in [TIME]
    Checked in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg("python@3.11")
        .arg("python")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Resolved in [TIME]
    Checked in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg("python311")
        .arg("python")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Resolved in [TIME]
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg("python>3.11,<3.13")
        .arg("python")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.12.[X]

    ----- stderr -----
    Resolved in [TIME]
    ");

    // The executed command isn't necessarily Python, but Python is in the PATH.
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg("python@3.11")
        .arg("bash")
        .arg("-c")
        .arg("python --version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Python 3.11.[X]

    ----- stderr -----
    Resolved in [TIME]
    ");
}

#[test]
fn tool_run_from_directory_uses_global_pin_when_within_requires_python_range() {
    let context = uv_test::test_context_with_versions!(&["3.13", "3.12", "3.11"])
        .with_filtered_counts()
        .with_tool_dirs();

    let foo_dir = context.temp_dir.child("foo");
    foo_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "foo"
        version = "0.1.0"
        requires-python = ">=3.11,<3.13"
        dependencies = []

        [project.scripts]
        foo = "foo.main:run"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#
        })
        .unwrap();
    foo_dir
        .child("src")
        .child("foo")
        .child("__init__.py")
        .touch()
        .unwrap();
    foo_dir
        .child("src")
        .child("foo")
        .child("main.py")
        .write_str(indoc! {r#"
        import sys

        def run():
            print(f"{sys.version_info.major}.{sys.version_info.minor}")
        "#
        })
        .unwrap();

    context
        .python_pin()
        .arg("3.11")
        .arg("--global")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg(foo_dir.as_os_str())
        .arg("foo"), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.11

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + foo==0.1.0 (from file://[TEMP_DIR]/foo)
    ");
}

#[test]
fn tool_run_from_directory_ignores_global_pin_outside_requires_python_range() {
    let context = uv_test::test_context_with_versions!(&["3.13", "3.12", "3.11"])
        .with_filtered_counts()
        .with_tool_dirs();

    let foo_dir = context.temp_dir.child("foo");
    foo_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "foo"
        version = "0.1.0"
        requires-python = ">=3.11,<3.13"
        dependencies = []

        [project.scripts]
        foo = "foo.main:run"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "#
        })
        .unwrap();
    foo_dir
        .child("src")
        .child("foo")
        .child("__init__.py")
        .touch()
        .unwrap();
    foo_dir
        .child("src")
        .child("foo")
        .child("main.py")
        .write_str(indoc! {r#"
        import sys

        def run():
            print(f"{sys.version_info.major}.{sys.version_info.minor}")
        "#
        })
        .unwrap();

    context
        .python_pin()
        .arg("3.13")
        .arg("--global")
        .assert()
        .success();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg(foo_dir.as_os_str())
        .arg("foo"), @"
    exit_code: 0 (success)
    ----- stdout -----
    3.12

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + foo==0.1.0 (from file://[TEMP_DIR]/foo)
    ");
}

#[test]
fn run_with_env_file() -> anyhow::Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts().with_tool_dirs(); // Create a project with a custom script.
    let foo_dir = context.temp_dir.child("foo");
    let foo_pyproject_toml = foo_dir.child("pyproject.toml");
    foo_pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = []

        [project.scripts]
        script = "foo.main:run"

        [build-system]
        requires = ["uv_build>=0.7,<10000"]
        build-backend = "uv_build"
        "# })?; // Create the `foo` module.
    let foo_project_src = foo_dir.child("src");
    let foo_module = foo_project_src.child("foo");
    foo_module.child("__init__.py").touch()?;
    let foo_main_py = foo_module.child("main.py");
    foo_main_py.write_str(indoc! { r#"
        def run():
            import os

            print(os.environ.get('THE_EMPIRE_VARIABLE'))
            print(os.environ.get('REBEL_1'))
            print(os.environ.get('REBEL_2'))
            print(os.environ.get('REBEL_3'))

        __name__ == "__main__" and run()
       "# })?;
    uv_snapshot!(
        context.filters(),
        context.tool_run().arg("--from").arg("./foo").arg("script"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    None
    None
    None
    None

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/foo)
    "
    );
    context.temp_dir.child(".file").write_str(indoc! { "
        THE_EMPIRE_VARIABLE=palpatine
        REBEL_1=leia_organa
        REBEL_2=obi_wan_kenobi
        REBEL_3=C3PO
       " })?;
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--env-file")
            .arg(".file")
            .arg("--from")
            .arg("./foo")
            .arg("script"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    palpatine
    leia_organa
    obi_wan_kenobi
    C3PO

    ----- stderr -----
    Resolved [N] packages in [TIME]
    "
    );
    let evil_tools = context.temp_dir.child(".evil-tools");
    let evil_tool = evil_tools.child("foo");
    let evil_python = if cfg!(windows) {
        let scripts = evil_tool.child("Scripts");
        scripts.create_dir_all()?;
        scripts.child("python.exe")
    } else {
        let bin = evil_tool.child("bin");
        bin.create_dir_all()?;
        bin.child("python3")
    };
    evil_python.write_str(indoc! { r"
        #!/bin/sh
        echo queried > queried.txt
        exit 1
    " })?;
    #[cfg(unix)]
    {
        let mut permissions = metadata(evil_python.path())?.permissions();
        permissions.set_mode(0o755);
        set_permissions(evil_python.path(), permissions)?;
    }
    context.temp_dir.child(".file").write_str(indoc! { "
        UV_TOOL_DIR=.evil-tools
        THE_EMPIRE_VARIABLE=palpatine
        REBEL_1=leia_organa
        REBEL_2=obi_wan_kenobi
        REBEL_3=C3PO
       " })?;
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--from")
            .arg("./foo")
            .arg("script")
            .env(EnvVars::UV_ENV_FILE, ".file"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    palpatine
    leia_organa
    obi_wan_kenobi
    C3PO

    ----- stderr -----
    Resolved [N] packages in [TIME]
    "
    );
    assert!(!context.temp_dir.child("queried.txt").exists());
    Ok(())
}

#[test]
fn tool_run_from_at() {
    let context = uv_test::test_context!("3.12")
        .with_exclude_newer("2025-01-18T00:00:00Z")
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg("executable-application@latest")
        .arg("app")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    executable-application 0.3.0

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + executable-application==0.3.0
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg("executable-application@0.2.0")
        .arg("app")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    executable-application 0.2.0

    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + executable-application==0.2.0
    ");
}

#[test]
fn tool_run_verbatim_name() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();

    // The normalized package name is `change-wheel-version`, but the executable is `change_wheel_version`.
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("verbatim_tool")
        .arg("--help"), @"
    exit_code: 0 (success)
    ----- stdout -----
    verbatim-tool 0.5.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + verbatim-tool==0.5.0
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("verbatim-tool")
        .arg("--help"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved [N] packages in [TIME]
    An executable named `verbatim-tool` is not provided by package `verbatim-tool`.
    The following executables are available:
    - verbatim_tool

    Use `uv tool run --from verbatim-tool verbatim_tool` instead.
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg("verbatim-tool")
        .arg("verbatim_tool")
        .arg("--help"), @"
    exit_code: 0 (success)
    ----- stdout -----
    verbatim-tool 0.5.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    ");
}

#[test]
fn tool_run_with_existing_py_script() -> anyhow::Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts();
    context.temp_dir.child("script.py").touch()?;

    uv_snapshot!(context.filters(), context.tool_run().arg("script.py"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: It looks like you tried to run a Python script at `script.py`, which is not supported by `uv tool run`

    hint: Use `uv run script.py` instead
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg(context.temp_dir.child("script.py").path()), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: It looks like you tried to run a Python script at `[TEMP_DIR]/script.py`, which is not supported by `uv tool run`

    hint: Use `uv run [TEMP_DIR]/script.py` instead
    ");
    Ok(())
}

#[test]
fn tool_run_with_existing_pyw_script() -> anyhow::Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts();
    context.temp_dir.child("script.pyw").touch()?;

    // We treat arguments before the command as uv arguments
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("script.pyw"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: It looks like you tried to run a Python script at `script.pyw`, which is not supported by `uv tool run`

    hint: Use `uv run script.pyw` instead
    ");
    Ok(())
}

#[test]
fn tool_run_with_nonexistent_py_script() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts();

    // We treat arguments before the command as uv arguments
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("script.py"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: It looks like you provided a Python script to run, which is not supported by `uv tool run`

    hint: We did not find a script at the requested path. If you meant to run a command from the `script-py` package, pass the normalized package name to `--from` to disambiguate, e.g., `uv tool run --from script-py script.py`
    ");
}

#[test]
fn tool_run_with_nonexistent_pyw_script() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts();

    // We treat arguments before the command as uv arguments
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("script.pyw"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: It looks like you provided a Python script to run, which is not supported by `uv tool run`

    hint: We did not find a script at the requested path. If you meant to run a command from the `script-pyw` package, pass the normalized package name to `--from` to disambiguate, e.g., `uv tool run --from script-pyw script.pyw`
    ");
}

#[test]
fn tool_run_with_from_script() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts();

    // We treat arguments before the command as uv arguments
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg("script.py")
        .arg("ruff"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: It looks like you provided a Python script to `--from`, which is not supported

    hint: If you meant to run a command from the `script-py` package, use the normalized package name instead to disambiguate, e.g., `uv tool run --from script-py ruff`
    ");
}

#[test]
fn tool_run_with_script_and_from_script() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts();

    // We treat arguments before the command as uv arguments
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--from")
        .arg("script.py")
        .arg("other-script.py"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: It looks like you provided a Python script to `--from`, which is not supported

    hint: If you meant to run a command from the `script-py` package, use the normalized package name instead to disambiguate, e.g., `uv tool run --from script-py other-script.py`
    ");
}

/// A repository can be named `foo.py`, so a URL requirement is not a script path.
#[test]
#[cfg(feature = "test-git")]
fn tool_run_with_url_ending_in_py() {
    let context = uv_test::test_context!("3.12").with_filtered_counts();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--offline")
        .arg("git+https://github.com/uPesy/easyeda2kicad.py"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to resolve `--with` requirement
      cause: Git operation failed
      cause: failed to fetch into: [CACHE_DIR]/git-v0/db/efbd3507bcbea33c
      cause: Remote Git fetches are not allowed because network connectivity is disabled (i.e., with `--offline`)
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--offline")
        .arg("easyeda2kicad @ git+https://github.com/uPesy/easyeda2kicad.py"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to resolve tool requirement
      cause: Failed to download and build `easyeda2kicad @ git+https://github.com/uPesy/easyeda2kicad.py`
      cause: Git operation failed
      cause: failed to fetch into: [CACHE_DIR]/git-v0/db/efbd3507bcbea33c
      cause: Remote Git fetches are not allowed because network connectivity is disabled (i.e., with `--offline`)
    ");
}

/// As above, but passed to `--from`.
#[test]
#[cfg(feature = "test-git")]
fn tool_run_with_from_url_ending_in_py() {
    let context = uv_test::test_context!("3.12").with_filtered_counts();

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--offline")
        .arg("--from")
        .arg("git+https://github.com/uPesy/easyeda2kicad.py")
        .arg("easyeda2kicad"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to resolve `--with` requirement
      cause: Git operation failed
      cause: failed to fetch into: [CACHE_DIR]/git-v0/db/efbd3507bcbea33c
      cause: Remote Git fetches are not allowed because network connectivity is disabled (i.e., with `--offline`)
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--offline")
        .arg("--from")
        .arg("easyeda2kicad @ git+https://github.com/uPesy/easyeda2kicad.py")
        .arg("easyeda2kicad"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to resolve tool requirement
      cause: Failed to download and build `easyeda2kicad @ git+https://github.com/uPesy/easyeda2kicad.py`
      cause: Git operation failed
      cause: failed to fetch into: [CACHE_DIR]/git-v0/db/efbd3507bcbea33c
      cause: Remote Git fetches are not allowed because network connectivity is disabled (i.e., with `--offline`)
    ");
}

/// Test that when a user provides `--verbose` to the subcommand,
/// we show a helpful hint.
#[test]
fn tool_run_verbose_hint() {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts().with_tool_dirs(); // Test with --verbose flag
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("nonexistent-package-foo")
            .arg("--verbose"),
        @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to run tool
      cause: No solution found when resolving dependencies
      cause: Because nonexistent-package-foo was not found in the package registry and you require nonexistent-package-foo, we can conclude that your requirements are unsatisfiable.

    hint: You provided `--verbose` to `nonexistent-package-foo`. Did you mean to provide it to `uv tool run`? e.g., `uv tool run --verbose nonexistent-package-foo`
    "
    ); // Test with -v flag
    uv_snapshot!(
        context.filters(),
        context.tool_run().arg("nonexistent-package-bar").arg("-v"),
        @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to run tool
      cause: No solution found when resolving dependencies
      cause: Because nonexistent-package-bar was not found in the package registry and you require nonexistent-package-bar, we can conclude that your requirements are unsatisfiable.

    hint: You provided `-v` to `nonexistent-package-bar`. Did you mean to provide it to `uv tool run`? e.g., `uv tool run -v nonexistent-package-bar`
    "
    ); // Test with -vv flag
    uv_snapshot!(
        context.filters(),
        context.tool_run().arg("nonexistent-package-baz").arg("-vv"),
        @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to run tool
      cause: No solution found when resolving dependencies
      cause: Because nonexistent-package-baz was not found in the package registry and you require nonexistent-package-baz, we can conclude that your requirements are unsatisfiable.

    hint: You provided `-vv` to `nonexistent-package-baz`. Did you mean to provide it to `uv tool run`? e.g., `uv tool run -vv nonexistent-package-baz`
    "
    ); // Test for false positives
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("nonexistent-package-quux")
            .arg("-version"),
        @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving tool dependencies
      cause: Because nonexistent-package-quux was not found in the package registry and you require nonexistent-package-quux, we can conclude that your requirements are unsatisfiable.
    "
    );
}

#[test]
fn tool_run_with_compatible_build_constraints() -> Result<()> {
    let context = uv_test::test_context!("3.9")
        .with_packse_index("packages/tool-build-constraints.toml")
        .with_exclude_newer("2024-05-04T00:00:00Z")
        .with_filtered_counts()
        .with_filtered_exe_suffix();
    let constraints_txt = context.temp_dir.child("build_constraints.txt");
    constraints_txt.write_str("setuptools>=40")?;

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--with")
        .arg("legacy-build-requirement==1.2")
        .arg("--build-constraints")
        .arg("build_constraints.txt")
        .arg("build-tool")
        .arg("--version"), @"
    exit_code: 0 (success)
    ----- stdout -----
    build-tool 1.0.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + build-tool==1.0.0
     + legacy-build-requirement==1.2.0
    ");

    Ok(())
}

#[test]
fn tool_run_with_incompatible_build_constraints() -> Result<()> {
    let context = uv_test::test_context!("3.9")
        .with_packse_index("packages/tool-build-constraints.toml")
        .with_exclude_newer("2024-05-04T00:00:00Z")
        .with_filtered_counts()
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    let constraints_txt = context.temp_dir.child("build_constraints.txt");
    constraints_txt.write_str("setuptools==2")?;

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--with")
        .arg("legacy-build-requirement==1.2")
        .arg("--build-constraints")
        .arg("build_constraints.txt")
        .arg("build-tool")
        .arg("--version")
        .env(EnvVars::PATH, bin_dir.as_os_str()), @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: Failed to download and build `legacy-build-requirement==1.2.0`
      cause: Failed to resolve requirements from `setup.py` build
      cause: No solution found when resolving: `setuptools>=40.8.0`
      cause: Because you require setuptools>=40.8.0 and setuptools==2, we can conclude that your requirements are unsatisfiable.
    ");

    Ok(())
}

#[test]
fn tool_run_with_dependencies_from_script() -> Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context
        .with_filtered_counts()
        .with_filtered_missing_file_error();

    let script_contents = indoc! {r#"
        # /// script
        # requires-python = ">=3.11"
        # dependencies = [
        #   "no-script",
        # ]
        # ///

        import no_script
    "#};

    let script = context.temp_dir.child("script.py");
    script.write_str(script_contents)?;

    let script_without_extension = context.temp_dir.child("script-no-ext");
    script_without_extension.write_str(script_contents)?;

    // script dependencies are now installed.
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--with-requirements")
        .arg("script.py")
        .arg("format-tool")
        .arg("script.py")
        .arg("-q"), @"
    exit_code: 0 (success)
    ----- stdout -----
    format-tool 24.3.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + format-support==1.0.0
     + format-tool==24.3.0
     + no-script==2.31.0
     + no-script-leaf==1.0.0
    ");

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--with-requirements")
        .arg("script-no-ext")
        .arg("format-tool")
        .arg("script-no-ext")
        .arg("-q"), @"
    exit_code: 0 (success)
    ----- stdout -----
    format-tool 24.3.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    ");

    // Error when the script is not a valid PEP723 script.
    let script = context.temp_dir.child("not_pep723_script.py");
    script.write_str("import no_script")?;

    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--with-requirements")
        .arg("not_pep723_script.py")
        .arg("format-tool"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: `not_pep723_script.py` does not contain inline script metadata
    ");

    // Error when the script doesn't exist.
    uv_snapshot!(context.filters(), context.tool_run()
        .arg("--with-requirements")
        .arg("missing_file.py")
        .arg("format-tool"), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: failed to read from file `missing_file.py`: [OS ERROR 2]
    ");

    Ok(())
}

/// Test windows runnable types, namely console scripts and legacy setuptools scripts.
/// Console Scripts <https://packaging.python.org/en/latest/guides/writing-pyproject-toml/#console-scripts>
/// Legacy Scripts <https://packaging.python.org/en/latest/guides/distributing-packages-using-setuptools/#scripts>.
///
/// This tests for uv tool run of windows runnable types defined by [`WindowsRunnable`].
#[cfg(windows)]
#[test]
fn tool_run_windows_runnable_types() -> anyhow::Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts().with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let foo_dir = context.temp_dir.child("foo");
    let foo_pyproject_toml = foo_dir.child("pyproject.toml"); // Use `script-files` which enables legacy scripts packaging.
    foo_pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.8"
        dependencies = []

        [project.scripts]
        custom_pydoc = "foo.main:run"

        [tool.setuptools]
        script-files = [
            "misc/custom_pydoc.bat",
            "misc/custom_pydoc.cmd",
            "misc/custom_pydoc.ps1"
        ]

        [build-system]
        requires = ["setuptools>=42"]
        build-backend = "setuptools.build_meta"
        "# })?; // Create the legacy scripts
    let custom_pydoc_bat = foo_dir.child("misc").child("custom_pydoc.bat");
    let custom_pydoc_cmd = foo_dir.child("misc").child("custom_pydoc.cmd");
    let custom_pydoc_ps1 = foo_dir.child("misc").child("custom_pydoc.ps1");
    custom_pydoc_bat.write_str("python.exe -m pydoc %*")?;
    custom_pydoc_cmd.write_str("python.exe -m pydoc %*")?;
    custom_pydoc_ps1.write_str("python.exe -m pydoc $args")?; // Create the foo module
    let foo_project_src = foo_dir.child("src");
    let foo_module = foo_project_src.child("foo");
    let foo_main_py = foo_module.child("main.py");
    foo_main_py.write_str(indoc! { r#"
        import pydoc, sys

        def run():
            sys.argv[0] = "pydoc"
            pydoc.cli()

        __name__ == "__main__" and run()
       "# })?; // Install `foo` tool.
    context
        .tool_install()
        .arg(foo_dir.as_os_str())
        .env(EnvVars::PATH, bin_dir.as_os_str())
        .assert()
        .success();
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--from")
            .arg("foo")
            .arg("does_not_exist")
            .env(EnvVars::PATH, bin_dir.as_os_str()),
        @r###"
    exit_code: 1 (failure)
    ----- stderr -----
    An executable named `does_not_exist` is not provided by package `foo`.
    The following executables are available:
    - custom_pydoc.bat
    - custom_pydoc.cmd
    - custom_pydoc.exe
    - custom_pydoc.ps1
    "###
    ); // Test with explicit .bat extension
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--from")
            .arg("foo")
            .arg("custom_pydoc.bat"),
        @r###"
    exit_code: 0 (success)
    ----- stdout -----
    pydoc - the Python documentation tool

    pydoc <name> ...
        Show text documentation on something.  <name> may be the name of a
        Python keyword, topic, function, module, or package, or a dotted
        reference to a class or function within a module or module in a
        package.  If <name> contains a '\', it is used as the path to a
        Python source file to document. If name is 'keywords', 'topics',
        or 'modules', a listing of these things is displayed.

    pydoc -k <keyword>
        Search for a keyword in the synopsis lines of all available modules.

    pydoc -n <hostname>
        Start an HTTP server with the given hostname (default: localhost).

    pydoc -p <port>
        Start an HTTP server on the given port on the local machine.  Port
        number 0 can be used to get an arbitrary unused port.

    pydoc -b
        Start an HTTP server on an arbitrary unused port and open a web browser
        to interactively browse documentation.  This option can be used in
        combination with -n and/or -p.

    pydoc -w <name> ...
        Write out the HTML documentation for a module to a file in the current
        directory.  If <name> contains a '\', it is treated as a filename; if
        it names a directory, documentation is written for all the contents.

    "###
    ); // Test with explicit .cmd extension
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--from")
            .arg("foo")
            .arg("custom_pydoc.cmd"),
        @r###"
    exit_code: 0 (success)
    ----- stdout -----
    pydoc - the Python documentation tool

    pydoc <name> ...
        Show text documentation on something.  <name> may be the name of a
        Python keyword, topic, function, module, or package, or a dotted
        reference to a class or function within a module or module in a
        package.  If <name> contains a '\', it is used as the path to a
        Python source file to document. If name is 'keywords', 'topics',
        or 'modules', a listing of these things is displayed.

    pydoc -k <keyword>
        Search for a keyword in the synopsis lines of all available modules.

    pydoc -n <hostname>
        Start an HTTP server with the given hostname (default: localhost).

    pydoc -p <port>
        Start an HTTP server on the given port on the local machine.  Port
        number 0 can be used to get an arbitrary unused port.

    pydoc -b
        Start an HTTP server on an arbitrary unused port and open a web browser
        to interactively browse documentation.  This option can be used in
        combination with -n and/or -p.

    pydoc -w <name> ...
        Write out the HTML documentation for a module to a file in the current
        directory.  If <name> contains a '\', it is treated as a filename; if
        it names a directory, documentation is written for all the contents.

    "###
    ); // Test with explicit .ps1 extension
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--from")
            .arg("foo")
            .arg("custom_pydoc.ps1"),
        @r###"
    exit_code: 0 (success)
    ----- stdout -----
    pydoc - the Python documentation tool

    pydoc <name> ...
        Show text documentation on something.  <name> may be the name of a
        Python keyword, topic, function, module, or package, or a dotted
        reference to a class or function within a module or module in a
        package.  If <name> contains a '\', it is used as the path to a
        Python source file to document. If name is 'keywords', 'topics',
        or 'modules', a listing of these things is displayed.

    pydoc -k <keyword>
        Search for a keyword in the synopsis lines of all available modules.

    pydoc -n <hostname>
        Start an HTTP server with the given hostname (default: localhost).

    pydoc -p <port>
        Start an HTTP server on the given port on the local machine.  Port
        number 0 can be used to get an arbitrary unused port.

    pydoc -b
        Start an HTTP server on an arbitrary unused port and open a web browser
        to interactively browse documentation.  This option can be used in
        combination with -n and/or -p.

    pydoc -w <name> ...
        Write out the HTML documentation for a module to a file in the current
        directory.  If <name> contains a '\', it is treated as a filename; if
        it names a directory, documentation is written for all the contents.

    "###
    ); // Test with explicit .exe extension
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--from")
            .arg("foo")
            .arg("custom_pydoc")
            .env(EnvVars::PATH, bin_dir.as_os_str()),
        @r###"
    exit_code: 0 (success)
    ----- stdout -----
    pydoc - the Python documentation tool

    pydoc <name> ...
        Show text documentation on something.  <name> may be the name of a
        Python keyword, topic, function, module, or package, or a dotted
        reference to a class or function within a module or module in a
        package.  If <name> contains a '\', it is used as the path to a
        Python source file to document. If name is 'keywords', 'topics',
        or 'modules', a listing of these things is displayed.

    pydoc -k <keyword>
        Search for a keyword in the synopsis lines of all available modules.

    pydoc -n <hostname>
        Start an HTTP server with the given hostname (default: localhost).

    pydoc -p <port>
        Start an HTTP server on the given port on the local machine.  Port
        number 0 can be used to get an arbitrary unused port.

    pydoc -b
        Start an HTTP server on an arbitrary unused port and open a web browser
        to interactively browse documentation.  This option can be used in
        combination with -n and/or -p.

    pydoc -w <name> ...
        Write out the HTML documentation for a module to a file in the current
        directory.  If <name> contains a '\', it is treated as a filename; if
        it names a directory, documentation is written for all the contents.

    "###
    ); // Test without explicit extension (.exe should be used)
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--from")
            .arg("foo")
            .arg("custom_pydoc")
            .env(EnvVars::PATH, bin_dir.as_os_str()),
        @r###"
    exit_code: 0 (success)
    ----- stdout -----
    pydoc - the Python documentation tool

    pydoc <name> ...
        Show text documentation on something.  <name> may be the name of a
        Python keyword, topic, function, module, or package, or a dotted
        reference to a class or function within a module or module in a
        package.  If <name> contains a '\', it is used as the path to a
        Python source file to document. If name is 'keywords', 'topics',
        or 'modules', a listing of these things is displayed.

    pydoc -k <keyword>
        Search for a keyword in the synopsis lines of all available modules.

    pydoc -n <hostname>
        Start an HTTP server with the given hostname (default: localhost).

    pydoc -p <port>
        Start an HTTP server on the given port on the local machine.  Port
        number 0 can be used to get an arbitrary unused port.

    pydoc -b
        Start an HTTP server on an arbitrary unused port and open a web browser
        to interactively browse documentation.  This option can be used in
        combination with -n and/or -p.

    pydoc -w <name> ...
        Write out the HTML documentation for a module to a file in the current
        directory.  If <name> contains a '\', it is treated as a filename; if
        it names a directory, documentation is written for all the contents.

    "###
    );
    Ok(())
}

#[test]
fn tool_run_reresolve_python() -> anyhow::Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context_with_versions!(&["3.11", "3.12"])
        .with_default_index(&_server.index_url());
    let context = context.with_filtered_counts().with_tool_dirs();
    let foo_dir = context.temp_dir.child("foo");
    let foo_pyproject_toml = foo_dir.child("pyproject.toml");
    foo_pyproject_toml.write_str(indoc! { r#"
        [project]
        name = "foo"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = []

        [project.scripts]
        foo = "foo:run"
        "# })?;
    let foo_project_src = foo_dir.child("src");
    let foo_module = foo_project_src.child("foo");
    let foo_init = foo_module.child("__init__.py");
    foo_init.write_str(indoc! { r#"
        import sys

        def run():
            print(".".join(str(key) for key in sys.version_info[:2]))
       "# })?; // Although 3.11 is first on the path, we'll re-resolve with 3.12 because the `requires-python`
    // is not compatible with 3.11.
    uv_snapshot!(
        context.filters(),
        context.tool_run().arg("--from").arg("./foo").arg("foo"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    3.12

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + foo==1.0.0 (from file://[TEMP_DIR]/foo)
    "
    ); // When an incompatible Python version is explicitly requested, we should not re-resolve
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--from")
            .arg("./foo")
            .arg("--python")
            .arg("3.11")
            .arg("foo"),
        @"
    exit_code: 1 (failure)
    ----- stderr -----
    error: No solution found when resolving tool dependencies
      cause: Because the current Python version (3.11.[X]) does not satisfy Python>=3.12 and foo==1.0.0 depends on Python>=3.12, we can conclude that foo==1.0.0 cannot be used.
             And because only foo==1.0.0 is available and you require foo, we can conclude that your requirements are unsatisfiable.
    "
    ); // Unless the discovered interpreter is compatible with the request
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--from")
            .arg("./foo")
            .arg("--python")
            .arg(">=3.11")
            .arg("foo"),
        @"
    exit_code: 0 (success)
    ----- stdout -----
    3.12

    ----- stderr -----
    Resolved [N] packages in [TIME]
    "
    );
    Ok(())
}

/// Test that Windows executable resolution works correctly for package names with dots.
/// This test verifies the fix for the bug where package names containing dots were
/// incorrectly handled when adding Windows executable extensions.
#[cfg(windows)]
#[test]
fn tool_run_windows_dotted_package_name() -> anyhow::Result<()> {
    let _server = uv_test::packse::PackseServer::new("packages/tool-run.toml");
    let context = uv_test::test_context!("3.12").with_default_index(&_server.index_url());
    let context = context.with_filtered_counts().with_tool_dirs(); // Copy the test package to a temporary location
    let workspace_packages = context.workspace_root.join("test").join("packages");
    let test_package_source = workspace_packages.join("package.name.with.dots");
    let test_package_dest = context.temp_dir.child("package.name.with.dots");
    copy_dir_all(&test_package_source, &test_package_dest)?; // Test that uv tool run can find and execute the dotted package name
    uv_snapshot!(
        context.filters(),
        context
            .tool_run()
            .arg("--from")
            .arg(test_package_dest.path())
            .arg("package.name.with.dots"),
        @r###"
    exit_code: 0 (success)
    ----- stdout -----
    package.name.with.dots version 0.1.0

    ----- stderr -----
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + package-name-with-dots==0.1.0 (from file://[TEMP_DIR]/package.name.with.dots)
    "###
    );
    Ok(())
}

/// Regression test for <https://github.com/astral-sh/uv/issues/17436>
#[tokio::test]
async fn tool_run_latest_keyring_auth() {
    let keyring_context = uv_test::test_context!("3.12");

    // Install our keyring plugin
    keyring_context
        .pip_install()
        .arg(
            keyring_context
                .workspace_root
                .join("test/packages/keyring_stub"),
        )
        .arg(
            keyring_context
                .workspace_root
                .join("test")
                .join("packages")
                .join("keyring_test_plugin"),
        )
        .assert()
        .success();

    let proxy = crate::pypi_proxy::start().await;

    let context = uv_test::test_context!("3.12")
        .with_exclude_newer("2025-01-18T00:00:00Z")
        .with_filtered_counts()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");

    // Combine keyring venv bin with tool bin directory to avoid PATH warnings.
    let path = std::env::join_paths([venv_bin_path(&keyring_context.venv), bin_dir.to_path_buf()])
        .unwrap();

    // Test that the keyring is consulted during the @latest version lookup.
    uv_snapshot!(context.filters(), context.tool_install()
        .arg("--index")
        .arg(proxy.username_url("public", "/basic-auth/simple"))
        .arg("--keyring-provider")
        .arg("subprocess")
        .arg("executable-application@latest")
        .env(EnvVars::KEYRING_TEST_CREDENTIALS, format!(r#"{{"{host}": {{"public": "heron"}}}}"#, host = proxy.host_port()))
        .env(EnvVars::PATH, path), @"
    exit_code: 0 (success)
    ----- stderr -----
    Keyring request for public@http://[LOCALHOST]/basic-auth/simple
    Keyring request for public@[LOCALHOST]
    Resolved [N] packages in [TIME]
    Prepared [N] packages in [TIME]
    Installed [N] packages in [TIME]
     + executable-application==0.3.0
    Installed 1 executable: app
    ");
}
