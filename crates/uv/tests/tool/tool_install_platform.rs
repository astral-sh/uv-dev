use anyhow::Result;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use indoc::formatdoc;
use predicates::prelude::*;

use uv_static::EnvVars;
use uv_test::packse::{PackseServer, scenario::Scenario};
use uv_test::{site_packages_path, uv_snapshot};

/// Changing the requested platform must replace the installed wheel even at the same version.
#[test]
fn tool_install_python_platform() -> Result<()> {
    const MACOS_TAG: &str = "py3-none-macosx_11_0_arm64";
    const LINUX_TAG: &str = "py3-none-manylinux_2_17_x86_64";

    let context = uv_test::test_context!("3.12")
        .with_filtered_exe_suffix()
        .with_tool_dirs();
    let bin_dir = context.temp_dir.child("bin");
    let scenario: Scenario = toml::from_str(&formatdoc! {r#"
        name = "platform-tool"
        [root]
        [expected]
        satisfiable = true
        [packages.platform-tool.versions."1.0.0"]
        sdist = false
        entry_points = ["platform-tool"]
        wheel_tags = ["{MACOS_TAG}", "{LINUX_TAG}"]
    "#})?;
    let index = PackseServer::from_scenario(&scenario);
    let wheel_metadata = ChildPath::new(site_packages_path(
        context.temp_dir.child("tools/platform-tool").path(),
        "python3.12",
    ))
    .child("platform_tool-1.0.0.dist-info/WHEEL");

    uv_snapshot!(context.filters(), context.tool_install()
        .args(["platform-tool", "--python-platform", "macos"])
        .arg("--index-url")
        .arg(index.index_url())
        .env(EnvVars::PATH, bin_dir.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + platform-tool==1.0.0
    Installed 1 executable: platform-tool
    ");
    wheel_metadata.assert(predicate::str::contains(format!("Tag: {MACOS_TAG}\n")));

    uv_snapshot!(context.filters(), context.tool_install()
        .args(["platform-tool", "--python-platform", "linux"])
        .arg("--index-url")
        .arg(index.index_url())
        .env(EnvVars::PATH, bin_dir.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ platform-tool==1.0.0
    Installed 1 executable: platform-tool
    ");
    wheel_metadata.assert(predicate::str::contains(format!("Tag: {LINUX_TAG}\n")));

    Ok(())
}
