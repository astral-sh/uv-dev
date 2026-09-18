#!/usr/bin/env bash

set -euo pipefail

token="$(depot pull-token --project "$DEPOT_PROJECT_ID" "$BUILD_ID")"
echo "::add-mask::$token"
printf '%s' "$token" | docker login registry.depot.dev --username x-token --password-stdin
