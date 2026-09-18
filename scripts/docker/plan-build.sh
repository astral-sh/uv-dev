#!/usr/bin/env bash

set -euo pipefail

# Each repository has its own development cache and saved images.
# Release plans in `uv`, including dry-runs, use the production project.
case "$GITHUB_REPOSITORY" in
  astral-sh/uv)
    if [ "$RELEASE_BUILD" == "true" ] && [ "$PUSH_DEV" != "true" ]; then
      depot_project_id="$DEPOT_PROJECT_PROD_UV"
    else
      depot_project_id="$DEPOT_PROJECT_DEV_UV"
    fi
    ;;
  astral-sh/uv-dev) depot_project_id="$DEPOT_PROJECT_DEV_UV_DEV" ;;
  *) echo "No Depot project configured for $GITHUB_REPOSITORY" >&2; exit 1 ;;
esac

{
  echo "depot-project-id=$depot_project_id"
  if [ "${PUSH_DEV}" == "true" ] && { [ "${IS_LOCAL_PR}" == "true" ] || [ "${IS_UV_DEV_PR}" == "true" ]; }; then
    echo "login=${IS_LOCAL_PR}"
    echo "push=true"
    echo "push-version=false"
    echo "save=true"
    echo "tag=sha"
  elif [ "${DRY_RUN}" == "false" ]; then
    echo "login=true"
    echo "push=true"
    echo "push-version=true"
    echo "save=true"
    echo "tag=${TAG}"
  else
    echo "login=${IS_LOCAL_PR}"
    echo "push=false"
    echo "push-version=false"
    echo "save=${RELEASE_BUILD}"
    echo "tag=dry-run"
  fi
} >> "$GITHUB_OUTPUT"
