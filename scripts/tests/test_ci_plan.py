# /// script
# requires-python = ">=3.12"
# dependencies = []
#
# [tool.uv]
# no-build = true
# exclude-newer = "P7D"
# ///
"""Exercise the CI planner against real, shallow Git histories."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW_PATH = ROOT / ".github/workflows/plan.yml"
RETAIN_DIRECTORY: Path | None = None
LABEL_EXPRESSION = re.compile(
    r"\$\{\{ contains\(github\.event\.pull_request\.labels\.\*\.name, '([^']+)'\) \}\}"
)
OUTPUT_EXPRESSION = re.compile(r"\$\{\{ steps\.plan\.outputs\.([a-z_]+) \}\}")


def block(lines: list[str], header: str, indent: int) -> list[str]:
    """Read one block in the repository's formatted workflow source."""
    marker = " " * indent + header
    starts = [index for index, line in enumerate(lines) if line == marker]
    if len(starts) != 1:
        raise ValueError(f"Expected exactly one workflow field: {marker!r}")
    result = []
    for line in lines[starts[0] + 1 :]:
        if line.strip() and len(line) - len(line.lstrip()) <= indent:
            break
        result.append(line)
    return result


def scalar(value: str) -> str:
    if value.startswith('"'):
        value = json.loads(value)
        if not isinstance(value, str):
            raise TypeError("Expected a quoted string")
        return value
    if value.startswith("'"):
        if not value.endswith("'"):
            raise ValueError("Unterminated quoted string")
        return value[1:-1].replace("''", "'")
    return value.partition(" # ")[0]


def flat_mapping(lines: list[str], indent: int) -> dict[str, str]:
    """Reject nested or multiline fields instead of silently misreading them."""
    values = {}
    pattern = re.compile(" " * indent + r"([A-Za-z0-9_-]+): (.+)")
    for line in lines:
        if not line.strip():
            continue
        match = pattern.fullmatch(line)
        if match is None or match.group(1) in values:
            raise ValueError(f"Unsupported workflow mapping line: {line!r}")
        values[match.group(1)] = scalar(match.group(2))
    return values


class Step:
    def __init__(self, lines: list[str]) -> None:
        # Treat the list item's first field like its following mapping fields.
        self.lines = ["        " + lines[0].removeprefix("      - "), *lines[1:]]

    def field(self, name: str, default: str | None = None) -> str:
        prefix = f"        {name}: "
        values = [
            line.removeprefix(prefix) for line in self.lines if line.startswith(prefix)
        ]
        if not values and default is not None:
            return default
        if len(values) != 1:
            raise ValueError(f"Expected exactly one step field: {name!r}")
        return scalar(values[0])

    def mapping(self, name: str) -> dict[str, str]:
        return flat_mapping(block(self.lines, f"{name}:", 8), 10)

    def require_fields(self, expected: set[str]) -> None:
        pattern = re.compile(r"        ([A-Za-z0-9_-]+):(?: .*|)")
        names = [
            match.group(1) for line in self.lines if (match := pattern.fullmatch(line))
        ]
        if len(names) != len(set(names)) or set(names) != expected:
            raise ValueError(f"Unsupported workflow step fields: {names}")

    def script(self) -> str:
        lines = block(self.lines, "run: |", 8)
        if any(line.strip() and not line.startswith(" " * 10) for line in lines):
            raise ValueError("Unsupported workflow literal block")
        return "\n".join(line[10:] for line in lines) + "\n"


