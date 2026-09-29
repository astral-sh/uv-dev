"""Offline reconstruction of a source-bound resolver-error benchmark export."""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import math
import random
import re
import stat
import statistics
import tarfile
from pathlib import Path

SCHEMA = "uv-future.resolver-error-portable-manifest.v3"
READER_SHA = "6d720b09fde5f3d1287b05f8393a61b23c4b928c9b6bd914cf07da0062c872d5"
PHASES = ("materialize", "acquire", "build", "specimen", "timing")
RAW_NAMES = ("sample.json", "estimates.json", "benchmark.json", "tukey.json")
SOURCE_ROWS = (
    ("resolver33", "665b7226e7ebfb7d02cb11d0932887c051a8bcb4", "2b90709abf0eeac276e4b2e8b124339a533b8f48", "27bda345089dd3ef623d23840e9aece778043e21e4cce563c7d77b8d34933a1d"),
    ("resolver34", "30f87e00463fee882a09397804da77d0a06eb0a5", "bb27607c2a32ababdc3edb8a3d495fdba059ffb3", "27bda345089dd3ef623d23840e9aece778043e21e4cce563c7d77b8d34933a1d"),
    ("resolver35", "81e7621b79d985c3999e872e9655fea0c0f6bf03", "1f03139993b6a2a8cf4afe1eccf5cae0a578add0", "dc800b15e7c470073b8f46f74106413cda5a61c8d1194609ed1f570cfd8f1944"),
)
SOURCES = {role: {"head": head, "tree": tree, "overlay_sha256": overlay} for role, head, tree, overlay in SOURCE_ROWS}
REAL_CASES = tuple(f"resolver_errors/{clock}/{fixture}" for fixture in ("rooster", "httpx", "numpy") for clock in ("render", "resolve_and_render"))
STRESS_CASES = {
    1785: ("comb_plain_32", "comb_plain_1024", "balanced_plain_1024", "comb_excluded_256", "balanced_excluded_256"),
    1788: ("comb_plain_32", "shared_identified_12", "shared_identified_16", "shared_unidentified_10"),
}
FIXTURES = [
    {"name": "rooster", "python": "3.10", "requirements": ["rooster-blue"], "size": "small"},
    {"name": "httpx", "python": "3.6", "requirements": ["httpx>=0.23"], "size": "medium"},
    {"name": "numpy", "python": "3.7", "requirements": ["numpy>=1.22"], "size": "large"},
]
METHOD = {
    "builds": 6, "specimens": 8, "timed_processes": 96, "matched_pairs": 48,
    "case_observations": 504, "samples_per_case": 20, "pairs_per_endpoint": 12,
    "resamples": 20000, "confidence": 0.95, "ratio": "candidate/parent",
    "estimator": "geometric mean of matched per-process median ratios",
    "interval": "pointwise percentile paired bootstrap over log ratios",
    "seed": "PR number independently for each endpoint",
    "real_render_clock": "Fresh offline resolution is outside the timer; first Display and report/error destruction are inside.",
    "real_resolve_and_render_clock": "Offline resolution, first Display, and report/error destruction are inside.",
    "stress_clock": "Fixture and fresh-error construction are outside the timer; first Display, rendered hints, and output/error destruction are inside.",
    "scope": "No successful-resolution, network, end-to-end CLI, or whole-application claim; no real/stress aggregate.",
}
PROFILE = {"name": "profiling", "source_requested": {"inherits": "release", "strip": False, "debug": "full", "lto": False, "command_line_debug": 0}, "build_environment": {"incremental": False, "jobs": 2, "offline": True}, "effective_artifact_fields": ["opt_level", "debuginfo", "debug_assertions", "overflow_checks", "test"]}
PAYLOAD = frozenset({"README.md", "analyze.py", "analysis.json", "specimens.json", "criterion-raw.tar.gz", "overlays/resolver33-34.patch", "overlays/resolver35.patch"})
MAX_MEMBER = 1024 * 1024
MAX_RAW = 128 * 1024 * 1024
MAX_ARCHIVE = 64 * 1024 * 1024
MAX_JSON = 16 * 1024 * 1024


def require(value, message):
    if not value:
        raise RuntimeError(message)


