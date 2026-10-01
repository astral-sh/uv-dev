"""Replay the complete recorded package-filter study without running uv."""

from __future__ import annotations

import contextlib
import hashlib
import io
import json
import re
import sys
import tarfile
import types
from pathlib import Path

HERE = Path(__file__).resolve().parent
MANIFEST_SHA256 = "0cb85425f778d3b4b894df77d32a3ea3c9c6570b0667c6a0b3a2fe8436d113bb"
ANALYZER_SHA256 = "b89f53f1182a22616938e566030dca6c9db226d7735a39084225e907e5b85770"
GUARD_SHA256 = "646ec7e07eb77b16188e145afa981b8e85a2ee9a885d8574c4072f90eae71fd5"
COMMITS = {
    "base": "0ded765b4df49fe2dbe7e0497908edb80d9c8fbc",
    "head": "018919a0813966ce887628b3170be0f03625e282",
}
TREES = {
    "base": "b1fdd4959e7af04fbecf50f459eb4e2ba192b746",
    "head": "178ef40051cac3f1af2ba4ebf157b582caab2f33",
}
MAX_FILE = 4 * 1024**2
MAX_EXPANDED = 8 * 1024**2
RECORDS = {"calibration.json", "aa.json", "fixed-design.json", "ab.json", "report.json"}
FIXTURES = {
    "packse.lock",
    "packse.pyproject.toml",
    "uv.lock",
    "uv.pyproject.toml",
    "prefect.lock",
    "prefect.pyproject.toml",
}
MEMBERS = (
    RECORDS
    | {"fixtures/" + name for name in FIXTURES}
    | {
        "owner/fixtures.json",
        "owner/environments.json",
        "fixture-census.json",
        "prepare_fixture_census.py",
        "analyze_pairs.py",
        "analyze_pairs_guard.py",
        "plan.json",
    }
)
PROVENANCE = [
    "literal public commits, Git trees, product blobs, common overlay patches",
    "toolchain version/target and cargo/rustc/rustdoc content identities",
    "retained executable and build-manifest content identities",
    "original ordered inner-clock samples and relative native-record hashes",
    "public source-pinned fixtures and metadata-only portable census",
    "unchanged estimator, strict fixed-design guard, calibration policy, and all outcomes",
]
SAMPLE_KEYS = {
    "schema",
    "case",
    "clock",
    "census_sha256",
    "iterations",
    "elapsed_ns",
    "queries_per_iteration",
    "included_per_iteration",
    "included_checksum",
    "exit_status",
    "executable_sha256",
    "overlaid_tree",
    "process_sequence",
    "global_command_sequence",
    "process_result",
}
INTERVAL_KEYS = {
    "pairs",
    "mean_log_ratio",
    "paired_log_sd",
    "log_half_width",
    "ratio",
    "pointwise_95_ci",
}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def identity(contents: bytes) -> dict:
    return {"sha256": hashlib.sha256(contents).hexdigest(), "bytes": len(contents)}


def keys(value: dict, expected: set[str], label: str) -> None:
    require(
        type(value) is dict and set(value) == expected,
        "Public field allowlist: " + label,
    )


def public_bytes(name: str, raw: bytes) -> None:
    require(len(raw) <= MAX_FILE, "Oversized public member: " + name)
    for forbidden in (
        b"/" + b"Users/",
        b"/" + b"home/",
        b"gateway." + b"internal",
        b"openai." + b"org",
        b"uv-pr252-" + b"perf-",
        b"uv-future-" + b"general-",
        b"BEGIN " + b"PRIVATE KEY",
        b"AWS_SECRET_" + b"ACCESS_KEY",
    ):
        require(forbidden not in raw, "Private token in public member: " + name)
    require(
        re.search(
            rb"(?:ghp" + rb"_[A-Za-z0-9]{36}|github_pat" + rb"_[A-Za-z0-9_]{40,})", raw
        )
        is None,
        "Credential in public member: " + name,
    )


def content_ref(value: dict, label: str) -> None:
    keys(value, {"path", "sha256", "bytes"}, label)
    require(
        re.fullmatch(r"raw/[0-9]+-native\.json", value["path"]) is not None
        and re.fullmatch(r"[0-9a-f]{64}", value["sha256"]) is not None
        and type(value["bytes"]) is int
        and 0 < value["bytes"] <= MAX_FILE,
        "Public native identity: " + label,
    )