class Workflow:
    def __init__(self, source: str) -> None:
        lines = source.splitlines()
        self.workflow_call = block(block(lines, "on:", 0), "workflow_call:", 2)
        self.job = block(block(lines, "jobs:", 0), "plan:", 2)
        step_lines = block(self.job, "steps:", 4)
        starts = [
            index
            for index, line in enumerate(step_lines)
            if line.startswith("      - ")
        ]
        if not starts or any(line.strip() for line in step_lines[: starts[0]]):
            raise ValueError("Unsupported workflow step list")
        ends = [*starts[1:], len(step_lines)]
        self.steps = [
            Step(step_lines[start:end]) for start, end in zip(starts, ends, strict=True)
        ]
        for step in self.steps:
            if step.field("uses", "").startswith("actions/checkout@"):
                step.require_fields({"uses", "with"})
            elif step.field("name", "") == "Plan":
                step.require_fields({"name", "id", "shell", "env", "run"})
                if step.field("id") != "plan" or step.field("shell") != "bash":
                    raise ValueError("Unsupported Plan execution settings")
            elif step.field("name", "") == "Fetch main":
                step.require_fields({"name", "if", "env", "run"})
                if step.mapping("env") != {"GH_TOKEN": "${{ github.token }}"}:
                    raise ValueError("Unsupported Fetch main environment")
                step.script()
            else:
                raise ValueError("Unsupported CI planner step")
        self.plan = self.step("Plan")
        self.plan_inputs = self.plan.mapping("env")
        self.plan_script = self.plan.script()
        if "${{" in self.plan_script:
            raise ValueError(
                "Plan expressions must be supplied through its environment"
            )
        self.output_names = set()
        for expression in flat_mapping(block(self.job, "outputs:", 4), 6).values():
            match = OUTPUT_EXPRESSION.fullmatch(expression)
            if match is None:
                raise ValueError(f"Unsupported planner output: {expression}")
            self.output_names.add(match.group(1))

    @classmethod
    def load(cls, path: Path) -> Workflow:
        return cls(path.read_text(encoding="utf-8"))

    def step(self, name: str) -> Step:
        steps = [step for step in self.steps if step.field("name", "") == name]
        if len(steps) != 1:
            raise ValueError(f"Expected exactly one {name!r} step")
        return steps[0]

    def environment(
        self, reference: str, labels: tuple[str, ...], base_sha: str
    ) -> dict[str, str]:
        environment = {}
        for name, expression in self.plan_inputs.items():
            if name == "GH_REF":
                environment[name] = reference
            elif name == "BASE_SHA":
                environment[name] = base_sha
            else:
                match = LABEL_EXPRESSION.fullmatch(expression)
                if match is None:
                    raise ValueError(f"Unsupported planner input: {name}={expression}")
                environment[name] = str(match.group(1) in labels).lower()
        return environment


