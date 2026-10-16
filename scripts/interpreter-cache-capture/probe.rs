use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::io::Write as _;
use uv_cache::Cache;
use uv_python_interpreter::Interpreter;

fn main() -> Result<()> {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    ensure!(
        arguments.len() == 2,
        "Expected executable and cache directory"
    );
    let cache = Cache::from_path(&arguments[1])
        .init_no_wait()?
        .context("Interpreter cache is locked")?;
    let interpreter = Interpreter::query(&arguments[0], &cache)?;
    writeln!(
        std::io::stdout().lock(),
        "{}",
        json!({
            "python_full_version": interpreter.python_full_version().to_string(),
            "sys_prefix": interpreter.sys_prefix(),
            "sys_base_prefix": interpreter.sys_base_prefix(),
            "sys_executable": interpreter.sys_executable(),
            "real_executable": interpreter.real_executable(),
            "runtime_site_packages": interpreter.runtime_site_packages(),
        })
    )?;
    Ok(())
}
