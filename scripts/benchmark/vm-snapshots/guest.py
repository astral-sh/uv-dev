#!/app/.venv/bin/python -IB
"""Guest init for the disk snapshot experiment."""

import ctypes
import importlib
import os
import subprocess
import sys
import traceback
from pathlib import Path


def probe() -> None:
    subprocess.run(["/bin/mount", "-t", "proc", "proc", "/proc"], check=True)
    options = dict(
        arg.split("=", 1)
        for arg in Path("/proc/cmdline").read_text().split()
        if arg.startswith("uv_snapshot_") and "=" in arg
    )
    mode = options["uv_snapshot_mode"]
    module = importlib.import_module(options["uv_snapshot_module"])
    source = Path(module.__file__).resolve()
    if source.suffix != ".py" or not source.is_relative_to(sys.prefix):
        raise ValueError(
            f"Expected Python source inside the guest environment, got {source}"
        )
    print(f"UV_IMPORT_OK {mode}", flush=True)
    marker = b"# uv disk snapshot probe"
    if mode not in {"write", "clean", "readback"} or (
        marker in source.read_bytes()
    ) != (mode == "readback"):
        raise ValueError(f"Unexpected initial contents in {mode}")
    if mode == "write":
        with source.open("ab") as f:
            f.write(b"\n" + marker + b"\n")
        if marker not in source.read_bytes():
            raise ValueError("Guest write did not persist")
    print(f"UV_SNAPSHOT_OK {mode}", flush=True)


if __name__ == "__main__":
    try:
        probe()
    finally:
        if sys.exc_info()[0] is not None:
            traceback.print_exc()
        os.sync()
        subprocess.run(["/bin/mount", "-o", "remount,ro", "/"], check=False)
        # Linux `reboot(2)` takes `LINUX_REBOOT_CMD_POWER_OFF`.
        ctypes.CDLL(None).reboot(0x4321FEDC)
