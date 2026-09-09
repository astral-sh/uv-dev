import json
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch

from uv_automations.artifacts import CommitBundle, CommitRange
from uv_automations.models import CommitSha, RepositoryIdentity, RepositoryName
from uv_automations.rebase_cli import (
    CloseEmpty,
    Persist,
    VerifyEmpty,
    create_parser,
    main,
    parse_command,
    run,
)
from uv_automations.workflows.rebase import (
    EmptyRebase,
    PersistedRebase,
    PreparedRebase,
    RebaseSource,
    SkippedRebase,
    VerifiedEmptyRebase,
)

UV_DEV = RepositoryIdentity(RepositoryName("astral-sh/uv-dev"), 1302176231)
BASE = CommitSha("a" * 40)
HEAD = CommitSha("b" * 40)
REBASE = PreparedRebase(
    RebaseSource(
        repository=UV_DEV,
        number=123,
        base_ref="main",
        head_repository=UV_DEV.name,
        head_ref="feature",
        head_sha=HEAD,
    ),
    BASE,
)


class RebaseCliTests(unittest.TestCase):
    def test_parse_close_empty(self) -> None:
        parsed = create_parser().parse_args(
            [
                "close-empty",
                "--repo",
                str(UV_DEV.name),
                "--repository-id",
                str(UV_DEV.database_id),
                "--pull-request",
                "123",
                "--base-ref",
                "main",
                "--base-sha",
                str(BASE),
                "--head-repository",
                str(UV_DEV.name),
                "--head-repository-id",
                str(UV_DEV.database_id),
                "--head-ref",
                "feature",
                "--head-sha",
                str(HEAD),
                "--run-id",
                "456",
                "--summary",
                "summary.md",
            ]
        )
        self.assertEqual(
            parse_command(parsed),
            CloseEmpty(
                verified=VerifiedEmptyRebase(REBASE, UV_DEV),
                run_id=456,
                summary=Path("summary.md"),
            ),
        )

    def test_persist_outputs_both_empty_and_nonempty_results(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            bundle = root / "rebased.bundle"
            for name, result, empty, head in (
                ("empty", EmptyRebase(BASE), True, BASE),
                (
                    "nonempty",
                    PersistedRebase(CommitBundle(bundle, CommitRange(BASE, HEAD))),
                    False,
                    HEAD,
                ),
            ):
                with self.subTest(name=name):
                    output = root / name
                    with patch(
                        "uv_automations.rebase_cli.persist_rebase", return_value=result
                    ):
                        run(
                            Persist(
                                checkout=root,
                                base_sha=BASE,
                                bundle=bundle,
                                github_output=output,
                            )
                        )
                    self.assertEqual(
                        output.read_text(),
                        f"empty={json.dumps(empty)}\nhead_sha={head}\n",
                    )

    def test_verify_outputs_only_an_independently_verified_identity(self) -> None:
        with TemporaryDirectory() as directory:
            root = Path(directory)
            for name, result, expected in (
                (
                    "verified",
                    VerifiedEmptyRebase(REBASE, UV_DEV),
                    "close=true\nhead_repository_id=1302176231\n",
                ),
                ("skipped", SkippedRebase("Still has changes"), "close=false\n"),
            ):
                with self.subTest(name=name):
                    output = root / name
                    summary = root / f"{name}.md"
                    with patch(
                        "uv_automations.rebase_cli.verify_empty_rebase",
                        return_value=result,
                    ):
                        run(
                            VerifyEmpty(
                                checkout=root,
                                rebase=REBASE,
                                github_output=output,
                                summary=summary,
                            )
                        )
                    self.assertEqual(output.read_text(), expected)
                    if isinstance(result, SkippedRebase):
                        self.assertEqual(summary.read_text(), "Still has changes\n")

    def test_standalone_main_dispatches_without_live_github(self) -> None:
        with patch("uv_automations.rebase_cli.run") as execute:
            main(
                [
                    "persist",
                    "--checkout",
                    ".",
                    "--base-sha",
                    str(BASE),
                    "--bundle",
                    "rebased.bundle",
                    "--github-output",
                    "output",
                ]
            )
        execute.assert_called_once_with(
            Persist(
                checkout=Path("."),
                base_sha=BASE,
                bundle=Path("rebased.bundle"),
                github_output=Path("output"),
            )
        )
