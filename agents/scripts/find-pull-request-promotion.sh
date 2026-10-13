#!/usr/bin/env bash
# Find an upstream promotion and verify its source identity without changing either pull request.
set -euo pipefail

readonly UV_REPOSITORY_ID=699532645
readonly UV_DEV_REPOSITORY_ID=1302176231
readonly AUTOMATIONS_BOT_ID=305554984

verify_rebased_legacy_promotion() {
  local source_repository="$1" source_number="$2" source_ref="$3" source_sha="$4" upstream="$5"
  local source_repository_id source upstream_number comments history query
  if ! jq --exit-status --argjson uv_id "$UV_REPOSITORY_ID" --argjson bot_id "$AUTOMATIONS_BOT_ID" \
    '.head.repo.id == $uv_id and .head.repo.full_name == "astral-sh/uv" and .user.id == $bot_id' \
    <<< "$upstream" > /dev/null; then
    echo "The rebased legacy promotion is not an upstream branch created by the automation bot." >&2
    return 1
  fi
  if [ "$source_repository" = astral-sh/uv-dev ]; then
    source_repository_id="$UV_DEV_REPOSITORY_ID"
  else
    source_repository_id=$(gh api "repos/$source_repository" --jq .id) || return 1
  fi
  source=$(gh api "repos/$source_repository/pulls/$source_number") || return 1
  if ! jq --exit-status --arg source_ref "$source_ref" --arg source_sha "$source_sha" \
    --arg source_repository "$source_repository" --argjson source_id "$source_repository_id" '
      .state == "closed" and .head.ref == $source_ref and .head.sha == $source_sha
      and .base.repo.id == $source_id and .head.repo.id == $source_id
      and .base.repo.full_name == $source_repository and .head.repo.full_name == $source_repository
    ' <<< "$source" > /dev/null; then
    echo "The closed source of the legacy promotion no longer matches its verified revision." >&2
    return 1
  fi
  upstream_number=$(jq --raw-output .number <<< "$upstream")
  comments=$(gh api "repos/$source_repository/issues/$source_number/comments" --paginate --slurp) || return 1
  if ! jq --exit-status --argjson bot_id "$AUTOMATIONS_BOT_ID" \
    --arg legacy_body "Promoted to [#$upstream_number](https://github.com/astral-sh/uv/pull/$upstream_number)." \
    --arg body "Promoted to astral-sh/uv#$upstream_number." '
      any(.[][]; .user.id == $bot_id and .performed_via_github_app.slug == "astral-automations-bot"
        and (.body == $legacy_body or .body == $body))
    ' <<< "$comments" > /dev/null; then
    echo "The automation bot did not record this legacy promotion on its source pull request." >&2
    return 1
  fi
  query=$(cat <<'GRAPHQL'
query($number: Int!, $endCursor: String) {
  repository(owner: "astral-sh", name: "uv") {
    pullRequest(number: $number) {
      timelineItems(first: 100, after: $endCursor, itemTypes: [HEAD_REF_FORCE_PUSHED_EVENT]) {
        nodes {
          ... on HeadRefForcePushedEvent {
            beforeCommit { oid }
            afterCommit { oid }
          }
        }
        pageInfo { hasNextPage endCursor }
      }
    }
  }
}
GRAPHQL
  )
  history=$(gh api graphql --paginate --slurp --field number="$upstream_number" --raw-field query="$query") || return 1
  if ! jq --exit-status --arg source_sha "$source_sha" \
    'any(.[].data.repository.pullRequest.timelineItems.nodes[]; .beforeCommit.oid == $source_sha or .afterCommit.oid == $source_sha)' \
    <<< "$history" > /dev/null; then
    echo "The source revision is not recorded in the upstream force-push history." >&2
    return 1
  fi
}

