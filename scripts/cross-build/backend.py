"""Example cross-compiling backend layered on uv-build's pure Python packaging."""

import base64
import hashlib
import importlib.util
import json
import platform
import subprocess
import tempfile
import zipfile
from pathlib import Path

import uv_build


def write_wheel(path, entries, *, executable=()):
    """Write a wheel and its RECORD from a mapping of archive paths to bytes."""
    record = next(
        name.rsplit("/", 1)[0] + "/RECORD"
        for name in entries
        if name.endswith(".dist-info/METADATA")
    )
    lines = []
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as wheel:
        for name, contents in entries.items():
            if name.endswith("/RECORD"):
                continue
            digest = (
                base64.urlsafe_b64encode(hashlib.sha256(contents).digest())
                .rstrip(b"=")
                .decode()
            )
            lines.append(f"{name},sha256={digest},{len(contents)}\n")
            info = zipfile.ZipInfo(name)
            info.external_attr = (0o100755 if name in executable else 0o100644) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            wheel.writestr(info, contents)
        lines.append(f"{record},,\n")
        wheel.writestr(record, "".join(lines))


def get_requires_for_build_wheel(config_settings=None):
    return ["poc-codegen==1.0.0"]


def get_requires_for_target_wheel(config_settings=None):
    import poc_codegen
    import poc_sdk

    assert poc_sdk.__version__ == "1.0.0"
    assert poc_codegen.VALUE == 2
    return ["poc-runtime==1.0.0; python_version >= '3.14'"]


def inputs(target, target_wheels):
    assert target is not None and Path(target).is_absolute()
    assert target_wheels is not None
    assert importlib.util.find_spec("poc_runtime") is None
    return {
        "target": target,
        "description_hash": hashlib.sha256(Path(target).read_bytes()).hexdigest(),
        "wheels": {
            path: hashlib.sha256(Path(path).read_bytes()).hexdigest()
            for path in target_wheels
        },
    }


def wheel_metadata(tag):
    return f"Wheel-Version: 1.0\nGenerator: uv-cross-poc\nRoot-Is-Purelib: false\nTag: {tag}\n".encode()


def prepare_metadata_for_build_wheel(
    metadata_directory, config_settings=None, *, target=None, target_wheels=None
):
    directory = uv_build.prepare_metadata_for_build_wheel(
        metadata_directory, target=target, target_wheels=target_wheels
    )
    prepared = Path(metadata_directory) / directory
    (prepared / "WHEEL").write_bytes(wheel_metadata(config_settings["wheel-tag"]))
    (prepared / "cross-inputs.json").write_text(
        json.dumps(inputs(target, target_wheels))
    )
    return directory


def build_wheel(
    wheel_directory,
    config_settings=None,
    metadata_directory=None,
    *,
    target=None,
    target_wheels=None,
):
    import poc_codegen
    import poc_sdk

    assert poc_sdk.__version__ == "1.0.0"
    prepared = Path(metadata_directory)
    assert json.loads((prepared / "cross-inputs.json").read_text()) == inputs(
        target, target_wheels
    )
    details = json.loads(Path(target).read_text())
    prefix = Path(details["base_prefix"])
    if not prefix.is_absolute():
        prefix = Path(target).parent / prefix
    headers = prefix / details["c_api"]["headers"]
    extension = "_answer" + details["abi"]["extension_suffix"]
    tag = config_settings["wheel-tag"]

    with tempfile.TemporaryDirectory(dir=wheel_directory) as directory:
        staging = Path(directory)
        header_wheel = next(
            path
            for path in target_wheels
            if Path(path).name.startswith("poc_sdk-2.0.0-")
        )
        with zipfile.ZipFile(header_wheel) as wheel:
            (staging / "target.h").write_bytes(wheel.read("poc_sdk/target.h"))
        (staging / "generated.h").write_text(
            f"#define GENERATED_VALUE {poc_codegen.VALUE}\n"
        )
        subprocess.run(
            [
                "clang",
                "-arch",
                config_settings["arch"],
                "-mmacosx-version-min=13.0",
                "-bundle",
                "-undefined",
                "dynamic_lookup",
                "-I",
                str(headers),
                "-I",
                str(staging),
                "answer.c",
                "-o",
                str(staging / extension),
            ],
            check=True,
        )
        # uv-build packages the Python files; this wrapper adds the compiled extension and tags.
        pure_name = uv_build.build_wheel(
            wheel_directory, target=target, target_wheels=target_wheels
        )
        with zipfile.ZipFile(Path(wheel_directory) / pure_name) as wheel:
            entries = {name: wheel.read(name) for name in wheel.namelist()}
        entries[f"cross_poc/{extension}"] = (staging / extension).read_bytes()
        entries["cross_poc-0.1.0.dist-info/METADATA"] = (
            prepared / "METADATA"
        ).read_bytes()
        entries["cross_poc-0.1.0.dist-info/WHEEL"] = (prepared / "WHEEL").read_bytes()
        report = {
            "build_python": platform.python_version(),
            "build_machine": platform.machine(),
            "build_sdk_version": poc_sdk.__version__,
            "target_python": details["language"]["version"],
            "target_platform": details["platform"],
            "target_wheels": sorted(Path(path).name for path in target_wheels),
            "extension_architecture": subprocess.check_output(
                ["lipo", "-archs", str(staging / extension)], text=True
            ).strip(),
            "wheel_tag": tag,
        }
        Path("report.json").write_text(json.dumps(report, indent=2) + "\n")
        name = f"cross_poc-0.1.0-{tag}.whl"
        write_wheel(Path(wheel_directory) / name, entries)
        (Path(wheel_directory) / pure_name).unlink()
    return name
