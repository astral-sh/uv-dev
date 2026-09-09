"""The single GitHub mutation used by post-sync promotion retargeting."""

from uv_automations.github_promotion import (
    PromotionGitHub,
    PromotionReadError,
    decode_promotion_pull_request,
)
from uv_automations.promotion_models import (
    PromotionPullRequest,
    PromotionScope,
    require_promotion_source,
)


class PromotionRetargetGitHub(PromotionGitHub):
    def retarget_to_main(self, source: PromotionScope) -> PromotionPullRequest:
        require_promotion_source(source.repository)
        response = self._api(
            "PATCH",
            f"repos/{source.repository.name}/pulls/{source.number}",
            payload={"base": "main"},
        )
        try:
            return decode_promotion_pull_request(response, source)
        except KeyError, TypeError, ValueError:
            raise PromotionReadError("Invalid GitHub retarget response") from None
