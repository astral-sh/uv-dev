"""Build a macOS extension using a foreign CPython SDK and the local uv binaries."""

import argparse
import json
import os
import shutil
import subprocess
import sys
import sysconfig
import zipfile
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--target",
        type=Path,
        required=True,
        help="Path to the SDK's build-details.json",
    )
    parser.add_argument(
        "--work-dir", type=Path, required=True, help="A new directory for the PoC"
    )
    parser.add_argument(
        "--verify",
        action="store_true",
        help="Run the resulting extension with the target Python after building",
    )
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error(
            "This compiler example currently requires macOS and Xcode command line tools"
        )
    root = Path(__file__).resolve().parents[2]
    uv = root / "target/debug/uv"
    uv_build = root / "target/debug/uv-build"
    if not uv.is_file() or not uv_build.is_file():
        parser.error("First run `cargo build -p uv -p uv-build`")
    target = args.target.absolute()
    details = json.loads(target.read_text())
    if details["abi"]["flags"]:
        parser.error("This compiler example requires a standard CPython ABI")
    arch = details["platform"].rsplit("-", 1)[1]
    if arch not in {"x86_64", "arm64"}:
        parser.error("The target must be a macOS x86_64 or arm64 SDK")
    language = details["language"]["version"].replace(".", "")
    target_tag = f"cp{language}-cp{language}-macosx_13_0_{arch}"
    work = args.work_dir.resolve()
    work.mkdir(parents=True)
    wheels = work / "wheels"
    wheels.mkdir()
    project = work / "project"
    project.mkdir()
    (work / "tmp").mkdir()
    for name in ("pyproject.toml", "backend.py", "answer.c"):
        shutil.copyfile(Path(__file__).with_name(name), project / name)
    package = project / "src/cross_poc"
    package.mkdir(parents=True)
    (package / "__init__.py").write_text("from ._answer import answer\n")

    # Import the wheel writer without installing the development backend in the driver environment.
    sys.path.insert(0, str(root / "crates/uv-build/python"))
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from backend import write_wheel

    def make_wheel(
        name, version, tag, files, requires=(), requires_python=">=3.8", executable=()
    ):
        normalized = name.replace("-", "_")
        dist_info = f"{normalized}-{version}.dist-info"
        metadata = f"Metadata-Version: 2.3\nName: {name}\nVersion: {version}\nRequires-Python: {requires_python}\n"
        metadata += "".join(
            f"Requires-Dist: {requirement}\n" for requirement in requires
        )
        entries = {
            f"{dist_info}/METADATA": metadata.encode(),
            f"{dist_info}/WHEEL": f"Wheel-Version: 1.0\nRoot-Is-Purelib: false\nTag: {tag}\n".encode(),
            **files,
        }
        write_wheel(
            wheels / f"{normalized}-{version}-{tag}.whl", entries, executable=executable
        )

    version = subprocess.check_output([uv, "--version"], text=True).split()[1]
    native_platform = sysconfig.get_platform().replace("-", "_").replace(".", "_")
    script = f"uv_build-{version}.data/scripts/uv-build"
    shim = root / "crates/uv-build/python/uv_build"
    make_wheel(
        "uv-build",
        version,
        f"py3-none-{native_platform}",
        {
            "uv_build/__init__.py": (shim / "__init__.py").read_bytes(),
            "uv_build/__main__.py": (shim / "__main__.py").read_bytes(),
            "uv_build/py.typed": b"",
            script: uv_build.read_bytes(),
        },
        executable=(script,),
    )
    make_wheel(
        "poc-sdk",
        "1.0.0",
        "py3-none-any",
        {"poc_sdk/__init__.py": b'__version__ = "1.0.0"\n'},
    )
    make_wheel(
        "poc-codegen",
        "1.0.0",
        "py3-none-any",
        {"poc_codegen/__init__.py": b"VALUE = 2\n"},
    )
    make_wheel(
        "poc-sdk",
        "2.0.0",
        target_tag,
        {
            "poc_sdk/__init__.py": b'raise RuntimeError("Target code was imported during the build")\n',
            "poc_sdk/target.h": b"#define TARGET_VALUE 21\n",
        },
        requires=("poc-runtime==1.0.0",),
        requires_python=">=3.14",
    )
    make_wheel(
        "poc-runtime",
        "1.0.0",
        target_tag,
        {
            "poc_runtime/__init__.py": b'raise RuntimeError("Target code was imported during the build")\n',
            "poc_runtime.pth": b'import sys; sys.exit("Target .pth was executed during the build")\n',
        },
        requires_python=">=3.14",
    )

    triple = "x86_64-apple-darwin" if arch == "x86_64" else "aarch64-apple-darwin"
    environment = {
        **os.environ,
        "TMPDIR": str(work / "tmp"),
        "MACOSX_DEPLOYMENT_TARGET": "13.0",
    }
    subprocess.run(
        [
            uv,
            "build",
            project,
            "--no-config",
            "--wheel",
            "--preview-features",
            "cross-builds",
            "--python",
            sys.executable,
            "--target",
            target,
            "--python-platform",
            triple,
            "--no-index",
            "--find-links",
            wheels,
            "--cache-dir",
            work / "cache",
            "-C",
            f"arch={arch}",
            "-C",
            f"wheel-tag={target_tag}",
        ],
        env=environment,
        check=True,
    )
    report = json.loads((project / "report.json").read_text())
    if args.verify:
        # Target execution is a separate, optional verification step after the build has finished.
        installed = work / "installed"
        with zipfile.ZipFile(next((project / "dist").glob("*.whl"))) as wheel:
            wheel.extractall(installed)
        prefix = target.parent / details["base_prefix"]
        interpreter = prefix / details["base_interpreter"]
        check = f"import sys; sys.path.insert(0, {str(installed)!r}); import cross_poc; assert cross_poc.answer() == 42; print(cross_poc.answer())"
        result = subprocess.check_output(
            [
                uv,
                "run",
                "--no-config",
                "--no-project",
                "--python",
                interpreter,
                "--cache-dir",
                work / "cache",
                "python",
                "-I",
                "-c",
                check,
            ],
            env=environment,
            text=True,
        )
        report["target_result"] = int(result.strip())
    (work / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
