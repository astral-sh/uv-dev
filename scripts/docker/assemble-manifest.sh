#!/usr/bin/env bash

set -euo pipefail

# Load the per-image records and require one base image plus the
# expected number of distinct variants. Split the result into
# base and extra images so the publisher can publish the base last.
jq -se --argjson expected "$EXTRA_IMAGES" '
  if length != (1 + ($expected["image-mapping"] | length))
    or (map(.name) | unique | length) != length
    or (map(select(.name == "uv")) | length) != 1
  then error("incomplete Docker image manifest")
  else {base: map(select(.name == "uv"))[0],
        extra: map(select(.name != "uv"))}
  end
' images/*.json > docker-manifest.json