def canonical_json(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def file_identity(data):
    return {"sha256": digest(data), "bytes": len(data)}


def hash_identity(value):
    require(isinstance(value, dict) and set(value) == {"sha256", "bytes"} and isinstance(value["sha256"], str) and re.fullmatch(r"[0-9a-f]{64}", value["sha256"]) and type(value["bytes"]) is int and value["bytes"] > 0, "invalid content identity")
    return value


def parse_json(data):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, "duplicate JSON key")
            result[key] = value
        return result
    require(isinstance(data, bytes) and len(data) <= MAX_JSON, "JSON size differs")
    return json.loads(data, object_pairs_hook=pairs, parse_constant=lambda _: (_ for _ in ()).throw(RuntimeError("non-finite JSON")))


def read_regular(path, maximum):
    path = Path(path)
    info = path.lstat()
    require(path.is_absolute() and path.resolve(strict=True) == path and stat.S_ISREG(info.st_mode) and not path.is_symlink() and info.st_size <= maximum, "noncanonical or oversized input")
    data = path.read_bytes()
    require(len(data) == info.st_size, "input changed during read")
    return data


def timing_rows():
    result = []
    for pr, parent, candidate in ((1785, "resolver33", "resolver34"), (1788, "resolver34", "resolver35")):
        for kind in ("real", "stress"):
            for pair in range(1, 13):
                for role in (("parent", "candidate") if pair % 2 else ("candidate", "parent")):
                    result.append({"cases": list(REAL_CASES if kind == "real" else STRESS_CASES[pr]), "kind": kind, "label": f"pr{pr}-{kind}-pair{pair:02d}-{role}", "mode": "measure", "pair": pair, "pr": pr, "role": role, "source": parent if role == "parent" else candidate, "suite": "real" if kind == "real" else "exclude_newer" if pr == 1785 else "shared_derivation"})
    return result


def finite(value, *, positive=False):
    return type(value) in (int, float) and math.isfinite(value) and (value > 0 if positive else value >= 0)


def close(actual, expected):
    return type(actual) in (int, float) and math.isfinite(actual) and math.isclose(actual, expected, rel_tol=1e-10, abs_tol=1e-8)


def percentile(values, fraction):
    ordered = sorted(values)
    rank = fraction * (len(ordered) - 1)
    lower = math.floor(rank)
    upper = math.ceil(rank)
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (rank - lower)


def validate_samples(sample, estimates, benchmark, tukey, case):
    group, function, project = case.split("/")
    require(benchmark == {"group_id": group, "function_id": function, "value_str": project, "throughput": None,
        "full_id": case, "directory_name": case, "title": case}, "Criterion benchmark identity differs")
    require(isinstance(sample, dict) and set(sample) == {"sampling_mode", "iters", "times"}
        and sample["sampling_mode"] in {"Linear", "Flat"} and len(sample["iters"]) == len(sample["times"]) == 20
        and all(finite(item, positive=True) and item == int(item) for item in sample["iters"])
        and all(finite(item, positive=True) for item in sample["times"]), "Criterion sample is incomplete or invalid")
    counts, times = sample["iters"], sample["times"]
    first = counts[0]
    require(counts == ([first] * 20 if sample["sampling_mode"] == "Flat" else [first * index for index in range(1, 21)]), "Criterion iteration schedule differs")
    values = [elapsed / count for elapsed, count in zip(times, counts, strict=True)]
    require(isinstance(estimates, dict) and set(estimates) == {"mean", "median", "median_abs_dev", "slope", "std_dev"}
        and (estimates["slope"] is None) == (sample["sampling_mode"] == "Flat"), "Criterion estimates or sampling mode differ")
    for name, estimate in estimates.items():
        if estimate is None:
            require(name == "slope", "missing Criterion estimate")
            continue
        require(set(estimate) == {"confidence_interval", "point_estimate", "standard_error"}
            and finite(estimate["point_estimate"], positive=name in {"mean", "median", "slope"}) and finite(estimate["standard_error"]), "invalid Criterion estimate")
        interval = estimate["confidence_interval"]
        require(set(interval) == {"confidence_level", "lower_bound", "upper_bound"}
            and interval["confidence_level"] == 0.95 and finite(interval["lower_bound"])
            and finite(interval["upper_bound"]) and interval["lower_bound"] <= interval["upper_bound"], "invalid Criterion confidence interval")
    median = percentile(values, 0.5)
    require(close(estimates["median"]["point_estimate"], median), "Criterion median is not derived from raw per-operation samples")
    q1, q3 = percentile(values, 0.25), percentile(values, 0.75)
    iqr = q3 - q1
    fences = [q1 - 3 * iqr, q1 - 1.5 * iqr, q3 + 1.5 * iqr, q3 + 3 * iqr]
    require(isinstance(tukey, list) and len(tukey) == 4 and all(close(item, wanted) for item, wanted in zip(tukey, fences, strict=True)), "Criterion outlier fences differ")
    outliers = {name: 0 for name in ("low_severe", "low_mild", "ordinary", "high_mild", "high_severe")}
    for item in values:
        label = "low_severe" if item < fences[0] else "high_severe" if item > fences[3] else "low_mild" if item < fences[1] else "high_mild" if item > fences[2] else "ordinary"
        outliers[label] += 1
    mean = statistics.fmean(values)
    return {"case": case, "sampling_mode": sample["sampling_mode"], "sample_count": 20, "total_iterations": sum(counts),
        "total_measured_ns": sum(times), "median_ns": median, "mean_ns": mean, "standard_deviation_ns": statistics.stdev(values),
        "relative_standard_deviation": statistics.stdev(values) / mean, "q1_ns": q1, "q3_ns": q3, "min_ns": min(values), "max_ns": max(values),
        "outliers": outliers, "criterion_median_interval": estimates["median"]["confidence_interval"]}