def build_identity(value: dict) -> None:
    keys(
        value,
        {
            "base",
            "head",
            "matching_admission_bytes_sha256",
            "execution_admission_sha256",
        },
        "build identity",
    )
    for side in ("base", "head"):
        keys(
            value[side],
            {"build_manifest_sha256", "executable_sha256", "overlaid_tree"},
            "build side",
        )
        for field, digest in value[side].items():
            require(
                re.fullmatch(
                    r"[0-9a-f]{40}" if field == "overlaid_tree" else r"[0-9a-f]{64}",
                    digest,
                )
                is not None,
                "Public build digest",
            )
    for field in ("matching_admission_bytes_sha256", "execution_admission_sha256"):
        require(
            re.fullmatch(r"[0-9a-f]{64}", value[field]) is not None,
            "Public admission digest",
        )


def public_sample(sample: dict) -> None:
    keys(sample, SAMPLE_KEYS, "sample")
    content_ref(sample["process_result"], "sample native")


def public_records(members: dict[str, bytes]) -> None:
    for name in RECORDS:
        public_bytes(name, members[name])
    calibration = json.loads(members["calibration.json"])
    keys(calibration, {"selected", "attempts"}, "calibration")
    for row in calibration["selected"]:
        keys(row, {"case", "clock", "iterations"}, "calibration selection")
    for row in calibration["attempts"]:
        keys(
            row,
            {"case", "clock", "attempt", "iterations", "samples"},
            "calibration attempt",
        )
        keys(row["samples"], {"base", "head"}, "calibration arms")
        for sample in row["samples"].values():
            public_sample(sample)
    for phase in ("aa", "ab"):
        value = json.loads(members[phase + ".json"])
        keys(
            value,
            {"census_sha256", "contrasts", "identity", "phase", "plan_sha256", "schema"}
            | ({"design_sha256"} if phase == "ab" else set()),
            phase,
        )
        build_identity(value["identity"])
        for entry in value["contrasts"]:
            keys(entry, {"blocks", "case", "clock", "iterations"}, "contrast")
            for block in entry["blocks"]:
                keys(block, {"index", "observed_order", "samples"}, "paired block")
                keys(block["samples"], {"A", "B"}, "paired arms")
                for sample in block["samples"].values():
                    public_sample(sample)
    design = json.loads(members["fixed-design.json"])
    keys(
        design,
        {
            "aa_sha256",
            "contrasts",
            "identity",
            "plan_sha256",
            "schema",
            "selection_uses_ab_observations",
        },
        "design",
    )
    build_identity(design["identity"])
    for row in design["contrasts"]:
        keys(
            row,
            {"case", "clock", "iterations", "aa", "ab_pairs", "status"},
            "design contrast",
        )
        keys(row["aa"], INTERVAL_KEYS, "A/A interval")
    report = json.loads(members["report.json"])
    keys(
        report,
        {
            "contrasts",
            "design_sha256",
            "identity",
            "interval_scope",
            "observations_sha256",
            "plan_sha256",
            "ratio_direction",
            "schema",
            "statistical_unit",
            "whole_command_claim",
        },
        "report",
    )
    build_identity(report["identity"])
    for row in report["contrasts"]:
        keys(
            row,
            {"case", "clock", "status"}
            | (set() if row["status"] == "inconclusive-aa" else INTERVAL_KEYS),
            "report contrast",
        )


