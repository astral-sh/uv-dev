use anyhow::Result;
use assert_fs::prelude::*;

use uv_test::uv_snapshot;

/// Applying the inactive requirement would force the older available version.
#[test]
fn select_matching_requirement_marker() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let links = context.temp_dir.child("links");
    links.create_dir_all()?;
    for version in ["1.0.0", "2.0.0"] {
        let filename = format!("ok-{version}-py3-none-any.whl");
        fs_err::copy(
            context.workspace_root.join("test/links").join(&filename),
            links.child(filename),
        )?;
    }
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("ok ; python_version >= '3.12'\nok==1.0.0 ; python_version < '3.12'\n")?;

    uv_snapshot!(context.filters(), context.pip_sync()
        .args(["requirements.txt", "--no-index", "--find-links"])
        .arg(links.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + ok==2.0.0
    ");

    context.assert_installed("ok", "2.0.0");
    Ok(())
}
