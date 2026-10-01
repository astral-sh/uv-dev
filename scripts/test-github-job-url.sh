#!/usr/bin/env bash

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
test_dir=$(mktemp -d "${TMPDIR:-/tmp}/github-job-url.XXXXXXXXXX")
trap 'rm -rf "$test_dir"' EXIT

export GITHUB_SERVER_URL=https://github.com
export GITHUB_REPOSITORY=astral-sh/uv
export GITHUB_RUN_ID=36940242911
export GITHUB_RUN_ATTEMPT=1
export TEST_JOB_RESPONSE="$test_dir/jobs.json"
job_name='check-crates-policies / crates policies'
expected_url="$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID/job/110630026137"

gh() {
    if [[ "$*" != "api --paginate --slurp repos/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID/attempts/$GITHUB_RUN_ATTEMPT/jobs?per_page=100" ]]; then
        echo "Unexpected GitHub API arguments: $*" >&2
        return 1
    fi
    if [[ "${TEST_API_FAILURE:-0}" == 1 ]]; then
        return 1
    fi
    cat "$TEST_JOB_RESPONSE"
}
export -f gh

expect_url() {
    local actual_url
    actual_url=$(bash "$script_dir/github-job-url.sh" "$job_name" 2> "$test_dir/stderr")
    if [[ "$actual_url" != "$1" ]]; then
        printf 'Expected %s, got %s\n' "$1" "$actual_url" >&2
        cat "$test_dir/stderr" >&2
        exit 1
    fi
}

# Other failures and similarly named reusable jobs must not win the lookup.
# GitHub's paginated response can put the current, still-running job last.
cat > "$TEST_JOB_RESPONSE" <<EOF
[
  {"jobs": [
    {"name": "check-publish / cargo publish dry-run", "conclusion": "failure", "html_url": "https://example.invalid/other"},
    {"name": "other / crates policies", "conclusion": "failure", "html_url": "https://example.invalid/similar"}
  ]},
  {"jobs": [{"name": "$job_name", "status": "in_progress", "conclusion": null, "html_url": "$expected_url"}]}
]
EOF
expect_url "$expected_url"

# Reruns have new job IDs and must use the requested attempt's canonical URL.
export GITHUB_RUN_ATTEMPT=2
expected_url="$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID/job/110630099999"
cat > "$TEST_JOB_RESPONSE" <<EOF
[{"jobs": [{"name": "$job_name", "status": "completed", "conclusion": "failure", "html_url": "$expected_url"}]}]
EOF
expect_url "$expected_url"

# A lookup problem must not suppress the release pull request notification.
fallback_url="$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID/attempts/$GITHUB_RUN_ATTEMPT"
printf '%s\n' '[{"jobs": []}]' > "$TEST_JOB_RESPONSE"
expect_url "$fallback_url"
cat > "$TEST_JOB_RESPONSE" <<EOF
[{"jobs": [
  {"name": "$job_name", "html_url": "$expected_url"},
  {"name": "$job_name", "html_url": "https://example.invalid/ambiguous"}
]}]
EOF
expect_url "$fallback_url"
printf '[{"jobs": [{"name": "%s", "html_url": ""}]}]\n' "$job_name" > "$TEST_JOB_RESPONSE"
expect_url "$fallback_url"
export TEST_API_FAILURE=1
expect_url "$fallback_url"

echo "GitHub job URL tests passed."
