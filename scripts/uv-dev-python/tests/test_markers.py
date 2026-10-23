import json
import subprocess
import sys
from pathlib import Path

import pytest
from uv_dev import Marker, Requirement


def test_marker_constants(environment: dict[str, str]) -> None:
    always = Marker("python_version >= '3' or python_version < '3'")
    never = Marker("python_version >= '3' and python_version < '3'")
    assert always.is_true
    assert always.normalized is None
    assert never.is_false
    assert not never.evaluate(environment)


def test_marker_environment_must_be_complete() -> None:
    marker = Marker("python_version >= '3.12'")
    with pytest.raises(ValueError, match="Invalid marker environment: missing field"):
        marker.evaluate({"python_version": "3.14"})


def test_requirement_evaluates_normalized_extras(environment: dict[str, str]) -> None:
    requirement = Requirement("demo; extra == 'fast-mode'")
    assert requirement.evaluate(environment, extras=["Fast_Mode"])
    assert not requirement.evaluate(environment)


def test_inspect_requirement_with_environment(
    tmp_path: Path, environment: dict[str, str]
) -> None:
    environment_file = tmp_path / "environment.json"
    environment_file.write_text(json.dumps(environment), encoding="utf-8")
    result = subprocess.run(
        [
            sys.executable,
            "-m",
            "uv_dev.inspect",
            "Some_Package[Fast_Mode]>=1; extra == 'fast-mode'",
            "--environment",
            str(environment_file),
            "--extra",
            "Fast_Mode",
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    assert json.loads(result.stdout) == {
        "name": "some-package",
        "extras": ["fast-mode"],
        "specifiers": ">=1",
        "url": None,
        "marker": "extra == 'fast-mode'",
        "applies": True,
    }


def test_inspect_rejects_incomplete_environment(tmp_path: Path) -> None:
    environment = tmp_path / "environment.json"
    environment.write_text("{}", encoding="utf-8")
    result = subprocess.run(
        [
            sys.executable,
            "-m",
            "uv_dev.inspect",
            "demo",
            "--environment",
            str(environment),
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 1
    assert result.stdout == ""
    assert result.stderr.startswith("Invalid marker environment: missing field")
