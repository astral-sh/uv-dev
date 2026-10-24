use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use anyhow::{Result, ensure};
use nix::mount::{MsFlags, mount};
use nix::sched::{CloneFlags, unshare};
use nix::sys::stat::Mode;
use nix::unistd::{chdir, mkdir};
use tokio::process::Command;

/// Mount at the path used by installed scripts and cached bytecode.
pub(super) fn configure(process: &mut Command, root: &Path, scratch: &Path) -> Result<()> {
    let root = fs_err::canonicalize(root)?;
    let scratch = fs_err::canonicalize(scratch)?;
    ensure!(
        !scratch.starts_with(&root),
        "The uv cache for `--overlay` must be outside the environment"
    );
    for path in [&root, &scratch] {
        ensure!(
            !path
                .as_os_str()
                .as_bytes()
                .iter()
                .any(|c| matches!(c, b',' | b':' | b'\\')),
            "OverlayFS mount paths cannot contain commas, colons, or backslashes"
        );
    }
    let cwd = CString::new(std::env::current_dir()?.as_os_str().as_bytes())?;
    let root = CString::new(root.as_os_str().as_bytes())?;
    let upper = CString::new(scratch.join("upper").as_os_str().as_bytes())?;
    let work = CString::new(scratch.join("work").as_os_str().as_bytes())?;
    let scratch = CString::new(scratch.as_os_str().as_bytes())?;
    let options = CString::new(
        [
            b"lowerdir=",
            root.as_bytes(),
            b",upperdir=",
            upper.as_bytes(),
            b",workdir=",
            work.as_bytes(),
        ]
        .concat(),
    )?;

    // SAFETY: All path buffers are allocated before fork. The callback only invokes Linux
    // syscalls with borrowed C strings and returns errors as raw OS errors.
    #[expect(unsafe_code)]
    unsafe {
        process.pre_exec(move || {
            unshare(CloneFlags::CLONE_NEWNS)?;
            mount(
                None::<&CStr>,
                c"/",
                None::<&CStr>,
                MsFlags::MS_REC | MsFlags::MS_PRIVATE,
                None::<&CStr>,
            )?;
            let flags = MsFlags::MS_NODEV | MsFlags::MS_NOSUID;
            // The kernel retains the upper layer while any descendant holds the namespace.
            mount(
                Some(c"tmpfs"),
                scratch.as_c_str(),
                Some(c"tmpfs"),
                flags,
                Some(c"mode=0700"),
            )?;
            mkdir(upper.as_c_str(), Mode::S_IRWXU)?;
            mkdir(work.as_c_str(), Mode::S_IRWXU)?;
            mount(
                Some(c"overlay"),
                root.as_c_str(),
                Some(c"overlay"),
                flags,
                Some(options.as_c_str()),
            )?;
            // An inherited cwd under the lower directory would bypass copy-up on relative writes.
            chdir(cwd.as_c_str())?;
            Ok(())
        });
    }
    Ok(())
}
