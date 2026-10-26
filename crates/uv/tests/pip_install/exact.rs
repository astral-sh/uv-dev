use std::process::Command;

use anyhow::Result;
use assert_cmd::prelude::*;
use indoc::indoc;
use uv_test::packse::PackseServer;
use uv_test::packse::scenario::Scenario;
use uv_test::{TestContext, uv_snapshot};

fn install(context: &TestContext, index: &PackseServer) -> Command {
    let mut command = context.pip_install();
    command.arg("--index-url").arg(index.index_url());
    command
}

fn assert_inventory(context: &TestContext, expected: &str) {
    context
        .assert_command(indoc! {r#"
            import importlib.metadata as metadata
            installed = sorted(f"{dist.metadata['Name']}=={dist.version}" for dist in metadata.distributions())
            print(", ".join(installed), end="")
        "#})
        .success()
        .stdout(expected.to_owned());
}

/// Exact installation removes unused packages without removing shared dependencies.
#[test]
fn removes_extraneous_packages() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let scenario: Scenario = toml::from_str(indoc! {r#"
        name = "exact-install"
        [root]
        [expected]
        satisfiable = true

        [packages.kept.versions."1.0.0"]
        requires = ["shared==1.0.0"]
        [packages.extraneous.versions."1.0.0"]
        requires = ["shared==1.0.0", "orphan==1.0.0"]
        [packages.shared.versions."1.0.0"]
        [packages.orphan.versions."1.0.0"]
        [packages.added.versions."1.0.0"]
    "#})?;
    let index = PackseServer::from_scenario(&scenario);
    let requirements = context.temp_dir.join("requirements.txt");
    fs_err::write(&requirements, "kept==1.0.0\n")?;
    install(&context, &index)
        .args(["--exact", "-r", "requirements.txt"])
        .assert()
        .success();
    install(&context, &index)
        .arg("extraneous")
        .assert()
        .success();

    uv_snapshot!(install(&context, &index).args(["--exact", "-r", "requirements.txt"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Uninstalled 2 packages in [TIME]
     - extraneous==1.0.0
     - orphan==1.0.0
    ");
    assert_inventory(&context, "kept==1.0.0, shared==1.0.0");

    install(&context, &index)
        .arg("extraneous")
        .assert()
        .success();
    fs_err::write(&requirements, "kept==1.0.0\nadded==1.0.0\n")?;
    uv_snapshot!(install(&context, &index).args(["--exact", "-r", "requirements.txt"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 3 packages in [TIME]
    Prepared 1 package in [TIME]
    Uninstalled 2 packages in [TIME]
    Installed 1 package in [TIME]
     + added==1.0.0
     - extraneous==1.0.0
     - orphan==1.0.0
    ");
    assert_inventory(&context, "added==1.0.0, kept==1.0.0, shared==1.0.0");
    Ok(())
}
