#!/usr/bin/env bash

set -euo pipefail

jq -en \
  --arg name "$NAME" \
  --arg build_id "$BUILD_ID" \
  --arg digest "$DIGEST" \
  --arg tags "$TAGS" \
  --arg annotations "$ANNOTATIONS" \
  '{name: $name, build_id: $build_id, digest: $digest,
    tags: ($tags | split("\n")), annotations: ($annotations | split("\n"))}
    | select(.build_id != "" and (.digest | test("^sha256:[0-9a-f]{64}$")))' > "$NAME.json"
