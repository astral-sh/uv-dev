import json
import subprocess
import sys
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from uv_automations.workflows.labels import (
    MAX_LABELS,
    LabelPlan,
    LabelRecommendation,
    plan_labels,
)


class LabelTests(unittest.TestCase):
    def test_plan_labels(self) -> None:
        for labels in [(), ("bug",), ("testing", "bug"), ("bug", "testing", "ci")]:
            with self.subTest(labels=labels):
                recommendation = LabelRecommendation.from_json(
                    {"labels": list(labels), "summary": "Reason"}
                )
                self.assertEqual(
                    plan_labels(recommendation, {"bug", "testing", "ci"}),
                    LabelPlan(labels),
                )

    def test_reject_invalid_recommendation(self) -> None:
        values: list[object] = [
            None,
            [],
            {},
            {"labels": ["bug"]},
            {"labels": [], "summary": None},
            {"labels": "bug", "summary": ""},
            {"labels": [1], "summary": ""},
            {"labels": ["a", "b", "c", "d"], "summary": ""},
            {"labels": [], "summary": "", "extra": True},
        ]
        for value in values:
            with self.subTest(value=value), self.assertRaises((TypeError, ValueError)):
                LabelRecommendation.from_json(value)

    def test_reject_invalid_plan(self) -> None:
        for labels in [("bug", "bug"), ("unknown",)]:
            with self.subTest(labels=labels), self.assertRaises(ValueError):
                plan_labels(LabelRecommendation(labels, ""), {"bug"})

    def test_schema_contract(self) -> None:
        repository = Path(__file__).resolve().parents[3]
        schema = json.loads(
            (repository / "agents/schemas/pull-request-labels.json").read_text()
        )
        self.assertEqual(set(schema["required"]), {"labels", "summary"})
        self.assertEqual(set(schema["properties"]), {"labels", "summary"})
        self.assertIs(schema["additionalProperties"], False)
        self.assertEqual(schema["properties"]["labels"]["maxItems"], MAX_LABELS)
        self.assertEqual(schema["properties"]["labels"]["items"], {"type": "string"})
        self.assertEqual(schema["properties"]["summary"], {"type": "string"})

    def test_cli(self) -> None:
        with TemporaryDirectory() as directory:
            allowed = Path(directory) / "allowed.json"
            allowed.write_text('["bug", "testing"]', encoding="utf-8")
            output = Path(directory) / "github-output"
            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "uv_automations",
                    "labels",
                    "validate",
                    "--allowed",
                    str(allowed),
                    "--github-output",
                    str(output),
                ],
                input='{"labels":["testing","bug"],"summary":"Reason"}',
                text=True,
                capture_output=True,
                check=True,
            )
            self.assertEqual(result.stdout, "")
            self.assertEqual(result.stderr, "")
            self.assertEqual(output.read_text(), 'labels=["testing","bug"]\n')

    def test_cli_rejects_invalid_json(self) -> None:
        with TemporaryDirectory() as directory:
            allowed = Path(directory) / "allowed.json"
            allowed.write_text('["bug"]', encoding="utf-8")
            output = Path(directory) / "github-output"
            for value in [
                '{"labels":[],"summary":""}\n{"labels":["bug"],"summary":""}',
                '{"labels":["bug"],"labels":[],"summary":""}',
                '{"labels":[],"summary":NaN}',
            ]:
                with self.subTest(value=value):
                    result = subprocess.run(
                        [
                            sys.executable,
                            "-m",
                            "uv_automations",
                            "labels",
                            "validate",
                            "--allowed",
                            str(allowed),
                            "--github-output",
                            str(output),
                        ],
                        input=value,
                        text=True,
                        capture_output=True,
                        check=False,
                    )
                    self.assertEqual(result.returncode, 2)
                    self.assertFalse(output.exists())