def public_manifest(manifest: dict) -> None:
    keys(
        manifest,
        {
            "schema",
            "product_pull_request",
            "recorded_on",
            "source_pair",
            "normalized_differences",
            "build",
            "original",
            "portable_fixture",
            "estimator",
            "archive",
            "overlays",
            "command_count",
            "public_provenance_allowlist",
            "native_process_receipts",
            "whole_command_claim",
            "new_samples_collected",
        },
        "manifest",
    )
    keys(manifest["source_pair"], {"base", "head"}, "source pair")
    for source in manifest["source_pair"].values():
        keys(
            source,
            {
                "commit",
                "original_tree",
                "overlaid_tree",
                "original_blob_count",
                "production_blob",
            },
            "source identity",
        )
    require(
        {side: row["commit"] for side, row in manifest["source_pair"].items()}
        == COMMITS
        and {
            side: row["overlaid_tree"] for side, row in manifest["source_pair"].items()
        }
        == TREES,
        "Exact literal measured pair",
    )
    require(
        len(manifest["normalized_differences"]) == 1,
        "One normalized production difference",
    )
    for row in manifest["normalized_differences"]:
        keys(row, {"path", "base", "head"}, "production difference")
        require(
            row["path"] == "crates/uv-configuration/src/install_options.rs",
            "Unexpected product path",
        )
    build = manifest["build"]
    keys(
        build,
        {
            "rust",
            "target",
            "profile",
            "jobs",
            "allocator",
            "separate_base_head_targets",
            "production_artifacts_recompiled",
            "identity",
            "executables",
            "toolchain_binaries",
        },
        "build provenance",
    )
    require(
        (
            build["rust"],
            build["target"],
            build["profile"],
            build["jobs"],
            build["allocator"],
        )
        == ("1.97.1", "x86_64-unknown-linux-gnu", "release", 1, "production allocator")
        and build["separate_base_head_targets"]
        is build["production_artifacts_recompiled"]
        is True,
        "Exact public build profile",
    )
    build_identity(build["identity"])
    keys(build["executables"], {"base", "head"}, "executables")
    keys(
        build["toolchain_binaries"], {"cargo", "rustc", "rustdoc"}, "toolchain binaries"
    )
    for pin in list(build["executables"].values()) + list(
        build["toolchain_binaries"].values()
    ):
        keys(pin, {"sha256", "bytes"}, "public binary identity")
        require(
            re.fullmatch(r"[0-9a-f]{64}", pin["sha256"]) is not None
            and type(pin["bytes"]) is int
            and pin["bytes"] > 0,
            "Invalid public binary identity",
        )
    keys(
        manifest["original"],
        {
            "plan_sha256",
            "fixture_sha256",
            "overlay_sha256",
            "completed_study_admission_sha256",
            "complete_readback_sha256",
            "records",
        },
        "original provenance",
    )
    keys(manifest["original"]["records"], RECORDS, "original record inventory")
    for pin in manifest["original"]["records"].values():
        keys(pin, {"sha256", "bytes"}, "original record identity")
    keys(
        manifest["portable_fixture"],
        {
            "original_sha256",
            "sha256",
            "bytes",
            "change",
            "case_content_and_order_unchanged",
            "historical_samples_rewritten",
            "existing_owner",
            "source_inputs",
        },
        "portable fixture",
    )
    for row in manifest["portable_fixture"]["source_inputs"]:
        keys(row, {"filename", "url", "sha256", "bytes"}, "public fixture source")
        require(
            row["url"].startswith("https://raw.githubusercontent.com/"),
            "Public pinned fixture URL",
        )
    require(
        len(manifest["portable_fixture"]["source_inputs"]) == 6
        and {row["filename"] for row in manifest["portable_fixture"]["source_inputs"]}
        == FIXTURES,
        "Exact public fixture inventory",
    )
    keys(
        manifest["estimator"],
        {
            "original_source_sha256",
            "strict_guard_sha256",
            "calibration_source_sha256",
            "unchanged_estimator",
            "portable_plan_bridge",
        },
        "estimator",
    )
    require(
        manifest["estimator"]["original_source_sha256"] == ANALYZER_SHA256
        and manifest["estimator"]["strict_guard_sha256"] == GUARD_SHA256
        and manifest["estimator"]["unchanged_estimator"] is True
        and manifest["whole_command_claim"]
        is manifest["new_samples_collected"]
        is False,
        "Exact estimator and evidence-only scope",
    )
    keys(manifest["archive"], {"path", "sha256", "bytes", "members"}, "archive")
    require(
        manifest["archive"]["path"] == "data.tar.gz"
        and set(manifest["archive"]["members"]) == MEMBERS
        and set(manifest["overlays"]) == {"overlays/base.patch", "overlays/head.patch"}
        and manifest["public_provenance_allowlist"] == PROVENANCE,
        "Exact public file/provenance inventory",
    )
    for pin in list(manifest["archive"]["members"].values()) + list(
        manifest["overlays"].values()
    ):
        keys(pin, {"sha256", "bytes"}, "public file identity")
    public_bytes("manifest.json", json.dumps(manifest, sort_keys=True).encode())


def read_file(path: Path) -> bytes:
    require(
        not path.is_symlink() and path.is_file() and path.stat().st_size <= MAX_FILE,
        "Invalid bounded file",
    )
    raw = path.read_bytes()
    require(len(raw) <= MAX_FILE, "Oversized file")
    return raw


