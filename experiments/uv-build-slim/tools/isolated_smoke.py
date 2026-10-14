"""Exercise the distributed backend in an isolated interpreter with no PATH."""

import argparse
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--wheel", required=True, type=Path)
    args = parser.parse_args()
    wheel = args.wheel.resolve()
    root = Path(__file__).resolve().parents[1]
    scratch = Path.home() / "code/tmp/uv-build-slim-poc/tmp"
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=scratch, prefix="isolated-") as temporary:
        project = Path(temporary) / "project"
        shutil.copytree(root / "examples/hello", project)
        output = Path(temporary) / "dist"
        output.mkdir()
        program = """
import importlib, json, os, pathlib, sys
wheel, project, output = sys.argv[1:]
sys.path.insert(0, wheel)
import uv_build_slim
assert uv_build_slim.__file__.startswith(wheel + "/")
os.chdir(project)
filename = uv_build_slim.build_wheel(output)
sys.path.insert(0, str(pathlib.Path(output) / filename))
import hello_slim
print(json.dumps({"wheel": filename, "greeting": hello_slim.greeting(), "path": os.environ["PATH"], "backend": uv_build_slim.__file__}))
"""
        environment = {"PATH": "", "LANG": "C.UTF-8", "PYTHONDONTWRITEBYTECODE": "1"}
        result = subprocess.run(
            [
                sys.executable,
                "-I",
                "-S",
                "-c",
                program,
                str(wheel),
                str(project),
                str(output),
            ],
            cwd=temporary,
            env=environment,
            text=True,
            capture_output=True,
            check=True,
        )
        result = json.loads(result.stdout)
        if result["path"] or result["wheel"] != "hello_slim-1.0.0-py3-none-any.whl":
            raise SystemExit("Isolated smoke check failed")
        print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
