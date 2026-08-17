"""Exercise promotion planning and publication without GitHub writes."""

from __future__ import annotations

import copy
import subprocess
import tempfile
import unittest
from pathlib import Path
from typing import Any
from unittest.mock import patch

from scripts import pull_request_promotion as promotion

MAIN = "a" * 40
PARENT = "b" * 40
CHILD = "c" * 40
MERGE = "d" * 40
REBASED = "e" * 40


def repository(name: str) -> dict[str, Any]:
    return {"id": promotion.REPOSITORY_IDS[name], "full_name": name}


def pull_request(
    number: int,
    head: str,
    sha: str,
    *,
    base: str = "main",
    base_sha: str = MAIN,
    head_repository: str = promotion.UV_DEV,
    base_repository: str = promotion.UV_DEV,
    state: str = "open",
    merged: bool = False,
    body: str = "Related to #12 and astral-sh/uv#34.",
) -> dict[str, Any]:
    return {
        "number": number,
        "title": "A focused change",
        "body": body,
        "state": "closed" if merged else state,
        "draft": False,
        "merged_at": "2026-08-17T00:00:00Z" if merged else None,
        "merge_commit_sha": MERGE if merged else None,
        "user": {"login": promotion.BOT},
        "base": {"ref": base, "sha": base_sha, "repo": repository(base_repository)},
        "head": {"ref": head, "sha": sha, "repo": repository(head_repository)},
        "labels": [{"name": "bug"}],
    }


class FakeGitHub(promotion.GitHub):
    def __init__(self) -> None:
        self.pull_requests: dict[tuple[str, int], dict[str, Any]] = {}
        self.refs = {(promotion.UV, "main"): MAIN, (promotion.UV_DEV, "main"): MAIN}
        self.writes: list[tuple[str, str, dict[str, Any]]] = []
        self.commands: list[tuple[str, ...]] = []
        self.comparison = "ahead"
        self.human_promoter = True
        self.move_source_on_create = False

    def add(self, source: dict[str, Any]) -> dict[str, Any]:
        self.pull_requests[(source["base"]["repo"]["full_name"], source["number"])] = (
            source
        )
        return source

    def promoted(
        self, source: dict[str, Any], *, merged: bool = False, sha: str | None = None
    ) -> dict[str, Any]:
        return self.add(
            pull_request(
                20000 + source["number"],
                promotion.promotion_ref(source["number"]),
                sha or source["head"]["sha"],
                base_repository=promotion.UV,
                merged=merged,
                body=promotion.marker(source),
            )
        )

    def api(self, path: str, method: str = "GET", **data: Any) -> Any:
        if method == "GET":
            if "/compare/" in path:
                return {"status": self.comparison}
            repository_name, number = path.removeprefix("repos/").split("/pulls/")
            return copy.deepcopy(self.pull_requests[(repository_name, int(number))])
        self.writes.append((path, method, data))
        if path == f"repos/{promotion.UV_DEV}/git/refs":
            self.refs[(promotion.UV_DEV, data["ref"].removeprefix("refs/heads/"))] = (
                data["sha"]
            )
        elif path == f"repos/{promotion.UV}/pulls":
            branch = data["head"].removeprefix("astral-sh:")
            created = self.add(
                pull_request(
                    21000,
                    branch,
                    self.refs[(promotion.UV_DEV, branch)],
                    base=data["base"],
                    base_repository=promotion.UV,
                    body=data["body"],
                )
            )
            if self.move_source_on_create:
                self.pull_requests[(promotion.UV_DEV, 7)]["head"]["sha"] = CHILD
            return copy.deepcopy(created)
        return None

    def pulls(self, repository: str, **filters: str) -> list[dict[str, Any]]:
        return [
            copy.deepcopy(source)
            for (owner, _), source in self.pull_requests.items()
            if owner == repository
            and ("head" not in filters or source["head"]["ref"] == filters["head"])
            and ("state" not in filters or source["state"] == filters["state"])
        ]

    def events(self, number: int) -> list[dict[str, Any]]:
        return [
            {
                "event": "ready_for_review",
                "actor": {
                    "type": "User" if self.human_promoter else "Bot",
                    "login": "zanieb",
                },
            }
        ]

    def ref(self, repository: str, branch: str) -> str | None:
        return self.refs.get((repository, branch))

    def run(self, *arguments: str, input: str | None = None) -> str:
        self.commands.append(arguments)
        if arguments[:3] == ("gh", "pr", "close"):
            self.pull_requests[(promotion.UV_DEV, int(arguments[3]))]["state"] = (
                "closed"
            )
        elif arguments[0] == "git" and "push" in arguments:
            sha, branch = arguments[-1].split(":refs/heads/")
            self.refs[(promotion.UV_DEV, branch)] = sha
        else:
            raise AssertionError(arguments)
        return ""


class PromotionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.github = FakeGitHub()
        self.source = self.github.add(pull_request(7, "zb/fix", PARENT))
        self.run = patch.object(promotion, "run", self.github.run)
        self.run.start()
        self.addCleanup(self.run.stop)

    def test_fresh_branch_is_created_only_in_uv_dev(self) -> None:
        expected = promotion.plan(self.github, 7)
        self.assertEqual(promotion.publish(self.github, expected, PARENT), 21000)
        self.assertEqual(
            self.github.refs[(promotion.UV_DEV, "automations/promote/7")], PARENT
        )
        self.assertNotIn((promotion.UV, "zb/fix"), self.github.refs)
        self.assertEqual(self.source["head"]["sha"], PARENT)
        self.assertEqual(self.source["state"], "closed")
        self.assertEqual(
            self.github.writes[:2],
            [
                (
                    f"repos/{promotion.UV_DEV}/git/refs",
                    "POST",
                    {
                        "ref": "refs/heads/automations/promote/7",
                        "sha": PARENT,
                    },
                ),
                (
                    f"repos/{promotion.UV}/pulls",
                    "POST",
                    {
                        "title": "A focused change",
                        "body": f"Related to astral-sh/uv-dev#12 and astral-sh/uv#34.\n\n{promotion.marker(self.source)}",
                        "base": "main",
                        "head": "astral-sh:automations/promote/7",
                        "head_repo": promotion.UV_DEV,
                    },
                ),
            ],
        )

    def test_retry_reuses_an_unclaimed_matching_branch(self) -> None:
        self.github.refs[(promotion.UV_DEV, promotion.promotion_ref(7))] = PARENT
        promotion.publish(self.github, promotion.plan(self.github, 7), PARENT)
        self.assertFalse(
            any(path.endswith("/git/refs") for path, _, _ in self.github.writes)
        )

    def test_branch_collision_is_not_overwritten(self) -> None:
        self.github.refs[(promotion.UV_DEV, promotion.promotion_ref(7))] = CHILD
        with self.assertRaisesRegex(promotion.PromotionError, "different revision"):
            promotion.publish(self.github, promotion.plan(self.github, 7), PARENT)
        self.assertEqual(self.github.writes, [])
        self.assertEqual(self.github.commands, [])

    def test_retry_reuses_an_independently_advanced_promotion(self) -> None:
        existing = self.github.promoted(self.source, sha=REBASED)
        self.assertEqual(
            promotion.publish(self.github, promotion.plan(self.github, 7), PARENT),
            existing["number"],
        )
        self.assertFalse(
            any(
                path.endswith(("/pulls", "/git/refs"))
                for path, _, _ in self.github.writes
            )
        )
        self.assertEqual(self.source["head"]["sha"], PARENT)

    def test_legacy_copied_and_direct_promotions_are_recognized(self) -> None:
        for head_repository in (promotion.UV, promotion.UV_DEV):
            with self.subTest(head_repository=head_repository):
                github = FakeGitHub()
                source = github.add(copy.deepcopy(self.source))
                existing = github.add(
                    pull_request(
                        20007,
                        "zb/fix",
                        PARENT,
                        head_repository=head_repository,
                        base_repository=promotion.UV,
                    )
                )
                self.assertEqual(promotion.find_promotion(github, source), existing)

    def test_wrong_source_marker_is_rejected(self) -> None:
        existing = self.github.promoted(self.source)
        existing["body"] = existing["body"].replace(PARENT, CHILD)
        with self.assertRaisesRegex(promotion.PromotionError, "source revision"):
            promotion.plan(self.github, 7)

    def test_duplicate_promotions_are_rejected(self) -> None:
        self.github.promoted(self.source)
        self.github.add(
            pull_request(
                20008,
                "zb/fix",
                PARENT,
                head_repository=promotion.UV,
                base_repository=promotion.UV,
            )
        )
        with self.assertRaisesRegex(promotion.PromotionError, "multiple upstream"):
            promotion.plan(self.github, 7)

    def test_human_readiness_and_expected_head_are_required(self) -> None:
        self.github.human_promoter = False
        with self.assertRaisesRegex(promotion.PromotionError, "human promoter"):
            promotion.plan(self.github, 7)
        self.github.human_promoter = True
        with self.assertRaisesRegex(promotion.PromotionError, "head changed"):
            promotion.plan(self.github, 7, CHILD)

    def test_source_change_prevents_publication(self) -> None:
        expected = promotion.plan(self.github, 7)
        self.source["head"]["sha"] = CHILD
        with self.assertRaisesRegex(
            promotion.PromotionError, "changed before publication"
        ):
            promotion.publish(self.github, expected, PARENT)
        self.assertEqual(self.github.writes, [])

    def test_source_change_prevents_closure(self) -> None:
        self.github.move_source_on_create = True
        with self.assertRaisesRegex(
            promotion.PromotionError, "before it could be closed"
        ):
            promotion.publish(self.github, promotion.plan(self.github, 7), PARENT)
        self.assertEqual(self.source["state"], "open")
        self.assertEqual(self.github.commands, [])

    def child(self) -> dict[str, Any]:
        return self.github.add(
            pull_request(8, "zb/child", CHILD, base="zb/fix", base_sha=PARENT)
        )

    def test_child_waits_even_when_a_copied_base_exists(self) -> None:
        self.child()
        self.github.refs[(promotion.UV, "zb/fix")] = PARENT
        self.assertEqual(promotion.plan(self.github, 8)["action"], "wait")
        self.github.promoted(self.source)
        self.assertEqual(promotion.plan(self.github, 8)["action"], "wait")
        self.assertEqual(self.github.writes, [])

    def test_merged_parent_uses_original_source_tip(self) -> None:
        child = self.child()
        self.github.promoted(self.source, merged=True, sha=REBASED)
        expected = promotion.plan(self.github, 8)
        self.assertEqual(
            {
                key: expected[key]
                for key in (
                    "action",
                    "base_ref",
                    "base_sha",
                    "previous_sha",
                    "merge_sha",
                )
            },
            {
                "action": "rebase",
                "base_ref": "main",
                "base_sha": MAIN,
                "previous_sha": PARENT,
                "merge_sha": MERGE,
            },
        )
        promotion.publish(self.github, expected, REBASED)
        self.assertEqual(child["head"]["sha"], CHILD)
        self.assertEqual(child["base"]["ref"], "zb/fix")
        self.assertEqual(
            self.github.refs[(promotion.UV_DEV, promotion.promotion_ref(8))], REBASED
        )
        self.assertIn(
            "--force-with-lease=refs/heads/automations/promote/8:",
            self.github.commands[0],
        )

    def test_child_waits_for_the_fork_sync(self) -> None:
        self.child()
        self.github.promoted(self.source, merged=True)
        self.github.comparison = "behind"
        self.assertEqual(promotion.plan(self.github, 8)["action"], "wait")

    def test_legacy_ancestors_must_also_merge(self) -> None:
        self.child()
        parent = self.github.promoted(self.source, merged=True)
        parent["base"]["ref"] = "zb/grandparent"
        ancestor = self.github.add(
            pull_request(
                19999,
                "zb/grandparent",
                REBASED,
                head_repository=promotion.UV,
                base_repository=promotion.UV,
            )
        )
        self.assertEqual(promotion.plan(self.github, 8)["action"], "wait")
        ancestor.update(state="closed", merged_at="2026-08-17", merge_commit_sha=MERGE)
        expected = promotion.plan(self.github, 8)
        self.assertEqual((expected["action"], expected["base_ref"]), ("rebase", "main"))

    def test_stale_parent_tip_is_rejected(self) -> None:
        child = self.child()
        child["base"]["sha"] = REBASED
        with self.assertRaisesRegex(promotion.PromotionError, "stacked base"):
            promotion.plan(self.github, 8)