find_promotion() {
  local source_repository="$1" source_number="$2" source_ref="$3" source_sha="$4"
  local promotion_ref marker promotion_repository promotion_repository_id
  local dedicated historical candidates number pull_request
  if [ "$source_repository" = astral-sh/uv-security ]; then
    promotion_ref="automations/promote/security/$source_number"
    marker="<!-- uv-security-promotion:$source_number:$source_sha -->"
    promotion_repository=astral-sh/uv
    promotion_repository_id="$UV_REPOSITORY_ID"
  else
    promotion_ref="automations/promote/$source_number"
    marker="<!-- uv-dev-promotion:$source_number:$source_sha -->"
    promotion_repository=astral-sh/uv-dev
    promotion_repository_id="$UV_DEV_REPOSITORY_ID"
  fi
  dedicated=$(gh pr list --repo astral-sh/uv --state all --head "$promotion_ref" --limit 1000 \
    --json number,headRefName,headRepository) || return 1
  historical=$(gh pr list --repo astral-sh/uv --state all --head "$source_ref" --limit 1000 \
    --json number,headRefName,headRepository) || return 1
  candidates=$(jq --null-input --compact-output \
    --argjson dedicated "$dedicated" --argjson historical "$historical" \
    --arg promotion_ref "$promotion_ref" --arg promotion_repository "$promotion_repository" \
    --arg source_ref "$source_ref" --arg source_repository "$source_repository" '
      [$dedicated[] | select(.headRefName == $promotion_ref and .headRepository.nameWithOwner == $promotion_repository)]
      + [$historical[] | select(
          .headRefName == $source_ref
          and (.headRepository.nameWithOwner == "astral-sh/uv"
            or ($source_repository == "astral-sh/uv-dev" and .headRepository.nameWithOwner == $source_repository))
        )]
    ') || return 1
  if [ "$(jq 'length' <<< "$candidates")" -gt 1 ]; then
    echo "Found multiple upstream promotions for $source_repository#$source_number." >&2
    return 1
  fi
  number=$(jq --raw-output '.[0].number // empty' <<< "$candidates")
  if [ -z "$number" ]; then
    echo null
    return
  fi
  pull_request=$(gh api "repos/astral-sh/uv/pulls/$number") || return 1
  if ! jq --exit-status \
    --arg promotion_ref "$promotion_ref" --arg source_ref "$source_ref" \
    --arg source_sha "$source_sha" --arg marker "$marker" --arg promotion_repository "$promotion_repository" \
    --arg source_repository "$source_repository" \
    --argjson uv_id "$UV_REPOSITORY_ID" --argjson uv_dev_id "$UV_DEV_REPOSITORY_ID" \
    --argjson promotion_id "$promotion_repository_id" --argjson bot_id "$AUTOMATIONS_BOT_ID" '
      .base.repo.id == $uv_id and .base.repo.full_name == "astral-sh/uv"
      and (
        (.head.ref == $promotion_ref and .head.repo.full_name == $promotion_repository
          and .head.repo.id == $promotion_id and .user.id == $bot_id
          and .user.login == "astral-automations-bot[bot]"
          and ((.body // "" | sub("\\s+$"; "")) | endswith($marker)))
        or
        (.head.ref == $source_ref
          and ((.head.repo.id == $uv_id and .head.repo.full_name == "astral-sh/uv")
            or ($source_repository == "astral-sh/uv-dev"
              and .head.repo.id == $uv_dev_id and .head.repo.full_name == $source_repository)))
      )
    ' <<< "$pull_request" > /dev/null; then
    echo "The upstream promotion does not match $source_repository#$source_number at its verified revision." >&2
    return 1
  fi
  if jq --exit-status --arg source_ref "$source_ref" --arg source_sha "$source_sha" \
    '.head.ref == $source_ref and .head.sha != $source_sha' <<< "$pull_request" > /dev/null; then
    verify_rebased_legacy_promotion "$source_repository" "$source_number" "$source_ref" "$source_sha" "$pull_request" || return 1
  fi
  jq --compact-output . <<< "$pull_request"
}

if [ "$#" != 4 ]; then
  echo "Usage: $0 <source-repository> <source-number> <source-ref> <source-sha>" >&2
  exit 2
fi
case "$1" in
  astral-sh/uv-dev|astral-sh/uv-security) ;;
  *) echo "Unsupported promotion source repository." >&2; exit 2 ;;
esac
if [[ ! "$2" =~ ^[1-9][0-9]*$ ]] || [[ ! "$4" =~ ^[0-9a-f]{40}$ ]]; then
  echo "Invalid promotion source number or revision." >&2
  exit 2
fi
git check-ref-format "refs/heads/$3"
find_promotion "$@"
