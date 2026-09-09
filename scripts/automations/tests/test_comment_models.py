import json
import unittest
from dataclasses import replace
from datetime import UTC, datetime
from pathlib import Path

from uv_automations.comment_models import (
    AUTOMATION_LOGINS,
    ActorKind,
    AuthorAssociation,
    CommentAction,
    CommentAuthor,
    CommentOutcome,
    CommentRecommendation,
    CommentTarget,
    CommentTargetKind,
    sanitize_comment_body,
)
from uv_automations.json import as_boolean, require_keys
from uv_automations.models import CommitSha, Timestamp

HEAD = CommitSha("b" * 40)
TRUSTED = CommentAuthor("maintainer", ActorKind.USER, AuthorAssociation.MEMBER)


class CommentModelTests(unittest.TestCase):
    def test_checked_in_schema_is_generated_from_the_decoder(self) -> None:
        root = Path(__file__).resolve().parents[3]
        schema = json.loads(
            (root / "agents/schemas/pull-request-comments.json").read_text()
        )
        self.assertEqual(schema, CommentRecommendation.schema())

    def test_recommendation_rejects_ambiguous_or_incomplete_actions(self) -> None:
        target = CommentTarget(CommentTargetKind.REVIEW_THREAD, "PRRT_example")
        valid = CommentAction(target, CommentOutcome.COMMIT_AND_RESOLVE, "", HEAD)
        payload = CommentRecommendation("Addressed feedback", (valid,)).to_json()
        self.assertEqual(CommentRecommendation.from_json(payload).to_json(), payload)
        for action in [
            {**valid.to_json(), "addressing_commit": None},
            {**valid.to_json(), "body": "unexpected reply"},
            {
                **valid.to_json(),
                "outcome": "RESPOND",
                "body": "",
                "addressing_commit": None,
            },
            {**valid.to_json(), "outcome": "IGNORE"},
            {**valid.to_json(), "outcome": "NO_ACTION"},
            {**valid.to_json(), "extra": True},
            {
                **valid.to_json(),
                "target": "CONVERSATION_COMMENT",
                "id": "0",
                "body": "done",
            },
        ]:
            with (
                self.subTest(action=action),
                self.assertRaises((ValueError, TypeError)),
            ):
                CommentRecommendation.from_json(
                    {"summary": "test", "actions": [action]}
                )
        with self.assertRaisesRegex(ValueError, "only one action"):
            CommentRecommendation("duplicate", (valid, valid))
        no_action = CommentAction(target, CommentOutcome.NO_ACTION, "", None)
        self.assertEqual(CommentAction.from_json(no_action.to_json()), no_action)

    def test_comment_sanitization_is_idempotent(self) -> None:
        value = (
            "  Hello &#64;maintainer and @org/team. <!-- fake marker -->\r\nThanks. "
        )
        expected = "Hello @\u200bmaintainer and @\u200borg/team. &lt;!-- fake marker --&gt;\nThanks."
        self.assertEqual(sanitize_comment_body(value), expected)
        self.assertEqual(sanitize_comment_body(expected), expected)
        for value in (
            "",
            " \n",
            "bad\x00body",
            "bad\rbody",
            "bad&#13;body",
            "bad\u202ebody",
        ):
            with self.subTest(value=value), self.assertRaises(ValueError):
                sanitize_comment_body(value)

    def test_only_human_collaborator_feedback_is_actionable(self) -> None:
        self.assertTrue(TRUSTED.can_trigger)
        bot = CommentAuthor(
            "astral-automations-bot[bot]", ActorKind.BOT, AuthorAssociation.MEMBER
        )
        self.assertFalse(bot.can_trigger)
        self.assertTrue(bot.is_publisher)
        self.assertFalse(replace(bot, kind=ActorKind.USER).is_publisher)
        for association in AuthorAssociation:
            with self.subTest(association=association):
                author = replace(TRUSTED, association=association)
                self.assertEqual(
                    author.can_trigger,
                    association
                    in {
                        AuthorAssociation.OWNER,
                        AuthorAssociation.MEMBER,
                        AuthorAssociation.COLLABORATOR,
                    },
                )

    def test_automation_logins_cannot_trigger_even_as_users(self) -> None:
        for login in AUTOMATION_LOGINS:
            with self.subTest(login=login):
                author = replace(TRUSTED, login=login)
                self.assertTrue(author.is_automation)
                self.assertFalse(author.can_trigger)

    def test_json_booleans_and_fields_are_exact(self) -> None:
        self.assertTrue(as_boolean(True))
        self.assertFalse(as_boolean(False))
        for value in (0, 1, "true", None):
            with self.subTest(value=value), self.assertRaises(TypeError):
                as_boolean(value)
        require_keys({"one": 1}, {"one"})
        with self.assertRaises(ValueError):
            require_keys({"one": 1, "extra": 2}, {"one"})

    def test_timestamps_are_canonical_and_support_overlapping_windows(self) -> None:
        timestamp = Timestamp.parse("2026-09-08T12:00:00Z")
        self.assertEqual(str(timestamp.overlap()), "2026-09-08T11:59:59Z")
        for value in (
            "2026-9-8T12:00:00Z",
            "2026-09-08T12:00:00+00:00",
            "2026-09-08T12:00:00.1Z",
        ):
            with self.subTest(value=value), self.assertRaises(ValueError):
                Timestamp.parse(value)
        with self.assertRaises(ValueError):
            Timestamp(datetime(2026, 9, 8, tzinfo=UTC, microsecond=1))
