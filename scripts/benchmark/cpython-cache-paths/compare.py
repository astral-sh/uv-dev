"""Compare shared directory imports through `pyvenv.cfg` and a plain `.pth` file.

Apply the adjacent patch to CPython v3.12.13 (`patch -p1 < cache-path.patch`)
before running `./configure --without-ensurepip` and `make`. From a POSIX host:

    python3 compare.py /path/to/patched/python /path/to/site-packages --module numpy --scratch .

Pass additional directories in import order to try one directory per wheel.
Setup is untimed. Each sample starts a new interpreter and imports the module.
"""

import argparse
import json
import subprocess
from pathlib import Path
from tempfile import TemporaryDirectory
from time import perf_counter_ns


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("python", type=Path)
    parser.add_argument("roots", type=Path, nargs="+")
    parser.add_argument("--module", required=True)
    parser.add_argument("--scratch", type=Path, required=True)
    parser.add_argument("--rounds", type=int, default=24)
    args = parser.parse_args()
    roots = [p.resolve(strict=True) for p in args.roots]
    if args.rounds < 1 or len(set(roots)) != len(roots):
        parser.error("use a positive round count and distinct directories")
    if any(
        not p.is_dir() or len(str(p).splitlines()) != 1 or str(p).rstrip() != str(p)
        for p in roots
    ):
        parser.error(
            "each path must be a directory that fits on one configuration line"
        )
    with TemporaryDirectory(dir=args.scratch.resolve()) as temp:
        interpreters = {}
        for mode in ("pth", "cfg"):
            env = Path(temp) / mode
            subprocess.run(
                [args.python.resolve(), "-Im", "venv", "--without-pip", env], check=True
            )
            python = env / "bin/python"
            if mode == "pth":
                site = subprocess.check_output(
                    [
                        python,
                        "-IBc",
                        "import sysconfig; print(sysconfig.get_path('purelib'))",
                    ],
                    text=True,
                ).strip()
                (Path(site) / "shared.pth").write_text("".join(f"{p}\n" for p in roots))
            else:
                with (env / "pyvenv.cfg").open("a", encoding="utf-8") as f:
                    for p in roots:
                        f.write(f"uv-cache-path = {p}\n")
            check = subprocess.check_output(
                [
                    python,
                    "-IBc",
                    (
                        "import importlib, json, site, sys; "
                        "m = importlib.import_module(sys.argv[1]); "
                        "print(json.dumps([site.__spec__.origin, sys.path, m.__file__]))"
                    ),
                    args.module,
                ],
                text=True,
            )
            origin, search, source = json.loads(check)
            selected = [Path(p) for p in search if Path(p) in roots]
            if (
                origin != "frozen"
                or selected != roots
                or not source
                or not any(Path(source).is_relative_to(p) for p in roots)
            ):
                raise ValueError(
                    f"{mode} did not load the requested module from the supplied directories"
                )
            interpreters[mode] = python
        samples = []
        for index in range(args.rounds):
            for mode in ("pth", "cfg") if index % 2 == 0 else ("cfg", "pth"):
                start = perf_counter_ns()
                subprocess.run(
                    [
                        interpreters[mode],
                        "-IBc",
                        "import importlib, sys; importlib.import_module(sys.argv[1])",
                        args.module,
                    ],
                    check=True,
                    stdout=subprocess.DEVNULL,
                )
                samples.append(
                    {
                        "round": index + 1,
                        "mode": mode,
                        "ms": (perf_counter_ns() - start) / 1e6,
                    }
                )
        print(json.dumps(samples, indent=2))


if __name__ == "__main__":
    main()
