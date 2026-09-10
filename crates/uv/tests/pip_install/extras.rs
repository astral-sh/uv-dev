use std::process::Command;

use anyhow::Result;
use assert_fs::prelude::*;
use insta::allow_duplicates;

use uv_test::{TestContext, uv_snapshot};

fn pip_install(context: &TestContext) -> Command {
    let mut command = context.pip_install();
    command.env_clear();
    context.add_shared_env(&mut command, false);
    command.args(["--no-config", "--offline", "--no-index", "--no-build"]);
    command
}

#[test]
fn single_package_extras_hint() {
    let context = uv_test::test_context_with_versions!(&[]);

    uv_snapshot!(pip_install(&context).args([
        "--extra", "dev", "uv-opportunity-example",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Requesting extras requires a `pylock.toml`, `pyproject.toml`, `setup.cfg`, or `setup.py` file

    hint: Use `uv-opportunity-example[dev]` syntax instead
    ");

    uv_snapshot!(pip_install(&context).args([
        "--extra", "docs,DEV", "--extra", "dev", "UV_Opportunity.Example",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Requesting extras requires a `pylock.toml`, `pyproject.toml`, `setup.cfg`, or `setup.py` file

    hint: Use `uv-opportunity-example[dev,docs]` syntax instead
    ");

    // A marker that simplifies to true has no condition to preserve.
    uv_snapshot!(pip_install(&context).args([
        "--extra", "dev",
        "uv-opportunity-example; python_version < '3' or python_version >= '3'",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Requesting extras requires a `pylock.toml`, `pyproject.toml`, `setup.cfg`, or `setup.py` file

    hint: Use `uv-opportunity-example[dev]` syntax instead
    ");

    assert!(!context.venv.exists());
}

#[test]
fn generic_extras_hint() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    context.temp_dir.child("project").create_dir_all()?;
    context.temp_dir.child("requirements.txt").write_str("")?;

    uv_snapshot!(pip_install(&context).args([
        "--all-extras", "uv-opportunity-example",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Requesting extras requires a `pylock.toml`, `pyproject.toml`, `setup.cfg`, or `setup.py` file

    hint: Use `package[extra]` syntax instead
    ");

    allow_duplicates! {
        for args in [
            vec!["--extra", "dev", "uv-opportunity-example==1"],
            vec![
                "--extra",
                "dev",
                "uv-opportunity-example; python_version >= '3.10'",
            ],
            vec!["--extra", "dev", "uv-opportunity-example[docs]"],
            vec![
                "--extra",
                "dev",
                "uv-opportunity-example @ https://example.invalid/package.whl",
            ],
            vec!["--extra", "dev", "https://example.invalid/package.whl"],
            vec!["--extra", "dev", "./project"],
            vec!["--extra", "dev", "uv-opportunity-example", "other-package"],
            vec!["--extra", "dev", "-r", "requirements.txt"],
        ] {
            uv_snapshot!(pip_install(&context).args(args), @"
            exit_code: 2 (failure)
            ----- stderr -----
            error: Requesting extras requires a `pylock.toml`, `pyproject.toml`, `setup.cfg`, or `setup.py` file

            hint: Use `package[dev]` syntax instead
            ");
        }
    }

    uv_snapshot!(pip_install(&context).args([
        "--extra", "dev", "--editable", "./project",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Requesting extras requires a `pylock.toml`, `pyproject.toml`, `setup.cfg`, or `setup.py` file

    hint: Use `<dir>[dev]` syntax or `-r <file>` instead
    ");

    assert!(!context.venv.exists());
    Ok(())
}

#[test]
fn configured_extras_hint() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let config = context.temp_dir.child("uv.toml");
    config.write_str("[pip]\nextra = ['dev', 'docs']\nno-extra = ['dev']\n")?;

    uv_snapshot!(pip_install(&context).args([
        "--config-file", "uv.toml", "uv-opportunity-example",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Requesting extras requires a `pylock.toml`, `pyproject.toml`, `setup.cfg`, or `setup.py` file

    hint: Use `uv-opportunity-example[docs]` syntax instead
    ");

    allow_duplicates! {
        for contents in [
            "[pip]\nextra = ['dev']\nno-extra = ['dev']\n",
            "[pip]\nextra = ['dev']\nall-extras = true\nno-extra = ['docs']\n",
        ] {
            config.write_str(contents)?;
            uv_snapshot!(pip_install(&context).args([
                "--config-file", "uv.toml", "uv-opportunity-example",
            ]), @"
            exit_code: 2 (failure)
            ----- stderr -----
            error: Requesting extras requires a `pylock.toml`, `pyproject.toml`, `setup.cfg`, or `setup.py` file

            hint: Use `package[extra]` syntax instead
            ");
        }
        Ok::<(), anyhow::Error>(())
    }?;

    assert!(!context.venv.exists());
    Ok(())
}
