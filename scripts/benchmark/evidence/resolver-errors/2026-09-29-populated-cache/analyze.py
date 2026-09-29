"""Offline reconstruction of the populated-cache Python-conflict measurements."""

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

SCHEMA = "uv-future.resolver-error-populated-portable-manifest.v1"
READER_SHA = "5275fbaf3945ac0e1c8f60347e7c814bbe513e6113d0bcc19a8b1299c1e62cbe"
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
METHOD = {'builds': 6, 'specimens': 4, 'timed_processes': 48, 'matched_pairs': 24, 'case_observations': 288, 'samples_per_case': 20, 'pairs_per_endpoint': 12, 'resamples': 20000, 'confidence': 0.95, 'ratio': 'candidate/parent', 'estimator': 'geometric mean of matched per-process median ratios', 'interval': 'pointwise percentile paired bootstrap over log ratios', 'seed': 'PR number independently for each endpoint', 'real_render_clock': 'Fresh offline resolution is outside the timer; first Display and report/error destruction are inside.', 'real_resolve_and_render_clock': 'Offline resolution, first Display, and report/error destruction are inside.', 'scope': 'No successful-resolution, network, end-to-end CLI, whole-application, or pooled historical/populated-cache claim.', 'new_builds': 0, 'executed_binaries': 3}
PROFILE = {"name": "profiling", "source_requested": {"inherits": "release", "strip": False, "debug": "full", "lto": False, "command_line_debug": 0}, "build_environment": {"incremental": False, "jobs": 2, "offline": True}, "effective_artifact_fields": ["opt_level", "debuginfo", "debug_assertions", "overflow_checks", "test"]}
PAYLOAD = frozenset({"README.md", "analyze.py", "analysis.json", "specimens.json", "criterion-raw.tar.gz", "overlays/resolver33-34.patch", "overlays/resolver35.patch"})
MAX_MEMBER = 1024 * 1024
MAX_RAW = 128 * 1024 * 1024
MAX_ARCHIVE = 64 * 1024 * 1024
MAX_JSON = 16 * 1024 * 1024

ORIGINAL_MANIFEST = {'sha256': 'adcb592c484a575cba57509a6702eebf76989866552d2f250029a8a7660aaee8', 'bytes': 628946}
SHARED_PROVENANCE_SHA = '4ff45f20e8067ee4fcd7b520212312fd2cae81bf5b80961494fb696e2fcde9aa'
FIXTURE_PROVENANCE_SHA = '04e306ec94c2d94944a8fb71f2e91a516de4679045bb85d2680bed2630102911'
EXPECTED_ADMISSION = {'reader_sha256': '5275fbaf3945ac0e1c8f60347e7c814bbe513e6113d0bcc19a8b1299c1e62cbe', 'phases': {'materialize': {'terminal': {'sha256': '28849a9dc6d1c6cad89633a4164a0ff003ba3b0906896610a6f8afe0386f6597', 'bytes': 304}, 'result': {'sha256': 'ea8d8326130b4926bb5a2e5c377dd88879945ed8abd80ff6631dddda2d854d1b', 'bytes': 21145}}, 'acquire': {'terminal': {'sha256': '2457d6ab6105f56baf37f60d61ca5b66fa4aae0b1065e99ebbd3e3efbc5791d8', 'bytes': 309}, 'result': {'sha256': 'fb9a7ec63b97f66e5f8d7049f2b6b1856cad7f2f4ae13dc219c09e4228d0b78c', 'bytes': 50891}}, 'build': {'terminal': {'sha256': '53380dec353d11e2bdc1035a054e0b1eae44ecdfe92466be2519417c561b473c', 'bytes': 292}, 'result': {'sha256': '0cb0f8ee621ed3f7b8f7b25a08ca8d180caa5d2b90b55b87c9d8f0abd7610c44', 'bytes': 42510}}, 'specimen': {'terminal': {'sha256': '6972d5f442f5d608b3c1cae381fcb9c4a55e1808923f745700c93189f0392bc7', 'bytes': 311}, 'result': {'sha256': 'c2de8ff8cd2ff1feb4c76c674d786cd44ab3f37d88318a083449e29fa6e2d58b', 'bytes': 96966}}, 'timing': {'terminal': {'sha256': 'd125b14cd2fede55b4a447b61f659b4d5170866ecfae04715369215ec8267364', 'bytes': 309}, 'result': {'sha256': '9a91e880966b402bae37a3d66acdba26631f10d71a22771f1e66fc9c32e06b03', 'bytes': 1340660}}}}
RULES = {
    "rooster": {"root": "rooster-blue", "required_python_floor": "3.11", "root_requirement": "rooster-blue"},
    "httpx": {"root": "httpx", "required_python_floor": "3.7", "root_requirement": "httpx>=0.23"},
    "numpy": {"root": "numpy", "required_python_floor": "3.8", "root_requirement": "numpy>=1.22"},
}
FORBIDDEN = (
    r"not found in (?:the )?cache", r"cache[- ]miss", r"network (?:connectivity )?(?:is |was )?disabled",
    r"network-disabled", r"offline", r"not found in (?:the )?package registry", r"failed to (?:download|fetch)",
    r"no matching (?:distribution|package)", r"unable to (?:download|fetch)", r"request failed",
)



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
        for kind in ("real",):
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
        for kind in ("real",):
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
    require(set(analysis) == {"1785", "1788"}, "complete PR analysis required")
    for pr in (1785, 1788):
        require(set(analysis[str(pr)]) == {"real"} and set(analysis[str(pr)]["real"]) == set(REAL_CASES), "complete real endpoint inventory required")
        for case in REAL_CASES:
            value = analysis[str(pr)]["real"][case]
            bounds = value["paired_bootstrap"]
            require(len(value["pairs"]) == 12 and bounds["seed"] == pr and bounds["resamples"] == 20000 and bounds["confidence"] == .95, "fixed paired estimator differs")
            result.append({"pr": pr, "kind": "real", "case": case, "geometric_mean_ratio": value["geometric_mean_ratio"],
                           "lower": bounds["lower"], "upper": bounds["upper"],
                           "classification": "faster" if bounds["upper"] < 1 else "slower" if bounds["lower"] > 1 else "inconclusive"})
    require(len(result) == 12, "all twelve endpoints required")
    return result


