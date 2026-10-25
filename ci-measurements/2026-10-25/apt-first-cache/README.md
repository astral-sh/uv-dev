# uv cache modes with package installation first

Both variants install OS packages before restoring Rust caches and installing Python, as in
[#2992](https://github.com/astral-sh/uv-dev/pull/2992). The variants differ only in workspace
artifact retention and isolated cache and artifact names. They use the same action, active compiler,
Cargo settings, and complete Linux test commands within each pair.

| PR cache-save policy      | Dependencies only (s) | Full workspace (s) |   Speedup (90% CI) | Saved (s; 90% CI) |
| ------------------------- | --------------------: | -----------------: | -----------------: | ----------------: |
| Normal policy, 16 pairs   |                 187.6 |              177.3 | 1.06× (1.01–1.12×) |   10.3 (2.3–20.6) |
| Writes disabled, 18 pairs |                 190.4 |              178.0 | 1.07× (1.02–1.12×) |   12.4 (4.5–20.7) |

Times cover the first through last runner-log timestamps, including setup and post steps. Queue time
is excluded. Speedup is dependency-only time divided by full-cache time; values below 1× mean a
slowdown.

For each source change, use one measured cache-producing pair when the uv PR planner enables saves
and the cache key is not an exact hit; otherwise use the mean of three read-only pairs. Each source
change has equal weight in the aggregate.

90% percentile intervals from 50,000 circular moving-block bootstrap resamples of paired per-commit
means, block length 2 commits. Repetitions are averaged within each commit; they are not treated as
independent source changes. Intervals describe the observed histories, not arbitrary future
workloads.

The six consecutive source commits run from
[`f158ef25`](https://github.com/astral-sh/uv-dev/commit/f158ef25a3cf2fb9bc089ca38cc06f659066f566)
through
[`6fe4f0f5`](https://github.com/astral-sh/uv-dev/commit/6fe4f0f5cfa8322e97af4479cbfb3d53a3ce9596).
The initial cache is populated at the preceding cache-key change,
[`c559c3bd`](https://github.com/astral-sh/uv-dev/commit/c559c3bd8a3d92e03e022dd56ec29eb7a5131bce).
Two intervening commits leave its key unchanged. Source-only changes keep the immutable entry; a
non-exact read is followed by a successful cache-producing run before advancing to the next source
commit.

This is a new paired cohort for the same source window as the package-order experiment. It compares
cache modes with the new ordering; the timing estimates are not constructed by combining arms from
separate experiments. The six-commit window does not estimate behavior across arbitrary future
source changes.

Action:
[`8139eb47`](https://github.com/astral-sh/github-actions/tree/8139eb474ac22fe39bb1e33b50afa426dcdaf455/rust-workspace-cache).
Workflow:
[`2138f61d`](https://github.com/astral-sh/uv-dev/blob/2138f61ddcb1036f51d7a9ba773c957d628e5948/.github/workflows/ci.yml).
Runner: `depot-ubuntu-24.04-16`. Cargo profile: `fast-build-nightly`.

## Job phases

| Phase                 | Dependencies only (s) | Full workspace (s) |
| --------------------- | --------------------: | -----------------: |
| Rust cache restore    |                   2.9 |                4.6 |
| Main test compilation |                  50.2 |               40.9 |
| Test execution        |                  94.6 |               94.5 |
| OS package setup      |                  17.8 |               15.5 |
| Other job work        |                  22.2 |               21.8 |

Most of the observed saving comes from the final workflow-only commit, which restores the cache
populated at the preceding commit. Main test compilation averages 47.5 → 0.2 s, and the entire job
averages 187.8 → 139.0 s across three pairs. This commit contributes 8.1 s to the 10.3 s mean saving
across the six commits; the other five average 2.5 s saved under the PR save policy.

The short window therefore depends strongly on this reuse case. A one-commit bootstrap block gives a
wider 90% savings interval of 0.0–24.8 s; the headline uses the preselected two-commit blocks.

Read-only restores average 415 MiB for dependency-only caches and 797 MiB for full workspace caches.
`metadata.json` also records cache ages, transfer times, and bootstrap block-length sensitivities.

## Reproduce the summary

`samples.csv` retains individual durations, source and workflow revisions, cache identities, and
direct job links.

```sh
uv run --script summarize.py
uv run --script summarize.py --read-only
```

The script selects the appropriate read or write samples, averages repetitions within each source
commit, and checks the recomputed estimates against `metadata.json`.
