#!/usr/bin/env bash
set -euo pipefail

export GIT_CONFIG_GLOBAL=/dev/null
export GIT_CONFIG_NOSYSTEM=1
export GIT_NO_REPLACE_OBJECTS=1

git_safe() {
    git -c core.hooksPath=/dev/null -c gc.auto=0 -c maintenance.auto=false "$@"
}

head_sha="${HEAD_SHA:?}"
if [ "$head_sha" = HEAD ]; then
    head_sha="$(git_safe rev-parse --verify HEAD)"
fi
if ! [[ "$BASE_SHA" =~ ^[0-9a-f]{40}$ ]] ||
    ! [[ "$head_sha" =~ ^[0-9a-f]{40}$ ]] ||
    [ "$(git_safe cat-file -t "$BASE_SHA")" != commit ] ||
    [ "$(git_safe cat-file -t "$head_sha")" != commit ]; then
    echo "The base and head must be full commit SHAs." >&2
    exit 1
fi
if [ "$BASE_SHA" = "$head_sha" ] ||
    ! git_safe merge-base --is-ancestor "$BASE_SHA" "$head_sha"; then
    echo "The persisted commit must extend its base." >&2
    exit 1
fi

directory="$(mktemp -d "$RUNNER_TEMP/persist-commit.XXXXXX")"
reference="refs/uv-automations/commits/$head_sha"
# Create a new ref without replacing an existing one. Delete only our own ref.
git_safe update-ref "$reference" "$head_sha" ""
trap 'git_safe update-ref -d "$reference" "$head_sha"' EXIT
git_safe bundle create "$directory/commit.bundle" "$BASE_SHA..$reference"
git_safe bundle verify "$directory/commit.bundle"
if [ "$(git_safe bundle list-heads "$directory/commit.bundle")" != "$head_sha $reference" ]; then
    echo "The bundle does not advertise exactly the persisted commit." >&2
    exit 1
fi

printf 'head-sha=%s\npath=%s\n' "$head_sha" "$directory/commit.bundle" >> "$GITHUB_OUTPUT"
