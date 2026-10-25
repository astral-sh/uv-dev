use std::ffi::CString;
use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::thread;

use anyhow::{Context, Result, ensure};
use rustix::fs::{Mode, OFlags, StatVfsMountFlags, open, statvfs};
use rustix::mount::{MountFlags, MountPropagationFlags, mount, mount_change};
use rustix::process::fchdir;
use rustix::thread::{
    CapabilitySet, LinkNameSpaceType, UnshareFlags, capabilities, move_into_link_name_space,
    unshare_unsafe,
};
use tempfile::TempDir;
use tokio::process::Command;
use uv_cache::Cache;
use uv_distribution_types::{BuiltDist, Dist, Resolution, ResolvedDist};
use uv_environment_operations::EnvironmentError;
use uv_python_interpreter::{PythonEnvironment, virtualenv_python_executable};

/// Reuse a nonempty selection of hashed registry wheels that do not require revalidation.
pub(super) fn eligible(resolution: &Resolution, cache: &Cache) -> bool {
    !resolution.is_empty()
        && statvfs(cache.root()).is_ok_and(|stat| !stat.f_flag.contains(StatVfsMountFlags::NOEXEC))
        && resolution.hashes().all(|(dist, hashes)| {
            let ResolvedDist::Installable { dist, .. } = dist else {
                return false;
            };
            let Dist::Built(BuiltDist::Registry(registry)) = dist.as_ref() else {
                return false;
            };
            !hashes.is_empty()
                && !cache.must_revalidate_package(&registry.best_wheel().filename.name)
        })
}

/// Creating a mount namespace requires `CAP_SYS_ADMIN`. Joining it also requires
/// `CAP_SYS_CHROOT` in the current user namespace.
pub(super) fn supported() -> bool {
    capabilities(None).is_ok_and(|set| {
        set.effective
            .contains(CapabilitySet::SYS_ADMIN | CapabilitySet::SYS_CHROOT)
    })
}

/// Inspect the prepared tree without executing Python, which could run its `.pth` or
/// `sitecustomize` hooks outside the child's writable view.
pub(super) fn open_lower(root: PathBuf) -> Result<PathBuf, EnvironmentError> {
    fs_err::metadata(root.join("pyvenv.cfg"))?;
    fs_err::metadata(virtualenv_python_executable(&root))?;
    Ok(root)
}

pub(super) struct PrivateOverlay {
    namespace: OwnedFd,
    cwd: OwnedFd,
    pub(super) scripts: PathBuf,
    _scratch: TempDir,
}

impl PrivateOverlay {
    /// Construct the mounts before starting the command so a denied mount can fall back to an install.
    pub(super) fn new(
        cache: &Cache,
        lower: &Path,
        environment: &PythonEnvironment,
    ) -> Result<Self> {
        let scratch = cache.venv_dir()?;
        let scratch_path = fs_err::canonicalize(scratch.path())?;
        let lower = fs_err::canonicalize(lower)?;
        let target = environment.root();
        for path in [target, lower.as_path()] {
            ensure!(
                !statvfs(path)?.f_flag.contains(StatVfsMountFlags::NOEXEC),
                "Private environment mount would bypass a noexec filesystem"
            );
        }
        for path in [&lower, &scratch_path] {
            ensure!(
                !path
                    .as_os_str()
                    .as_bytes()
                    .iter()
                    .any(|b| matches!(b, b':' | b',' | b'\\')),
                "Unsupported OverlayFS mount option path: {}",
                path.display()
            );
        }
        let options = CString::new(
            [
                b"lowerdir=".as_slice(),
                lower.as_os_str().as_bytes(),
                b",upperdir=",
                scratch_path.join("upper").as_os_str().as_bytes(),
                b",workdir=",
                scratch_path.join("work").as_os_str().as_bytes(),
            ]
            .concat(),
        )?;
        let cwd = std::env::current_dir()?;
        let (namespace, cwd) = thread::scope(|scope| -> Result<_> {
            thread::Builder::new()
                .spawn_scoped(scope, || -> Result<_> {
                    // SAFETY: NEWNS isolates only the mount namespace and filesystem context of this
                    // dedicated thread. It continues to share the process's file descriptor table.
                    #[expect(unsafe_code)]
                    unsafe { unshare_unsafe(UnshareFlags::NEWNS) }?;
                    mount_change(
                        "/",
                        MountPropagationFlags::REC | MountPropagationFlags::PRIVATE,
                    )?;
                    let flags = MountFlags::NODEV | MountFlags::NOSUID;
                    mount("tmpfs", &scratch_path, "tmpfs", flags, c"mode=0700")?;
                    fs_err::create_dir(scratch_path.join("upper"))?;
                    fs_err::create_dir(scratch_path.join("work"))?;
                    mount("overlay", target, "overlay", flags, options.as_c_str())?;
                    // The overlay keeps its lower mount reference. Hide the backing path from the
                    // child so an absolute path cannot accidentally modify the shared installation.
                    mount(
                        "tmpfs",
                        &lower,
                        "tmpfs",
                        flags | MountFlags::RDONLY | MountFlags::NOEXEC,
                        c"mode=000,size=4k",
                    )?;
                    let namespace = open(
                        "/proc/thread-self/ns/mnt",
                        OFlags::RDONLY | OFlags::CLOEXEC,
                        Mode::empty(),
                    )?;
                    // Joining the namespace can be denied independently of creating it.
                    move_into_link_name_space(namespace.as_fd(), Some(LinkNameSpaceType::Mount))?;
                    let cwd = open(
                        cwd,
                        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
                        Mode::empty(),
                    )?;
                    Ok((namespace, cwd))
                })?
                .join()
                .map_err(|_| io::Error::other("private mount thread panicked"))?
        })
        .context("Failed to prepare a private environment mount")?;
        Ok(Self {
            namespace,
            cwd,
            scripts: lower.join(&environment.interpreter().virtualenv().scripts),
            _scratch: scratch,
        })
    }

    pub(super) fn configure(self, command: &mut Command) {
        // SAFETY: Both descriptors were opened before fork. The child makes only setns and
        // fchdir syscalls before exec, without allocating or touching a lock.
        #[expect(unsafe_code)]
        unsafe {
            command.pre_exec(move || self.enter());
        }
    }

    fn enter(&self) -> io::Result<()> {
        move_into_link_name_space(self.namespace.as_fd(), Some(LinkNameSpaceType::Mount))?;
        fchdir(&self.cwd)?;
        Ok(())
    }
}
