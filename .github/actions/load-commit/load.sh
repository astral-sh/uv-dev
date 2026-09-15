#!/usr/bin/env bash
set -euo pipefail

export GIT_CONFIG_GLOBAL=/dev/null
export GIT_CONFIG_NOSYSTEM=1
export GIT_NO_REPLACE_OBJECTS=1

git_safe() {
    git -c core.hooksPath=/dev/null -c gc.auto=0 -c maintenance.auto=false "$@"
}

if ! [[ "$BASE_SHA" =~ ^[0-9a-f]{40}$ ]] ||
    ! [[ "$HEAD_SHA" =~ ^[0-9a-f]{40}$ ]] ||
    [ "$(git_safe cat-file -t "$BASE_SHA")" != commit ]; then
    echo "The base must be a known commit and the head must be a full SHA." >&2
    exit 1
fi
if [ ! -f "$BUNDLE" ] || [ -L "$BUNDLE" ]; then
    echo "The commit artifact does not contain a regular Git bundle." >&2
    exit 1
fi

reference="refs/uv-automations/commits/$HEAD_SHA"
git_safe bundle verify "$BUNDLE"
if [ "$(git_safe bundle list-heads "$BUNDLE")" != "$HEAD_SHA $reference" ]; then
    echo "The bundle does not advertise exactly the expected commit." >&2
    exit 1
fi

# Import objects only. Keep the trusted worktree, refs, and FETCH_HEAD untouched.
git_safe fetch --no-tags --no-recurse-submodules --no-write-fetch-head "$BUNDLE" "$reference"
if [ "$(git_safe cat-file -t "$HEAD_SHA")" != commit ] ||
    [ "$BASE_SHA" = "$HEAD_SHA" ] ||
    ! git_safe merge-base --is-ancestor "$BASE_SHA" "$HEAD_SHA"; then
    echo "The imported commit does not extend its trusted base." >&2
    exit 1
fi

printf 'head-sha=%s\n' "$HEAD_SHA" >> "$GITHUB_OUTPUT"
