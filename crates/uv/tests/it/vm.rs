use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use predicates::str::contains;

#[test]
fn vm_missing_image() {
    let context = uv_test::test_context_with_versions!(&[]);
    context
        .command()
        .args([
            "vm",
            "run",
            "--image",
            "absent.ext4",
            "--kernel",
            "vmlinuz",
            "--",
            "python",
            "-V",
        ])
        .assert()
        .failure()
        .stderr(contains("absent.ext4"));
}

/// Exercise the kernel's PID 1 handoff and a fresh writable root on each boot.
#[test]
#[ignore = "requires QEMU and a prepared image in UV_TEST_VM_IMAGE and UV_TEST_VM_KERNEL"]
fn vm_guest_status_and_disposable_root() -> Result<()> {
    let context = uv_test::test_context_with_versions!(&[]);
    let image = std::env::var_os("UV_TEST_VM_IMAGE")
        .ok_or_else(|| anyhow::anyhow!("Set UV_TEST_VM_IMAGE"))?;
    let kernel = std::env::var_os("UV_TEST_VM_KERNEL")
        .ok_or_else(|| anyhow::anyhow!("Set UV_TEST_VM_KERNEL"))?;
    let mut write = context.command();
    write.args(["vm", "run", "--image"]).arg(&image).arg("--kernel").arg(&kernel)
        .args(["--", "python", "-Ic", "import pathlib, sys; assert sys.argv[1:] == ['a b', '', '--x']; pathlib.Path('private').write_text('job'); sys.exit(37)", "a b", "", "--x"])
        .assert().code(37);
    context.command().args(["vm", "run", "--image"]).arg(image).arg("--kernel").arg(kernel)
        .args(["--", "python", "-Ic", "import os, pathlib, sys; assert sys.prefix == '/app/.venv'; assert os.path.isdir('/proc/self'); assert not pathlib.Path('private').exists()"])
        .assert().success();
    Ok(())
}