class RangeRebaseTests(unittest.TestCase):
    def test_squashed_parent_is_not_replayed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)

            def git(*arguments: str) -> str:
                return subprocess.run(
                    [
                        "git",
                        "-C",
                        str(root),
                        "-c",
                        "core.hooksPath=/dev/null",
                        *arguments,
                    ],
                    check=True,
                    text=True,
                    capture_output=True,
                ).stdout.strip()

            git("init", "--initial-branch=main")
            git("config", "user.name", "Promotion Test")
            git("config", "user.email", "promotion@example.com")
            (root / "base").write_text("base\n")
            git("add", ".")
            git("commit", "-m", "base")
            git("checkout", "-b", "parent")
            (root / "parent").write_text("original parent\n")
            git("add", ".")
            git("commit", "-m", "parent")
            parent = git("rev-parse", "HEAD")
            git("checkout", "-b", "child")
            (root / "child").write_text("child\n")
            git("add", ".")
            git("commit", "-m", "child")
            child = git("rev-parse", "HEAD")
            git("checkout", "main")
            (root / "parent").write_text("reviewed parent\n")
            git("add", ".")
            git("commit", "-m", "squashed parent")
            git("checkout", "--detach", child)
            git("rebase", "--onto", "main", parent)
            self.assertEqual(git("log", "--format=%s", "main..HEAD"), "child")
            self.assertEqual((root / "parent").read_text(), "reviewed parent\n")
            self.assertEqual(git("rev-parse", "child"), child)
            git("bundle", "create", str(root / "rebased.bundle"), "main..HEAD")
            git("bundle", "verify", str(root / "rebased.bundle"))


if __name__ == "__main__":
    unittest.main()
