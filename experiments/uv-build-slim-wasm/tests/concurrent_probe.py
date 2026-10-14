"""Compare concurrent hook execution with native Rust, including free-threading."""

import argparse
import concurrent.futures
import contextlib
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
import uv_build_slim_wasm as backend


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--reference", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--program", type=Path)
    args = parser.parse_args()
    report_path = args.report.resolve()
    scratch = Path(
        os.environ.get("TMPDIR", Path.home() / "code/tmp/uv-build-slim-poc/tmp")
    )
    with contextlib.ExitStack() as cleanup:
        temporary = cleanup.enter_context(tempfile.TemporaryDirectory(dir=str(scratch)))
        cleanup.callback(os.chdir, Path.cwd())
        root = Path(temporary)
        source = root / "source"
        (source / "src/demo").mkdir(parents=True)
        (source / "src/demo/__init__.py").write_bytes(b"VALUE = 1\n")
        (source / "pyproject.toml").write_text(
            '[project]\nname="demo"\nversion="1.0"\nlicense="MIT"\ndependencies=["some-package>=1; python_version < \'3.15\'"]\n[build-system]\nrequires=["uv_build>=0.13,<0.14"]\nbuild-backend="uv_build"\n'
        )
        os.chdir(source)
        native = root / "native"
        native.mkdir()
        filename = subprocess.check_output(
            [str(args.reference), "build-wheel", str(native)], text=True
        ).strip()
        expected = (native / filename).read_bytes()
        backend._PROGRAM = (
            backend.Program(args.program.resolve())
            if args.program
            else backend._new_program()
        )
        program = backend._PROGRAM
        # Exercise unrelated generated functions while build hooks compile their
        # own paths, so cache eviction overlaps with real packaging work.
        selected = {}
        for index, description in enumerate(program.description["functions"]):
            selected.setdefault(description["chunk"], index)

        def load_functions():
            for index in reversed(list(selected.values())):
                blocks = program.function(index)
                assert blocks and all(callable(block) for block in blocks)

        def build(index):
            output = root / ("output-" + str(index))
            output.mkdir()
            actual = backend.build_wheel(str(output))
            assert actual == filename
            data = (output / actual).read_bytes()
            assert data == expected
            return hashlib.sha256(data).hexdigest()

        with concurrent.futures.ThreadPoolExecutor(max_workers=5) as pool:
            loader = pool.submit(load_functions)
            results = list(pool.map(build, range(4)))
            loader.result()
        report = {
            "python": sys.version,
            "gil_enabled": getattr(sys, "_is_gil_enabled", lambda: True)(),
            "concurrent_builds": len(results),
            "equal": True,
            "wheel_sha256": results[0],
            "wasm_sha256": program.description["wasm_sha256"],
        }
        report_path.write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report))


if __name__ == "__main__":
    main()
