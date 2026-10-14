use std::path::{Path, PathBuf};

use anyhow::Result;
use assert_cmd::prelude::*;
use uv_test::packse::generate_wheel;
use uv_test::{TestContext, site_packages_path};

/// Create a wheel containing exactly one Python source file.
fn bytecode_wheel(context: &TestContext) -> Result<PathBuf> {
    let (filename, bytes) = generate_wheel(
        &"bytecode-test".parse()?,
        &"1.0.0".parse()?,
        &[],
        &Default::default(),
        None,
        "py3-none-any",
        &[],
    );
    let wheel = context.temp_dir.join(filename);
    fs_err::write(&wheel, bytes)?;
    Ok(wheel)
}

/// Install the wheel and verify that the only compiled source is in the requested destination.
fn install_compiled_wheel(
    context: &TestContext,
    wheel: &Path,
    site_packages: &Path,
    args: &[&str],
) -> PathBuf {
    context
        .pip_install()
        .arg(wheel)
        .arg("--offline")
        .arg("--compile-bytecode")
        .args(args)
        .assert()
        .success()
        .stderr(predicates::str::contains("Bytecode compiled 1 file in "));

    let source = site_packages.join("bytecode_test/__init__.py");
    context
        .python_command()
        .arg("-c")
        .arg(indoc::indoc! {r#"
            import importlib.util
            import sys
            from pathlib import Path

            bytecode = Path(importlib.util.cache_from_source(sys.argv[1]))
            assert bytecode.is_file(), f"missing compiled bytecode: {bytecode}"
        "#})
        .arg(&source)
        .assert()
        .success();
    source
}

/// Compile the installed source without compiling unrelated standard-library files.
#[test]
fn compile_bytecode_excludes_stdlib() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = bytecode_wheel(&context)?;
    install_compiled_wheel(&context, &wheel, &context.site_packages(), &[]);
    Ok(())
}

/// Compile symlinked source files installed by the current operation.
#[test]
#[cfg(unix)]
fn compile_bytecode_with_symlink_link_mode() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = bytecode_wheel(&context)?;
    let source = install_compiled_wheel(
        &context,
        &wheel,
        &context.site_packages(),
        &["--link-mode", "symlink"],
    );
    assert!(source.is_symlink(), "{} is not a symlink", source.display());
    Ok(())
}

/// Compile bytecode when installing into a relative `--target` or `--prefix` path.
#[test]
fn compile_bytecode_for_relative_install_root() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = bytecode_wheel(&context)?;
    install_compiled_wheel(
        &context,
        &wheel,
        &context.temp_dir.join("target"),
        &["--target", "target"],
    );
    install_compiled_wheel(
        &context,
        &wheel,
        &site_packages_path(&context.temp_dir.join("prefix"), "python3.12"),
        &["--prefix", "prefix"],
    );
    Ok(())
}
