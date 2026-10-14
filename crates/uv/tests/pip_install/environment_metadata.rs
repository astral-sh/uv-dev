use anyhow::Result;

use uv_test::uv_snapshot;

/// Installing a package updates the `site-packages` directory modification time.
#[test]
fn site_packages_mtime_updated() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let site_packages = context.site_packages();
    // Move the baseline away from the current clock tick, even on coarse-resolution filesystems.
    filetime::set_file_mtime(
        &site_packages,
        filetime::FileTime::from_unix_time(946_684_800, 0),
    )?;
    let before = site_packages.metadata()?.modified()?;
    let wheel = context
        .workspace_root
        .join("test/links/tqdm-1000.0.0-py3-none-any.whl");

    uv_snapshot!(context.filters(), context.pip_install().arg(wheel).arg("--no-index"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tqdm==1000.0.0 (from file://[WORKSPACE]/test/links/tqdm-1000.0.0-py3-none-any.whl)
    ");

    let after = site_packages.metadata()?.modified()?;
    assert!(
        after > before,
        "site-packages mtime did not advance: {before:?} -> {after:?}"
    );
    Ok(())
}
