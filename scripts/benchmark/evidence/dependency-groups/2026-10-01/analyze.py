"""Replay the recorded dependency-group study without running either product."""

from __future__ import annotations

import contextlib
import hashlib
import io
import json
import sys
import tarfile
import types
from pathlib import Path

HERE = Path(__file__).resolve().parent
MANIFEST_SHA256 = "d5fbd5f1c30822339a252a2211b93bf606b09fb89ff0e82f44a2b86cf5c3eecc"
ANALYZER_SHA256 = "c8d76aad41b5aaa097eb5570bdef984250cc4216f5f16f7c8d95713cbc3952d2"
PREFLIGHT_SHA256 = "231298b9e961bc7d5879dd460e7fd1f5f72ec2362fa325d3aed9a18910535ccd"
MAX_BYTES = 4 * 1024 * 1024


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def identity(contents: bytes) -> dict:
    return {"sha256": hashlib.sha256(contents).hexdigest(), "bytes": len(contents)}


def read_file(path: Path) -> bytes:
    require(
        not path.is_symlink() and path.is_file(), f"Expected an ordinary file: {path}"
    )
    require(path.stat().st_size <= MAX_BYTES, f"Oversized file: {path}")
    contents = path.read_bytes()
    require(len(contents) <= MAX_BYTES, f"Oversized file: {path}")
    return contents


def load() -> tuple[dict, dict[str, bytes]]:
    manifest_bytes = read_file(HERE / "manifest.json")
    require(identity(manifest_bytes)["sha256"] == MANIFEST_SHA256, "Manifest changed")
    manifest = json.loads(manifest_bytes)
    archive_bytes = read_file(HERE / "data.tar.gz")
    require(
        identity(archive_bytes)
        == {key: manifest["archive"][key] for key in ("sha256", "bytes")},
        "Data archive changed",
    )
    members = {}
    with tarfile.open(fileobj=io.BytesIO(archive_bytes), mode="r:gz") as archive:
        for member in archive:
            require(
                member.isfile()
                and member.name not in members
                and member.name in manifest["archive"]["members"]
                and member.size <= MAX_BYTES,
                "Unexpected archive member",
            )
            stream = archive.extractfile(member)
            require(stream is not None, "Missing member contents")
            contents = stream.read(MAX_BYTES + 1)
            require(
                identity(contents) == manifest["archive"]["members"][member.name],
                f"Archive member changed: {member.name}",
            )
            members[member.name] = contents
    require(set(members) == set(manifest["archive"]["members"]), "Incomplete archive")
    require(sum(map(len, members.values())) <= MAX_BYTES, "Oversized expanded archive")
    return manifest, members


def module(contents: bytes, filename: str, expected: str) -> types.ModuleType:
    require(identity(contents)["sha256"] == expected, f"Source changed: {filename}")
    value = types.ModuleType(filename.removesuffix(".py"))
    value.__file__ = str(Path("recorded-study") / filename)
    # Both accepted source hashes are fixed above, independently of archive metadata.
    exec(compile(contents, value.__file__, "exec"), value.__dict__)  # noqa: S102
    return value


def calibration_sequence(calibration: dict, plan: dict, recorded_identity: dict) -> int:
    expected = [
        (case, clock)
        for case in plan["statistics"]["first_pass_case_ids"]
        for clock in plan["statistics"]["clock_names"]
    ]
    attempts = iter(calibration["attempts"])
    selected = []
    sequence = 3  # The two complete behavior comparisons precede calibration.
    for case, clock in expected:
        last_valid = None
        for attempt in range(21):
            row = next(attempts)
            iterations = 1 << attempt
            require(
                (row["case"], row["clock"], row["attempt"], row["iterations"])
                == (case, clock, attempt, iterations),
                "Calibration order or iteration count changed",
            )
            elapsed = []
            for side in ("base", "head"):
                sample = row["samples"][side]
                binding = recorded_identity[side]
                require(
                    sample["schema"] == "uv-pr244-api-sample-v1"
                    and (sample["case"], sample["clock"]) == (case, clock)
                    and sample["fixture_sha256"] == plan["fixtures"]["fixture_sha256"]
                    and sample["iterations"]
                    == sample["completed_iterations"]
                    == iterations
                    and sample["exit_status"] == 0
                    and sample["process_sequence"] is None
                    and sample["global_command_sequence"] == sequence
                    and sample["executable_sha256"] == binding["executable_sha256"]
                    and sample["overlaid_tree"] == binding["overlaid_tree"]
                    and type(sample["elapsed_ns"]) is int
                    and sample["elapsed_ns"] > 0,
                    "Calibration sample changed",
                )
                elapsed.append(sample["elapsed_ns"])
                sequence += 1
            if all(1_000_000 <= value <= 10_000_000_000 for value in elapsed):
                last_valid = iterations
            if (
                all(value >= 10_000_000 for value in elapsed)
                or any(value >= 10_000_000_000 for value in elapsed)
                or iterations == 1_048_576
            ):
                break
        require(last_valid is not None, "Uncalibrated planned contrast")
        selected.append({"case": case, "clock": clock, "iterations": last_valid})
    require(next(attempts, None) is None, "Extra calibration attempt")
    require(selected == calibration["selected"], "Calibration selection changed")
    return sequence


