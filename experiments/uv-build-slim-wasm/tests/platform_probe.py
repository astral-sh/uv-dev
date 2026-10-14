"""Record native artifact, filesystem, concurrency, and wheel-only comparisons."""

import argparse
import hashlib
import importlib
import json
import os
import platform
import struct
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--reference", required=True, type=Path)
    parser.add_argument("--wheel", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--streaming", action="store_true")
    args = parser.parse_args()
    source, reference, wheel, report = map(
        Path.resolve, (args.source, args.reference, args.wheel, args.report)
    )
    report.mkdir(parents=True, exist_ok=True)
    os.environ["UV_SLIM_REFERENCE"] = str(reference)
    result = {
        "python": sys.version,
        "platform": platform.platform(),
        "pointer_width": struct.calcsize("P") * 8,
        "gil_enabled": getattr(sys, "_is_gil_enabled", lambda: True)(),
        "reference_sha256": hashlib.sha256(reference.read_bytes()).hexdigest(),
        "wheel_sha256": hashlib.sha256(wheel.read_bytes()).hexdigest(),
        "checks": {},
    }

    def run(name, script, extra):
        command = [
            sys.executable,
            str(ROOT / script),
            "--reference",
            str(reference),
            "--report",
            str(report / (name + ".json")),
            *map(str, extra),
        ]
        with (report / (name + ".log")).open("w", encoding="utf-8") as log:
            completed = subprocess.run(
                command, stdout=log, stderr=subprocess.STDOUT, check=False
            )
        result["checks"][name] = {"exit_status": completed.returncode}
        print(name, completed.returncode, flush=True)

    run("artifacts", "tests/runtime_smoke.py", ["--source", source])
    run("concurrent", "tests/concurrent_probe.py", [])
    run("isolated", "tools/isolated_smoke.py", ["--source", source, "--wheel", wheel])
    for module_name in ["test_edges"] + (["test_host"] if os.name != "nt" else []):
        module = importlib.import_module(module_name)
        with (report / (module_name + ".log")).open("w", encoding="utf-8") as log:
            tests = unittest.TextTestRunner(stream=log, verbosity=2).run(
                unittest.defaultTestLoader.loadTestsFromModule(module)
            )
        result["checks"][module_name] = {
            "exit_status": 0 if tests.wasSuccessful() else 1,
            "run": tests.testsRun,
            "passed": tests.testsRun
            - len(tests.skipped)
            - len(tests.errors)
            - len(tests.failures),
            "skipped": [(str(test), reason) for test, reason in tests.skipped],
            "errors": [(str(test), reason) for test, reason in tests.errors],
            "failures": [(str(test), reason) for test, reason in tests.failures],
        }
        print(module_name, tests.wasSuccessful(), flush=True)
    if args.streaming:
        run("streaming", "tests/streaming_probe.py", [])
    result["successful"] = all(
        check["exit_status"] == 0 for check in result["checks"].values()
    )
    (report / "summary.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result), flush=True)
    raise SystemExit(0 if result["successful"] else 1)


if __name__ == "__main__":
    main()
