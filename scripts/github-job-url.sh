#!/usr/bin/env bash

set -euo pipefail

job_name="${1:?expected a GitHub Actions job name}"
run_url="$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID/attempts/$GITHUB_RUN_ATTEMPT"

# The job is still in progress while it posts its failure comment. Scope the
# lookup to this attempt so a rerun cannot link to an earlier job.
if job_url=$(gh api --paginate --slurp \
    "repos/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID/attempts/$GITHUB_RUN_ATTEMPT/jobs?per_page=100" | \
    jq --exit-status --raw-output --arg name "$job_name" '
      [.[].jobs[] | select(.name == $name)]
      | if length == 1 and (.[0].html_url | type == "string" and length > 0)
        then .[0].html_url
        else error("expected exactly one job URL")
        end'); then
    printf '%s\n' "$job_url"
else
    echo "::warning::Could not resolve the GitHub Actions job URL for $job_name; using the workflow run attempt." >&2
    printf '%s\n' "$run_url"
fi
