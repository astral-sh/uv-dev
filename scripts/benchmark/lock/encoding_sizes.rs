use std::error::Error;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::time::Instant;

use serde_json::{Value, json};
use uv_lock::{BuildEncoding, Lock};

/// Serialize an existing lock's resolved graphs and verify each representation round-trips.
fn sizes(path: &Path) -> Result<Value, Box<dyn Error>> {
    let lock = Lock::from_toml(&fs::read_to_string(path)?)?;
    let expected = lock.with_build_encoding(BuildEncoding::Dependency);
    let mut results = Vec::new();
    for (name, encoding) in [
        ("dependency", BuildEncoding::Dependency),
        ("package", BuildEncoding::Package),
        ("conflict-marker", BuildEncoding::ConflictMarker),
    ] {
        let graph = expected.clone().with_build_encoding(encoding);
        let mut samples_ns = [0; 3];
        let mut encoded = String::new();
        for elapsed in &mut samples_ns {
            let start = Instant::now();
            let rendered = graph.to_toml()?;
            *elapsed = start.elapsed().as_nanos();
            encoded = rendered;
        }
        let mut ordered_ns = samples_ns;
        ordered_ns.sort_unstable();
        let decoded = Lock::from_toml(&encoded)?.with_build_encoding(BuildEncoding::Dependency);
        results.push(json!({
            "encoding": name,
            "bytes": encoded.len(),
            "entries": encoded.lines().filter(|line| *line == "[[package]]").count(),
            "serialize_ns": samples_ns,
            "median_serialize_ns": ordered_ns[1],
            "roundtrip_equal": decoded == expected,
        }));
    }
    Ok(json!({"builds_locked": expected.locks_builds(), "encodings": results}))
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut output = BufWriter::new(io::stdout().lock());
    for file in std::env::args_os().skip(1) {
        let path = Path::new(&file);
        let row = match sizes(path) {
            Ok(sizes) => json!({"path": path, "sizes": sizes}),
            Err(error) => json!({"path": path, "error": error.to_string()}),
        };
        serde_json::to_writer(&mut output, &row)?;
        writeln!(output)?;
    }
    output.flush()?;
    Ok(())
}