def load() -> tuple[dict, dict[str, bytes]]:
    raw = read_file(HERE / "manifest.json")
    require(identity(raw)["sha256"] == MANIFEST_SHA256, "Manifest changed")
    manifest = json.loads(raw)
    public_manifest(manifest)
    compressed = read_file(HERE / "data.tar.gz")
    require(
        identity(compressed)
        == {key: manifest["archive"][key] for key in ("sha256", "bytes")},
        "Archive changed",
    )
    members = {}
    with tarfile.open(fileobj=io.BytesIO(compressed), mode="r:gz") as archive:
        for member in archive:
            require(
                member.isfile()
                and member.name not in members
                and member.name in manifest["archive"]["members"]
                and member.size <= MAX_FILE,
                "Unexpected archive member",
            )
            stream = archive.extractfile(member)
            require(stream is not None, "Missing archive member")
            contents = stream.read(MAX_FILE + 1)
            require(
                identity(contents) == manifest["archive"]["members"][member.name],
                "Archive member changed",
            )
            public_bytes(member.name, contents)
            members[member.name] = contents
    require(
        set(members) == set(manifest["archive"]["members"])
        and sum(map(len, members.values())) <= MAX_EXPANDED,
        "Incomplete or oversized archive",
    )
    return manifest, members


def module(raw: bytes, filename: str, wanted: str) -> types.ModuleType:
    require(identity(raw)["sha256"] == wanted, "Exact estimator source changed")
    value = types.ModuleType(filename.removesuffix(".py"))
    value.__file__ = str(Path("recorded-study") / filename)
    # The recorded estimator is executable only after its exact content hash matches.
    exec(compile(raw, value.__file__, "exec"), value.__dict__)  # noqa: S102
    return value


def calibration_sequence(
    calibration: dict, plan: dict, recorded_identity: dict
) -> tuple[int, list[dict]]:
    policy = plan["recorded_calibration"]
    require(
        (
            policy["minimum_retained_ns"],
            policy["target_ns"],
            policy["maximum_retained_ns"],
            policy["slow_guard_ns"],
            policy["maximum_iterations"],
            policy["maximum_attempts"],
            policy["command_cap_seconds"],
        )
        == (1_000_000, 2_000_000, 10_000_000_000, 6_000_000_000, 1_048_576, 21, 15),
        "Calibration bounds changed",
    )
    initial = {
        (row["case"], row["clock"]): row["iterations"]
        for row in policy["initial_counts"]
    }
    require(
        len(initial) == len(policy["initial_counts"]), "Duplicate calibration start"
    )
    expected = [
        (case, clock)
        for case in plan["statistics"]["first_pass_case_ids"]
        for clock in plan["statistics"]["clock_names"]
    ]
    attempts = iter(calibration["attempts"])
    selected = []
    sequence = 3  # Two complete behavior comparisons precede calibration.
    for case, clock in expected:
        count = initial.get((case, clock), 1)
        chosen = None
        slow_numerator, slow_denominator = 0, 1
        for attempt in range(21):
            row = next(attempts)
            require(
                (row["case"], row["clock"], row["attempt"], row["iterations"])
                == (case, clock, attempt, count),
                "Calibration order or count changed",
            )
            elapsed = []
            for side in ("base", "head"):
                sample = row["samples"][side]
                binding = recorded_identity[side]
                require(
                    sample["schema"] == "uv-pr252-api-sample-v1"
                    and (sample["case"], sample["clock"]) == (case, clock)
                    and sample["census_sha256"] == plan["fixtures"]["census_sha256"]
                    and sample["iterations"] == count
                    and sample["exit_status"] == 0
                    and sample["process_sequence"] is None
                    and sample["global_command_sequence"] == sequence
                    and sample["executable_sha256"] == binding["executable_sha256"]
                    and sample["overlaid_tree"] == binding["overlaid_tree"]
                    and type(sample["elapsed_ns"]) is int
                    and sample["elapsed_ns"] > 0
                    and sample["included_checksum"]
                    == sample["included_per_iteration"] * count,
                    "Calibration sample changed",
                )
                elapsed.append(sample["elapsed_ns"])
                sequence += 1
            for key in (
                "iterations",
                "queries_per_iteration",
                "included_per_iteration",
                "included_checksum",
            ):
                require(
                    row["samples"]["base"][key] == row["samples"]["head"][key],
                    "Calibration semantic work changed",
                )
            slow = max(elapsed)
            if slow * slow_denominator > slow_numerator * count:
                slow_numerator, slow_denominator = slow, count
            if all(1_000_000 <= value <= 10_000_000_000 for value in elapsed):
                chosen = count
            if (
                all(value >= 2_000_000 for value in elapsed)
                or count == 1_048_576
                or any(value >= 10_000_000_000 for value in elapsed)
            ):
                break
            candidate = count * 2
            if candidate * slow_numerator * 5 > 6_000_000_000 * slow_denominator * 4:
                break
            count = candidate
        require(chosen is not None, "Uncalibrated planned contrast")
        selected.append({"case": case, "clock": clock, "iterations": chosen})
    require(
        next(attempts, None) is None and selected == calibration["selected"],
        "Calibration selection changed",
    )
    return sequence, selected


