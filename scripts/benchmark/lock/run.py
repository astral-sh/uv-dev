"""Run an index-backed lock benchmark in an isolated Docker container."""

import argparse
import fcntl
import hashlib
import json
import os
import re
import subprocess
import tomllib
import uuid
from pathlib import Path
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
HARNESS = "50ed1c0b195e773da684d72db6e863aba32ffcde"
CUTOFF = "2026-10-01T00:00:00Z"
PYTHONS = {
    "3.11": "db20d8506e2f15b6db39b40b6015ca46bae855dec6dd326d462cfe79ae972893",
    "3.12": "b18b261e1b4974b4f321451ad5251ded85abb22c2b8b262da527dda1d031ab30",
    "3.14": "930e1abba7c4e8f96ed93e8012c35d174d7732752731bbd5e8e757d7ec6f3966",
}
PROJECTS = {
    "airflow": "test/requirements/airflow.in",
    "black": "test/requirements/black.in",
    "jupyter": "test/requirements/jupyter.in",
    "transformers": "test/requirements/transformers/pyproject.toml",
    "transformers-torch-vision": "scripts/benchmark/lock/fixtures/transformers-torch-vision.toml",
    "trio": "test/requirements/trio.in",
    "warehouse": "test/ecosystem/warehouse/pyproject.toml",
}


def mount(source, destination, *, writable=False):
    path = source.resolve(strict=True)
    if "," in str(path):
        raise ValueError(f"Docker bind mount path contains a comma: {path}")
    access = "" if writable else ",readonly"
    return ["--mount", f"type=bind,src={path},dst={destination}{access}"]


