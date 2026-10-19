use anyhow::Result;
use assert_fs::fixture::{ChildPath, FileWriteBin, PathChild};

use uv_test::TestContext;
use uv_test::packse::WheelBuilder;

/// Create a local wheel for a tool with known entry points.
pub(super) fn local_tool_wheel(
    context: &TestContext,
    name: &str,
    entry_points: &[&str],
) -> Result<ChildPath> {
    let entry_points = entry_points
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let (filename, contents) = WheelBuilder::new(&name.parse()?, &"1.0.0".parse()?)
        .entry_points(&entry_points)
        .build();
    let wheel = context.temp_dir.child(filename);
    wheel.write_binary(&contents)?;
    Ok(wheel)
}
