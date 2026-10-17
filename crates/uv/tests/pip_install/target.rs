use assert_cmd::prelude::*;

/// Install into the current directory via `--target`. See astral-sh/uv#21694.
#[test]
fn install_target_current_directory() {
    let context = uv_test::test_context!("3.12");
    let wheel = context
        .workspace_root
        .join("test/links/tqdm-1000.0.0-py3-none-any.whl");
    context
        .pip_install()
        .arg(wheel)
        .arg("--target")
        .arg(".")
        .arg("--no-index")
        .assert()
        .success();

    context
        .assert_command(indoc::indoc! {r#"
            import sysconfig
            from pathlib import Path
            import tqdm

            expected = Path.cwd() / "tqdm" / "__init__.py"
            actual = Path(tqdm.__file__)
            assert actual.samefile(expected), f"imported {actual}, expected {expected}"
            venv_package = Path(sysconfig.get_path("purelib")) / "tqdm"
            assert not venv_package.exists(), f"installed into virtual environment: {venv_package}"
        "#})
        .success();
}