def raw_cases(row, files):
    results = {}
    for case in row["cases"]:
        sample, estimates, benchmark, tukey = [parse_json(files[f"{row['label']}/{case}/new/{name}"]) for name in RAW_NAMES]
        if row["kind"] == "real":
            result = validate_samples(sample, estimates, benchmark, tukey, case)
        else:
            require(benchmark == {"group_id": case, "function_id": None, "value_str": None, "throughput": None, "full_id": case, "directory_name": case, "title": case}, "stress benchmark identity differs")
            mapped = "stress/report/" + case
            synthetic = {"group_id": "stress", "function_id": "report", "value_str": case, "throughput": None, "full_id": mapped, "directory_name": mapped, "title": mapped}
            result = validate_samples(sample, estimates, synthetic, tukey, mapped) | {"case": case}
        results[case] = result
    return {"cases": results}


def paired_analysis(measurements):
    rows = timing_rows()
    require(set(measurements) == {row["label"] for row in rows}, "complete timing inventory required")
    result = {}
    for pr in (1785, 1788):
        result[str(pr)] = {}
        for kind in ("real", "stress"):
            selected = [row for row in rows if row["pr"] == pr and row["kind"] == kind]
            result[str(pr)][kind] = {}
            for case in selected[0]["cases"]:
                pairs = []
                for index in range(0, len(selected), 2):
                    pair = selected[index:index+2]
                    values = {row["role"]: measurements[row["label"]]["cases"][case]["median_ns"] for row in pair}
                    pairs.append({"pair": pair[0]["pair"], "order": [row["role"] for row in pair], "parent_ns": values["parent"], "candidate_ns": values["candidate"], "ratio": values["candidate"] / values["parent"]})
                logs = [math.log(item["ratio"]) for item in pairs]
                rng = random.Random(pr)
                samples = sorted(math.exp(statistics.fmean(rng.choices(logs, k=12))) for _ in range(20000))
                result[str(pr)][kind][case] = {"pairs": pairs, "geometric_mean_ratio": math.exp(statistics.fmean(logs)), "paired_bootstrap": {"seed": pr, "resamples": 20000, "confidence": .95, "lower": percentile(samples, .025), "upper": percentile(samples, .975)}}
    return result


def classify(analysis):
    result = []
    for pr in (1785, 1788):
        for kind, cases in (("real", REAL_CASES), ("stress", STRESS_CASES[pr])):
            require(set(analysis[str(pr)][kind]) == set(cases), "endpoint inventory differs")
            for case in cases:
                value = analysis[str(pr)][kind][case]
                bounds = value["paired_bootstrap"]
                result.append({"pr": pr, "kind": kind, "case": case, "geometric_mean_ratio": value["geometric_mean_ratio"], "lower": bounds["lower"], "upper": bounds["upper"], "classification": "faster" if bounds["upper"] < 1 else "slower" if bounds["lower"] > 1 else "inconclusive"})
    require(len(result) == 21, "complete endpoint classification required")
    return result