def replay(manifest: dict, members: dict[str, bytes]) -> bytes:
    require(
        set(members) == set(manifest["archive"]["members"]), "Incomplete member set"
    )
    for name, expected in manifest["archive"]["members"].items():
        require(identity(members[name]) == expected, f"Evidence member changed: {name}")
    for name, expected in manifest["original"]["records"].items():
        require(
            identity(members[name]) == expected, f"Historical record changed: {name}"
        )
    plan = json.loads(members["plan.json"])
    require(
        plan["source_pair"] == manifest["source_pair"], "Measured source pair changed"
    )
    require(
        plan["original_plan_sha256"] == manifest["original"]["plan_sha256"],
        "Historical plan identity changed",
    )
    analyzer = module(members["analyze_pairs.py"], "analyze_pairs.py", ANALYZER_SHA256)
    strict = module(
        members["analyze_pairs_preflight_v2.py"],
        "analyze_pairs_preflight_v2.py",
        PREFLIGHT_SHA256,
    )
    directory = Path("recorded-study")
    analyzer.HERE = directory
    snapshots = {directory / name: contents for name, contents in members.items()}

    def frozen_read(path: Path) -> dict:
        require(path in snapshots, "Unexpected estimator read")
        return json.loads(snapshots[path])

    def frozen_digest(path: Path) -> str:
        require(path in snapshots, "Unexpected estimator digest")
        if path == directory / "plan.json":
            # The portable plan retains the statistical policy and supplies the
            # measured normalized trees. Historical samples keep their original digest.
            return manifest["original"]["plan_sha256"]
        return identity(snapshots[path])["sha256"]

    analyzer.read = frozen_read
    analyzer.digest = frozen_digest
    aa = json.loads(members["aa.json"])
    ab = json.loads(members["ab.json"])
    design = json.loads(members["fixed-design.json"])
    require(
        aa["identity"] == ab["identity"] == manifest["build"]["identity"],
        "Build identity changed",
    )
    sequence = calibration_sequence(
        json.loads(members["calibration.json"]), plan, aa["identity"]
    )
    for observations in (aa, ab):
        if observations is ab:
            sequence += 1  # The sealed A/A design is a separate analysis command.
        strict.observations(observations)
        for contrast in observations["contrasts"]:
            for block in contrast["blocks"]:
                for arm in block["observed_order"]:
                    require(
                        block["samples"][arm]["global_command_sequence"] == sequence,
                        "Global process order changed",
                    )
                    sequence += 1
    strict.fixed_design(design, analyzer.contrasts(plan))

    def analyze(mode: str) -> bytes:
        arguments = [
            "analyze_pairs.py",
            mode,
            str(directory / ("aa.json" if mode == "choose" else "ab.json")),
        ]
        if mode == "report":
            arguments.extend(["--design", str(directory / "fixed-design.json")])
        previous = sys.argv
        output = io.StringIO()
        try:
            sys.argv = arguments
            with contextlib.redirect_stdout(output):
                analyzer.main()
        finally:
            sys.argv = previous
        return output.getvalue().encode()

    require(
        analyze("choose") == members["fixed-design.json"], "Fixed design did not replay"
    )
    report = analyze("report")
    require(report == members["report.json"], "Complete report did not replay")
    require(sequence + 1 == 755, "Complete command count changed")
    return report


def main() -> None:
    manifest, members = load()
    sys.stdout.buffer.write(replay(manifest, members))


if __name__ == "__main__":
    main()
