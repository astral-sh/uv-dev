"""Persistent Python oracle for the original Rust backend tests."""

import contextlib
import hashlib
import io
import json
import os
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
import uv_build_slim_wasm as backend

if os.environ.get("UV_SLIM_PROGRAM"):
    backend._PROGRAM = backend.Program(Path(os.environ["UV_SLIM_PROGRAM"]))


def inventory(root):
    result = {}
    if not root.exists():
        return result
    for path in sorted(root.rglob("*")):
        key = str(path.relative_to(root))
        if path.is_symlink():
            result[key] = ["symlink", os.readlink(path)]
        elif path.is_dir():
            result[key] = ["directory", path.stat().st_mode & 0o777]
        elif path.is_file():
            result[key] = [
                "file",
                hashlib.sha256(path.read_bytes()).hexdigest(),
                path.stat().st_mode & 0o777,
            ]
    return result


def check(request):
    os.chdir(request["cwd"])
    if request["preview"]:
        os.environ["UV_PREVIEW_FEATURES"] = "metadata-json"
    else:
        os.environ.pop("UV_PREVIEW_FEATURES", None)
    os.environ.pop("UV_PREVIEW", None)
    os.environ["UV_LEGACY_TAR_BACKEND"] = "1" if request.get("legacy_tar") else "0"
    output = Path(request["python_output"])
    stderr = io.StringIO()
    stdout = io.StringIO()
    arguments = [request["operation"], str(output)]
    if request.get("metadata"):
        arguments.append(request["metadata"])
    success = True
    filename = None
    started = time.monotonic()
    try:
        with contextlib.redirect_stderr(stderr), contextlib.redirect_stdout(stdout):
            filename = backend.call(
                arguments, _version=request["version"], _source_tree=request["source"]
            )
    except SystemExit:
        success = False
    except Exception:
        import traceback

        success = False
        traceback.print_exc(file=stderr)
    differences = []
    if success != request["success"]:
        differences.append("success status")
    if success and filename != request["filename"]:
        differences.append("returned filename")
    actual = inventory(output)
    expected = inventory(Path(request["native_output"]))
    for key in sorted(actual.keys() | expected.keys()):
        if actual.get(key) != expected.get(key):
            differences.append(key)
    if not request["success"] and request["error"] not in stderr.getvalue().replace(
        str(output), request["native_output"]
    ):
        differences.append("error text")
    return {
        "test": request["test"],
        "operation": request["operation"],
        "equal": not differences,
        "differences": differences,
        "native_success": request["success"],
        "python_success": success,
        "file_differences": {
            key: {"native": expected.get(key), "python": actual.get(key)}
            for key in actual.keys() | expected.keys()
            if actual.get(key) != expected.get(key)
        },
        "native_error": request["error"],
        "python_stderr": stderr.getvalue(),
        "seconds": time.monotonic() - started,
    }


for line in sys.stdin:
    try:
        result = check(json.loads(line))
    except Exception:
        import traceback

        result = {"equal": False, "server_error": traceback.format_exc()}
    print(json.dumps(result), flush=True)
