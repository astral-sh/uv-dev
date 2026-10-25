"""A backend that checks the cross-build hook contract without needing a compiler."""

import base64
import hashlib
import importlib.util
import json
import shutil
import sys
import zipfile
from pathlib import Path


def get_requires_for_build_wheel(config_settings=None):
    if (config_settings or {}).get("check-roles"):
        return [
            "cross-tool==1.0.0; python_version < '3.13'",
            "missing-build-dependency; python_version >= '3.13'",
        ]
    return []


def get_requires_for_target_wheel(config_settings=None):
    settings = config_settings or {}
    if settings.get("check-roles"):
        import cross_tool

        assert cross_tool.__version__ == "1.0.0"
        return ["cross-extra==1.0.0; python_version >= '3.14'"]
    if settings.get("target-requirement"):
        return [settings["target-requirement"]]
    return []


def inputs(target, target_wheels, settings):
    assert sys.version_info[:2] == (3, 12)
    assert target_wheels is not None
    assert all(
        Path(wheel).is_absolute() and zipfile.is_zipfile(wheel)
        for wheel in target_wheels
    )
    details = json.loads(Path(target).read_text()) if target else None
    if target:
        assert Path(target).is_absolute()
    result = {
        "build_python": f"{sys.version_info.major}.{sys.version_info.minor}",
        "target_python": details["language"]["version"] if details else None,
        "target_wheels": sorted(Path(wheel).name for wheel in target_wheels),
    }
    if settings.get("check-roles"):
        import cross_library
        import cross_tool

        assert cross_library.__version__ == "1.0.0"
        assert cross_tool.__version__ == "1.0.0"
        assert importlib.util.find_spec("cross_runtime") is None
        assert importlib.util.find_spec("cross_extra") is None
        assert shutil.which("target-runtime") is None
        assert not Path("target-pth-executed").exists()
        header_wheel = next(
            path
            for path in target_wheels
            if Path(path).name.startswith("cross_library-2")
        )
        with zipfile.ZipFile(header_wheel) as wheel:
            assert wheel.read("cross_library/target.h") == b"#define ANSWER 42\n"
        result["build_library"] = cross_library.__version__
    return result


def metadata(settings):
    requires_python = settings.get("requires-python", ">=3.8")
    return f"Metadata-Version: 2.3\nName: cross-build-poc\nVersion: 0.1.0\nRequires-Python: {requires_python}\n"


def prepare_metadata_for_build_wheel(
    metadata_directory, config_settings=None, *, target=None, target_wheels=None
):
    settings = config_settings or {}
    trace = inputs(target, target_wheels, settings)
    directory = Path(metadata_directory) / "cross_build_poc-0.1.0.dist-info"
    directory.mkdir()
    (directory / "METADATA").write_bytes(metadata(settings).encode())
    # This auxiliary file must survive the frontend's metadata handoff, but need not be in the wheel.
    (directory / "target-inputs.json").write_text(
        json.dumps([target, target_wheels, trace])
    )
    if settings.get("mutate-target"):
        with Path(target).open("a") as file:
            file.write("\n")
    return directory.name


def build_wheel(
    wheel_directory,
    config_settings=None,
    metadata_directory=None,
    *,
    target=None,
    target_wheels=None,
):
    settings = config_settings or {}
    trace = inputs(target, target_wheels, settings)
    assert metadata_directory is not None
    prepared = Path(metadata_directory)
    assert prepared.name == "cross_build_poc-0.1.0.dist-info"
    assert json.loads((prepared / "target-inputs.json").read_text()) == [
        target,
        target_wheels,
        trace,
    ]
    Path("protocol.json").write_text(json.dumps(trace))
    tag = settings.get("wheel-tag", "py3-none-any")
    metadata_tag = settings.get("metadata-tag", tag)
    name = f"cross_build_poc-0.1.0-{tag}.whl"
    contents = metadata(settings)
    if settings.get("change-metadata"):
        contents += "Summary: changed after preparation\n"
    entries = {
        "cross_build_poc/__init__.py": "",
        "cross_build_poc-0.1.0.dist-info/METADATA": contents,
        "cross_build_poc-0.1.0.dist-info/WHEEL": f"Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: {metadata_tag}\n",
    }
    record = []
    with zipfile.ZipFile(Path(wheel_directory) / name, "w") as wheel:
        for path, contents in entries.items():
            data = contents.encode()
            digest = (
                base64.urlsafe_b64encode(hashlib.sha256(data).digest())
                .rstrip(b"=")
                .decode()
            )
            record.append(f"{path},sha256={digest},{len(data)}\n")
            wheel.writestr(path, data)
        record.append("cross_build_poc-0.1.0.dist-info/RECORD,,\n")
        wheel.writestr("cross_build_poc-0.1.0.dist-info/RECORD", "".join(record))
    return name
