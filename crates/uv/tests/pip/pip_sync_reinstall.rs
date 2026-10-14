use std::fmt::Write as _;
use std::process::Command;

use anyhow::Result;
use assert_cmd::prelude::*;
use uv_test::packse::PackseServer;
use uv_test::packse::scenario::{ArtifactMetadata, Package, PackageMetadata, Scenario};
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
                versions: [(
                    version.parse()?,
                    PackageMetadata {
                        wheel: Some(ArtifactMetadata::default()),
                        ..PackageMetadata::default()
                    },
                )]
                .into(),
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
