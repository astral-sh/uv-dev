#!/usr/bin/env bash
# Print the files published in a GitHub release, one path per line.
# The manifest and its release assets must be in the same directory.
# Requires `jq`.

set -euo pipefail

manifest=${1:?path to dist-manifest.json is required}
directory=$(dirname -- "$manifest")

# The manifest does not include itself in its release asset list.
printf '%s\n' "$manifest"
jq -r --arg directory "$directory" \
    '.releases[].artifacts[] | "\($directory)/\(.)"' "$manifest"
