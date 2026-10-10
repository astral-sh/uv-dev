use std::collections::BTreeMap;
use std::str::FromStr;

use anyhow::Result;
use assert_fs::prelude::*;

use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_static::EnvVars;
use uv_test::packse::PackseServer;
use uv_test::packse::scenario::{ArtifactMetadata, Package, PackageMetadata, Scenario};
use uv_test::uv_snapshot;

/// Upgrading a previously selected yanked version can select an older, unyanked version.
#[test]
fn upgrade_yanked_preference() -> Result<()> {
    let context = uv_test::test_context!("3.12").with_env(EnvVars::UV_NO_CONFIG, "1");

    let mut scenario = Scenario::empty();
    scenario.packages.insert(
        PackageName::from_str("yanked-target")?,
        Package {
            versions: BTreeMap::from([
                (
                    Version::from_str("1.0.0")?,
                    PackageMetadata {
                        wheel: Some(ArtifactMetadata::default()),
                        ..PackageMetadata::default()
                    },
                ),
                (
                    Version::from_str("2.0.0")?,
                    PackageMetadata {
                        wheel: Some(ArtifactMetadata::default()),
                        yanked: true,
                        ..PackageMetadata::default()
                    },
                ),
            ]),
        },
    );
    let server = PackseServer::from_scenario(&scenario);

    let requirements_in = context.temp_dir.child("requirements.in");
    requirements_in.write_str("yanked-target>=1")?;
    let requirements_txt = context.temp_dir.child("requirements.txt");
    requirements_txt.write_str("yanked-target==2.0.0")?;

    // An existing output pin permits the yanked version without an upgrade.
    uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in")
        .arg("--output-file")
        .arg("requirements.txt")
        .arg("--index-url")
        .arg(server.index_url())
        .arg("--no-build")
        .arg("--no-header")
        .arg("--no-annotate"), @"
    exit_code: 0 (success)
    ----- stdout -----
    yanked-target==2.0.0

    ----- stderr -----
    Resolved 1 package in [TIME]
    warning: `yanked-target==2.0.0` is yanked
    ");
    requirements_txt.assert("yanked-target==2.0.0\n");

    // Upgrading removes that preference, so the unyanked release is selected.
    uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in")
        .arg("--output-file")
        .arg("requirements.txt")
        .arg("--index-url")
        .arg(server.index_url())
        .arg("--no-build")
        .arg("--no-header")
        .arg("--no-annotate")
        .arg("--upgrade"), @"
    exit_code: 0 (success)
    ----- stdout -----
    yanked-target==1.0.0

    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    requirements_txt.assert("yanked-target==1.0.0\n");

    // An explicit input pin still permits a yanked release during an upgrade.
    requirements_in.write_str("yanked-target==2.0.0")?;
    uv_snapshot!(context.filters(), context.pip_compile()
        .arg("requirements.in")
        .arg("--output-file")
        .arg("requirements.txt")
        .arg("--index-url")
        .arg(server.index_url())
        .arg("--no-build")
        .arg("--no-header")
        .arg("--no-annotate")
        .arg("--upgrade"), @"
    exit_code: 0 (success)
    ----- stdout -----
    yanked-target==2.0.0

    ----- stderr -----
    Resolved 1 package in [TIME]
    warning: `yanked-target==2.0.0` is yanked
    ");
    requirements_txt.assert("yanked-target==2.0.0\n");

    Ok(())
}
