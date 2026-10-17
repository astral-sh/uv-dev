#!/usr/bin/env bash
# Print the URL for a named job in the current GitHub Actions run attempt.
#
# Usage:
#   bash scripts/github-job-url.sh <job-name>
#
# The name must match GitHub's displayed job name, including any reusable
# workflow prefix. The lookup searches every result page and includes jobs
# that are still running.
#
# Requires `gh`, `jq`, and the GitHub Actions variables `GITHUB_SERVER_URL`,
# `GITHUB_REPOSITORY`, `GITHUB_RUN_ID`, and `GITHUB_RUN_ATTEMPT`. Authenticate
# `gh` with Actions read access, for example through `GH_TOKEN`.
#
# Writes the matching job URL to stdout. If a unique URL cannot be found,
# writes a warning to stderr and the run-attempt URL to stdout.

set -euo pipefail

job_name="${1:?expected a GitHub Actions job name}"
run_url="$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID/attempts/$GITHUB_RUN_ATTEMPT"

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
