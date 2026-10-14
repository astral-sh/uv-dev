use std::path::Path;

use anyhow::{Context, Result};
use uv_cache::Cache;
use uv_platform::{Arch, Os};
use uv_python_interpreter::Interpreter;
use uv_test::TestContext;

/// Query the selected interpreter, which may have a different architecture than the test binary.
pub(super) fn interpreter_platform(context: &TestContext, executable: &Path) -> Result<(Os, Arch)> {
    // The setup query must not populate the command's discovery cache.
    let cache = Cache::from_path(context.root.join("interpreter-platform-cache"))
        .init_no_wait()?
        .context("fixture interpreter cache is unexpectedly locked")?;
    let interpreter = Interpreter::query(executable, &cache)?;
    Ok((interpreter.os(), interpreter.arch()))
}
