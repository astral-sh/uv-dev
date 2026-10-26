use std::fmt::Write as _;
use std::process::Command;

use anyhow::Result;
use assert_cmd::prelude::*;
use assert_fs::assert::PathAssert;
use assert_fs::fixture::ChildPath;
use uv_test::packse::PackseServer;
use uv_test::packse::scenario::{Package, PackageMetadata, Scenario};
use uv_test::{TestContext, uv_snapshot};

const PACKAGES: [(&str, &str); 2] = [("markupsafe", "2.1.3"), ("tomli", "2.0.1")];

fn setup() -> Result<(TestContext, PackseServer)> {
    let context = uv_test::test_context!("3.12");
    let mut scenario = Scenario::empty();
    let mut requirements = String::new();
    for (name, version) in PACKAGES {
        scenario.packages.insert(
            name.parse()?,
            Package {
                versions: [(version.parse()?, PackageMetadata::wheel_only())].into(),
            },
        );
        writeln!(requirements, "{name}=={version}")?;
    }
    fs_err::write(context.temp_dir.join("requirements.txt"), requirements)?;
    let index = PackseServer::from_scenario(&scenario);
    sync(&context, &index).assert().success();

    for (name, _) in PACKAGES {
        let module = context.site_packages().join(name).join("__init__.py");
        // Unlink first so editing an installed hardlink cannot modify the cached wheel.
        fs_err::remove_file(&module)?;
        fs_err::write(module, "__version__ = 'locally-modified'\n")?;
    }
    Ok((context, index))
}

fn sync(context: &TestContext, index: &PackseServer) -> Command {
    let mut command = context.pip_sync();
    command
        .args(["requirements.txt", "--strict", "--index-url"])
        .arg(index.index_url());
    command
}

#[test]
fn all_packages() -> Result<()> {
    let (context, index) = setup()?;
    uv_snapshot!(sync(&context, &index).arg("--reinstall"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 2 packages in [TIME]
    Uninstalled 2 packages in [TIME]
    Installed 2 packages in [TIME]
     ~ markupsafe==2.1.3
     ~ tomli==2.0.1
    ");
    context.assert_installed("markupsafe", "2.1.3");
    context.assert_installed("tomli", "2.0.1");
    Ok(())
}

#[test]
fn selected_package() -> Result<()> {
    let (context, index) = setup()?;
    // The version check must inspect the installed package even when the project shadows it.
    fs_err::write(
        context.temp_dir.join("tomli.py"),
        "__version__ = 'project-local'\n",
    )?;
    uv_snapshot!(sync(&context, &index).args(["--reinstall-package", "tomli"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     ~ tomli==2.0.1
    ");
    context.assert_installed("markupsafe", "locally-modified");
    context.assert_installed("tomli", "2.0.1");
    Ok(())
}

/// Changing a registry requirement to a direct URL must reinstall even at the same version.
#[test]
fn install_version_then_install_url() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let mut scenario = Scenario::empty();
    scenario.packages.insert(
        "werkzeug".parse()?,
        Package {
            versions: [("2.0.0".parse()?, PackageMetadata::wheel_only())].into(),
        },
    );
    let index = PackseServer::from_scenario(&scenario);
    let requirements = context.temp_dir.join("requirements.txt");
    fs_err::write(&requirements, "werkzeug==2.0.0")?;
    sync(&context, &index).assert().success();

    let provenance = ChildPath::new(
        context
            .site_packages()
            .join("werkzeug-2.0.0.dist-info/direct_url.json"),
    );
    provenance.assert(predicates::path::missing());
    let wheel_url = index.file_url("werkzeug-2.0.0-py3-none-any.whl");
    fs_err::write(&requirements, format!("werkzeug @ {wheel_url}"))?;

    uv_snapshot!(context.filters(), sync(&context, &index), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 1 package in [TIME]
    Installed 1 package in [TIME]
     - werkzeug==2.0.0
     + werkzeug==2.0.0 (from http://[LOCALHOST]/files/werkzeug-2.0.0-py3-none-any.whl)
    ");
    let direct_url: serde_json::Value =
        serde_json::from_reader(fs_err::File::open(provenance.path())?)?;
    assert_eq!(
        direct_url,
        serde_json::json!({"url": wheel_url, "archive_info": {}})
    );
    context.assert_installed("werkzeug", "2.0.0");
    Ok(())
}