def expected_raw_paths():
    return {f"{row['label']}/{case}/new/{name}" for row in timing_rows() for case in row["cases"] for name in RAW_NAMES}


def read_archive(data, expected):
    require(len(data) <= MAX_ARCHIVE and set(expected) == expected_raw_paths() and len(expected) == 1152, "raw archive inventory differs")
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


def reject_fallback(text):
    require(isinstance(text, str), "diagnostic text required")
    folded = " ".join(text.lower().split())
    require(not any(re.search(pattern, folded) for pattern in FORBIDDEN), "offline/cache-miss/transport fallback is not a Python-conflict specimen")

def validate_diagnostic(fixture, report, hints):
    """Require causal, fixture-specific Python-incompatibility evidence."""
    require(fixture in FIXTURES and isinstance(report, str) and isinstance(hints, list)
         and all(isinstance(item, str) for item in hints), "complete known fixture/report/hints required")
    reject_fallback(report + "\n" + "\n".join(hints))
    rule = RULES[fixture["name"]]
    text = " ".join(report.split())
    target = re.escape(fixture["python"])
    floor = re.escape(rule["required_python_floor"])
    require(re.search(r"(?:current|requested) Python version \((?:>=)?" + target + r"(?:\.[0-9]+|\.\[X\])?\)", text), "fixture Python version is not proved")
    require(re.search(r"does not satisfy Python\s*>=\s*" + floor + r"(?:[,.\s]|$)", text), "required incompatible Python floor is not proved")
    require(rule["root"] in text and rule["root_requirement"] in text, "root requirement is not proved")
    require(re.search(re.escape(rule["root"]) + r".{0,300}\bdepend(?:s)? on Python", text), "root Requires-Python incompatibility is not proved")
    require("Because " in text and "And because " in text and "we can conclude that" in text
         and "your requirements are unsatisfiable" in text, "complete causal unsatisfiability derivation required")
    return {"fixture": fixture["name"], "root": rule["root"], "python": fixture["python"],
            "required_python_floor": rule["required_python_floor"], "report": file_identity(report.encode()),
            "ordered_hints": file_identity(canonical_json(hints)), "classification": "populated-cache-python-incompatibility"}

