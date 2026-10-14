"""Create an isolated Rust sysroot with native host I/O diagnostics for WASI."""

import argparse
import shutil
import subprocess
from pathlib import Path


def link_children(source, destination, exclude):
    destination.mkdir(parents=True, exist_ok=True)
    for entry in source.iterdir():
        target = destination / entry.name
        if entry.name not in exclude and not target.exists():
            target.symlink_to(entry, target_is_directory=entry.is_dir())


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--destination", required=True, type=Path)
    args = parser.parse_args()
    original = Path(
        subprocess.check_output(
            ["rustc", "+1.99.0", "--print", "sysroot"], text=True
        ).strip()
    )
    root = args.destination.resolve()
    link_children(original, root, {"lib"})
    link_children(original / "lib", root / "lib", {"rustlib"})
    link_children(original / "lib/rustlib", root / "lib/rustlib", {"src"})
    source = original / "lib/rustlib/src/rust/library"
    library = root / "lib/rustlib/src/rust/library"
    if not library.exists():
        shutil.copytree(source, library)
    error = library / "core/src/io/error.rs"
    text = (source / "core/src/io/error.rs").read_text()
    for marker, count in [
        ('.field("code", &code)', 1),
        ('write!(fmt, "{detail} (os error {code})")', 1),
        ("format_os_error(code, fmt)", 2),
    ]:
        if text.count(marker) != count:
            raise ValueError("Rust 1.99.0 I/O formatting no longer matches: " + marker)
    # Keep the WASI errno used by program control flow; adapt only its display.
    text = text.replace(
        '.field("code", &code)', '.field("code", &display_os_error_code(code))'
    )
    text = text.replace(
        'write!(fmt, "{detail} (os error {code})")',
        'write!(fmt, "{detail} (os error {})", display_os_error_code(code))',
    )
    text = text.replace(
        "format_os_error(code, fmt)", "display_os_error_message(code, fmt)"
    )
    text += """
#[cfg(target_os = "wasi")]
fn display_os_error_code(code: RawOsError) -> RawOsError {
    #[link(wasm_import_module = "uv_build_slim")]
    unsafe extern "C" { fn native_errno(code: i32) -> i32; }
    // SAFETY: The host function maps one integer error code and retains no data.
    unsafe { native_errno(code) }
}
#[cfg(not(target_os = "wasi"))]
fn display_os_error_code(code: RawOsError) -> RawOsError { code }

#[cfg(target_os = "wasi")]
fn display_os_error_message(code: RawOsError, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    #[link(wasm_import_module = "uv_build_slim")]
    unsafe extern "C" {
        fn native_error_message(code: i32, offset: usize, buffer: *mut u8, capacity: usize) -> usize;
    }
    let mut buffer = [0u8; 256];
    let mut offset = 0;
    loop {
        // SAFETY: The buffer is live and writable for its stated capacity. The
        // host copies a complete UTF-8 prefix synchronously and retains no pointer.
        let written = unsafe { native_error_message(code, offset, buffer.as_mut_ptr(), buffer.len()) };
        if written == 0 { break; }
        let bytes = buffer.get(..written).ok_or(fmt::Error)?;
        let text = crate::str::from_utf8(bytes).map_err(|_|fmt::Error)?;
        formatter.write_str(text)?;
        offset += written;
    }
    Ok(())
}
#[cfg(not(target_os = "wasi"))]
fn display_os_error_message(code: RawOsError, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    format_os_error(code, formatter)
}
"""
    error.write_text(text)
    wrapper = root / "rustc-with-sysroot"
    wrapper.write_text(
        "#!/usr/bin/env python3\nimport os,sys\n"
        + f'os.execv({str(original / "bin/rustc")!r}, [{str(original / "bin/rustc")!r}, *sys.argv[1:], "--sysroot", {str(root)!r}])\n'
    )
    wrapper.chmod(0o755)
    print(wrapper)


if __name__ == "__main__":
    main()
