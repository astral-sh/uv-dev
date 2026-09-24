"""Narrow writes for promotion metadata and source completion."""

import json
import re
import subprocess
from datetime import UTC, datetime
from email.utils import parsedate_to_datetime
from typing import Literal, override

from uv_automations.github_promotion import (
    PromotionGitHub,
    PromotionReadError,
    decode_promotion_comment,
)
from uv_automations.json import loads
from uv_automations.promotion_models import (
    UV_REPOSITORY,
    PromotionScope,
    require_promotion_source,
)
from uv_automations.workflows.promotion_completion import (
    CompletionRequestError,
    PromotionCompletion,
)


def _retry_after(value: str | None, now: datetime) -> float | None:
    if value is None:
        return None
    if len(value) > 100:
        return float("inf") if value.isascii() and value.isdigit() else None
    if re.fullmatch(r"[0-9]+", value, flags=re.ASCII):
        # Keep oversized values outside the retry budget without parsing an
        # arbitrarily large integer or converting it to an overflowing float.
        return float(value) if len(value) <= 10 else float("inf")
    try:
        deadline = parsedate_to_datetime(value)
        if deadline.tzinfo is None:
            return None
        return max(0.0, (deadline - now).total_seconds())
    except OverflowError, TypeError, ValueError:
        return None


def _response(value: str) -> tuple[int | None, str | None, str]:
    header, separator, body = value.replace("\r\n", "\n").partition("\n\n")
    if not separator or len(header) > 65_536:
        return None, None, ""
    lines = header.split("\n")
    status = re.fullmatch(r"HTTP/[0-9.]+ ([1-5][0-9]{2})(?: [^\r\n]*)?", lines[0])
    if status is None:
        return None, None, ""
    retry_after: list[str] = []
    for line in lines[1:]:
        key, colon, content = line.partition(":")
        if not colon or re.fullmatch(r"[!#$%&'*+.^_`|~0-9A-Za-z-]+", key) is None:
            raise CompletionRequestError(int(status[1]), float("inf"))
        if key.lower() == "retry-after":
            retry_after.append(content.strip(" \t"))
    if len(retry_after) > 1:
        raise CompletionRequestError(int(status[1]), float("inf"))
    return int(status[1]), retry_after[0] if retry_after else None, body


class PromotionCompletionGitHub(PromotionGitHub):
    @override
    def _api(
        self,
        method: Literal["GET", "POST", "PATCH", "DELETE"],
        path: str,
        *,
        payload: object | None = None,
    ) -> object:
        return self._completion_api(method, path, payload)

    def _completion_api(
        self,
        method: Literal["GET", "POST", "PATCH", "DELETE"],
        path: str,
        payload: object | None,
    ) -> object:
        arguments = [self.executable, "api", "--method", method, path, "--include"]
        if payload is not None:
            arguments.extend(("--input", "-"))
        try:
            response = subprocess.run(
                arguments,
                input=json.dumps(payload, allow_nan=False)
                if payload is not None
                else None,
                check=False,
                text=True,
                capture_output=True,
                env=self._environment(),
                timeout=60,
            )
        except OSError, subprocess.TimeoutExpired:
            raise CompletionRequestError(None) from None
        status, header, body = _response(response.stdout)
        if response.returncode != 0 or status is None or not 200 <= status <= 299:
            raise CompletionRequestError(
                status, _retry_after(header, datetime.now(UTC))
            ) from None
        try:
            return loads(body) if body.strip() else None
        except TypeError, ValueError:
            raise PromotionReadError("Invalid GitHub promotion response") from None

    def add_promotion_labels(
        self, upstream: PromotionScope, labels: tuple[str, ...]
    ) -> None:
        if upstream.repository != UV_REPOSITORY:
            raise ValueError("Unexpected promotion metadata destination")
        self._completion_api(
            "POST",
            f"repos/{UV_REPOSITORY.name}/issues/{upstream.number}/labels",
            {"labels": labels},
        )

    def assign_promotion(self, upstream: PromotionScope, login: str) -> None:
        if upstream.repository != UV_REPOSITORY:
            raise ValueError("Unexpected promotion metadata destination")
        self._completion_api(
            "POST",
            f"repos/{UV_REPOSITORY.name}/issues/{upstream.number}/assignees",
            {"assignees": [login]},
        )

    def record_promotion(self, completion: PromotionCompletion) -> None:
        source = completion.source
        comment = decode_promotion_comment(
            self._completion_api(
                "POST",
                f"repos/{source.repository.name}/issues/{source.number}/comments",
                {"body": completion.comment},
            ),
            source,
        )
        if not comment.is_automation or comment.body != completion.comment:
            raise ValueError("GitHub did not create the expected promotion record")

    def close_promotion_source(self, source: PromotionScope) -> None:
        require_promotion_source(source.repository)
        self._completion_api(
            "PATCH",
            f"repos/{source.repository.name}/pulls/{source.number}",
            {"state": "closed"},
        )
