"""Check every hook from only the wheel, with native execution unavailable."""

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

CHILD = r"""
import json, os, pathlib, sys
wheel, source, output = sys.argv[1:]
def audit(event, arguments):
    if event in {'subprocess.Popen', 'os.system', 'socket.__new__'}:
        raise AssertionError('runtime attempted external execution or networking: ' + event)
    if event == 'import' and arguments[0].startswith('pywasm'):
        raise AssertionError('runtime imported the WebAssembly parser/interpreter')
sys.addaudithook(audit)
sys.path.insert(0, wheel)
import uv_build
assert uv_build.__file__.replace('\\', '/').startswith(wheel.replace('\\', '/') + '/')
os.chdir(source)
result = {}
for name in ['get_requires_for_build_wheel', 'get_requires_for_build_sdist', 'get_requires_for_build_editable']:
    result[name] = getattr(uv_build, name)()
    assert result[name] == []
for name in ['prepare_metadata_for_build_wheel', 'prepare_metadata_for_build_editable', 'build_wheel', 'build_sdist', 'build_editable']:
    directory = pathlib.Path(output) / name
    directory.mkdir()
    arguments = [str(directory)]
    if name in {'build_wheel', 'build_editable'}:
        arguments.extend([None, str(pathlib.Path(output) / 'prepare_metadata_for_build_wheel' / result['prepare_metadata_for_build_wheel'])])
    result[name] = getattr(uv_build, name)(*arguments)
print(json.dumps(result))
"""


def inventory(root):
    return {
        str(path.relative_to(root)): [
            hashlib.sha256(path.read_bytes()).hexdigest(),
            path.stat().st_mode & 0o777,
        ]
        for path in root.rglob("*")
        if path.is_file()
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--wheel", required=True, type=Path)
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--reference", required=True, type=Path)
    parser.add_argument("--python", type=Path, default=Path(sys.executable))
    parser.add_argument("--report", required=True, type=Path)
    args = parser.parse_args()
    report_path = args.report.resolve()
    wheel = args.wheel.resolve()
    scratch = Path(
        os.environ.get("TMPDIR", Path.home() / "code/tmp/uv-build-slim-poc/tmp")
    )
    environment = {"PATH": "", "LANG": "C.UTF-8", "PYTHONDONTWRITEBYTECODE": "1"}
    if os.name == "nt":
        for name in ["SystemRoot", "WINDIR"]:
            if name in os.environ:
                environment[name] = os.environ[name]
    with tempfile.TemporaryDirectory(dir=str(scratch)) as temporary:
        root = Path(temporary)
        source = root / "source"
        shutil.copytree(args.source, source)
        native, generated = root / "native", root / "python"
        native.mkdir()
        generated.mkdir()
        result = subprocess.run(
            [
                str(args.python),
                "-I",
                "-S",
                "-c",
                CHILD,
                str(wheel),
                str(source),
                str(generated),
            ],
            env=environment,
            cwd=root,
            text=True,
            capture_output=True,
            check=True,
        )
        if result.stderr:
            raise AssertionError(result.stderr)
        actual = json.loads(result.stdout)
        expected = {}
        for hook in actual:
            if hook.startswith("get_requires"):
                expected[hook] = []
                continue
            destination = native / hook
            destination.mkdir()
            command = [str(args.reference), hook.replace("_", "-"), str(destination)]
            if hook in {"build_wheel", "build_editable"}:
                command.append(
                    str(
                        native
                        / "prepare_metadata_for_build_wheel"
                        / expected["prepare_metadata_for_build_wheel"]
                    )
                )
            expected[hook] = subprocess.check_output(
                command, env=environment, cwd=source, text=True
            ).strip()
        assert actual == expected, (actual, expected)
        assert inventory(native) == inventory(generated)
        report = {
            "python": subprocess.check_output(
                [str(args.python), "--version"], text=True
            ).strip(),
            "hooks": actual,
            "artifact_bytes_and_modes_equal": True,
            "empty_path": True,
            "subprocess_and_network_blocked": True,
            "wheel_sha256": hashlib.sha256(wheel.read_bytes()).hexdigest(),
        }
        report_path.write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report))


if __name__ == "__main__":
    main()
