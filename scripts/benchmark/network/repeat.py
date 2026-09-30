"""Repeat a completed network pilot with its recorded binaries and workload."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "network_bench", Path(__file__).with_name("bench.py")
)
assert spec is not None and spec.loader is not None
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def git_repositories(root: Path) -> dict:
    return {
        path.name: subprocess.check_output(
            ["git", "--git-dir", str(path), "show-ref", "--head"], text=True
        ).splitlines()
        for path in sorted(root.resolve().glob("*.git"))
    }


def check_inputs(pilot: dict, args: argparse.Namespace) -> float:
    require("summary" in pilot, "Pilot is incomplete")
    require(bench.summary(pilot["pairs"]) == pilot["summary"], "Pilot summary differs")
    require(pilot["parent_sha"] != pilot["head_sha"], "Source revisions must differ")
    for side in ("parent", "head"):
        revision = pilot[f"{side}_sha"]
        require(
            re.fullmatch(r"[0-9a-f]{40}", revision) is not None, "Use full commit IDs"
        )
        binary = pilot["binaries"][side]
        path = Path(binary["path"])
        require(bench.digest(path) == binary["sha256"], f"{side} binary hash differs")
        version = subprocess.check_output([path, "--version"], text=True).strip()
        require(
            version == binary["version"] and revision[:9] in version,
            f"{side} binary version differs",
        )
    require(
        bench.digest(args.manifest) == pilot["manifest_sha256"],
        "Fixture manifest differs",
    )
    require(
        bench.netem_profile() == pilot.get("netem", {}),
        "Kernel network profile differs",
    )
    python_request = pilot.get("python_request")
    require(
        isinstance(python_request, str), "Pilot did not record its Python executable"
    )
    python = Path(python_request)
    require(
        python.is_file()
        and pilot.get("python_executable_sha256") is not None
        and bench.digest(python) == pilot["python_executable_sha256"],
        "Pilot must identify the same concrete Python executable",
    )
    timeout = args.timeout if args.timeout is not None else pilot.get("timeout_seconds")
    require(
        timeout is not None and timeout > 0,
        "Pass --timeout for a pilot that did not record it",
    )
    if "timeout_seconds" in pilot:
        require(timeout == pilot["timeout_seconds"], "Timeout differs from the pilot")
    if pilot.get("git_repositories") is not None:
        require(args.git_root is not None, "Pass --git-root for this pilot")
        require(
            git_repositories(args.git_root) == pilot["git_repositories"],
            "Git fixture refs differ",
        )
        require(
            subprocess.check_output(["git", "--version"], text=True).strip()
            == pilot["git_version"],
            "Git version differs",
        )
    else:
        require(args.git_root is None, "Pilot did not use Git fixtures")
    if proxy := pilot.get("http2_proxy"):
        require(
            args.tls_certificate is not None and args.tls_key is not None,
            "Pass the pilot's TLS certificate and key",
        )
        require(args.tls_key.is_file(), "TLS key is missing")
        require(
            bench.digest(Path(proxy["binary"])) == proxy["sha256"],
            "HTTP/2 proxy differs",
        )
        require(
            bench.digest(args.tls_certificate) == proxy["certificate_sha256"],
            "TLS certificate differs",
        )
    else:
        require(
            args.tls_certificate is None and args.tls_key is None,
            "Pilot did not use HTTP/2",
        )
    return timeout


def command(
    pilot: dict, args: argparse.Namespace, temporary: Path, timeout: float
) -> list[str]:
    profiles = temporary / "profiles.json"
    profiles.write_text(json.dumps({"repeat": pilot["profile"]}) + "\n")
    uv = str(args.uv.resolve()) if args.uv is not None else shutil.which("uv")
    require(uv is not None, "uv is required to run the benchmark")
    result = [
        uv,
        "run",
        "--no-project",
        "--offline",
        "--python",
        sys.executable,
        "python",
        "-S",
        str(Path(__file__).with_name("bench.py")),
        "--manifest",
        str(args.manifest),
        "--directory",
        str(args.directory),
        "run",
        "--parent",
        pilot["binaries"]["parent"]["path"],
        "--parent-sha",
        pilot["parent_sha"],
        "--head",
        pilot["binaries"]["head"]["path"],
        "--head-sha",
        pilot["head_sha"],
        "--profiles",
        str(profiles),
        "--profile",
        "repeat",
        "--python",
        pilot["python_request"],
        "--pairs",
        str(args.pairs),
        "--warmups",
        str(args.warmups),
        "--cache-mode",
        pilot["cache_mode"],
        "--timeout",
        str(timeout),
        "--work-dir",
        str(args.work_dir),
        "--output",
        str(args.output),
    ]
    for key, flag in (
        ("required_bytes", "--required-bytes"),
        ("required_waves", "--required-waves"),
        ("required_latency_ms", "--required-latency-ms"),
        ("required_wait_ms", "--required-wait-ms"),
    ):
        if (value := pilot["lower_bound"].get(key)) is not None:
            result.extend((flag, str(value)))
    for key, flag in (
        ("requirements", "--requirement"),
        ("normalize_tree_file", "--normalize-tree-file"),
        ("normalize_tree_symlink", "--normalize-tree-symlink"),
        ("verify_file", "--verify-file"),
    ):
        for value in pilot.get(key, []):
            result.extend((flag, value))
    if pilot.get("verify_tree"):
        result.extend(("--verify-tree", pilot["verify_tree"]))
    if pilot.get("compare_stderr"):
        result.append("--compare-stderr")
    for key, value in pilot.get("environment_overrides", {}).items():
        result.extend(("--env", f"{key}={value}"))
    for name, flag in (
        ("uv.toml", "--config-template"),
        ("pyproject.toml", "--project-template"),
        ("uv.lock", "--lock-template"),
        ("pylock.toml", "--pylock-template"),
    ):
        if name in pilot.get("templates", {}):
            path = temporary / name
            path.write_text(pilot["templates"][name])
            result.extend((flag, str(path)))
    if pilot.get("setup_commands"):
        path = temporary / "setup.json"
        path.write_text(json.dumps(pilot["setup_commands"]) + "\n")
        result.extend(("--setup-commands", str(path)))
    if args.git_root is not None:
        result.extend(("--git-root", str(args.git_root)))
    if proxy := pilot.get("http2_proxy"):
        result.extend(
            (
                "--http2-proxy",
                proxy["binary"],
                "--tls-certificate",
                str(args.tls_certificate),
                "--tls-key",
                str(args.tls_key),
            )
        )
    return [*result, "--", *pilot["command"]]


def check_repeat(pilot: dict, repeated: dict) -> None:
    defaults = {
        "netem": {},
        "requirements": [],
        "templates": {},
        "setup_commands": [],
        "environment_overrides": {},
        "normalize_tree_file": [],
        "normalize_tree_symlink": [],
        "verify_file": [],
    }
    for key in (
        "parent_sha",
        "head_sha",
        "binaries",
        "manifest_sha256",
        "profile",
        "netem",
        "command",
        "python_request",
        "python_executable_sha256",
        "requirements",
        "templates",
        "setup_commands",
        "environment_overrides",
        "verify_tree",
        "normalize_tree_file",
        "normalize_tree_symlink",
        "verify_file",
        "http2_proxy",
        "cache_mode",
        "git_repositories",
        "git_version",
    ):
        require(
            repeated.get(key, defaults.get(key)) == pilot.get(key, defaults.get(key)),
            f"Repeated {key} differs",
        )
    require(
        repeated.get("compare_stderr", False) == pilot.get("compare_stderr", False),
        "Repeated stderr comparison differs",
    )
    for key in (
        "required_bytes",
        "required_waves",
        "required_latency_ms",
        "required_wait_ms",
        "seconds",
    ):
        default = 0 if key == "required_wait_ms" else None
        require(
            repeated["lower_bound"].get(key, default)
            == pilot["lower_bound"].get(key, default),
            f"Repeated bound {key} differs",
        )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pilot", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--pairs", type=int, default=30)
    parser.add_argument("--warmups", type=int, default=3)
    parser.add_argument("--timeout", type=float)
    parser.add_argument("--uv", type=Path, help="uv executable used to run the harness")
    parser.add_argument("--git-root", type=Path)
    parser.add_argument("--tls-certificate", type=Path)
    parser.add_argument("--tls-key", type=Path)
    args = parser.parse_args()
    if args.pairs < 20 or args.warmups < 0:
        parser.error("use at least 20 pairs and a nonnegative warmup count")
    if args.output.exists():
        parser.error("output already exists; retain the existing study")
    pilot_bytes = args.pilot.read_bytes()
    pilot_sha256 = hashlib.sha256(pilot_bytes).hexdigest()
    pilot = json.loads(pilot_bytes)
    timeout = check_inputs(pilot, args)
    args.work_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="repeat-", dir=args.work_dir) as temporary:
        subprocess.run(command(pilot, args, Path(temporary), timeout), check=True)
    repeated = json.loads(args.output.read_text())
    check_repeat(pilot, repeated)
    require(bench.digest(args.pilot) == pilot_sha256, "Pilot changed during the repeat")
    require(
        len(repeated["pairs"]) == repeated["summary"]["pairs"] == args.pairs,
        "Repeated sample count differs",
    )
    require(repeated["warmups"] == args.warmups, "Repeated warmups differ")
    require(repeated["timeout_seconds"] == timeout, "Repeated timeout differs")
    repeated["repeated_from"] = {
        "pilot": str(args.pilot),
        "sha256": pilot_sha256,
    }
    args.output.write_text(json.dumps(repeated, indent=2) + "\n")


if __name__ == "__main__":
    main()
