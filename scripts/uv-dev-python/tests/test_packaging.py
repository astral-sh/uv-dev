import importlib.util
import json
from pathlib import Path

import pytest
from uv_dev import Requirement, Version, VersionSpecifiers, normalize_package_name


def test_versions_compare_and_hash_by_value() -> None:
    assert Version("3.15.0rc2") < Version("3.15.0rc10") < Version("3.15.0")
    assert Version("1.0") == Version("1")
    assert len({Version("1.0"), Version("1")}) == 1
    assert Version("3.15.0rc2").release == (3, 15, 0)
    assert Version("3.15.0rc2").is_prerelease
    assert str(Version("v1.0RC1")) == "1.0rc1"
    with pytest.raises(ValueError):
        Version("not a version")


def test_specifiers_use_rust_comparisons() -> None:
    specifiers = VersionSpecifiers(">=1.0,!=1.2,<2")
    assert Version("1.1rc1") in specifiers
    assert Version("1.2") not in specifiers
    assert Version("2.0") not in specifiers


def test_requirements_expose_normalized_fields() -> None:
    requirement = Requirement("My_Package[Fast_Mode]>=1.2; python_version < '3.14'")
    assert requirement.name == "my-package"
    assert requirement.extras == ("fast-mode",)
    assert requirement.specifiers is not None
    assert Version("1.3") in requirement.specifiers
    assert requirement.url is None
    assert requirement.marker == "python_full_version < '3.14'"
    assert normalize_package_name("My.Package_Name") == "my-package-name"
    with pytest.raises(ValueError):
        normalize_package_name("not a name")


def test_url_requirement() -> None:
    requirement = Requirement("Example @ https://example.com/example.whl")
    assert requirement.url == "https://example.com/example.whl"
    assert requirement.specifiers is None
    assert requirement.marker is None


def test_version_constants_prefer_stable_releases(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = Path(__file__).resolve().parents[3]
    spec = importlib.util.spec_from_file_location(
        "sync_python_version_constants",
        root / "scripts/sync-python-version-constants.py",
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    monkeypatch.setattr(module, "ROOT", tmp_path)

    metadata_dir = tmp_path / "crates/uv-python-managed"
    metadata_dir.mkdir(parents=True)
    (metadata_dir / "download-metadata.json").write_text(
        json.dumps(
            {
                "stable": {"name": "cpython", "major": 3, "minor": 14, "patch": 0},
                "next-patch": {
                    "name": "cpython",
                    "major": 3,
                    "minor": 14,
                    "patch": 1,
                    "prerelease": "rc1",
                },
                "rc2": {
                    "name": "cpython",
                    "major": 3,
                    "minor": 15,
                    "patch": 0,
                    "prerelease": "rc2",
                },
                "rc10": {
                    "name": "cpython",
                    "major": 3,
                    "minor": 15,
                    "patch": 0,
                    "prerelease": "rc10",
                },
            }
        ),
        encoding="utf-8",
    )
    lib_path = tmp_path / "crates/uv-test/src/lib.rs"
    lib_path.parent.mkdir(parents=True)
    lib_path.write_text(
        'const LATEST_PYTHON_3_14: &str = "3.14.0rc1";\n'
        'const LATEST_PYTHON_3_15: &str = "3.15.0rc2";\n',
        encoding="utf-8",
    )
    module.main()
    assert lib_path.read_text(encoding="utf-8") == (
        'const LATEST_PYTHON_3_14: &str = "3.14.0";\n'
        'const LATEST_PYTHON_3_15: &str = "3.15.0rc10";\n'
    )