def validate_specimens(value):
    expected = {f"specimen-pr{pr}-{role}-real" for pr in (1785, 1788) for role in ("parent", "candidate")}
    require(type(value) is dict and set(value) == expected, "four exact real specimens required")
    for item in value.values():
        require(set(item) == {"mode", "observations"} and item["mode"] == "inspect" and len(item["observations"]) == 3, "real specimen envelope differs")
        for observation, fixture in zip(item["observations"], FIXTURES, strict=True):
            require(set(observation) == {"fixture", "report", "hints"} and observation["fixture"] == fixture, "exact fixture order required")
            validate_diagnostic(fixture, observation["report"], observation["hints"])
    for pr in (1785, 1788):
        require(value[f"specimen-pr{pr}-parent-real"] == value[f"specimen-pr{pr}-candidate-real"], "paired complete diagnostics differ")
    require(value["specimen-pr1785-candidate-real"] == value["specimen-pr1788-parent-real"], "shared source diagnostic differs")
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
        and type(value["pairs"]) is list and len(value["pairs"]) == 24,
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
    require(value == EXPECTED_ADMISSION, "exact complete populated-phase provenance differs")
    for phase in PHASES:
        for record in value["phases"][phase].values():
            hash_identity(record)


def validate_provenance(value):
    require(type(value) is dict and set(value) == {"original_public_manifest", "retained_build", "fixture", "background"}
        and value["original_public_manifest"] == ORIGINAL_MANIFEST
        and digest(canonical_json(value["retained_build"])) == SHARED_PROVENANCE_SHA
        and digest(canonical_json(value["fixture"])) == FIXTURE_PROVENANCE_SHA,
        "exact source/build/compatible-cache provenance differs")
    require(value["fixture"]["seed"]["cache_namespace"] == "simple-v25"
        and value["fixture"]["specimen_semantics"]["unique_source_fixture_diagnostics"] == 9,
        "all positive source/fixture semantics required")
    validate_background(value["background"])
    content_scan(canonical_json(value))


def admit_portable(bundle: Path, expected_manifest_sha256: str) -> dict:
    """Verify an externally pinned export; no products or network are used."""
    bundle = Path(bundle)
    require(bundle.is_absolute() and bundle.resolve(strict=True) == bundle and bundle.is_dir() and not bundle.is_symlink(), "canonical bundle required")
    data = read_regular(bundle / "manifest.json", MAX_JSON)
    require(re.fullmatch(r"[0-9a-f]{64}", expected_manifest_sha256 or "") and digest(data) == expected_manifest_sha256, "externally pinned manifest required")
    manifest = parse_json(data)
    require(set(manifest) == {"schema", "sources", "method", "admission", "provenance", "schedule", "raw_files", "original_raw_files", "payload"} and manifest["schema"] == SCHEMA and manifest["sources"] == SOURCES and manifest["method"] == METHOD and manifest["schedule"] == timing_rows() and set(manifest["payload"]) == PAYLOAD, "portable manifest contract differs")
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
    original = manifest["original_raw_files"]
    require(set(original) == {name.replace("/new/", "/"+kind+"/") for name in expected_raw_paths() for kind in ("new", "base")}
        and len(original) == 2304
        and all(original[name] == original[name.replace("/new/", "/base/")] == manifest["raw_files"][name] for name in expected_raw_paths()),
        "complete original new/base identity mapping differs")
    measurements = {row["label"]: raw_cases(row, raw) for row in timing_rows()}
    analysis = paired_analysis(measurements)
    endpoints = classify(analysis)
    exported = parse_json(payload["analysis.json"])
    require(exported == {"schema": "uv-future.resolver-error-populated-portable-analysis.v1", "analysis": analysis, "endpoints": endpoints}, "raw reconstruction differs from admitted analysis")
    validate_specimens(parse_json(payload["specimens.json"]))
    return {"schema": "uv-future.resolver-error-populated-portable-admission.v1", "accepted": True, "manifest_sha256": expected_manifest_sha256, "analysis": analysis, "endpoints": endpoints, "admission": manifest["admission"], "provenance": manifest["provenance"], "sources": manifest["sources"], "method": manifest["method"], "publication_authorized": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    parser.add_argument("--manifest-sha256", required=True)
    args = parser.parse_args()
    print(json.dumps(admit_portable(args.bundle, args.manifest_sha256), indent=2, sort_keys=True, allow_nan=False))


if __name__ == "__main__":
    main()
