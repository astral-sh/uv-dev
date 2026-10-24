"""Boot independent Linux jobs from a prepared raw ext4 image.

The x86-64 image needs a synced environment at `/app/.venv`, a Python source
package to import, `/bin/mount`, and `/proc`. Prepare the project with
`uv sync --locked --no-editable --link-mode copy` inside the guest root.
A directory tree can be packed with `mkfs.ext4 -d ROOTFS image.ext4 1G`.
The kernel must have virtio PCI/block, ext4, devtmpfs and a serial console built in.

    python3 snapshot.py image.ext4 vmlinuz new-results --module installed_package

The native Linux tools are `cp`, `debugfs`, `qemu-img`, `qemu-system-x86_64`
and `timeout`. The default accelerator is KVM. Pass `--accel tcg` for emulation.
Each boot timing starts before QEMU launches and ends when the host receives
confirmation of the selected module's import. The driver measures deriving a
seed from the supplied image and creating each overlay separately.
"""

import argparse
import hashlib
import json
import shutil
import subprocess
from pathlib import Path
from time import perf_counter_ns


def digest(path: Path) -> str:
    with path.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("image", type=Path)
    parser.add_argument("kernel", type=Path)
    parser.add_argument("out", type=Path)
    parser.add_argument(
        "--module", required=True, help="Python source module installed in the venv"
    )
    parser.add_argument("--accel", choices=["kvm", "tcg"], default="kvm")
    args = parser.parse_args()
    if not all(
        name.isascii() and name.isidentifier() for name in args.module.split(".")
    ):
        parser.error("expected a dotted ASCII module name")
    image = args.image.resolve(strict=True)
    kernel = args.kernel.resolve(strict=True)
    out = args.out.resolve()
    out.mkdir(mode=0o700)
    start = perf_counter_ns()
    seed = out / "seed.ext4"
    subprocess.run(["cp", "--sparse=always", "--reflink=auto", image, seed], check=True)
    seed.chmod(0o600)
    probe = out / "guest.py"
    shutil.copyfile(Path(__file__).with_name("guest.py"), probe)
    subprocess.run(
        ["debugfs", "-w", "-f", "-", seed],
        cwd=out,
        check=True,
        input=b"write guest.py /uv-snapshot-probe\nset_inode_field /uv-snapshot-probe mode 0100755\n",
        stdout=subprocess.DEVNULL,
    )
    if (
        subprocess.check_output(["debugfs", "-R", "cat /uv-snapshot-probe", seed])
        != probe.read_bytes()
    ):
        raise ValueError("Could not install the guest probe in the derived seed")
    seed.chmod(0o400)
    original = digest(seed)
    result = {
        "accel": args.accel,
        "module": args.module,
        "seed_sha256": original,
        "seed_setup_ms": (perf_counter_ns() - start) / 1e6,
        "snapshots": {},
        "boots": {},
    }
    for name in ("a", "b"):
        disk = out / f"{name}.qcow2"
        start = perf_counter_ns()
        subprocess.run(
            ["qemu-img", "create", "-q", "-f", "qcow2", "-F", "raw", "-b", seed, disk],
            check=True,
        )
        result["snapshots"][name] = {
            "create_ms": (perf_counter_ns() - start) / 1e6,
            "allocated_bytes": disk.stat().st_blocks * 512,
        }
    for name, mode in (("a", "write"), ("b", "clean"), ("a", "readback")):
        command = [
            "timeout",
            "--kill-after=5",
            "120",
            "qemu-system-x86_64",
            "-accel",
            args.accel,
            "-cpu",
            "host" if args.accel == "kvm" else "max",
            "-m",
            "384M",
            "-nodefaults",
            "-display",
            "none",
            "-monitor",
            "none",
            "-serial",
            "stdio",
            "-nic",
            "none",
            "-no-reboot",
            "-kernel",
            kernel,
            "-append",
            f"console=ttyS0 root=/dev/vda rw rootwait init=/uv-snapshot-probe loglevel=3 uv_snapshot_mode={mode} uv_snapshot_module={args.module}",
            "-drive",
            f"file={name}.qcow2,format=qcow2,if=virtio",
        ]
        imported = None
        verified = False
        start = perf_counter_ns()
        log = out / f"{name}-{mode}.log"
        with (
            log.open("w") as f,
            subprocess.Popen(
                command,
                cwd=out,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
            ) as proc,
        ):
            for line in proc.stdout:
                f.write(line)
                if line.rstrip() == f"UV_IMPORT_OK {mode}":
                    imported = (perf_counter_ns() - start) / 1e6
                if line.rstrip() == f"UV_SNAPSHOT_OK {mode}":
                    verified = True
            if proc.wait() or imported is None or not verified:
                raise RuntimeError(f"Guest failed during {mode}; see {log}")
        result["boots"][mode] = {"launch_to_import_ms": imported}
    if digest(seed) != original:
        raise ValueError("The backing image changed")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
