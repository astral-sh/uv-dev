use std::collections::BTreeMap;

use anyhow::Result;
use assert_fs::fixture::{ChildPath, FileWriteBin, PathChild};

use uv_test::TestContext;
use uv_test::packse::generate_wheel;

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
    let (filename, contents) = generate_wheel(
        &name.parse()?,
        &"1.0.0".parse()?,
        &[],
        &BTreeMap::new(),
        None,
        "py3-none-any",
        &entry_points,
    );
    let wheel = context.temp_dir.child(filename);
    wheel.write_binary(&contents)?;
    Ok(wheel)
}
