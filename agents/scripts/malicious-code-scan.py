#!/usr/bin/env -S uv run --script
#
# /// script
# requires-python = ">=3.12"
# dependencies = []
# [tool.uv]
# no-build = true
# exclude-newer = "P7D"
# ///

"""Prepare immutable Git evidence and validate a malicious-code scan report."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
from pathlib import Path
from typing import Any

ZERO_SHA = "0" * 40


def git(git_dir: Path, *arguments: str, stdin: bytes | None = None) -> bytes:
    return subprocess.run(
        [
            "git",
            "--no-pager",
            "--git-dir",
            str(git_dir),
            "-c",
            "core.hooksPath=/dev/null",
            *arguments,
        ],
        input=stdin,
        check=True,
        capture_output=True,
        timeout=120,
    ).stdout


def prepare(git_dir: Path, output: Path, kind: str, base: str, head: str) -> None:
    for revision in (base, head):
        if not re.fullmatch(r"[0-9a-f]{40}", revision):
            raise ValueError("scan revisions must be full Git commit SHAs")
    if head == ZERO_SHA or (kind == "PULL_REQUEST" and base == ZERO_SHA):
        raise ValueError(
            "the scan needs a head commit and pull requests need a base commit"
        )
    if git(git_dir, "rev-parse", "--is-bare-repository").strip() != b"true":
        raise ValueError(
            "scan input must be a bare repository, without a checked-out tree"
        )
    git(git_dir, "cat-file", "-e", f"{head}^{{commit}}")
    empty_tree = (
        git(git_dir, "hash-object", "-w", "-t", "tree", "--stdin", stdin=b"")
        .decode()
        .strip()
    )
    if base == ZERO_SHA:
        comparison = empty_tree
        revision_range = head
    else:
        git(git_dir, "cat-file", "-e", f"{base}^{{commit}}")
        comparison = (
            git(git_dir, "merge-base", base, head).decode().strip()
            if kind == "PULL_REQUEST"
            else base
        )
        revision_range = f"{base}..{head}"

    commits = (
        git(git_dir, "rev-list", "--reverse", "--topo-order", revision_range)
        .decode()
        .splitlines()
    )
    output.mkdir(parents=True)
    (output / "commits").mkdir()
    diff_options = ("--no-ext-diff", "--no-textconv", "--no-renames")
    (output / "net.diff").write_bytes(
        git(git_dir, "diff", *diff_options, comparison, head)
    )
    range_paths = git(
        git_dir, "diff", *diff_options, "--name-only", "-z", comparison, head
    )
    entries = []
    for commit in commits:
        parents = git(git_dir, "show", "-s", "--format=%P", commit).decode().split()
        parent = parents[0] if parents else empty_tree
        paths = git(git_dir, "diff", *diff_options, "--name-only", "-z", parent, commit)
        # Preserve unusual filenames without treating them as shell arguments or output paths.
        changed_paths = [
            name.decode("utf-8", errors="surrogateescape")
            for name in paths.split(b"\0")
            if name
        ]
        diff_path = f"commits/{commit}.diff"
        (output / diff_path).write_bytes(
            git(
                git_dir,
                "show",
                *diff_options,
                "--format=fuller",
                "--diff-merges=first-parent",
                commit,
            )
        )
        entries.append(
            {
                "sha": commit,
                "parents": parents,
                "paths": changed_paths,
                "diff": diff_path,
            }
        )

    receipt = {
        "kind": kind,
        "base_sha": base,
        "head_sha": head,
        "comparison_sha": comparison,
        "source_git_dir": str(git_dir.resolve()),
        "range_paths": [
            name.decode("utf-8", errors="surrogateescape")
            for name in range_paths.split(b"\0")
            if name
        ],
        "commits": entries,
    }
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")


def validate_report(receipt: dict[str, Any], report: dict[str, Any]) -> None:
    commits = {commit["sha"]: commit for commit in receipt["commits"]}
    reviewed = report["reviewed_commits"]
    if len(reviewed) != len(set(reviewed)) or not set(reviewed) <= commits.keys():
        raise ValueError("reviewed commits must be unique members of the scan range")
    outcome = report["outcome"]
    if outcome not in {"CLEAN", "SUSPICIOUS", "INCONCLUSIVE"}:
        raise ValueError("invalid scan outcome")
    if not report["summary"].strip():
        raise ValueError("the scan must explain its outcome")
    if outcome == "CLEAN" and (
        set(reviewed) != commits.keys()
        or report["reviewed_net_diff"] is not True
        or report["coverage_gaps"]
        or report["findings"]
    ):
        raise ValueError(
            "a clean scan must cover every commit without findings or coverage gaps"
        )
    if (outcome == "SUSPICIOUS") != bool(report["findings"]):
        raise ValueError(
            "suspicious scans must contain findings, and other outcomes must not"
        )
    if outcome == "INCONCLUSIVE" and not report["coverage_gaps"]:
        raise ValueError("an inconclusive scan must explain its coverage gaps")
    for finding in report["findings"]:
        commit = commits.get(finding["commit_sha"])
        in_commit = commit is not None and finding["path"] in commit["paths"]
        # A force push can restore old code without introducing any new commits.
        in_range = (
            finding["commit_sha"] == receipt["head_sha"]
            and finding["path"] in receipt["range_paths"]
        )
        if not in_commit and not in_range:
            raise ValueError(
                "findings must identify a changed path in a scanned commit"
            )


def summarize(receipt_path: Path, report_path: Path, summary_path: Path) -> bool:
    receipt = json.loads(receipt_path.read_text())
    report = json.loads(report_path.read_text())
    validate_report(receipt, report)
    with summary_path.open("a") as summary:
        summary.write("## Malicious-code scan\n\n")
        summary.write(f"Range: `{receipt['base_sha']}` to `{receipt['head_sha']}`\n\n")
        # JSON escaping prevents report text from closing the Markdown code block.
        summary.write("```json\n" + json.dumps(report, indent=2) + "\n```\n")
    return report["outcome"] == "CLEAN"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    prepare_parser = commands.add_parser("prepare")
    prepare_parser.add_argument("--git-dir", required=True, type=Path)
    prepare_parser.add_argument("--output", required=True, type=Path)
    prepare_parser.add_argument(
        "--kind", required=True, choices=["PULL_REQUEST", "PUSH"]
    )
    prepare_parser.add_argument("--base", required=True)
    prepare_parser.add_argument("--head", required=True)
    report_parser = commands.add_parser("report")
    report_parser.add_argument("--receipt", required=True, type=Path)
    report_parser.add_argument("--report", required=True, type=Path)
    report_parser.add_argument("--summary", required=True, type=Path)
    arguments = parser.parse_args()
    if arguments.command == "prepare":
        prepare(
            arguments.git_dir,
            arguments.output,
            arguments.kind,
            arguments.base,
            arguments.head,
        )
    else:
        clean = summarize(arguments.receipt, arguments.report, arguments.summary)
        if not clean:
            raise SystemExit(
                "The scan needs human review; see the report and coverage gaps."
            )


if __name__ == "__main__":
    main()
