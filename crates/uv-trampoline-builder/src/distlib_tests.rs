use std::path::Path;

use anyhow::Result;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use futures_lite::future::block_on;
use futures_lite::io::Cursor;

use super::relocate_distlib_script_inner as relocate_distlib_script;

const STUB: &[u8] = include_bytes!("../trampolines/uv-trampoline-x86_64-console.exe");

fn launcher(shebang: &[u8], filename: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut archive = ZipFileWriter::new(Cursor::new(Vec::new()));
    let entry = ZipEntryBuilder::new(filename.to_owned().into(), Compression::Stored);
    block_on(archive.write_entry_whole(entry, b"print('installed provider')\n"))?;
    let payload = block_on(archive.close())?.into_inner();
    let mut launcher = STUB.to_vec();
    launcher.extend_from_slice(shebang);
    launcher.extend_from_slice(&payload);
    Ok((launcher, payload))
}

#[test]
fn relocation_preserves_stub_payload_and_arguments() -> Result<()> {
    let old = Path::new(r"C:\old Python\π\python.exe");
    let new = Path::new(r"C:\new Python\python.exe");
    let (launcher, payload) = launcher(
        format!("#!\"{}\" -O\n", old.display()).as_bytes(),
        "__main__.py",
    )?;
    let relocated = relocate_distlib_script(&launcher, old, new)
        .ok_or_else(|| anyhow::anyhow!("distlib launcher should be recognized"))?;
    let mut expected = STUB.to_vec();
    expected.extend_from_slice(format!("#!\"{}\" -O\n", new.display()).as_bytes());
    expected.extend_from_slice(&payload);
    assert_eq!(relocated, expected);
    Ok(())
}

#[test]
fn unrelated_interpreter_is_unchanged() -> Result<()> {
    let (launcher, _) = launcher(b"#!C:\\other\\python.exe\n", "__main__.py")?;
    assert!(
        relocate_distlib_script(
            &launcher,
            Path::new(r"C:\old\python.exe"),
            Path::new(r"C:\new\python.exe"),
        )
        .is_none()
    );
    Ok(())
}

#[test]
fn unrelated_archive_is_unchanged() -> Result<()> {
    let (launcher, _) = launcher(b"#!C:\\old\\python.exe\n", "other.py")?;
    assert!(
        relocate_distlib_script(
            &launcher,
            Path::new(r"C:\old\python.exe"),
            Path::new(r"C:\new\python.exe"),
        )
        .is_none()
    );
    Ok(())
}

#[test]
fn executable_without_appended_script_is_unchanged() {
    assert!(
        relocate_distlib_script(
            STUB,
            Path::new(r"C:\old\python.exe"),
            Path::new(r"C:\new\python.exe"),
        )
        .is_none()
    );
}
