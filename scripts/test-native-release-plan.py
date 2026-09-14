# /// script
# requires-python = ">=3.12"
# dependencies = ["chevron-blue==0.3.0"]
# ///
"""Exercise the native release manifest and its publication invariants."""

import copy
import importlib.util
import subprocess
import tempfile
import unittest
from pathlib import Path
from types import ModuleType

ROOT = Path(__file__).resolve().parent.parent


def load(name: str) -> ModuleType:
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / f"{name}.py")
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PLAN = load("plan-release")
INSTALLERS = load("generate-native-installers")


def fixture(root: Path, version: str = "1.2.3") -> None:
    (root / "crates/uv").mkdir(parents=True)
    (root / "crates/uv/Cargo.toml").write_text(f'[package]\nversion = "{version}"\n')
    (root / "Cargo.toml").write_text(
        '[workspace.package]\nrepository = "https://github.com/astral-sh/uv"\n'
    )
    (root / "CHANGELOG.md").write_text(
        f"# Changelog\n\n## {version}\n\nRelease notes.\n\n## 0.1.0\n\nOld notes.\n"
    )
    (root / "release-targets.toml").write_text(
        'targets = ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"]\n[min-glibc-version]\n"*" = "2.17"\n'
    )


class NativeReleasePlan(unittest.TestCase):
    def test_version_and_inventory(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fixture(root)
            implicit = PLAN.release_plan(root=root)
            self.assertEqual(implicit["announcement_tag"], "v1.2.3")
            self.assertTrue(implicit["announcement_tag_is_implicit"])
            explicit = PLAN.release_plan("1.2.3", root=root)
            self.assertFalse(explicit["announcement_tag_is_implicit"])
            self.assertEqual(explicit["announcement_changelog"], "Release notes.")
            self.assertEqual(len(explicit["artifacts"]), 9)
            windows = explicit["artifacts"]["uv-x86_64-pc-windows-msvc.zip"]
            self.assertEqual(
                [a["path"] for a in windows["assets"]], ["uv.exe", "uvw.exe", "uvx.exe"]
            )
            for tag in ("1.2.4", "1.2", "1.2.3;echo bad"):
                with self.assertRaises(ValueError):
                    PLAN.release_plan(tag, root=root)
            with self.assertRaises(ValueError):
                PLAN.release_plan("1.2.3", "example/uv;bad", root=root)
            custom = PLAN.release_plan("1.2.3", "example/uv", root=root)
            self.assertEqual(
                custom["releases"][0]["hosting"]["simple"]["download_url"],
                "https://github.com/example/uv/releases/download/1.2.3",
            )

    def test_prerelease_and_parallel_configuration(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fixture(root, "1.2.3-rc.1+build.2")
            self.assertTrue(PLAN.release_plan(root=root)["announcement_is_prerelease"])
            (root / "dist-workspace.toml").write_text(
                "[dist]\ntargets = []\nmin-glibc-version = {}\n"
            )
            with self.assertRaisesRegex(ValueError, "configuration differs"):
                PLAN.release_plan(root=root)

    def test_publication_contract_comparison(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fixture(root)
            native = PLAN.release_plan(root=root)
            legacy = copy.deepcopy(native)
            legacy["dist_version"] = "0.32.0"
            legacy["announcement_github_body"] = "Different formatting"
            legacy["artifacts"]["uv-installer.sh"]["target_triples"] = [
                "expanded-alias"
            ]
            PLAN.compare_plans(native, legacy)
            legacy["artifacts"]["uv-x86_64-pc-windows-msvc.zip"]["assets"].pop()
            with self.assertRaisesRegex(ValueError, "artifacts"):
                PLAN.compare_plans(native, legacy)

    def test_finalization_verifies_all_published_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            fixture(source)
            subprocess.run(["git", "init", "--quiet", str(source)], check=True)
            subprocess.run(["git", "-C", str(source), "add", "."], check=True)
            subprocess.run(
                [
                    "git",
                    "-C",
                    str(source),
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.test",
                    "commit",
                    "--quiet",
                    "-m",
                    "fixture",
                ],
                check=True,
            )
            plan = PLAN.release_plan("1.2.3", root=source)
            assets = root / "assets"
            assets.mkdir()
            for name, artifact in plan["artifacts"].items():
                if artifact["kind"] == "executable-zip":
                    archive = assets / name
                    archive.write_bytes(name.encode())
                    (assets / artifact["checksum"]).write_text(
                        f"{PLAN.digest(archive)}  {name}\n"
                    )
            generated = INSTALLERS.generate(plan, assets, source)
            final = PLAN.finalize(generated, assets)
            self.assertEqual(len(final["upload_files"]), 9)
            self.assertEqual(
                final["artifacts"]["uv-installer.sh"]["checksums"]["sha256"],
                PLAN.digest(assets / "uv-installer.sh"),
            )
            tampered = copy.deepcopy(generated)
            tampered["artifacts"]["source.tar.gz"]["checksums"]["sha256"] = "0" * 64
            with self.assertRaisesRegex(ValueError, "Manifest checksum differs"):
                PLAN.finalize(tampered, assets)
            (assets / "sha256.sum").write_text("")
            with self.assertRaisesRegex(ValueError, "Unified checksum"):
                PLAN.finalize(generated, assets)


if __name__ == "__main__":
    unittest.main()
