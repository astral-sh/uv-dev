use std::collections::BTreeSet;
use std::io;
use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use fs_err as fs;
use serde_json::{Value, json};
use uv_cache::{CacheEntry, CachedByTimestamp};
use uv_cache_info::Timestamp;

use super::{InterpreterInfo, canonicalize_executable};

static SEQUENCE: AtomicUsize = AtomicUsize::new(0);

fn metadata(path: &Path) -> Value {
    match fs::metadata(path) {
        Ok(metadata) => json!({
            "length": metadata.len(),
            "mtime_ns": metadata.modified().ok().and_then(|value| value.duration_since(UNIX_EPOCH).ok()).map(|value| value.as_nanos().to_string()),
            "created_ns": metadata.created().ok().and_then(|value| value.duration_since(UNIX_EPOCH).ok()).map(|value| value.as_nanos().to_string()),
        }),
        Err(error) => json!({"error": error.to_string(), "kind": format!("{:?}", error.kind())}),
    }
}

fn snapshot(path: &Path, destination: &Path) -> io::Result<Value> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.len() <= 1_048_576 => {
            let bytes = fs::read(path)?;
            fs::write(destination, &bytes)?;
            Ok(json!({"state": "present", "length": bytes.len(), "artifact": destination}))
        }
        Ok(metadata) => Ok(json!({"state": "too_large", "length": metadata.len()})),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(json!({"state": "missing"})),
        Err(error) => Ok(json!({"state": "unreadable", "error": error.to_string()})),
    }
}

pub(super) fn record(
    event: &str,
    executable: &Path,
    entry: Option<&CacheEntry>,
    info: Option<&InterpreterInfo>,
    lookup_timestamp: Option<&Timestamp>,
) {
    let Some(directory) = std::env::var_os("UV_INTERNAL__INTERPRETER_CACHE_CAPTURE") else {
        return;
    };
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let directory = Path::new(&directory);
        fs::create_dir_all(directory)?;
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        if sequence >= 1024 {
            return Err(io::Error::other("Interpreter capture exceeded its event limit").into());
        }
        let unix_ns = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let stem = format!("{}-{unix_ns}-{sequence:04}", std::process::id());
        let absolute = std::path::absolute(executable)?;
        let canonical = canonicalize_executable(&absolute).ok();
        let python_executable = std::env::var_os("PYTHONEXECUTABLE").map(std::path::PathBuf::from);
        let pyvenv_launcher = std::env::var_os("__PYVENV_LAUNCHER__").map(std::path::PathBuf::from);
        let mut paths = BTreeSet::new();
        for path in std::iter::once(absolute.as_path())
            .chain(canonical.as_deref())
            .chain(python_executable.as_deref())
            .chain(pyvenv_launcher.as_deref())
        {
            let path = std::path::absolute(path)?;
            if let Some(parent) = path.parent() {
                paths.insert(parent.join("pyvenv.cfg"));
                if let Some(parent) = parent.parent() {
                    paths.insert(parent.join("pyvenv.cfg"));
                }
            }
        }
        let mut configurations = Vec::new();
        for (index, path) in paths.into_iter().enumerate() {
            configurations.push(json!({
                "path": path,
                "metadata": metadata(&path),
                "contents": snapshot(&path, &directory.join(format!("{stem}-config-{index}.bin")))?,
            }));
        }
        let cache = entry
            .map(|entry| -> Result<Value, Box<dyn std::error::Error>> {
                let contents = snapshot(
                    entry.path(),
                    &directory.join(format!("{stem}-cache.msgpack")),
                )?;
                let artifact = directory.join(format!("{stem}-cache.msgpack"));
                let decoded = fs::read(artifact).ok().and_then(|bytes| {
                    rmp_serde::from_slice::<CachedByTimestamp<InterpreterInfo>>(&bytes).ok()
                });
                Ok(json!({"path": entry.path(), "contents": contents, "decoded": decoded}))
            })
            .transpose()?;
        let record = json!({
            "event": event,
            "pid": std::process::id(),
            "sequence": sequence,
            "unix_ns": unix_ns.to_string(),
            "executable": executable,
            "absolute": absolute,
            "canonical": canonical,
            "absolute_metadata": metadata(&absolute),
            "canonical_metadata": canonical.as_deref().map(metadata),
            "current_timestamp": canonical.as_deref().and_then(|path| Timestamp::from_path(path).ok()),
            "lookup_timestamp": lookup_timestamp,
            "python_executable": python_executable,
            "pyvenv_launcher": pyvenv_launcher,
            "configurations": configurations,
            "cache": cache,
            "returned_info": info,
        });
        fs::write(
            directory.join(format!("{stem}.json")),
            serde_json::to_vec_pretty(&record)?,
        )?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = writeln!(io::stderr().lock(), "INTERPRETER_CAPTURE_ERROR: {error:#}");
    }
}
