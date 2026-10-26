use std::io::{Read, Seek, Write};
use std::path::Path;

use anyhow::{Context, bail, ensure};
use serde_json::json;
use tokio::process::Command;
use uv_cache::Cache;
use uv_cli::{VmAccel, VmRunArgs};
use uv_command_support::ExitStatus;
use uv_command_support::child::run_to_completion;

const GUEST_INIT: &str = include_str!("vm_guest.py");

/// Boot an external Linux kernel with a disposable writable layer above the prepared root disk.
pub(crate) async fn run(args: VmRunArgs, cache: &Cache) -> anyhow::Result<ExitStatus> {
    let image = fs_err::canonicalize(&args.image)?;
    let kernel = fs_err::canonicalize(&args.kernel)?;
    let scratch = cache.venv_dir()?;
    let overlay = scratch.path().join("root.qcow2");
    let status = Command::new("qemu-img")
        .args(["create", "-q", "-f", "qcow2", "-F", "raw", "-b"])
        .arg(image)
        .arg(&overlay)
        .status()
        .await
        .context("Failed to start `qemu-img`")?;
    ensure!(
        status.success(),
        "`qemu-img` failed to create the guest root overlay"
    );

    // The kernel starts the image's own Python directly, with this read-only disk as its script.
    let init = scratch.path().join("init.raw");
    let mut script = GUEST_INIT.to_owned();
    script.extend(std::iter::repeat_n(' ', (512 - script.len() % 512) % 512));
    fs_err::write(&init, script)?;

    let job = scratch.path().join("job.raw");
    let mut mailbox = fs_err::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&job)?;
    let request = serde_json::to_vec(&args.command)?;
    let length = u32::try_from(request.len()).context("Guest command is too large")?;
    mailbox.write_all(&length.to_le_bytes())?;
    mailbox.write_all(&request)?;
    mailbox.set_len((u64::from(length) + 4).next_multiple_of(512))?;
    mailbox.flush()?;

    let mut vm = Command::new("qemu-system-x86_64");
    let accel = match args.accel {
        VmAccel::Kvm => "kvm",
        VmAccel::Tcg => "tcg",
    };
    vm.args(["-nodefaults", "-no-reboot", "-display", "none", "-monitor", "none"])
        .args(["-serial", "stdio", "-nic", "none", "-accel", accel])
        .arg("-m").arg(args.memory.to_string())
        .arg("-kernel").arg(kernel)
        .arg("-append").arg("console=ttyS0 loglevel=3 root=/dev/vda rw rootwait panic=-1 init=/app/.venv/bin/python -- -IB /dev/vdb");
    attach_disk(&mut vm, "root", &overlay, "qcow2", false)?;
    attach_disk(&mut vm, "init", &init, "raw", true)?;
    attach_disk(&mut vm, "job", &job, "raw", false)?;
    let status = run_to_completion(
        vm.kill_on_drop(true)
            .spawn()
            .context("Failed to start `qemu-system-x86_64`")?,
    )
    .await?;
    if !matches!(status, ExitStatus::External(0)) {
        return Ok(status);
    }

    mailbox.rewind()?;
    let mut reply = [0; 9];
    mailbox.read_exact(&mut reply)?;
    if &reply[..8] != b"UVEXIT01" {
        bail!("Guest exited without reporting a command status");
    }
    Ok(ExitStatus::External(reply[8]))
}

fn attach_disk(
    vm: &mut Command,
    name: &str,
    path: &Path,
    format: &str,
    read_only: bool,
) -> anyhow::Result<()> {
    let filename = path
        .to_str()
        .context("The VM scratch directory must have a UTF-8 path")?;
    let block = json!({
        "driver": format,
        "node-name": name,
        "read-only": read_only,
        "file": { "driver": "file", "filename": filename },
    });
    vm.arg("-blockdev")
        .arg(block.to_string())
        .arg("-device")
        .arg(format!("virtio-blk-pci,drive={name}"));
    Ok(())
}
