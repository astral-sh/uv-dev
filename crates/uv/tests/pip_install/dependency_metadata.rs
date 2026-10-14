use anyhow::Result;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_test::packse::PackseServer;
use uv_test::uv_snapshot;

/// Apply configured extra metadata to an already installed distribution.
#[test]
fn static_metadata_installed_extra() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let server = PackseServer::new("simple/dependency-groups.toml");
    let index_url = server.index_url();
    let install = || {
        let mut command = context.pip_install();
        command.arg("--default-index").arg(&index_url);
        command
    };

    uv_snapshot!(context.filters(), install().arg("iniconfig==2.0.0"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + iniconfig==2.0.0
    ");

    context.temp_dir.child("uv.toml").write_str(indoc! {r#"
        [[dependency-metadata]]
        name = "iniconfig"
        version = "2.0.0"
        requires-dist = ["typing-extensions==4.10.0 ; extra == 'typing'"]
        provides-extras = ["typing"]
    "#})?;

    uv_snapshot!(context.filters(), install()
        .args(["iniconfig[typing]==2.0.0", "--no-deps"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");
    uv_snapshot!(context.filters(), install().arg("iniconfig==2.0.0"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");
    context.assert_not_installed("typing_extensions");

    uv_snapshot!(context.filters(), install().arg("iniconfig[typing]==2.0.0"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 2 packages in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + typing-extensions==4.10.0
    ");
    context.assert_installed("typing_extensions", "4.10.0");

    drop(server);
    uv_snapshot!(context.filters(), install()
        .args(["iniconfig[typing]==2.0.0", "--offline"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Checked 1 package in [TIME]
    ");
    Ok(())
}