def expected_raw_paths():
    return {f"{row['label']}/{case}/new/{name}" for row in timing_rows() for case in row["cases"] for name in RAW_NAMES}


def read_archive(data, expected):
    require(len(data) <= MAX_ARCHIVE and set(expected) == expected_raw_paths() and len(expected) == 2016, "raw archive inventory differs")
    result = {}
    total = 0
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        for member in archive:
            require(member.isfile() and member.name in expected and member.name not in result and not member.pax_headers and member.size <= MAX_MEMBER and member.uid == member.gid == member.mtime == 0 and member.uname == member.gname == "" and member.mode == 0o644, "unexpected raw archive member")
            total += member.size
            require(total <= MAX_RAW, "raw archive exceeds bound")
            stream = archive.extractfile(member)
            require(stream is not None, "raw member unreadable")
            value = stream.read(MAX_MEMBER + 1)
            require(file_identity(value) == hash_identity(expected[member.name]), "raw file changed")
            parse_json(value)
            result[member.name] = value
    require(set(result) == set(expected), "missing raw archive member")
    return result


def validate_specimens(value):
    expected = {f"specimen-pr{pr}-{role}-{kind}" for pr in (1785, 1788) for role in ("parent", "candidate") for kind in ("real", "stress")}
    require(isinstance(value, dict) and set(value) == expected, "full specimen inventory differs")
    for pr in (1785, 1788):
        for kind in ("real", "stress"):
            left, right = [value[f"specimen-pr{pr}-{role}-{kind}"] for role in ("parent", "candidate")]
            require(left == right, "full parent/candidate report or hints differ")
            if kind == "real":
                require(set(left) == {"mode", "observations"} and left["mode"] == "inspect" and len(left["observations"]) == 3, "real specimen envelope differs")
                for item, fixture in zip(left["observations"], FIXTURES, strict=True):
                    require(set(item) == {"fixture", "report", "hints"} and item["fixture"] == fixture and isinstance(item["report"], str) and "unsatisfiable" in item["report"] and isinstance(item["hints"], list) and all(isinstance(x, str) for x in item["hints"]), "full real diagnostic differs")
            else:
                require(set(left) == {"schema", "suite", "records"} and left["schema"] == "uv-future.no-solution-report-specimen.v1" and left["suite"] == ("exclude_newer" if pr == 1785 else "shared_derivation") and [x["case"] for x in left["records"]] == list(STRESS_CASES[pr]), "stress specimen inventory differs")
                for item in left["records"]:
                    require(set(item) == {"case", "input", "entrypoint", "fresh_cache", "cache_populated", "report", "hints"} and item["entrypoint"] == "NoSolutionError::fmt" and item["fresh_cache"] is True and item["cache_populated"] is True and isinstance(item["report"], str) and item["report"] and isinstance(item["hints"], list) and all(isinstance(x, str) for x in item["hints"]), "full stress diagnostic differs")
    return value


def content_scan(data):
    text = data.decode("utf-8")
    require(not re.search(r"(?i)(/Users/|/home/|file://|[a-z0-9]+-uv-dev(?:-[0-9]+)?|devbox-[0-9]|https?://[^/\s]+@|gh[pousr]_[A-Za-z0-9]{20}|github_pat_|-----BEGIN [A-Z ]*PRIVATE KEY-----)", text), "content requires private-information review")


BACKGROUND_POLICY = {"sha256": "3bdc0a399f05b7a78a9a533de8f883ed8af103f0a2420133f64ac5dd593316d1", "bytes": 12583}
BACKGROUND_METHOD = {
    "schema": "uv-future.resolver-error-background-method.v1",
    "environment": "The independently identified Git fsmonitor remains running.",
    "observation_boundary": "In the existing before/after-pair scheduling records, outside both native commands and their Criterion elapsed-time measurements.",
    "selection": "The original balanced schedule and every complete pair are retained; activity covariates never select, remove, top up, or reweight samples.",
    "conclusion_limit": "Effects apply to this host with the identified background service running; neither a quiet host nor absence of delayed cache or I/O effects is established.",
    "process_accounting_limit": "Pair-interval counters include the existing validation overhead. RUSAGE_CHILDREN is cumulative runner-child accounting, not isolated native-command CPU accounting.",
    "service_action_authorized": False,
    "publication_authorized": False,
}
BACKGROUND_CPU = ("utime_ticks", "stime_ticks", "cutime_ticks", "cstime_ticks")
BACKGROUND_IO = ("rchar", "wchar", "syscr", "syscw", "read_bytes", "write_bytes", "cancelled_write_bytes")
BACKGROUND_RUNNER_CPU = ("self_user_seconds", "self_system_seconds", "children_user_seconds", "children_system_seconds")


