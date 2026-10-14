use std::path::PathBuf;

use anyhow::Result;
use assert_cmd::prelude::*;
use uv_test::TestContext;
use uv_test::packse::generate_wheel;

/// Install a wheel containing exactly one Python source file and check its bytecode.
fn install_compiled_wheel(context: &TestContext, link_mode: Option<&str>) -> Result<PathBuf> {
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

    let mut command = context.pip_install();
    command
        .arg(wheel)
        .arg("--offline")
        .arg("--compile-bytecode");
    if let Some(link_mode) = link_mode {
        command.arg("--link-mode").arg(link_mode);
    }
    command
        .assert()
        .success()
        .stderr(predicates::str::contains("Bytecode compiled 1 file in "));

    let source = context.site_packages().join("bytecode_test/__init__.py");
    context
        .python_command()
        .arg("-B")
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
    Ok(source)
}

/// Compile the installed source without compiling unrelated standard-library files.
#[test]
fn compile_bytecode_excludes_stdlib() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    install_compiled_wheel(&context, None)?;
    Ok(())
}

/// Compile symlinked source files installed by the current operation.
#[test]
#[cfg(unix)]
fn compile_bytecode_with_symlink_link_mode() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let source = install_compiled_wheel(&context, Some("symlink"))?;
    assert!(source.is_symlink(), "{} is not a symlink", source.display());
    Ok(())
}
