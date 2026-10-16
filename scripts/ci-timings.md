# CI timing reports

`pull-request-ci-timings.yml` reports potential CI runtime regressions on `astral-sh/uv-dev` pull
requests. The automation dispatcher starts it after a `ci.yml` PR run completes. It reads GitHub's
run, job, and step metadata without checking out PR code or downloading PR artifacts. It does not
run on `astral-sh/uv`.

The baseline contains the first attempts of up to 20 CI runs created in the preceding 14 days, each
from a different PR branch. Repeated commits and the current PR are excluded. PR history provides
samples for the benchmark jobs that `uv-dev` skips on `main`. Successful jobs can contribute even
when an unrelated job failed, but every sampled step must have finished before the source run was
created.

Jobs are matched by their full display name and runner labels. Their runtime is the sum of
successful step durations, so queue and dependency waits do not trigger warnings. Jobs with
duplicate names and runner labels, incomplete timings, or failed steps are excluded. A warning
requires at least five samples and an increase over the median of at least all of:

- 60 seconds;
- 25%;
- three scaled median absolute deviations (`3 × 1.4826 × MAD`).

These are conservative operational thresholds, not a statistical claim that a PR caused a slowdown.
Cache misses, runner load, and differences between PRs can still affect the result. For the first
ten flagged jobs, the report includes up to three step increases of at least 15 seconds, provided
each step has at least five matching samples. Jobs without sufficient matching samples are counted
separately instead of treated as healthy.

The workflow retains its samples and rendered report as a GitHub Actions artifact for 30 days. The
automation creates a comment only when it detects a regression, then updates that same
`astral-automations-bot` comment. A subsequent comparable result updates the warning. If an attempt
lacks comparable results for a previously flagged job at the same commit, its earlier warning
remains. The automation does not publish for closed PRs, superseded commits or attempts, or runs
superseded by a newer CI run.

To inspect an existing run without publishing a comment:

```shell
python3 scripts/ci-timings.py analyze \
  --run-id RUN_ID --head-sha FULL_HEAD_SHA --attempt ATTEMPT \
  --output ./ci-timings
```

Add `--historical` to inspect a closed PR or an older commit, and `--pull-request NUMBER` if its
deleted branch prevents GitHub from associating the run with the PR. Historical reports cannot be
published. The `publish` command requires a dispatch on `uv-dev`'s `main` branch, uses the brokered
`COMMENT_TOKEN` for writes, and rechecks the PR and run immediately before changing a comment.
