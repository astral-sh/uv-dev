import re
import tempfile
import unittest
from pathlib import Path

from check_release_versions import ROOT, VERSION_GROUPS, check_release_versions


def write_versions(root: Path, python_version: str, cargo_version: str) -> None:
    for section, manifests in VERSION_GROUPS:
        version = python_version if section == "project" else cargo_version
        for manifest in manifests:
            path = root / manifest
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f'[{section}]\nversion = "{version}"\n', encoding="utf-8")


class ReleaseVersionTests(unittest.TestCase):
    def test_repository_versions_match(self) -> None:
        check_release_versions(ROOT)

    def test_matching_versions(self) -> None:
        for python_version, cargo_version in (
            ("0.12.12", "0.12.12"),
            ("0.13.0rc1", "0.13.0-rc.1"),
        ):
            with (
                self.subTest(
                    python_version=python_version, cargo_version=cargo_version
                ),
                tempfile.TemporaryDirectory() as temp,
            ):
                root = Path(temp)
                write_versions(root, python_version, cargo_version)
                self.assertEqual(check_release_versions(root), python_version)

    def test_each_release_manifest_must_match(self) -> None:
        for section, (reference, *manifests) in VERSION_GROUPS:
            for manifest in manifests:
                with (
                    self.subTest(manifest=manifest),
                    tempfile.TemporaryDirectory() as temp,
                ):
                    root = Path(temp)
                    write_versions(root, "0.13.0rc1", "0.13.0-rc.1")
                    (root / manifest).write_text(
                        f'[{section}]\nversion = "0.12.11"\n', encoding="utf-8"
                    )
                    expected = "0.13.0rc1" if section == "project" else "0.13.0-rc.1"
                    with self.assertRaisesRegex(
                        ValueError,
                        re.escape(
                            f"Release versions do not match {reference} ({expected}):\n"
                            f"  {manifest}: 0.12.11"
                        ),
                    ):
                        check_release_versions(root)


if __name__ == "__main__":
    unittest.main()
