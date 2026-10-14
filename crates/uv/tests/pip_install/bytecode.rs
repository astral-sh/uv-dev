use std::path::{Path, PathBuf};

use anyhow::Result;
use assert_cmd::prelude::*;
use uv_static::EnvVars;
use uv_test::packse::generate_wheel;
use uv_test::{TestContext, site_packages_path};

use super::wheel_fixture::write_many_files_wheel;

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

/// Verify that the compiled source is in the requested destination.
fn assert_compiled_source(context: &TestContext, site_packages: &Path) -> PathBuf {
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
    context
        .pip_install()
        .arg(wheel)
        .args(["--offline", "--compile-bytecode"])
        .assert()
        .success()
        .stderr(predicates::str::contains("Bytecode compiled 1 file in "));
    assert_compiled_source(&context, &context.site_packages());
    Ok(())
}

/// Compile symlinked source files installed by the current operation.
#[test]
#[cfg(unix)]
fn compile_bytecode_with_symlink_link_mode() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = bytecode_wheel(&context)?;
    context
        .pip_install()
        .arg(wheel)
        .args(["--offline", "--compile-bytecode", "--link-mode", "symlink"])
        .assert()
        .success()
        .stderr(predicates::str::contains("Bytecode compiled 1 file in "));
    let source = assert_compiled_source(&context, &context.site_packages());
    assert!(source.is_symlink(), "{} is not a symlink", source.display());
    Ok(())
}

/// Compile bytecode when installing into a relative `--target` path.
#[test]
fn compile_bytecode_for_relative_target() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = bytecode_wheel(&context)?;
    context
        .pip_install()
        .arg(wheel)
        .args(["--offline", "--compile-bytecode", "--target", "target"])
        .assert()
        .success()
        .stderr(predicates::str::contains("Bytecode compiled 1 file in "));
    assert_compiled_source(&context, &context.temp_dir.join("target"));
    Ok(())
}

/// Compile bytecode when installing into a relative `--prefix` path.
#[test]
fn compile_bytecode_for_relative_prefix() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheel = bytecode_wheel(&context)?;
    context
        .pip_install()
        .arg(wheel)
        .args(["--offline", "--compile-bytecode", "--prefix", "prefix"])
        .assert()
        .success()
        .stderr(predicates::str::contains("Bytecode compiled 1 file in "));
    assert_compiled_source(
        &context,
        &site_packages_path(&context.temp_dir.join("prefix"), "python3.12"),
    );
    Ok(())
}

/// Compile every installed source when a single-worker queue exceeds its capacity.
#[test]
fn compile_bytecode_queue_backpressure() -> Result<()> {
    const SOURCE_FILES: usize = 16;

    let context = uv_test::test_context!("3.12");
    let wheel = context.temp_dir.join("large_wheel-1.0.0-py3-none-any.whl");
    write_many_files_wheel(&wheel, SOURCE_FILES)?;
    context
        .pip_install()
        .arg(wheel)
        .arg("--offline")
        .arg("--compile-bytecode")
        .env(EnvVars::UV_CONCURRENT_INSTALLS, "1")
        .assert()
        .success()
        .stderr(predicates::str::contains(format!(
            "Bytecode compiled {SOURCE_FILES} files in "
        )));

    context
        .python_command()
        .arg("-c")
        .arg(indoc::indoc! {r#"
            import importlib.util
            import sys
            from pathlib import Path

            root = Path(sys.argv[1])
            for index in range(int(sys.argv[2])):
                source = root / f"module_{index:05}.py"
                bytecode = Path(importlib.util.cache_from_source(str(source)))
                assert source.is_file(), f"missing installed source: {source}"
                assert bytecode.is_file(), f"missing compiled bytecode: {bytecode}"
        "#})
        .arg(context.site_packages().join("large_wheel"))
        .arg(SOURCE_FILES.to_string())
        .assert()
        .success();
    Ok(())
}