class History:
    """A local origin and checkout; no GitHub service or token is needed."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.source = root / "source"
        self.origin = root / "origin.git"
        self.counter = 0
        self.environment = {
            name: os.environ[name]
            for name in (
                "PATH",
                "SystemRoot",
                "SYSTEMROOT",
                "COMSPEC",
                "PATHEXT",
                "TMPDIR",
                "TMP",
                "TEMP",
            )
            if name in os.environ
        }
        self.environment.update(
            {
                "GIT_CONFIG_NOSYSTEM": "1",
                "GIT_CONFIG_GLOBAL": os.devnull,
                "GIT_TERMINAL_PROMPT": "0",
                "GIT_AUTHOR_DATE": "2000-01-01T00:00:00+00:00",
                "GIT_COMMITTER_DATE": "2000-01-01T00:00:00+00:00",
                "LC_ALL": "C",
                "LANG": "C",
            }
        )
        self.git(root, "init", "--quiet", "--bare", str(self.origin))
        self.git(
            root,
            "--git-dir",
            str(self.origin),
            "config",
            "uploadpack.allowFilter",
            "true",
        )
        self.git(root, "init", "--quiet", "--initial-branch=main", str(self.source))
        self.git(self.source, "remote", "add", "origin", self.origin.as_uri())
        self.initial = self.commit("initial", {"README.md": "fixture\n"})
        self.push("main")

    def git(
        self, directory: Path, *arguments: str, check: bool = True
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                "git",
                "-c",
                "user.name=CI Plan Fixture",
                "-c",
                "user.email=ci-plan@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                f"core.hooksPath={os.devnull}",
                "-c",
                "protocol.file.allow=always",
                *arguments,
            ],
            cwd=directory,
            env=self.environment,
            check=check,
            capture_output=True,
            text=True,
            timeout=30,
        )

    def commit(self, message: str, files: dict[str, str]) -> str:
        for name, contents in files.items():
            path = self.source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(contents, encoding="utf-8")
        self.git(self.source, "add", "--all")
        self.git(self.source, "commit", "--quiet", "--allow-empty", "-m", message)
        return self.git(self.source, "rev-parse", "HEAD").stdout.strip()

    def branch(self, name: str, start: str) -> None:
        self.git(self.source, "checkout", "--quiet", "-b", name, start)

    def checkout(self, revision: str) -> None:
        self.git(self.source, "checkout", "--quiet", revision)

    def push(self, *references: str) -> None:
        self.git(self.source, "push", "--quiet", "origin", *references)

    def merge(self, base: str, head: str, reference: str) -> str:
        self.git(self.source, "checkout", "--quiet", "--detach", base)
        self.git(
            self.source, "merge", "--quiet", "--no-ff", "-m", "synthetic merge", head
        )
        revision = self.git(self.source, "rev-parse", "HEAD").stdout.strip()
        self.push(f"{revision}:{reference}")
        return revision

    def fetched_checkout(self, reference: str, name: str = "checkout") -> Path:
        checkout = self.root / name
        self.git(self.root, "init", "--quiet", str(checkout))
        self.git(checkout, "remote", "add", "origin", self.origin.as_uri())
        if reference.startswith("refs/heads/"):
            destination = "refs/remotes/origin/" + reference.removeprefix("refs/heads/")
        elif reference.startswith("refs/pull/"):
            destination = "refs/remotes/" + reference.removeprefix("refs/")
        else:
            raise ValueError(f"Unsupported checkout ref: {reference}")
        self.git(
            checkout,
            "fetch",
            "--quiet",
            "--depth=2",
            "--no-tags",
            "origin",
            f"+{reference}:{destination}",
        )
        self.git(checkout, "checkout", "--quiet", "--detach", "FETCH_HEAD")
        return checkout

    def shell(
        self, checkout: Path, script: str, environment: dict[str, str]
    ) -> subprocess.CompletedProcess[str]:
        if "${{" in script:
            raise ValueError(
                "Workflow expressions must be provided through the step environment"
            )
        return subprocess.run(
            ["bash", "--noprofile", "--norc", "-e", "-o", "pipefail", "-c", script],
            cwd=checkout,
            env={**self.environment, **environment},
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )

    def fetch_main(
        self, workflow: Workflow, checkout: Path
    ) -> subprocess.CompletedProcess[str]:
        return self.shell(
            checkout,
            workflow.step("Fetch main").script(),
            {
                "GH_TOKEN": "fixture-token-not-a-secret",
                "GITHUB_SERVER_URL": "https://github.invalid",
            },
        )

    def plan(
        self,
        workflow: Workflow,
        checkout: Path,
        reference: str,
        *,
        repository: str = "astral-sh/uv",
        event: str = "pull_request",
        labels: tuple[str, ...] = (),
        base_sha: str = "",
    ) -> tuple[subprocess.CompletedProcess[str], dict[str, bool]]:
        self.counter += 1
        output = self.root / f"output-{self.counter}.txt"
        output.write_text("", encoding="utf-8")
        result = self.shell(
            checkout,
            workflow.plan_script,
            {
                **workflow.environment(reference, labels, base_sha),
                "GITHUB_REPOSITORY": repository,
                "GITHUB_EVENT_NAME": event,
                "GITHUB_REF": reference,
                "GITHUB_OUTPUT": str(output),
            },
        )
        values = {}
        for line in output.read_text(encoding="utf-8").splitlines():
            name, separator, value = line.partition("=")
            if not separator or name in values or value not in ("true", "false"):
                raise ValueError(f"Invalid planner output: {line!r}")
            values[name] = value == "true"
        if result.returncode == 0 and values.keys() != workflow.output_names:
            raise ValueError(f"Planner outputs differ from the workflow: {values}")
        return result, values


class WorkflowShape(unittest.TestCase):
    def test_changed_execution_shape_is_rejected(self) -> None:
        source = WORKFLOW_PATH.read_text(encoding="utf-8")
        before, separator, plan = source.partition('      - name: "Plan"\n')
        self.assertTrue(separator)
        for old, new in (
            ("        run: |\n", "        run: >\n"),
            ("        shell: bash\n", "        shell: sh\n"),
            (
                "        id: plan\n",
                "        id: plan\n        working-directory: nested\n",
            ),
            ("          GH_REF:", "         GH_REF:"),
            ("          GH_REF:", "          GH_REF: duplicate\n          GH_REF:"),
        ):
            with self.subTest(change=new.strip()):
                self.assertIn(old, plan)
                with self.assertRaises(ValueError):
                    Workflow(before + separator + plan.replace(old, new, 1))

    def test_nested_mapping_is_rejected(self) -> None:
        with self.assertRaises(ValueError):
            flat_mapping(["          first: value", "            nested: value"], 10)

    def test_duplicate_mapping_is_rejected(self) -> None:
        with self.assertRaises(ValueError):
            flat_mapping(["          first: one", "          first: two"], 10)


@unittest.skipUnless(os.name == "posix" and shutil.which("bash"), "requires POSIX Bash")
class CIPlan(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = Workflow.load(WORKFLOW_PATH)

    def setUp(self) -> None:
        if RETAIN_DIRECTORY is None:
            temporary = tempfile.TemporaryDirectory(prefix="uv-ci-plan-")
            self.addCleanup(temporary.cleanup)
            root = Path(temporary.name)
        else:
            root = RETAIN_DIRECTORY / self._testMethodName
            root.mkdir(parents=True, exist_ok=False)
        self.history = History(root)

    def assert_success(self, result: subprocess.CompletedProcess[str]) -> None:
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))

    def assert_plan_failure(
        self, result: subprocess.CompletedProcess[str], values: dict[str, bool]
    ) -> None:
        self.assertNotEqual(
            result.returncode, 0, (result.stdout, result.stderr, values)
        )
        self.assertEqual(values, {}, (result.stdout, result.stderr))

    def test_checkout_and_dispatch_contract(self) -> None:
        checkouts = [
            step
            for step in self.workflow.steps
            if step.field("uses", "").startswith("actions/checkout@")
        ]
        self.assertEqual(len(checkouts), 1)
        self.assertEqual(checkouts[0].mapping("with")["fetch-depth"], "2")
        self.assertEqual(checkouts[0].mapping("with")["persist-credentials"], "false")
        self.assertNotIn("ref", checkouts[0].mapping("with"))
        self.assertNotIn("    inputs:", self.workflow.workflow_call)
        self.assertEqual(
            self.workflow.step("Fetch main").field("if"),
            "${{ github.event_name == 'workflow_dispatch' && github.ref != 'refs/heads/main' }}",
        )

    def test_pull_request_uses_the_checked_out_merge_parent(self) -> None:
        history = self.history
        history.branch("feature", history.initial)
        feature = history.commit(
            "feature", {"crates/fixture/src/lib.rs": "pub fn feature() {}\n"}
        )
        history.checkout("main")
        history.commit("advance one", {"docs/one.md": "one\n"})
        base = history.commit("advance two", {"docs/two.md": "two\n"})
        history.push("main", "feature")
        history.merge(base, feature, "refs/pull/41/merge")
        checkout = history.fetched_checkout("refs/pull/41/merge")
        self.assertNotEqual(
            history.git(
                checkout, "cat-file", "-e", history.initial, check=False
            ).returncode,
            0,
        )
        result, values = history.plan(
            self.workflow, checkout, "refs/pull/41/merge", base_sha=history.initial
        )
        self.assert_success(result)
        self.assertTrue(values["test_code"], (result.stdout, result.stderr, values))
        self.assertTrue(values["run_bench"])
        self.assertFalse(values["save_rust_cache"])
        self.assertFalse(values["build_release_binaries"])

    def test_stacked_pull_request_does_not_include_its_base_changes(self) -> None:
        history = self.history
        history.branch("stack-base", history.initial)
        base = history.commit(
            "base code", {"crates/fixture/src/lib.rs": "pub fn base() {}\n"}
        )
        history.branch("stack-child", base)
        child = history.commit("child docs", {"docs/child.md": "child\n"})
        history.push("stack-base", "stack-child")
        history.merge(base, child, "refs/pull/42/merge")
        checkout = history.fetched_checkout("refs/pull/42/merge")
        result, values = history.plan(
            self.workflow, checkout, "refs/pull/42/merge", base_sha=history.initial
        )
        self.assert_success(result)
        self.assertFalse(values["test_code"], (result.stdout, result.stderr, values))
        self.assertFalse(values["run_bench"])
        self.assertTrue(values["run_checks"])

    def test_stale_merge_ref_does_not_follow_a_rewritten_base(self) -> None:
        history = self.history
        history.branch("feature", history.initial)
        feature = history.commit("feature", {"docs/feature.md": "feature\n"})
        history.checkout("main")
        base = history.commit(
            "base code", {"crates/fixture/src/lib.rs": "pub fn base() {}\n"}
        )
        history.push("feature", "main")
        history.merge(base, feature, "refs/pull/43/merge")
        tree = history.git(
            history.source, "rev-parse", f"{history.initial}^{{tree}}"
        ).stdout.strip()
        rewritten = history.git(
            history.source, "commit-tree", tree, "-m", "rewritten base"
        ).stdout.strip()
        history.push(f"+{rewritten}:refs/heads/main")
        checkout = history.fetched_checkout("refs/pull/43/merge")
        result, values = history.plan(
            self.workflow, checkout, "refs/pull/43/merge", base_sha=rewritten
        )
        self.assert_success(result)
        self.assertFalse(values["test_code"], (result.stdout, result.stderr, values))
        self.assertFalse(values["run_bench"])

    def test_branch_dispatch_fetches_diverged_history(self) -> None:
        history = self.history
        history.branch("feature", history.initial)
        for index in range(6):
            history.commit(
                f"feature {index}",
                {"crates/fixture/src/lib.rs": f"pub const N: u8 = {index};\n"},
            )
        history.checkout("main")
        for index in range(6):
            main = history.commit(f"main {index}", {"docs/main.md": f"{index}\n"})
        history.push("feature", "main")
        checkout = history.fetched_checkout("refs/heads/feature")
        self.assertEqual(
            history.git(
                checkout, "rev-parse", "--is-shallow-repository"
            ).stdout.strip(),
            "true",
        )
        self.assert_success(history.fetch_main(self.workflow, checkout))
        self.assertEqual(
            history.git(
                checkout, "rev-parse", "--is-shallow-repository"
            ).stdout.strip(),
            "false",
        )
        self.assertEqual(
            history.git(checkout, "rev-parse", "origin/main").stdout.strip(), main
        )
        self.assertEqual(
            history.git(checkout, "merge-base", "origin/main", "HEAD").stdout.strip(),
            history.initial,
        )
        result, values = history.plan(
            self.workflow, checkout, "refs/heads/feature", event="workflow_dispatch"
        )
        self.assert_success(result)
        self.assertTrue(values["test_code"])
        self.assertTrue(values["run_bench"])
        self.assertFalse(values["build_release_binaries"])

    def test_branch_dispatch_accepts_complete_short_history(self) -> None:
        history = self.history
        history.branch("feature", history.initial)
        history.push("feature")
        checkout = history.fetched_checkout("refs/heads/feature")
        self.assertEqual(
            history.git(
                checkout, "rev-parse", "--is-shallow-repository"
            ).stdout.strip(),
            "false",
        )
        self.assert_success(history.fetch_main(self.workflow, checkout))
        result, values = history.plan(
            self.workflow, checkout, "refs/heads/feature", event="workflow_dispatch"
        )
        self.assert_success(result)
        self.assertFalse(values["test_code"])
        self.assertFalse(values["run_bench"])
        self.assertTrue(values["run_checks"])

    def test_missing_comparison_ref_fails_without_outputs(self) -> None:
        history = self.history
        history.branch("feature", history.initial)
        history.commit(
            "feature", {"crates/fixture/src/lib.rs": "pub fn feature() {}\n"}
        )
        history.push("feature")
        checkout = history.fetched_checkout("refs/heads/feature")
        result, values = history.plan(
            self.workflow, checkout, "refs/heads/feature", event="workflow_dispatch"
        )
        self.assert_plan_failure(result, values)

    def test_unrelated_history_fails_without_outputs(self) -> None:
        history = self.history
        history.branch("feature", history.initial)
        feature = history.commit(
            "feature", {"crates/fixture/src/lib.rs": "pub fn feature() {}\n"}
        )
        tree = history.git(
            history.source, "rev-parse", f"{feature}^{{tree}}"
        ).stdout.strip()
        unrelated = history.git(
            history.source, "commit-tree", tree, "-m", "unrelated main"
        ).stdout.strip()
        history.push("feature", f"+{unrelated}:refs/heads/main")
        checkout = history.fetched_checkout("refs/heads/feature")
        history.git(
            checkout,
            "fetch",
            "--quiet",
            "origin",
            "+refs/heads/main:refs/remotes/origin/main",
        )
        self.assertNotEqual(
            history.git(
                checkout, "merge-base", "origin/main", "HEAD", check=False
            ).returncode,
            0,
        )
        result, values = history.plan(
            self.workflow, checkout, "refs/heads/feature", event="workflow_dispatch"
        )
        self.assert_plan_failure(result, values)

    def test_failed_dispatch_fetch_is_not_ignored(self) -> None:
        history = self.history
        history.branch("feature", history.initial)
        history.commit("feature", {"docs/feature.md": "feature\n"})
        history.push("feature")
        history.git(
            history.root,
            "--git-dir",
            str(history.origin),
            "update-ref",
            "-d",
            "refs/heads/main",
        )
        checkout = history.fetched_checkout("refs/heads/feature")
        result = history.fetch_main(self.workflow, checkout)
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--workflow", type=Path, default=WORKFLOW_PATH)
    parser.add_argument("--retain-fixtures", type=Path)
    arguments, remaining = parser.parse_known_args()
    WORKFLOW_PATH = arguments.workflow.resolve()
    RETAIN_DIRECTORY = arguments.retain_fixtures
    unittest.main(argv=[sys.argv[0], *remaining])
