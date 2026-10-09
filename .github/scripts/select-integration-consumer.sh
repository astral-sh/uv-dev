#!/usr/bin/env bash
# Select the integration consumer for one producing CI attempt.
set -euo pipefail

consumers=$(gh api --method GET "repos/$GITHUB_REPOSITORY/actions/workflows/test-integration-workflow-run.yml/runs" \
  -f event=workflow_dispatch -f "created=>=$INTEGRATION_STARTED_AT" -f per_page=100 --paginate --slurp | \
  jq -c '[.[].workflow_runs[] | select(.display_title == env.INTEGRATION_RUN_NAME and (.status == "in_progress" or .status == "completed"))]')
# A completed duplicate has no integration jobs. Actual test jobs retain the
# claimant even when a lower-ID notification starts later or tests are canceled.
candidates=$(jq -r 'sort_by(.id)[] | [.id, .run_attempt] | @tsv' <<< "$consumers")
while IFS=$'\t' read -r candidate attempt; do
  if [[ -z "$candidate" ]]; then
    continue
  fi
  jobs=$(gh api "repos/$GITHUB_REPOSITORY/actions/runs/$candidate/attempts/$attempt/jobs?per_page=100" --paginate --slurp)
  claimed=$(jq -r '[.[].jobs[] | select(.name | startswith("test-integration /"))] | any(.conclusion != "skipped")' <<< "$jobs")
  if [[ "$claimed" == true ]]; then
    echo "$candidate"
    exit 0
  fi
done <<< "$candidates"
# Workflow concurrency admits one active consumer, but does not schedule it
# in ID order. Queued and canceled unclaimed notifications cannot own work.
# An empty selection lets the CI waiter retry before a consumer has started.
jq -r '([.[] | select(.status == "in_progress")] | min_by(.id).id) //
  ([.[] | select(.conclusion == "failure" or .conclusion == "timed_out")] | min_by(.id).id) // empty' <<< "$consumers"
