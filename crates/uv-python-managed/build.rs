#[expect(clippy::disallowed_types)]
use std::fs::{File, FileTimes};
use std::io;
use std::path::PathBuf;
use std::{env, fs};

use zstd::stream::write::Encoder;

use uv_static::EnvVars;

fn main() {
    for provider in ["python-build-standalone", "pypy", "graalpy", "pyodide"] {
        compress_metadata(provider);
    }
}

fn compress_metadata(provider: &str) {
    let version_metadata = PathBuf::from(env::var(EnvVars::CARGO_MANIFEST_DIR).unwrap())
        .join("download-metadata")
        .join(format!("{provider}.ndjson"));

    let version_metadata_compressed =
        PathBuf::from(env::var(EnvVars::OUT_DIR).unwrap()).join(format!("{provider}.ndjson.zst"));

    println!(
        "cargo::rerun-if-changed={}",
        version_metadata.to_str().unwrap()
    );

    println!(
        "cargo::rerun-if-changed={}",
        version_metadata_compressed.to_str().unwrap()
    );

    #[expect(clippy::disallowed_types)]
    let mut in_file = File::open(&version_metadata).expect("Failed to open download metadata");
    #[expect(clippy::disallowed_types)]
    let out_file =
        File::create(version_metadata_compressed).expect("Failed to create compressed metadata");

    // Compress the embedded catalog to reduce the binary size.
    let mut encoder =
        Encoder::new(out_file, 19).expect("Failed to create download metadata encoder");
    io::copy(&mut in_file, &mut encoder).expect("Failed to compress download metadata");
    let out_file = encoder
        .finish()
        .expect("Failed to finish compressing download metadata");

    // Cargo uses the modified times of the paths specified in
    // `rerun-if-changed`, so fetch the current file times and set them the same
    // on the output file.
    #[expect(clippy::disallowed_methods)]
    let meta = fs::metadata(version_metadata).expect("Failed to read download metadata file times");

    out_file
        .set_times(
            FileTimes::new()
                .set_accessed(meta.accessed().unwrap())
                .set_modified(meta.modified().unwrap()),
        )
        .expect("Failed to write compressed download metadata file times");
}
