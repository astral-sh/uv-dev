use std::collections::BTreeMap;

use anyhow::Result;
use assert_fs::prelude::*;
use serde_json::json;

use uv_test::package_server::PackageServer;
use uv_test::packse::generate_wheel;
use uv_test::uv_snapshot;

async fn yanked_colorama() -> Result<PackageServer> {
    let name = "colorama".parse()?;
    let (filename, wheel) = generate_wheel(
        &name,
        &"0.4.2".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let index = PackageServer::new(&name).await;
    index
        .serve_with(
            &filename,
            &wheel,
            None,
            json!({"yanked": "Bad build, missing files, will not install"}),
        )
        .await;
    Ok(index)
}

#[tokio::test]
async fn warn_on_yanked() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let index = yanked_colorama().await?;
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("colorama==0.4.2")?;

    uv_snapshot!(context.filters(), windows_filters=false, context.pip_sync()
        .args(["requirements.txt", "--strict", "--default-index"])
        .arg(index.index_url()), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + colorama==0.4.2
    warning: `colorama==0.4.2` is yanked (reason: "Bad build, missing files, will not install")
    "#);
    context.assert_installed("colorama", "0.4.2");
    Ok(())
}

#[tokio::test]
async fn warn_on_yanked_dry_run() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let index = yanked_colorama().await?;
    context
        .temp_dir
        .child("requirements.txt")
        .write_str("colorama==0.4.2")?;

    uv_snapshot!(context.filters(), windows_filters=false, context.pip_sync()
        .args(["requirements.txt", "--dry-run", "--strict", "--default-index"])
        .arg(index.index_url()), @r#"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Would download 1 package
    Would install 1 package
     + colorama==0.4.2
    warning: `colorama==0.4.2` is yanked (reason: "Bad build, missing files, will not install")
    "#);
    context.assert_not_installed("colorama");
    Ok(())
}