def replay(manifest: dict, members: dict[str, bytes]) -> bytes:
    public_manifest(manifest)
    require(
        set(members) == set(manifest["archive"]["members"]), "Incomplete member set"
    )
    for name, expected in manifest["archive"]["members"].items():
        require(identity(members[name]) == expected, "Evidence member changed: " + name)
        public_bytes(name, members[name])
    for name, expected in manifest["original"]["records"].items():
        require(identity(members[name]) == expected, "Historical record changed")
    public_records(members)
    plan = json.loads(members["plan.json"])
    require(
        plan["source_pair"] == manifest["source_pair"]
        and plan["original_plan_sha256"] == manifest["original"]["plan_sha256"],
        "Measured source pair or plan changed",
    )
    analyzer = module(members["analyze_pairs.py"], "analyze_pairs.py", ANALYZER_SHA256)
    guard = module(
        members["analyze_pairs_guard.py"], "analyze_pairs_guard.py", GUARD_SHA256
    )
    directory = Path("recorded-study")
    analyzer.HERE = directory
    snapshots = {directory / name: value for name, value in members.items()}

    def frozen_read(path: Path) -> dict:
        require(path in snapshots, "Unexpected estimator read")
        return json.loads(snapshots[path])

    def frozen_digest(path: Path) -> str:
        require(path in snapshots, "Unexpected estimator digest")
        return (
            manifest["original"]["plan_sha256"]
            if path == directory / "plan.json"
            else identity(snapshots[path])["sha256"]
        )

    analyzer.read, analyzer.digest = frozen_read, frozen_digest
    aa, ab, design = (
        json.loads(members[name])
        for name in ("aa.json", "ab.json", "fixed-design.json")
    )
    require(
        aa["identity"] == ab["identity"] == manifest["build"]["identity"],
        "Build identity changed",
    )
    sequence, selected = calibration_sequence(
        json.loads(members["calibration.json"]), plan, aa["identity"]
    )
    require(
        [(r["case"], r["clock"], r["iterations"]) for r in aa["contrasts"]]
        == [(r["case"], r["clock"], r["iterations"]) for r in selected],
        "A/A calibration differs",
    )
    for observations in (aa, ab):
        if observations is ab:
            sequence += 1  # The sealed design is a separate analysis command.
        for entry in observations["contrasts"]:
            for block in entry["blocks"]:
                for arm in block["observed_order"]:
                    require(
                        block["samples"][arm]["global_command_sequence"] == sequence,
                        "Global command order changed",
                    )
                    sequence += 1
    guard.validate_fixed_design(design, analyzer.contrasts(plan))

    def analyze(mode: str) -> bytes:
        args = [
            "analyze_pairs.py",
            mode,
            str(directory / ("aa.json" if mode == "choose" else "ab.json")),
        ]
        if mode == "report":
            args.extend(["--design", str(directory / "fixed-design.json")])
        previous, output = sys.argv, io.StringIO()
        try:
            sys.argv = args
            with contextlib.redirect_stdout(output):
                analyzer.main()
        finally:
            sys.argv = previous
        return output.getvalue().encode()

    require(
        analyze("choose") == members["fixed-design.json"], "Fixed design did not replay"
    )
    report = analyze("report")
    require(
        report == members["report.json"]
        and sequence == manifest["command_count"] == 1294,
        "Complete report/command count did not replay",
    )
    return report


def main() -> None:
    manifest, members = load()
    sys.stdout.buffer.write(replay(manifest, members))


if __name__ == "__main__":
    main()
