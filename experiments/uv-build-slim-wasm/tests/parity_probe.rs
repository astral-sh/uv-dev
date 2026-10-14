use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Mutex, OnceLock};

struct Server {
    _child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

static SERVER: OnceLock<Mutex<Option<Server>>> = OnceLock::new();

pub(crate) struct Probe {
    temporary: tempfile::TempDir,
    native: PathBuf,
}

fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
    if !source.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if entry.file_type()?.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(entry.path())?, target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    std::fs::set_permissions(destination, std::fs::metadata(source)?.permissions())?;
    Ok(())
}

pub(crate) fn prepare(output: &Path) -> Option<Probe> {
    std::env::var_os("UV_SLIM_PARITY_SERVER")?;
    let temporary = tempfile::tempdir().expect("create parity output directory");
    copy_tree(output, &temporary.path().join("output")).expect("copy pre-build output state");
    Some(Probe {
        temporary,
        native: output.to_owned(),
    })
}

impl Probe {
    pub(crate) fn check<T: std::fmt::Display>(
        self,
        operation: &str,
        source: &Path,
        metadata: Option<&Path>,
        version: &str,
        legacy_tar: bool,
        result: &Result<T, crate::Error>,
    ) {
        let request = serde_json::json!({
            "test":std::thread::current().name().unwrap_or_default().to_owned(),
            "operation":operation,"source":source,"metadata":metadata,"version":version,
            "cwd":std::env::current_dir().expect("native working directory"),
            "legacy_tar":legacy_tar,
            "preview":uv_preview::is_enabled(uv_preview::PreviewFeature::MetadataJson),
            "python_output":self.temporary.path().join("output"),"native_output":self.native,
            "success":result.is_ok(),"filename":result.as_ref().ok().map(ToString::to_string),
            "error":result.as_ref().err().map(ToString::to_string).unwrap_or_default(),
        });
        let mutex = SERVER.get_or_init(|| Mutex::new(None));
        let mut guard = mutex.lock().expect("lock parity server");
        if guard.is_none() {
            let script = std::env::var_os("UV_SLIM_PARITY_SERVER").expect("parity server path");
            let mut child = Command::new("python3")
                .arg(script)
                .env("PYTHONDONTWRITEBYTECODE", "1")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .expect("start parity server");
            let input = child.stdin.take().expect("parity input");
            let output = BufReader::new(child.stdout.take().expect("parity output"));
            *guard = Some(Server {
                _child: child,
                input,
                output,
            });
        }
        let server = guard.as_mut().expect("parity server initialized");
        writeln!(server.input, "{request}").expect("write parity request");
        server.input.flush().expect("flush parity request");
        let mut line = String::new();
        server
            .output
            .read_line(&mut line)
            .expect("read parity response");
        let response: serde_json::Value =
            serde_json::from_str(&line).expect("parse parity response");
        if response["equal"] != true {
            eprintln!("PARITY MISMATCH: {response}");
        }
        let path = std::env::var_os("UV_SLIM_PARITY_REPORT").expect("parity report path");
        let mut report = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("open parity report");
        writeln!(report, "{response}").expect("write parity report");
        assert_eq!(
            response["equal"], true,
            "generated backend parity mismatch: {response}"
        );
    }
}
