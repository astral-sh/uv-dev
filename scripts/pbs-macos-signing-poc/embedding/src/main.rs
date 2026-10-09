use pyo3::prelude::*;

fn main() -> pyo3::PyResult<()> {
    Python::attach(|python| {
        let system = python.import("sys")?;
        let version: String = system.getattr("version")?.extract()?;
        println!("Embedded Python: {version}");
        Ok(())
    })
}
