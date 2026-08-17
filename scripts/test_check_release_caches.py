"""Integration tests for the release cache policy checker."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("check_release_caches.py")
RELEASE_WORKFLOW = ".github/workflows/release.yml"
CACHEABLE_BUILD_WORKFLOW = ".github/workflows/build-release-binaries.yml"
BUILD_CACHE_MODE = "${{ inputs.allow-cache && 'write' || 'none' }}"
BUILD_UV_CACHE = "${{ inputs.allow-cache && 'auto' || 'false' }}"
CACHE_PROXY_USES = "astral-sh/uv-dev/.github/actions/disable-github-caches@c3892c0a9adbb81c11bbda1eb62e020455665b6a"


class ReleaseCachePolicyTest(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def write(self, path: str, document: dict) -> None:
        destination = self.root / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(json.dumps(document))

    def write_workflow(self, path: str, document: dict) -> None:
        self.write(path, {"env": {"ACTIONS_CACHE_MODE": "none"}, **document})

    def write_steps(self, steps: list[dict], path: str = RELEASE_WORKFLOW) -> None:
        self.write_workflow(path, {"jobs": {"test": {"steps": steps}}})

    def write_cacheable_build(
        self, *, cache_input: dict | None = None, steps: list[dict] | None = None
    ) -> None:
        if cache_input is None:
            cache_input = {"type": "boolean", "default": False}
        if steps is None:
            steps = [
                {
                    "uses": "astral-sh/setup-uv@revision",
                    "with": {"enable-cache": BUILD_UV_CACHE},
                }
            ]
        self.write_workflow(
            CACHEABLE_BUILD_WORKFLOW,
            {
                "on": {"workflow_call": {"inputs": {"allow-cache": cache_input}}},
                "env": {"ACTIONS_CACHE_MODE": BUILD_CACHE_MODE},
                "jobs": {"test": {"steps": steps}},
            },
        )

    def check(self, expected_error: str = "", *, count: int = 1) -> None:
        result = subprocess.run(
            [sys.executable, str(SCRIPT), str(self.root)],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 1 if expected_error else 0, result.stderr)
        self.assertEqual(
            result.stderr, f"error: {expected_error}\n" if expected_error else ""
        )
        self.assertEqual(
            result.stdout,
            ""
            if expected_error
            else f"Checked {count} release workflow/action files.\n",
        )

    def test_cache_actions_are_rejected(self) -> None:
        for action in (
            "actions/cache",
            "actions/cache/restore",
            "actions/cache/save",
            "Swatinem/rust-cache",
        ):
            with self.subTest(action=action):
                self.write_steps([{"uses": f"{action}@revision", "if": "false"}])
                self.check(
                    f"{RELEASE_WORKFLOW}: jobs.test.steps[0]: {action.lower()} uses GitHub caches"
                )

    def test_workflow_cache_mode_is_required(self) -> None:
        for value in (
            None,
            "",
            "read",
            "write",
            "write-only",
            "${{ vars.CACHE_MODE }}",
        ):
            with self.subTest(value=value):
                document = {"jobs": {"test": {"steps": [{"run": "echo ok"}]}}}
                if value is not None:
                    document["env"] = {"ACTIONS_CACHE_MODE": value}
                self.write(RELEASE_WORKFLOW, document)
                self.check(f"{RELEASE_WORKFLOW}: env.ACTIONS_CACHE_MODE must be 'none'")

    def test_cache_mode_overrides_are_rejected(self) -> None:
        for value in ("", "read", "write", "write-only", "${{ vars.CACHE_MODE }}"):
            for scope in ("job", "step"):
                with self.subTest(value=value, scope=scope):
                    step = {"run": "echo ok"}
                    job = {"steps": [step]}
                    settings = job if scope == "job" else step
                    settings["env"] = {"ACTIONS_CACHE_MODE": value}
                    self.write_workflow(RELEASE_WORKFLOW, {"jobs": {"test": job}})
                    location = "jobs.test" if scope == "job" else "jobs.test.steps[0]"
                    self.check(
                        f"{RELEASE_WORKFLOW}: {location}: env.ACTIONS_CACHE_MODE must be 'none'"
                    )

    def test_service_cache_mode_cannot_override_the_environment(self) -> None:
        for value in ("read", "write", "write-only", "${{ vars.CACHE_MODE }}"):
            for scope in ("workflow", "job"):
                with self.subTest(value=value, scope=scope):
                    job = {"steps": [{"run": "echo ok"}]}
                    document = {"jobs": {"test": job}}
                    settings = document if scope == "workflow" else job
                    settings["cache-mode"] = value
                    self.write_workflow(RELEASE_WORKFLOW, document)
                    location = "" if scope == "workflow" else ": jobs.test"
                    self.check(
                        f"{RELEASE_WORKFLOW}{location}: cache-mode must be 'none'"
                    )

    def test_reusable_workflow_needs_its_own_cache_mode(self) -> None:
        nested = ".github/workflows/nested.yml"
        self.write_workflow(
            RELEASE_WORKFLOW, {"jobs": {"nested": {"uses": f"./{nested}"}}}
        )
        self.write(nested, {"jobs": {"test": {"steps": [{"run": "echo ok"}]}}})
        self.check(f"{nested}: env.ACTIONS_CACHE_MODE must be 'none'")
        self.write_steps([{"run": "echo ok"}], nested)
        self.check(count=2)

    def test_composite_action_cache_mode_overrides(self) -> None:
        self.write_steps([{"uses": "./.github/actions/local"}])
        action = ".github/actions/local/action.yml"
        step = {"run": "echo ok", "env": {"ACTIONS_CACHE_MODE": "read"}}
        self.write(action, {"runs": {"using": "composite", "steps": [step]}})
        self.check(f"{action}: runs.steps[0]: env.ACTIONS_CACHE_MODE must be 'none'")
        step["env"]["ACTIONS_CACHE_MODE"] = "none"
        self.write(action, {"runs": {"using": "composite", "steps": [step]}})
        self.check(count=2)

    def test_cache_disabled_overrides_are_allowed(self) -> None:
        self.write_workflow(
            RELEASE_WORKFLOW,
            {
                "cache-mode": "none",
                "jobs": {
                    "test": {
                        "cache-mode": "none",
                        "env": {"ACTIONS_CACHE_MODE": "none"},
                        "steps": [
                            {"run": "echo ok", "env": {"ACTIONS_CACHE_MODE": "none"}}
                        ],
                    }
                },
            },
        )
        self.check()

    def test_cache_inputs_must_be_explicit(self) -> None:
        for action, name, disabled in (
            ("actions/setup-python", "cache", ""),
            ("astral-sh/setup-uv", "enable-cache", False),
            ("PyO3/maturin-action", "sccache", False),
        ):
            for value in (None, True, "${{ inputs.cache }}"):
                with self.subTest(action=action, value=value):
                    step = {"uses": f"{action}@revision"}
                    if value is not None:
                        step["with"] = {name: value}
                    self.write_steps([step])
                    expected = "false" if disabled is False else disabled
                    self.check(
                        f"{RELEASE_WORKFLOW}: jobs.test.steps[0]: {action.lower()} must set {name} to {expected!r}"
                    )
            self.write_steps([{"uses": f"{action}@revision", "with": {name: disabled}}])
            self.check()

    def test_ci_can_opt_into_build_caches(self) -> None:
        self.write_cacheable_build()
        for path, allow_cache in (
            (RELEASE_WORKFLOW, False),
            (".github/workflows/ci.yml", True),
        ):
            self.write_workflow(
                path,
                {
                    "jobs": {
                        "build": {
                            "uses": f"$/{CACHEABLE_BUILD_WORKFLOW}",
                            "with": {"allow-cache": allow_cache},
                        }
                    }
                },
            )
        self.check(count=2)

    def test_release_build_cache_opt_in_is_rejected(self) -> None:
        self.write_cacheable_build()
        for value in (None, True, "${{ inputs.allow-cache }}"):
            with self.subTest(value=value):
                build = {"uses": f"./{CACHEABLE_BUILD_WORKFLOW}"}
                if value is not None:
                    build["with"] = {"allow-cache": value}
                self.write_workflow(
                    RELEASE_WORKFLOW,
                    {
                        "jobs": {
                            "safe": {
                                "uses": f"$/{CACHEABLE_BUILD_WORKFLOW}",
                                "with": {"allow-cache": False},
                            },
                            "unsafe": build,
                        }
                    },
                )
                self.check(
                    f"{RELEASE_WORKFLOW}: jobs.unsafe: release builds must set allow-cache to 'false'"
                )

    def test_build_cache_input_defaults_to_disabled(self) -> None:
        self.write_workflow(
            RELEASE_WORKFLOW,
            {
                "jobs": {
                    "build": {
                        "uses": f"./{CACHEABLE_BUILD_WORKFLOW}",
                        "with": {"allow-cache": False},
                    }
                }
            },
        )
        for cache_input in (
            {},
            {"type": "string", "default": False},
            {"type": "boolean", "default": True},
            {"type": "boolean", "default": "${{ vars.ALLOW_CACHE }}"},
        ):
            with self.subTest(cache_input=cache_input):
                self.write_cacheable_build(cache_input=cache_input)
                self.check(
                    f"{CACHEABLE_BUILD_WORKFLOW}: allow-cache must be a boolean input with default 'false'"
                )

    def test_build_cache_expressions_are_not_allowed_elsewhere(self) -> None:
        self.write_workflow(
            RELEASE_WORKFLOW,
            {"env": {"ACTIONS_CACHE_MODE": BUILD_CACHE_MODE}, "jobs": {}},
        )
        self.check(f"{RELEASE_WORKFLOW}: env.ACTIONS_CACHE_MODE must be 'none'")
        step = {
            "uses": "astral-sh/setup-uv@revision",
            "with": {"enable-cache": BUILD_UV_CACHE},
        }
        self.write_steps([step])
        self.check(
            f"{RELEASE_WORKFLOW}: jobs.test.steps[0]: astral-sh/setup-uv must set enable-cache to 'false'"
        )

        self.write_workflow(
            RELEASE_WORKFLOW,
            {
                "jobs": {
                    "build": {
                        "uses": f"./{CACHEABLE_BUILD_WORKFLOW}",
                        "with": {"allow-cache": False},
                    }
                }
            },
        )
        self.write_cacheable_build(steps=[{"uses": "./.github/actions/local"}])
        action = ".github/actions/local/action.yml"
        self.write(action, {"runs": {"using": "composite", "steps": [step]}})
        self.check(
            f"{action}: runs.steps[0]: astral-sh/setup-uv must set enable-cache to 'false'"
        )

    def test_artifacts_and_unrelated_preparation_are_allowed(self) -> None:
        self.write_steps(
            [
                {
                    "uses": "actions/upload-artifact@revision",
                    "with": {"name": "cargo-dist-cache"},
                },
                {
                    "uses": "actions/download-artifact@revision",
                    "with": {"name": "cargo-dist-cache"},
                },
            ]
        )
        self.write_steps(
            [{"uses": "actions/cache@revision"}],
            ".github/workflows/release-prepare.yml",
        )
        self.check()

    def test_nested_workflows_and_composite_actions(self) -> None:
        nested = ".github/workflows/nested.yml"
        self.write_workflow(
            RELEASE_WORKFLOW, {"jobs": {"nested": {"uses": f"$/{nested}"}}}
        )
        self.write_workflow(
            nested,
            {
                "jobs": {
                    "cycle": {"uses": f"./{RELEASE_WORKFLOW}"},
                    "test": {"steps": [{"uses": "./.github/actions/nested"}]},
                }
            },
        )
        action = ".github/actions/nested/action.yaml"
        self.write(
            action,
            {
                "runs": {
                    "using": "composite",
                    "steps": [{"uses": "actions/cache/restore@revision"}],
                }
            },
        )
        self.check(f"{action}: runs.steps[0]: actions/cache/restore uses GitHub caches")
        self.write(
            action, {"runs": {"using": "composite", "steps": [{"run": "echo ok"}]}}
        )
        self.check(count=3)

    def test_unreviewed_actions_and_external_workflows_are_rejected(self) -> None:
        self.write_steps([{"uses": "example/new-action@revision"}])
        self.check(
            f"{RELEASE_WORKFLOW}: jobs.test.steps[0]: review example/new-action for GitHub cache use"
        )
        uses = "example/repo/.github/workflows/release.yml@revision"
        self.write_workflow(RELEASE_WORKFLOW, {"jobs": {"test": {"uses": uses}}})
        self.check(
            f"{RELEASE_WORKFLOW}: jobs.test: cannot inspect external workflow {uses}"
        )

    def test_local_javascript_actions_are_rejected(self) -> None:
        self.write_steps([{"uses": "./.github/actions/local"}])
        action = ".github/actions/local/action.yml"
        self.write(action, {"runs": {"using": "node24", "main": "index.js"}})
        self.check(f"{action}: cannot inspect non-composite local action")

    def test_reviewed_cache_proxy_is_allowed(self) -> None:
        self.write_steps([{"uses": CACHE_PROXY_USES}])
        action = ".github/actions/disable-github-caches/action.yml"
        self.write(
            action,
            {
                "runs": {
                    "using": "node24",
                    "pre": "pre.cjs",
                    "main": "main.cjs",
                    "post": "post.cjs",
                }
            },
        )
        self.check(count=2)

    def test_local_references_cannot_escape(self) -> None:
        self.write_workflow(
            RELEASE_WORKFLOW, {"jobs": {"test": {"uses": "./../outside.yml"}}}
        )
        self.check(
            f"{RELEASE_WORKFLOW}: jobs.test: local reference escapes the repository"
        )

    def test_depot_cache_backends(self) -> None:
        for name in ("cache-from", "cache-to"):
            for value in (
                "type=gha",
                '"type=gha,scope=release"',
                "type=registry,ref=image\ntype=gha,mode=max",
                "${{ inputs.cache }}",
            ):
                with self.subTest(name=name, value=value):
                    self.write_steps(
                        [
                            {
                                "uses": "depot/build-push-action@revision",
                                "with": {name: value},
                            }
                        ]
                    )
                    self.check(
                        f"{RELEASE_WORKFLOW}: jobs.test.steps[0]: {name} may use the GitHub cache backend"
                    )
        self.write_steps(
            [
                {
                    "uses": "depot/build-push-action@revision",
                    "with": {"cache-from": "type=registry,ref=image"},
                }
            ]
        )
        self.check()


if __name__ == "__main__":
    unittest.main()
