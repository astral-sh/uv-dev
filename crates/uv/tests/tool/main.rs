//! Integration tests for `uv tool`.

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
use uv_test::pypi_proxy;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod tool_audit;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod tool_dir;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod tool_install;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod tool_list;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod tool_run;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod tool_uninstall;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
mod tool_upgrade;

#[cfg(all(feature = "test-python", feature = "test-pypi"))]
fn write_python_version_tool(
    wheels: &assert_fs::fixture::ChildPath,
    requires_python: Option<&str>,
) -> anyhow::Result<()> {
    use assert_fs::fixture::{FileWriteBin, PathChild};
    let requires_python = requires_python.map(str::parse).transpose()?;
    let (filename, wheel) = uv_test::packse::generate_wheel_with_files(
        &"bound-tool".parse()?,
        &"1.0.0".parse()?,
        &[],
        &std::collections::BTreeMap::new(),
        requires_python.as_ref(),
        "py3-none-any",
        &[
            (
                "bound_tool/cli.py",
                "import sys\ndef main():\n    print(f'{sys.version_info.major}.{sys.version_info.minor}')\n",
            ),
            (
                "bound_tool-1.0.0.dist-info/entry_points.txt",
                "[console_scripts]\nbound-tool = bound_tool.cli:main\n",
            ),
        ],
    );
    wheels.child(filename).write_binary(&wheel)?;
    Ok(())
}
