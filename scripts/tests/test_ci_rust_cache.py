# /// script
# requires-python = ">=3.12"
# dependencies = []
#
# [tool.uv]
# no-build = true
# exclude-newer = "P7D"
# ///
"""Exercise the Linux Rust-cache contract without a cache service or Cargo build."""

from __future__ import annotations

import copy
import hashlib
import importlib.util
import itertools
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
ACTION = ROOT / ".github/actions/uv-rust-cache"
SPEC = importlib.util.spec_from_file_location("uv_ci_rust_cache", ACTION / "cache.py")
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("Could not load the Rust-cache helper")
cache = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cache)


def executable(path: Path, contents: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(contents)
    path.chmod(0o755)


def fake_executable(name: str, checksum: str) -> dict[str, Any]:
    return {
        "invocation": f"/fixture/bin/{name}",
        "resolved": f"/fixture/native/{name}",
        "sha256": checksum * 64,
        "size": 1,
    }


def fake_tools() -> dict[str, Any]:
    version = {
        "release": "1.98.1",
        "host": cache.HOST,
        "commit-hash": "a" * 40,
    }
    tools: dict[str, Any] = {
        "rustup": fake_executable("rustup", "1"),
        "active": f"1.98.1-{cache.HOST}",
        "cargo": {
            "invocation": fake_executable("cargo", "1"),
            "native": fake_executable("cargo", "2"),
            "version": version,
        },
        "rustc": {
            "invocation": fake_executable("rustc", "1"),
            "native": fake_executable("rustc", "3"),
            "version": version,
        },
        "nextest": {
            "executable": fake_executable("cargo-nextest", "4"),
            "version": {**version, "release": cache.NEXTEST_VERSION},
        },
        "native": {},
    }
    for name in ("cc", "c++", "ar", "ld", "mold", "pkg-config"):
        tools["native"][name] = {
            "executable": fake_executable(name, "5"),
            "version": f"{name} fixture 1.0",
        }
    return tools


def action_steps(path: Path) -> dict[str, list[str]]:
    lines = path.read_text().splitlines()
    start = lines.index("runs:")
    if lines[start + 1 : start + 3] != ["  using: composite", "  steps:"]:
        raise ValueError("Unsupported composite-action shape")
    steps: dict[str, list[str]] = {}
    current: list[str] | None = None
    for line in lines[start + 3 :]:
        if line.startswith("    - id: "):
            name = line.removeprefix("    - id: ")
            if not name or name in steps:
                raise ValueError("Duplicate or empty action step")
            current = []
            steps[name] = current
        elif line.startswith("    - "):
            raise ValueError("Action step has no expected id")
        elif current is not None:
            current.append(line)
        elif line.strip():
            raise ValueError("Unexpected action content")
    return steps


def action_field(lines: list[str], name: str) -> str:
    prefix = f"      {name}: "
    values = [line.removeprefix(prefix) for line in lines if line.startswith(prefix)]
    if len(values) != 1:
        raise ValueError(f"Expected exactly one action field: {name}")
    return values[0].partition(" # ")[0]


def action_mapping(lines: list[str], name: str) -> dict[str, str]:
    start = lines.index(f"      {name}:") + 1
    result = {}
    for line in lines[start:]:
        if not line.strip():
            continue
        if not line.startswith("        "):
            break
        key, separator, value = line[8:].partition(": ")
        if not separator or key in result or key.startswith(" "):
            raise ValueError("Unsupported action mapping")
        result[key] = value
    return result


class CacheContract(unittest.TestCase):
    def setUp(self) -> None:
        scratch = Path.home() / "code/tmp"
        scratch.mkdir(parents=True, exist_ok=True)
        self.directory = tempfile.TemporaryDirectory(
            prefix="uv-rust-cache-test-", dir=scratch
        )
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.workspace = self.root / "workspace"
        self.source = self.workspace / "source"
        self.home = self.root / "home"
        self.source.mkdir(parents=True)
        (self.home / ".cargo").mkdir(parents=True)
        self.environment = {
            "PATH": os.environ["PATH"],
            "HOME": str(self.home),
            "GITHUB_WORKSPACE": str(self.workspace),
            "CARGO_TARGET_DIR": str(self.workspace / "target"),
            **cache.REQUIRED_ENVIRONMENT,
        }
        self.git("init", "--quiet")
        self.git("remote", "add", "origin", "https://github.com/astral-sh/uv")
        self.write(".gitignore", "/target\n")
        self.write(
            "Cargo.toml", '[workspace]\nmembers = ["crates/example"]\nresolver = "2"\n'
        )
        self.write(
            "Cargo.lock",
            'version = 4\n[[package]]\nname = "example"\nversion = "0.1.0"\n',
        )
        self.write("rust-toolchain.toml", '[toolchain]\nchannel = "1.98.1"\n')
        self.write(".cargo/config.toml", '[alias]\ncheck-fixture = "check"\n')
        self.write(
            "crates/example/Cargo.toml",
            '[package]\nname = "example"\nversion = "0.1.0"\n',
        )
        self.write("crates/example/src/lib.rs", "pub fn example() {}\n")
        self.commit()
        self.platform = {
            "system": "Linux",
            "architecture": "x86_64",
            "distribution": "ubuntu",
            "distribution_version": "24.04",
            "libc": ["glibc", "2.39"],
            "image": {},
        }
        self.tools = fake_tools()
        self.implementation = {
            "commit": "b" * 40,
            "tree": "c" * 40,
            "script_sha256": "d" * 64,
            "cache_action": cache.CACHE_ACTION,
        }

    def git(self, *arguments: str, source: Path | None = None) -> str:
        return subprocess.check_output(
            ["git", *arguments],
            cwd=source or self.source,
            env=self.environment,
            text=True,
            stderr=subprocess.PIPE,
        ).strip()

    def write(self, name: str, value: str) -> None:
        path = self.source / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(value)

    def commit(self) -> str:
        self.git("add", ".")
        self.git(
            "-c",
            "user.name=zaniebot",
            "-c",
            "user.email=242828183+zaniebot@users.noreply.github.com",
            "commit",
            "--quiet",
            "-m",
            "Update cache fixture",
        )
        return self.git("rev-parse", "HEAD")

    def snapshot(
        self,
        *,
        source: Path | None = None,
        commit: str | None = None,
        repository: str = "astral-sh/uv",
        save: bool = True,
        environment: dict[str, str] | None = None,
    ) -> dict[str, Any]:
        source = source or self.source
        with (
            mock.patch.object(
                cache, "platform_identity", return_value=copy.deepcopy(self.platform)
            ),
            mock.patch.object(
                cache, "tools_identity", return_value=copy.deepcopy(self.tools)
            ),
            mock.patch.object(
                cache, "implementation_identity", return_value=self.implementation
            ),
            mock.patch.object(
                cache,
                "cargo_config_paths",
                side_effect=lambda root, _home: [
                    ("ancestor-0/config.toml", root / ".cargo/config.toml")
                ],
            ),
        ):
            return cache.make_identity(
                self.workspace,
                source,
                repository,
                commit or self.git("rev-parse", "HEAD", source=source),
                save,
                environment or self.environment,
            )

    def observation(
        self, manifest: dict[str, Any], kind: str, state: str
    ) -> dict[str, Any]:
        keys = manifest["keys"]
        if state == "exact":
            matched, hit = keys[kind], "true"
        elif state == "fallback":
            matched = keys[kind + "_restore"] + ("e" * (40 if kind == "target" else 64))
            hit = "false"
        else:
            matched, hit = "", ""
        return cache.restore_observation(keys, kind, keys[kind], matched, hit)

    def test_source_only_change_gets_a_new_exact_target_key(self) -> None:
        before = self.snapshot()
        self.write(
            "crates/example/src/lib.rs", "pub fn example() { let _changed = 1; }\n"
        )
        self.commit()
        after = self.snapshot()
        self.assertEqual(before["keys"]["downloads"], after["keys"]["downloads"])
        self.assertEqual(
            before["keys"]["target_restore"], after["keys"]["target_restore"]
        )
        self.assertNotEqual(before["keys"]["target"], after["keys"]["target"])

    def test_relocation_is_observed_without_changing_compatible_keys(self) -> None:
        before = self.snapshot()
        relocated = self.workspace / "relocated"
        self.git("clone", "--quiet", "--no-hardlinks", str(self.source), str(relocated))
        self.git(
            "remote",
            "set-url",
            "origin",
            "https://github.com/astral-sh/uv.git",
            source=relocated,
        )
        after = self.snapshot(source=relocated)
        self.assertEqual(before["keys"], after["keys"])
        self.assertNotEqual(
            before["identity"]["source"]["path"], after["identity"]["source"]["path"]
        )
        self.assertEqual(
            after["identity"]["paths"]["target_paths"], list(cache.TARGET_PATHS)
        )

    def test_dependency_change_keeps_only_download_fallback(self) -> None:
        before = self.snapshot()
        self.write(
            "Cargo.lock",
            'version = 4\n[[package]]\nname = "example"\nversion = "0.2.0"\n',
        )
        self.commit()
        after = self.snapshot()
        self.assertEqual(
            before["keys"]["downloads_restore"], after["keys"]["downloads_restore"]
        )
        self.assertNotEqual(before["keys"]["downloads"], after["keys"]["downloads"])
        self.assertNotEqual(
            before["keys"]["target_restore"], after["keys"]["target_restore"]
        )

    def test_configuration_toolchain_flags_and_binary_changes_invalidate_target(
        self,
    ) -> None:
        before = self.snapshot()["keys"]["target_restore"]
        for name, value in (
            (".cargo/config.toml", '[build]\nrustflags = ["--cfg", "fixture"]\n'),
            ("rust-toolchain.toml", '[toolchain]\nchannel = "1.98.2"\n'),
            (
                "crates/example/Cargo.toml",
                '[package]\nname = "example"\nversion = "0.2.0"\n',
            ),
        ):
            with self.subTest(path=name):
                original = (self.source / name).read_text()
                self.write(name, value)
                self.commit()
                self.assertNotEqual(before, self.snapshot()["keys"]["target_restore"])
                self.write(name, original)
                self.commit()
        self.assertNotEqual(
            before,
            self.snapshot(
                environment={**self.environment, "RUSTFLAGS": "--cfg fixture"}
            )["keys"]["target_restore"],
        )
        self.tools["rustc"]["native"]["sha256"] = "f" * 64
        self.assertNotEqual(before, self.snapshot()["keys"]["target_restore"])

    def test_dirty_or_unexpected_source_fails_closed(self) -> None:
        original = self.git("rev-parse", "HEAD")
        self.write("untracked", "unexpected")
        with self.assertRaisesRegex(cache.CacheError, "dirty"):
            self.snapshot()
        (self.source / "untracked").unlink()
        self.write("crates/example/src/lib.rs", "changed\n")
        with self.assertRaisesRegex(cache.CacheError, "dirty"):
            self.snapshot()
        self.commit()
        with self.assertRaisesRegex(cache.CacheError, "requested commit"):
            self.snapshot(commit=original)
        with self.assertRaisesRegex(cache.CacheError, "requested repository"):
            self.snapshot(repository="astral-sh/uv-dev")

    def test_index_flags_cannot_hide_changed_source(self) -> None:
        path = "crates/example/src/lib.rs"
        original = (self.source / path).read_text()
        for flag in ("assume-unchanged", "skip-worktree"):
            with self.subTest(flag=flag):
                self.git("update-index", "--" + flag, path)
                self.write(path, "hidden source change\n")
                self.assertFalse(self.git("status", "--porcelain"))
                with self.assertRaisesRegex(cache.CacheError, "index flags"):
                    self.snapshot()
                self.git("update-index", "--no-" + flag, path)
                self.write(path, original)

    def test_tracked_configuration_symlink_is_rejected(self) -> None:
        target = self.source / ".cargo/config.toml"
        target.unlink()
        target.symlink_to("../../Cargo.toml")
        self.commit()
        with self.assertRaisesRegex(
            cache.CacheError, "Unsupported source configuration"
        ):
            self.snapshot()

    def test_repository_parser_rejects_credentials_and_queries(self) -> None:
        self.assertEqual(
            cache.repository_name("git@github.com:astral-sh/uv.git"), "astral-sh/uv"
        )
        for remote in (
            "https://user:secret@github.com/astral-sh/uv",
            "https://github.com/astral-sh/uv?token=secret",
            "ssh://other/astral-sh/uv",
            "https://example.com/astral-sh/uv",
        ):
            with self.subTest(remote=remote), self.assertRaises(cache.CacheError):
                cache.repository_name(remote)

    def test_cache_layout_rejects_wrong_paths_and_symlink_roots(self) -> None:
        with self.assertRaisesRegex(cache.CacheError, "target directory"):
            self.snapshot(
                environment={
                    **self.environment,
                    "CARGO_TARGET_DIR": str(self.source / "target"),
                }
            )
        with self.assertRaisesRegex(cache.CacheError, "default Cargo home"):
            self.snapshot(
                environment={
                    **self.environment,
                    "CARGO_HOME": str(self.root / "other-cargo"),
                }
            )
        for name, value in (
            ("CARGO_BUILD_TARGET", cache.HOST),
            ("CARGO_BUILD_TARGET_DIR", str(self.workspace / "target")),
            ("CARGO_BUILD_BUILD_DIR", str(self.workspace / "target")),
        ):
            with (
                self.subTest(name=name),
                self.assertRaisesRegex(cache.CacheError, "explicit Cargo target"),
            ):
                self.snapshot(environment={**self.environment, name: value})
        target = self.workspace / "target"
        target.symlink_to(self.home, target_is_directory=True)
        with self.assertRaisesRegex(cache.CacheError, "symbolic link"):
            self.snapshot()

    def test_environment_values_are_hashed_without_credentials(self) -> None:
        secret = "cache-secret-canary-should-not-be-recorded"
        before = self.snapshot()
        after = self.snapshot(
            environment={**self.environment, "CARGO_REGISTRIES_PRIVATE_TOKEN": secret}
        )
        self.assertEqual(before["keys"], after["keys"])
        self.assertNotIn(secret, json.dumps(after))
        index = self.snapshot(
            environment={
                **self.environment,
                "CARGO_REGISTRIES_PRIVATE_INDEX": "https://user:"
                + secret
                + "@example.invalid/index",
            }
        )
        self.assertNotEqual(
            before["keys"]["downloads_restore"], index["keys"]["downloads_restore"]
        )
        self.assertNotIn(secret, json.dumps(index))
        for name in (
            "RUSTC_WRAPPER",
            "CC",
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER",
        ):
            with (
                self.subTest(name=name),
                self.assertRaisesRegex(cache.CacheError, "tool override"),
            ):
                self.snapshot(environment={**self.environment, name: "unobserved-tool"})

    def test_cargo_configuration_rejects_unmodeled_tool_selection(self) -> None:
        path = self.root / "config.toml"
        with mock.patch.object(
            cache, "cargo_config_paths", return_value=[("cargo-home/config.toml", path)]
        ):
            for value in (
                '[build]\nrustc-wrapper = "other"\n',
                f'[build]\ntarget = "{cache.HOST}"\n',
                '[build]\nbuild-dir = "target"\n',
                '[target.x86_64-unknown-linux-gnu]\nlinker = "other"\n',
                '[env]\nPATH = "other"\n',
                '[env]\nCARGO_TARGET_DIR = "other"\n',
            ):
                path.write_text(value)
                with self.subTest(value=value), self.assertRaises(cache.CacheError):
                    cache.cargo_configuration(self.source, self.home / ".cargo")
            path.write_text('[build]\nrustflags = ["--cfg", "fixture"]\n')
            result = cache.cargo_configuration(self.source, self.home / ".cargo")
            self.assertEqual(
                result[0]["sha256"], hashlib.sha256(path.read_bytes()).hexdigest()
            )

    def test_manifest_digest_size_duplicates_and_no_clobber(self) -> None:
        path = self.root / "manifest.json"
        checksum = cache.write_json(path, {"value": 1})
        self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
        self.assertEqual(cache.read_json(path, checksum), {"value": 1})
        with self.assertRaises(FileExistsError):
            cache.write_json(path, {"value": 2})
        with self.assertRaisesRegex(cache.CacheError, "digest"):
            cache.read_json(path, "0" * 64)
        duplicate = b'{"value":1,"value":2}\n'
        path.write_bytes(duplicate)
        with self.assertRaisesRegex(cache.CacheError, "Duplicate"):
            cache.read_json(path, hashlib.sha256(duplicate).hexdigest())
        nonfinite = b'{"value":NaN}\n'
        path.write_bytes(nonfinite)
        with self.assertRaisesRegex(cache.CacheError, "Non-finite"):
            cache.read_json(path, hashlib.sha256(nonfinite).hexdigest())
        path.write_bytes(b" " * (cache.MAX_JSON_BYTES + 1))
        with self.assertRaisesRegex(cache.CacheError, "size"):
            cache.read_json(path, "0" * 64)

    def test_manifest_paths_cannot_alias_the_workspace(self) -> None:
        manifest = self.snapshot()
        outside = self.root / "manifest.json"
        self.assertEqual(cache.manifest_output(outside, manifest), outside)
        link = self.root / "workspace-alias"
        link.symlink_to(self.workspace, target_is_directory=True)
        for path in (self.workspace / "manifest.json", link / "manifest.json"):
            with (
                self.subTest(path=path),
                self.assertRaisesRegex(
                    cache.CacheError, "outside the Actions workspace"
                ),
            ):
                cache.manifest_output(path, manifest)

    def test_restore_observations_reject_incompatible_or_ambiguous_keys(self) -> None:
        manifest = self.snapshot()
        keys = manifest["keys"]
        for kind in ("downloads", "target"):
            for state in ("miss", "fallback", "exact"):
                self.assertEqual(
                    self.observation(manifest, kind, state)["exact"], state == "exact"
                )
            for primary, matched, hit in (
                ("other", "", ""),
                (keys[kind], "other", "false"),
                (keys[kind], keys[kind], "false"),
                (keys[kind], "", "true"),
                ("", keys[kind], "true"),
            ):
                with (
                    self.subTest(kind=kind, values=(primary, matched, hit)),
                    self.assertRaises(cache.CacheError),
                ):
                    cache.restore_observation(keys, kind, primary, matched, hit)

    def test_save_policy_matrix_never_promotes_a_read_only_caller(self) -> None:
        manifests = {save: self.snapshot(save=save) for save in (False, True)}
        for save, downloads, target, status in itertools.product(
            (False, True),
            ("miss", "fallback", "exact"),
            ("miss", "fallback", "exact"),
            ("success", "failure", "cancelled"),
        ):
            manifest = manifests[save]
            observations = {
                kind: self.observation(manifest, kind, state)
                for kind, state in (("downloads", downloads), ("target", target))
            }
            state = cache.restored_manifest(manifest, observations)
            with mock.patch.object(cache, "make_identity", return_value=manifest):
                plan = cache.save_plan(state, self.environment, status)
            self.assertEqual(
                plan["save-downloads"],
                str(save and status == "success" and downloads != "exact").lower(),
            )
            self.assertEqual(
                plan["save-target"],
                str(save and status == "success" and target != "exact").lower(),
            )

    def test_save_rechecks_identity_and_bound_restore_state(self) -> None:
        manifest = self.snapshot()
        state = cache.restored_manifest(
            manifest,
            {
                kind: self.observation(manifest, kind, "miss")
                for kind in ("downloads", "target")
            },
        )
        changed = copy.deepcopy(manifest)
        changed["identity"]["tools"]["cargo"]["native"]["sha256"] = "0" * 64
        with (
            mock.patch.object(cache, "make_identity", return_value=changed),
            self.assertRaisesRegex(cache.CacheError, "changed"),
        ):
            cache.save_plan(state, self.environment, "success")
        state["restore"]["target"]["requested"] = "other"
        with (
            mock.patch.object(cache, "make_identity", return_value=manifest),
            self.assertRaisesRegex(cache.CacheError, "restore observation"),
        ):
            cache.save_plan(state, self.environment, "success")

    def test_manifest_identity_keeps_json_types_distinct(self) -> None:
        manifest = self.snapshot()
        for version in (True, 1.0):
            changed = {**manifest, "version": version}
            with (
                self.subTest(version=version),
                self.assertRaisesRegex(cache.CacheError, "Unsupported cache manifest"),
            ):
                cache.verify_identity(changed, self.environment)
        changed = copy.deepcopy(manifest)
        changed["identity"]["tools"]["native"]["cc"]["executable"]["size"] = True
        with (
            mock.patch.object(cache, "make_identity", return_value=manifest),
            self.assertRaisesRegex(cache.CacheError, "changed"),
        ):
            cache.verify_identity(changed, self.environment)

    def test_executable_invocation_and_target_are_distinct(self) -> None:
        binary = self.root / "bin/rustup"
        executable(binary, "fixture rustup\n")
        invocation = binary.with_name("cargo")
        invocation.symlink_to(binary.name)
        environment = {"PATH": str(binary.parent)}
        identity = cache.executable_identity("cargo", environment)
        self.assertEqual(identity["invocation"], str(invocation))
        self.assertEqual(identity["resolved"], str(binary))
        binary.write_text("changed fixture rustup\n")
        with self.assertRaisesRegex(cache.CacheError, "Executable changed"):
            cache.verify_executable("cargo", identity, environment)

    def rustup_fixture(self) -> tuple[dict[str, str], dict[str, Path], Any]:
        directory = self.root / "rustup-bin"
        rustup = directory / "rustup"
        executable(rustup, "fixture rustup\n")
        native = self.root / "toolchain/bin"
        paths = {"rustup": rustup}
        for name in ("cargo", "rustc"):
            paths[name] = directory / name
            paths[name].symlink_to("rustup")
            paths["native-" + name] = native / name
            executable(paths["native-" + name], f"fixture native {name}\n")
        environment = {**self.environment, "PATH": str(directory)}

        def run(
            arguments: list[str],
            _source: Path,
            _environment: dict[str, str],
            _description: str,
        ) -> str:
            if arguments == [str(rustup), "show", "active-toolchain"]:
                return f"1.98.1-{cache.HOST} (default)"
            if arguments[:2] == [str(rustup), "which"]:
                return str(paths["native-" + arguments[2]])
            for name, option in (("cargo", "-Vv"), ("rustc", "-vV")):
                if arguments in (
                    [str(paths[name]), option],
                    [str(paths["native-" + name]), option],
                ):
                    return f"{name} 1.98.1\nrelease: 1.98.1\nhost: {cache.HOST}\ncommit-hash: {'a' * 40}"
            raise AssertionError(f"Unexpected probe: {arguments}")

        return environment, paths, run

    def test_rustup_probes_use_the_captured_proxy_and_native_paths(self) -> None:
        environment, paths, run = self.rustup_fixture()
        with mock.patch.object(cache, "command", side_effect=run):
            tools = cache.rust_toolchain(self.source, environment)
        self.assertEqual(
            tools["cargo"]["invocation"]["invocation"], str(paths["cargo"])
        )
        self.assertEqual(
            tools["cargo"]["native"]["invocation"], str(paths["native-cargo"])
        )
        self.assertNotEqual(
            tools["cargo"]["native"]["sha256"], tools["rustup"]["sha256"]
        )

    def test_rustup_lookup_changed_before_version_probe_is_rejected(self) -> None:
        environment, paths, run = self.rustup_fixture()
        other = self.root / "other-rustup"
        executable(other, "different rustup\n")

        def changed(
            arguments: list[str],
            source: Path,
            probe_environment: dict[str, str],
            description: str,
        ) -> str:
            result = run(arguments, source, probe_environment, description)
            if arguments[1:] == ["show", "active-toolchain"]:
                paths["cargo"].unlink()
                paths["cargo"].symlink_to(other)
            return result

        with (
            mock.patch.object(cache, "command", side_effect=changed),
            self.assertRaisesRegex(cache.CacheError, "Executable changed"),
        ):
            cache.rust_toolchain(self.source, environment)

    def test_nextest_selection_is_checked_after_the_cargo_mediated_probe(self) -> None:
        environment, paths, rustup_run = self.rustup_fixture()
        with mock.patch.object(cache, "command", side_effect=rustup_run):
            toolchain = cache.rust_toolchain(self.source, environment)
        nextest = self.root / "nextest/bin/cargo-nextest"
        executable(nextest, "fixture nextest\n")
        other = self.root / "other/bin/cargo-nextest"
        executable(other, "other nextest\n")
        calls = 0

        def run(
            arguments: list[str],
            source: Path,
            probe_environment: dict[str, str],
            description: str,
        ) -> str:
            nonlocal calls
            if arguments == [str(paths["cargo"]), "--list", "--verbose"]:
                calls += 1
                return "Installed Commands:\n    nextest    " + str(
                    nextest if calls == 1 else other
                )
            if arguments in (
                [str(paths["cargo"]), "nextest", "--version"],
                [str(nextest), "nextest", "--version"],
            ):
                return f"cargo-nextest {cache.NEXTEST_VERSION}\nrelease: {cache.NEXTEST_VERSION}\nhost: {cache.HOST}\ncommit-hash: {'a' * 40}"
            return rustup_run(arguments, source, probe_environment, description)

        with (
            mock.patch.object(cache, "command", side_effect=run),
            self.assertRaisesRegex(cache.CacheError, "nextest changed"),
        ):
            cache.nextest_identity(self.source, environment, toolchain)

    def test_nextest_probe_cannot_reselect_native_cargo(self) -> None:
        environment, paths, rustup_run = self.rustup_fixture()
        with mock.patch.object(cache, "command", side_effect=rustup_run):
            toolchain = cache.rust_toolchain(self.source, environment)
        nextest = self.root / "nextest/bin/cargo-nextest"
        executable(nextest, "fixture nextest\n")

        def run(
            arguments: list[str],
            source: Path,
            probe_environment: dict[str, str],
            description: str,
        ) -> str:
            if arguments == [str(paths["cargo"]), "--list", "--verbose"]:
                return "Installed Commands:\n    nextest    " + str(nextest)
            if arguments == [str(paths["cargo"]), "nextest", "--version"]:
                paths["native-cargo"].write_text("changed native Cargo\n")
            if arguments in (
                [str(paths["cargo"]), "nextest", "--version"],
                [str(nextest), "nextest", "--version"],
            ):
                return f"cargo-nextest {cache.NEXTEST_VERSION}\nrelease: {cache.NEXTEST_VERSION}\nhost: {cache.HOST}\ncommit-hash: {'a' * 40}"
            return rustup_run(arguments, source, probe_environment, description)

        with (
            mock.patch.object(cache, "command", side_effect=run),
            self.assertRaisesRegex(cache.CacheError, "native Cargo changed"),
        ):
            cache.nextest_identity(self.source, environment, toolchain)

    def test_real_failed_command_does_not_echo_its_output(self) -> None:
        secret = "private-command-output-canary"
        with self.assertRaisesRegex(cache.CacheError, "Could not probe") as result:
            cache.command(
                [sys.executable, "-c", f"import sys; print({secret!r}); sys.exit(1)"],
                self.source,
                self.environment,
                "probe",
            )
        self.assertNotIn(secret, str(result.exception))

    def test_official_action_wrappers_have_no_implicit_save(self) -> None:
        restore = action_steps(ACTION / "restore/action.yml")
        save = action_steps(ACTION / "save/action.yml")
        self.assertEqual(list(restore), ["prepare", "downloads", "target", "observe"])
        self.assertEqual(list(save), ["plan", "downloads", "target"])
        for kind in ("downloads", "target"):
            self.assertEqual(
                action_field(restore[kind], "uses"),
                cache.CACHE_ACTION.replace("actions/cache@", "actions/cache/restore@"),
            )
            self.assertEqual(
                action_mapping(restore[kind], "with"),
                {
                    "path": "${{ steps.prepare.outputs." + kind + "-paths }}",
                    "key": "${{ steps.prepare.outputs." + kind + "-key }}",
                    "restore-keys": "${{ steps.prepare.outputs."
                    + kind
                    + "-restore-key }}",
                },
            )
            self.assertEqual(
                action_field(save[kind], "uses"),
                cache.CACHE_ACTION.replace("actions/cache@", "actions/cache/save@"),
            )
            self.assertEqual(
                action_field(save[kind], "if"),
                "${{ success() && steps.plan.outputs.save-" + kind + " == 'true' }}",
            )
            self.assertEqual(
                action_mapping(save[kind], "with"),
                {
                    "path": "${{ steps.plan.outputs." + kind + "-paths }}",
                    "key": "${{ steps.plan.outputs." + kind + "-key }}",
                },
            )
        self.assertEqual(action_field(save["plan"], "if"), "${{ success() }}")

    def test_action_extractor_rejects_changed_shape(self) -> None:
        path = self.root / "action.yml"
        path.write_text(
            "runs:\n  using: composite\n  steps:\n    - uses: unobserved/action\n"
        )
        with self.assertRaisesRegex(ValueError, "expected id"):
            action_steps(path)


if __name__ == "__main__":
    unittest.main()
