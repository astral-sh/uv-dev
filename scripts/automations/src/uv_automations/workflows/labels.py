"""Validate the existing pull-request label recommendation contract."""

from collections.abc import Collection
from dataclasses import dataclass

from uv_automations.json import as_array, as_object, as_string

MAX_LABELS = 3


@dataclass(frozen=True, slots=True)
class LabelRecommendation:
    labels: tuple[str, ...]
    summary: str

    def __post_init__(self) -> None:
        if len(self.labels) > MAX_LABELS:
            raise ValueError(f"Expected at most {MAX_LABELS} labels")

    @classmethod
    def from_json(cls, value: object) -> LabelRecommendation:
        data = as_object(value)
        if data.keys() != {"labels", "summary"}:
            raise ValueError("Expected exactly 'labels' and 'summary'")
        labels = tuple(as_string(label) for label in as_array(data["labels"]))
        return cls(labels, as_string(data["summary"]))


@dataclass(frozen=True, slots=True)
class LabelPlan:
    labels: tuple[str, ...]


def plan_labels(
    recommendation: LabelRecommendation, allowed: Collection[str]
) -> LabelPlan:
    if len(set(recommendation.labels)) != len(recommendation.labels):
        raise ValueError("Recommended labels must be unique")
    if disallowed := set(recommendation.labels).difference(allowed):
        raise ValueError(f"Disallowed pull request labels: {sorted(disallowed)!r}")
    return LabelPlan(recommendation.labels)