def background_observation(value):
    fields = {"position_ns", "duration_ns", "service_cpu_ticks", "service_io", "runner_cpu_seconds", "runner_io", "load_average"}
    require(type(value) is dict and set(value) == fields
        and all(type(value[k]) is int and value[k] >= 0 for k in ("position_ns", "duration_ns")),
        "public background observation differs")
    for key, names, integers in (("service_cpu_ticks", BACKGROUND_CPU, True), ("service_io", BACKGROUND_IO, True),
                                 ("runner_cpu_seconds", BACKGROUND_RUNNER_CPU, False), ("runner_io", BACKGROUND_IO, True)):
        row = value[key]
        require(type(row) is dict and set(row) == set(names)
            and all((type(x) is int and x >= 0) if integers else
                    (type(x) in (int, float) and math.isfinite(x) and x >= 0) for x in row.values()),
            "public background counters differ")
    require(type(value["load_average"]) is list and len(value["load_average"]) == 3
        and all(type(x) in (int, float) and math.isfinite(x) and x >= 0 for x in value["load_average"]),
        "public background load differs")
    return value


def background_interval(before, after):
    background_observation(before); background_observation(after)
    elapsed = after["position_ns"] - before["position_ns"] - before["duration_ns"]
    require(elapsed >= 0, "public background observations overlap")
    def difference(key):
        result = {name: after[key][name] - before[key][name] for name in before[key]}
        require(all(type(x) in (int, float) and math.isfinite(x) and x >= 0 for x in result.values()),
            "public cumulative background counter decreased")
        return result
    return {"between_observations_ns": elapsed, "service_cpu_ticks": difference("service_cpu_ticks"),
        "service_io": difference("service_io"), "runner_cpu_seconds": difference("runner_cpu_seconds"),
        "runner_io": difference("runner_io"), "sample_selection_effect": "none"}


def validate_background(value):
    require(type(value) is dict and set(value) == {"schema", "policy", "method", "clock_ticks_per_second", "initial", "pairs"}
        and value["schema"] == "uv-future.resolver-error-portable-background.v1"
        and value["policy"] == BACKGROUND_POLICY and value["method"] == BACKGROUND_METHOD
        and type(value["clock_ticks_per_second"]) is int and value["clock_ticks_per_second"] > 0
        and type(value["pairs"]) is list and len(value["pairs"]) == 48,
        "public background provenance differs")
    first = background_observation(value["initial"])
    require(first["position_ns"] == 0, "public background origin differs")
    previous = first
    rows = timing_rows()
    for index, pair in enumerate(value["pairs"]):
        require(type(pair) is dict and set(pair) == {"labels", "before", "after", "interval"}
            and pair["labels"] == [row["label"] for row in rows[2 * index:2 * index + 2]],
            "complete public background pair inventory differs")
        before, after = background_observation(pair["before"]), background_observation(pair["after"])
        require(previous["position_ns"] + previous["duration_ns"] <= before["position_ns"]
            and pair["interval"] == background_interval(before, after),
            "public background order or derived interval differs")
        previous = after
    return value


def validate_admission(value):
    require(isinstance(value, dict) and set(value) == {"reader_sha256", "phases"} and value["reader_sha256"] == READER_SHA and set(value["phases"]) == set(PHASES), "final-reader provenance differs")
    for phase in PHASES:
        require(set(value["phases"][phase]) == {"terminal", "result"}, "phase provenance differs")
        for record in value["phases"][phase].values():
            hash_identity(record)


