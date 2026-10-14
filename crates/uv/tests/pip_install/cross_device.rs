//! Installation with the cache and environment on different filesystems.
//! Requires `UV_INTERNAL__TEST_ALT_FS`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;
use assert_cmd::prelude::*;
use indoc::indoc;

use uv_test::packse::generate_wheel;
use uv_test::{TestContext, uv_snapshot};

fn fixture() -> Result<Option<(TestContext, PathBuf)>> {
    let Some(context) = uv_test::test_context!("3.12").with_cache_on_alt_fs()? else {
        return Ok(None);
    };
    context
        .python_command()
        .arg("-c")
        .arg(indoc! {r#"
            import os
            import sys

            cache_device = os.stat(sys.argv[1]).st_dev
            venv_device = os.stat(sys.argv[2]).st_dev
            assert cache_device != venv_device, f"cache and virtual environment share device {cache_device}"
        "#})
        .arg(context.cache_dir.path())
        .arg(context.venv.path())
        .assert()
        .success();
    let (filename, bytes) = generate_wheel(
        &"cross-device".parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &[],
    );
    let wheel = context.temp_dir.join(filename);
    fs_err::write(&wheel, bytes)?;
    Ok(Some((context, wheel)))
}

#[test]
fn default_fallback() -> Result<()> {
    let Some((context, wheel)) = fixture()? else {
        return Ok(());
    };

    uv_snapshot!(context.filters(), context.pip_install().arg(wheel), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    warning: Failed to hardlink files; falling back to full copy. This may lead to degraded performance.
             If the cache and target directories are on different filesystems, hardlinking may not be supported.
             If this is intentional, set `export UV_LINK_MODE=copy` or use `--link-mode=copy` to suppress this warning.
    Installed 1 package in [TIME]
     + cross-device==1.0.0 (from file://[TEMP_DIR]/cross_device-1.0.0-py3-none-any.whl)
    ");
    context.assert_installed("cross_device", "1.0.0");
    Ok(())
}

#[test]
fn explicit_copy() -> Result<()> {
    let Some((context, wheel)) = fixture()? else {
        return Ok(());
    };

    uv_snapshot!(context.filters(), context.pip_install().args(["--link-mode", "copy"]).arg(wheel), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + cross-device==1.0.0 (from file://[TEMP_DIR]/cross_device-1.0.0-py3-none-any.whl)
    ");
    context.assert_installed("cross_device", "1.0.0");
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlink() -> Result<()> {
    let Some((context, wheel)) = fixture()? else {
        return Ok(());
    };

    uv_snapshot!(context.filters(), context.pip_install().args(["--link-mode", "symlink"]).arg(wheel), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + cross-device==1.0.0 (from file://[TEMP_DIR]/cross_device-1.0.0-py3-none-any.whl)
    ");
    let module = context.site_packages().join("cross_device/__init__.py");
    assert!(
        module.is_symlink(),
        "expected symlink at {}",
        module.display()
    );
    let target = module.canonicalize()?;
    let cache = context.cache_dir.canonicalize()?;
    assert!(
        target.starts_with(&cache),
        "expected {} within {}",
        target.display(),
        cache.display()
    );
    context.assert_installed("cross_device", "1.0.0");
    Ok(())
}