def file_hash(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("project", choices=[*PROJECTS, "corpus"])
    parser.add_argument("--uv", type=Path, required=True)
    parser.add_argument(
        "--image-id",
        help="immutable local Docker image ID for an image with native build dependencies",
    )
    parser.add_argument(
        "--output", type=Path, required=True, help="fresh output directory"
    )
    parser.add_argument(
        "--cache", type=Path, required=True, help="dedicated writable cache"
    )
    parser.add_argument(
        "--seed", type=Path, help="cache tar archive; restored only inside Docker"
    )
    parser.add_argument(
        "--continue-seed-from",
        type=Path,
        help="continue online corpus seeding from a completed run using the same cache",
    )
    parser.add_argument(
        "--network", default="none", help="isolated internal Docker network for seeding"
    )
    parser.add_argument(
        "--proxy", help="credential-free PyPI-only HTTP proxy on that network"
    )
    parser.add_argument("--ca-bundle", type=Path)
    parser.add_argument("--harness", type=Path)
    parser.add_argument(
        "--wheels", type=Path, help="locked tomli-w and tqdm harness wheels"
    )
    parser.add_argument("--input", type=Path, help="CSV shard of the pinned corpus")
    parser.add_argument(
        "--excluded-root",
        choices=(
            "emmet-api",
            "edx-enterprise",
            "tf-models-nightly",
            "kcli",
            "llm-dialog-manager",
        ),
        help="measure one excluded corpus root separately using its one-row input CSV",
    )
    parser.add_argument("--limit", type=int)
    parser.add_argument(
        "--workers",
        choices=(2, 4, 8, 16),
        type=int,
        default=2,
        help="concurrent corpus resolutions",
    )
    parser.add_argument(
        "--preview", help="value of UV_PREVIEW_FEATURES for a candidate"
    )
    parser.add_argument(
        "--setting",
        action="append",
        default=[],
        help="explicit UV_INTERNAL__ experiment setting, NAME=VALUE",
    )
    parser.add_argument(
        "--source-pair",
        nargs=2,
        metavar=("SEPARATE_COMMIT", "INTEGRATED_COMMIT"),
        help="record both full candidate commits for a matched batch",
    )
    parser.add_argument(
        "--prefetch-diagnostics",
        action="store_true",
        help="collect verbose prefetch counters in a separate named-project run",
    )
    parser.add_argument("--timeout", type=int, default=3600)
    args = parser.parse_args()
    if args.source_pair and any(
        not re.fullmatch("[0-9a-f]{40}", commit) for commit in args.source_pair
    ):
        parser.error("--source-pair requires two full Git commit hashes")
    if os.getuid() == 0:
        parser.error("run as a non-root user")
    if args.prefetch_diagnostics and (args.project == "corpus" or not args.preview):
        parser.error("--prefetch-diagnostics requires a named project and a preview")
    if args.continue_seed_from and (
        args.project != "corpus" or args.network == "none" or args.seed
    ):
        parser.error(
            "--continue-seed-from requires online corpus seeding without --seed"
        )
    if args.excluded_root and (
        args.project != "corpus" or not args.input or args.limit is not None
    ):
        parser.error(
            "--excluded-root requires corpus and a one-row --input without --limit"
        )
    if args.network != "none":
        (network,) = json.loads(
            subprocess.check_output(["docker", "network", "inspect", args.network])
        )
        if (
            not network["Internal"]
            or network["Options"].get("com.docker.network.bridge.gateway_mode_ipv4")
            != "isolated"
            or network["EnableIPv6"]
        ):
            parser.error(
                "seeding requires an internal IPv4 network with gateway_mode_ipv4=isolated"
            )
        proxy = urlsplit(args.proxy or "")
        if not (
            proxy.scheme == "http"
            and proxy.hostname
            and not proxy.username
            and not proxy.password
        ):
            parser.error(
                "seeding requires a credential-free proxy on the isolated network"
            )
    elif args.proxy:
        parser.error("offline runs have no reachable proxy")

    args.cache.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(args.cache, os.O_RDONLY | os.O_DIRECTORY)
    try:
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            parser.error("another benchmark is using this cache")
        run(args, parser)
    finally:
        os.close(descriptor)


def run(args, parser):
    selector = "3.11" if args.project == "warehouse" else "3.12"
    if args.project == "corpus":
        selector = "3.14"
    image = "python@sha256:" + PYTHONS[selector]
    if args.image_id:
        if not re.fullmatch(r"sha256:[0-9a-f]{64}", args.image_id):
            parser.error("--image-id requires the full sha256 digest")
        image = args.image_id
    name = "uv-lock-" + uuid.uuid4().hex
    if not args.continue_seed_from and any(args.cache.iterdir()):
        parser.error("--cache must be a fresh directory")
    docker = [
        "docker",
        "run",
        "--pull",
        "never",
        "--rm",
        "--user",
        f"{os.getuid()}:{os.getgid()}",
        "--read-only",
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges",
        "--env",
        f"UV_EXCLUDE_NEWER={CUTOFF}",
    ]
    offline = [*docker, "--network", "none", "--memory", "1g", "--pids-limit", "32"]
    command = [
        *docker,
        "--name",
        name,
        "--network",
        args.network,
        "--dns",
        "127.0.0.1",
        "--pids-limit",
        "512",
        "--memory",
        "8g",
        "--cpus",
        "4",
        "--tmpfs",
        "/tmp:rw,nosuid,nodev,size=2g",
        *mount(args.uv, "/bin/uv-bench"),
        *mount(args.cache, "/cache", writable=True),
        "--workdir",
        "/tmp",
    ]
    env = {
        "HOME": "/tmp",
        "UV_CACHE_DIR": "/cache",
        "UV_PYTHON_DOWNLOADS": "never",
        "UV_NO_PROGRESS": "1",
        "UV_OFFLINE": str(args.network == "none").lower(),
        "PYTHONDONTWRITEBYTECODE": "1",
        "UV_DEFAULT_INDEX": "https://pypi.org/simple",
        "UV_CONCURRENT_DOWNLOADS": "8",
        "UV_CONCURRENT_BUILDS": "1",
        "RAYON_NUM_THREADS": "4",
        "UV_CONCURRENT_INSTALLS": "4",
    }
    # Override Docker client proxy defaults, which may contain host-specific credentials.
    for key in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"):
        env[key] = (args.proxy or "") if key in ("HTTP_PROXY", "HTTPS_PROXY") else ""
        env[key.lower()] = env[key]
    settings = {}
    for setting in args.setting:
        key, separator, value = setting.partition("=")
        if not (
            separator
            and value
            and key.startswith("UV_INTERNAL__")
            and key.isidentifier()
        ):
            parser.error("expected --setting UV_INTERNAL__NAME=VALUE")
        settings[key] = value
    env.update(settings)
    if args.prefetch_diagnostics:
        env["RUST_LOG"] = "uv_resolver::resolver::build_prefetch=debug"
    if args.preview:
        env["UV_PREVIEW_FEATURES"] = args.preview
    if args.ca_bundle:
        command += mount(args.ca_bundle, "/etc/uv-bench-ca.pem")
        env["SSL_CERT_FILE"] = "/etc/uv-bench-ca.pem"

    if args.project == "corpus":
        if not args.harness or not args.wheels:
            parser.error("corpus requires --harness and --wheels")
        revision = subprocess.check_output(
            ["git", "-C", args.harness, "rev-parse", "HEAD"], text=True
        ).strip()
        if revision != HARNESS or subprocess.check_output(
            ["git", "-C", args.harness, "status", "--porcelain"]
        ):
            parser.error(f"expected a clean harness checkout at {HARNESS}")
        command += mount(args.harness / "src", "/harness/src")
        command += mount(HERE / "time-limit.sh", "/bin/uv-with-time-limit")
        corpus = args.input or args.harness / "data/top-15k-pypi.csv"
        command += mount(corpus, "/harness/input.csv")
        with (args.harness / "uv.lock").open("rb") as file:
            packages = tomllib.load(file)["package"]
        pythonpath = ["/harness/src"]
        for package in packages:
            if package["name"] in ("tomli-w", "tqdm"):
                (wheel,) = package["wheels"]
                filename = Path(urlsplit(wheel["url"]).path).name
                local = args.wheels / filename
                if "sha256:" + file_hash(local) != wheel["hash"]:
                    parser.error(f"wrong wheel digest for {filename}")
                destination = f"/harness/{filename}"
                command += mount(local, destination)
                pythonpath.append(destination)
        env["PYTHONPATH"] = ":".join(pythonpath)
        # The pinned harness sizes its executor at twice os.cpu_count().
        env["PYTHON_CPU_COUNT"] = str(args.workers // 2)
        task = [
            "python",
            "-m",
            "uv_ecosystem_testing.resolve",
            "--mode",
            "lock",
            "--cache",
            "/cache",
            "--input",
            "/harness/input.csv",
            "--uv",
            "/bin/uv-with-time-limit",
            "--output",
            "/out/corpus",
            "--i-am-in-docker",
        ]
        if args.limit is not None:
            task += ["--limit", str(args.limit)]
        if args.excluded_root:
            # Read the cgroup OOM status before removing a stopped container.
            command.remove("--rm")
            command += mount(HERE / "excluded.py", "/excluded.py")
            task = ["python", "/excluded.py", args.excluded_root]
        input_hash = file_hash(corpus)
    else:
        fixture = ROOT / PROJECTS[args.project]
        destination = f"/fixture/{fixture.name}"
        command += mount(fixture, destination) + mount(HERE / "named.py", "/runner.py")
        task = ["python", "-I", "/runner.py", destination, f"/out/{args.project}"]
        if args.prefetch_diagnostics:
            task.append("--prefetch-diagnostics")
        input_hash = file_hash(fixture)

    receipt = {
        "complete": False,
        "container": name,
        "image": image,
        "binary_sha256": file_hash(args.uv),
        "input_sha256": input_hash,
        "python": subprocess.check_output(
            [*offline, image, "python", "-V"], text=True
        ).strip(),
        "project": args.project,
        "exclude_newer": CUTOFF,
        "offline": args.network == "none",
        "preview": args.preview,
        "settings": settings,
        "harness": HARNESS if args.project == "corpus" else None,
        "limit": args.limit,
        "workers": args.workers
        if args.project == "corpus" and not args.excluded_root
        else 1,
        "seed_sha256": file_hash(args.seed) if args.seed else None,
    }
    if args.prefetch_diagnostics:
        receipt["prefetch_diagnostics"] = True
    if args.source_pair:
        separate, integrated = args.source_pair
        receipt["source_pair"] = {"separate": separate, "integrated": integrated}
    if args.excluded_root:
        receipt["excluded_root"] = args.excluded_root
    if args.project == "corpus" and args.network != "none":
        cache = args.cache.stat()
        receipt["seed_cache_identity"] = [cache.st_dev, cache.st_ino]
    if args.continue_seed_from:
        try:
            previous = json.loads((args.continue_seed_from / "run.json").read_text())
        except (OSError, ValueError) as error:
            parser.error(f"cannot read previous seed run: {error}")
        if (
            not isinstance(previous, dict)
            or not previous.get("complete")
            or not previous.get("container")
            or any(
                previous.get(key) != receipt.get(key)
                for key in (
                    "seed_cache_identity",
                    "source_pair",
                    "project",
                    "image",
                    "input_sha256",
                    "python",
                    "exclude_newer",
                    "offline",
                    "harness",
                    "limit",
                    "workers",
                    "excluded_root",
                )
            )
        ):
            parser.error(
                "previous run must have completed on the same corpus and cache"
            )
        receipt["continued_from"] = previous["container"]
    if not receipt["python"].startswith(f"Python {selector}."):
        parser.error(f"image must provide CPython {selector}")
    args.output.mkdir(parents=True, exist_ok=False)
    # Keep host provenance outside the writable bind mount.
    results = args.output / "results"
    results.mkdir()
    command += mount(results, "/out", writable=True)
    (args.output / "run.json").write_text(json.dumps(receipt, indent=2) + "\n")
    for key, value in env.items():
        command += ["--env", f"{key}={value}"]
    if args.seed:
        # Extract package-created cache files only inside a disposable container.
        subprocess.run(
            [
                *offline,
                *mount(args.seed, "/seed.tar"),
                *mount(args.cache, "/cache", writable=True),
                image,
                "tar",
                "--no-same-owner",
                "-xf",
                "/seed.tar",
                "-C",
                "/cache",
            ],
            check=True,
        )
    try:
        subprocess.run([*command, image, *task], check=True, timeout=args.timeout)
        receipt["complete"] = True
        (args.output / "run.json").write_text(json.dumps(receipt, indent=2) + "\n")
    finally:
        if args.excluded_root:
            inspect = subprocess.run(
                ["docker", "inspect", "--format", "{{json .State}}", name],
                capture_output=True,
                text=True,
                check=False,
            )
            if inspect.returncode == 0:
                state = json.loads(inspect.stdout)
                receipt["container_state"] = {
                    key: state[key]
                    for key in ("OOMKilled", "ExitCode", "StartedAt", "FinishedAt")
                }
                (args.output / "run.json").write_text(
                    json.dumps(receipt, indent=2) + "\n"
                )
        subprocess.run(
            ["docker", "rm", "-f", name],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )


if __name__ == "__main__":
    main()