def validate_provenance(value):
    require(set(value) == {"machine", "toolchain", "profile", "artifacts", "fixtures", "background"}, "provenance fields differ")
    machine, toolchain = value["machine"], value["toolchain"]
    require(set(machine) == {"system", "machine", "release", "source_sha256", "hardware"} and machine["system"] == "Linux" and machine["machine"] == "x86_64" and machine["source_sha256"] == "9d01d656085a8e885dc61ddc1a653fcddc17b51b24fd23a0cfc8decd6a1ee98e", "machine provenance differs")
    hardware = machine["hardware"]
    require(set(hardware) == {"observed", "cpu_model", "logical_cpus", "memory_bytes", "affinity_cpus"} and hardware["observed"] == "after-final-admission" and isinstance(hardware["cpu_model"], str) and 0 < len(hardware["cpu_model"]) <= 160 and all(type(hardware[k]) is int and hardware[k] > 0 for k in ("logical_cpus", "memory_bytes", "affinity_cpus")) and hardware["affinity_cpus"] <= hardware["logical_cpus"], "actual hardware observation required")
    require(set(toolchain) == {"name", "cargo_version", "rustc_verbose", "cargo", "rustc", "source_sha256"} and toolchain["name"] == "1.98.1-x86_64-unknown-linux-gnu" and toolchain["source_sha256"] == machine["source_sha256"] and toolchain["cargo_version"].startswith("cargo 1.98.1 ") and toolchain["rustc_verbose"].startswith("rustc 1.98.1 "), "toolchain provenance differs")
    hash_identity(toolchain["cargo"]); hash_identity(toolchain["rustc"])
    require(value["profile"] == PROFILE and set(value["artifacts"]) == {f"{role}-{kind}" for role in SOURCES for kind in ("stress", "real")}, "six-artifact/profile provenance differs")
    for label, item in value["artifacts"].items():
        role, kind = label.rsplit("-", 1)
        require(set(item) == {"source", "kind", "executable", "package", "target", "profile", "features", "manifest_sha256", "cargo_lock_sha256", "overlay_commit", "overlay_tree"} and item["source"] == role and item["kind"] == kind and set(item["executable"]) == {"sha256", "bytes", "mode", "format"} and item["executable"]["format"] == "ELF-x86_64" and type(item["executable"]["mode"]) is int and item["executable"]["mode"] & 0o100 and item["package"] == {"name": "uv-resolver" if kind == "stress" else "uv-bench", "version": "0.0.80"} and item["target"] == {"name": "uv_resolver" if kind == "stress" else "resolver_errors", "kind": ["lib"] if kind == "stress" else ["bench"]} and isinstance(item["features"], list) and all(isinstance(x, str) and re.fullmatch(r"[a-zA-Z0-9_+.-]+", x) for x in item["features"]), "actual Cargo artifact identity differs")
        hash_identity({k: item["executable"][k] for k in ("sha256", "bytes")})
        require(item["profile"].get("opt_level") == "3" and item["profile"].get("debuginfo") in (0, None) and item["profile"].get("debug_assertions") is False and item["profile"].get("overflow_checks") is False and item["profile"].get("test") is True and set(item["profile"]) == {"opt_level", "debuginfo", "debug_assertions", "overflow_checks", "test"}, "actual profiling artifact differs")
        require(all(re.fullmatch(r"[0-9a-f]{64}", item[k]) for k in ("manifest_sha256", "cargo_lock_sha256")) and all(re.fullmatch(r"[0-9a-f]{40}", item[k]) for k in ("overlay_commit", "overlay_tree")), "artifact source hashes differ")
    fixture = value["fixtures"]
    require(set(fixture) == {"owner_commit", "cutoff", "python_platform", "requirements", "interpreter", "seed_cache", "environment", "runtime_cache", "common_sha256", "fixture_sha256", "harness_sha256"} and fixture["owner_commit"] == "2db64d33fc577a4b31b71eb361e0b13ebfda0290" and fixture["cutoff"] == "2024-12-01T00:00:00Z" and fixture["python_platform"] == "aarch64-apple-darwin" and [x["fixture"] for x in fixture["requirements"]] == FIXTURES, "real fixture provenance differs")
    for item in fixture["requirements"]:
        require(set(item) == {"fixture", "content"}, "requirement provenance differs")
        require(item["content"] == file_identity(("\n".join(item["fixture"]["requirements"]) + "\n").encode()), "requirement content differs")
    require(set(fixture["interpreter"]) == {"sha256", "bytes", "version"} and fixture["interpreter"]["version"] == [3, 12, 3], "interpreter provenance differs")
    hash_identity({k: fixture["interpreter"][k] for k in ("sha256", "bytes")})
    for name in ("seed_cache", "environment", "runtime_cache"):
        require(set(fixture[name]) == {"sha256", "entries"} and re.fullmatch(r"[0-9a-f]{64}", fixture[name]["sha256"]) and type(fixture[name]["entries"]) is int and fixture[name]["entries"] > 0, "fixture inventory digest differs")
    require(fixture["common_sha256"] == "25898599c3b2ec5f8bc9fa14c0628fea99ac8793e9c6ad146823119ef0b6ee44" and fixture["fixture_sha256"] == "efd917ca13d8bf2bc39627b7993a91c974d8c5da09f97f7a460d2aa6158cf86b" and fixture["harness_sha256"] == "9507be60903043bbd438635349150567fa652f0d966139fe3034b3407d3e008c", "fixture source differs")
    validate_background(value["background"])
    content_scan(canonical_json(value))


def admit_portable(bundle: Path, expected_manifest_sha256: str) -> dict:
    """Verify an externally pinned export; no products or network are used."""
    bundle = Path(bundle)
    require(bundle.is_absolute() and bundle.resolve(strict=True) == bundle and bundle.is_dir() and not bundle.is_symlink(), "canonical bundle required")
    data = read_regular(bundle / "manifest.json", MAX_JSON)
    require(re.fullmatch(r"[0-9a-f]{64}", expected_manifest_sha256 or "") and digest(data) == expected_manifest_sha256, "externally pinned manifest required")
    manifest = parse_json(data)
    require(set(manifest) == {"schema", "sources", "method", "admission", "provenance", "schedule", "raw_files", "payload"} and manifest["schema"] == SCHEMA and manifest["sources"] == SOURCES and manifest["method"] == METHOD and manifest["schedule"] == timing_rows() and set(manifest["payload"]) == PAYLOAD, "portable manifest contract differs")
    validate_admission(manifest["admission"]); validate_provenance(manifest["provenance"])
    actual_paths = set()
    for path in bundle.rglob("*"):
        info = path.lstat()
        require(not path.is_symlink() and path.resolve(strict=True) == path, "bundle contains a symlink")
        if stat.S_ISREG(info.st_mode):
            actual_paths.add(path.relative_to(bundle).as_posix())
        else:
            require(stat.S_ISDIR(info.st_mode), "bundle contains a nonregular member")
    require(actual_paths == PAYLOAD | {"manifest.json"}, "unlisted bundle member")
    payload = {}
    for name, expected in manifest["payload"].items():
        data = read_regular(bundle / name, MAX_ARCHIVE if name.endswith(".tar.gz") else MAX_JSON)
        require(file_identity(data) == hash_identity(expected), "portable payload differs")
        if not name.endswith(".tar.gz") and name != "analyze.py":
            content_scan(data)
        payload[name] = data
    require(digest(payload["analyze.py"]) == digest(read_regular(Path(__file__).resolve(), MAX_JSON)), "running analyzer differs from pinned payload")
    require(digest(payload["overlays/resolver33-34.patch"]) == SOURCE_ROWS[0][3] and digest(payload["overlays/resolver35.patch"]) == SOURCE_ROWS[2][3], "measured overlay bytes differ")
    raw = read_archive(payload["criterion-raw.tar.gz"], manifest["raw_files"])
    measurements = {row["label"]: raw_cases(row, raw) for row in timing_rows()}
    analysis = paired_analysis(measurements)
    endpoints = classify(analysis)
    exported = parse_json(payload["analysis.json"])
    require(exported == {"schema": "uv-future.resolver-error-portable-analysis.v1", "analysis": analysis, "endpoints": endpoints}, "raw reconstruction differs from admitted analysis")
    validate_specimens(parse_json(payload["specimens.json"]))
    return {"schema": "uv-future.resolver-error-portable-admission.v3", "accepted": True, "manifest_sha256": expected_manifest_sha256, "analysis": analysis, "endpoints": endpoints, "admission": manifest["admission"], "provenance": manifest["provenance"], "sources": manifest["sources"], "method": manifest["method"], "publication_authorized": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    parser.add_argument("--manifest-sha256", required=True)
    args = parser.parse_args()
    print(json.dumps(admit_portable(args.bundle, args.manifest_sha256), indent=2, sort_keys=True, allow_nan=False))


if __name__ == "__main__":
    main()

