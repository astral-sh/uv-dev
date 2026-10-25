# uv Linux Rust cache replay

Replay under the repository’s PR cache-save policy across 29 source commits, using 71 paired runs.
Both variants use
[`rust-workspace-cache@8139eb47`](https://github.com/astral-sh/github-actions/tree/8139eb474ac22fe39bb1e33b50afa426dcdaf455/rust-workspace-cache),
matching active compilers, Cargo freshness settings, and job commands within each pair. The variants
switch `cache-workspace-crates` and use isolated cache keys and artifact names.

| Dependencies only (s) | Full workspace (s) |   Speedup (90% CI) | Saved (s; 90% CI) |
| --------------------: | -----------------: | -----------------: | ----------------: |
|                 201.8 |              203.3 | 0.99× (0.98–1.01×) |   -1.4 (-4.8–1.8) |

Times cover the first through last runner-log timestamps, including setup and post steps. Queue time
is excluded. Speedup is dependency-only time divided by full-cache time; values below 1× mean a
slowdown.

For each source change, use one measured cache-producing pair when the uv PR planner enables saves
and the cache key is not an exact hit; otherwise use the mean of three read-only pairs. Each source
change has equal weight in the aggregate.

The read-only comparison uses 87 paired runs: 199.5 → 199.7 s, 1.00× (0.98–1.02×).

## Source and cache history

The measurement window is the 30 first-parent commits after
[`a7aa382e`](https://github.com/astral-sh/uv-dev/commit/a7aa382e69a6fd455cc515f7926fe1202dc2ad8e)
through
[`6fe4f0f5`](https://github.com/astral-sh/uv-dev/commit/6fe4f0f5cfa8322e97af4479cbfb3d53a3ce9596).
Each eligible source commit has three paired read-only repetitions. The headline applies the PR save
policy described above. The job definition is held fixed while source, dependencies, and the pinned
toolchain follow each source revision.

The initial caches are populated from
[`8b7edb87`](https://github.com/astral-sh/uv-dev/commit/8b7edb87b9cf22b465139cce232f82f9ad1e6e7f),
the preceding cache-key change, 8 commits before the window’s parent. Source-only changes keep the
immutable entry because source contents are absent from its key. An exact cache hit can therefore
still require workspace recompilation. A non-exact read is followed by a successful cache-producing
run before advancing to the next source commit.

The initial cohort covers commits 1–10. After the first refresh, the later cohort supplies the
remaining measurements. Both cohorts include commit 11 to compare the restored cache source, keys,
compiler, test counts, archive sizes, and workspace compilations. Only one copy of that overlapping
commit enters the aggregate. `metadata.json` retains this comparison.

At the overlapping commit, both cohorts restore the same cache source and cache keys apart from
isolation prefixes, use the same compiler, run the same test count, and compile the same workspace
crates. Archive sizes differ by less than 0.002% in both modes. Full-cache compilation averages
36.51 s and 36.53 s in the two cohorts. The later cohort therefore supplies the remaining history
after this cache refresh.

These isolated caches are reconstructed from historical source. The replay does not measure
long-term artifact accumulation, cache eviction, or queueing in production.

The `AGENTS.md`-only change at index 24 does not trigger this PR job and is excluded. The historical
cache-producing seed compiles all test targets with `--no-run`; measured jobs and later
cache-producing jobs execute the full test command. The alternate window-boundary seed executes the
full test command.

## Initial-cache sensitivity

A read-only sensitivity seeded at the start of the window gives 201.0 → 196.7 s, 1.02× (1.01–1.04×).
It uses a separate cohort for the first 10 commits and the same later runs. This is an alternate
reconstruction, not an isolated causal estimate of cache age, because the initial runner cohort also
differs.

The headline uses the preceding cache-key change to reconstruct an older immutable entry.
`fresh-initial-samples.csv` retains the alternate reconstruction; those initial samples are excluded
from the headline.

## Uncertainty

90% percentile intervals from 50,000 circular moving-block bootstrap resamples of paired per-commit
means, block length 5 commits. Repetitions are averaged within each commit; they are not treated as
independent source changes. Intervals describe the observed histories, not arbitrary future
workloads.

The source commits, rather than the individual runner repetitions, are the sampling units. The
intervals do not estimate variation across other runners, other CI jobs, or arbitrary future source
changes. `metadata.json` also contains one-, three-, and ten-commit block sensitivities.

## Job phases

| Phase                 | Dependencies only (s) | Full workspace (s) |
| --------------------- | --------------------: | -----------------: |
| Rust cache restore    |                   2.8 |                4.6 |
| Main test compilation |                  49.3 |               43.1 |
| Test execution        |                  94.2 |               93.9 |
| OS package setup      |                  28.2 |               35.3 |
| Other job work        |                  27.3 |               26.4 |

Other work includes tool setup and post steps. Only the main test compilation is separated from its
test execution.

| Read-only cache             | Dependencies only | Full workspace |
| --------------------------- | ----------------: | -------------: |
| Compressed archive (MiB)    |               422 |            803 |
| Cache age (source commits)  |              1–18 |           1–18 |
| Fetch after cache match (s) |               0.8 |            1.2 |
| Unpack (s)                  |               1.1 |            2.1 |

## Cache-producing jobs

There are 8 paired refreshes after the initial seed, with one paired run per refresh. The uv PR
aggregate uses these runs when its planner permits a write and the cache key is not an exact hit.
These means describe the cache-producing jobs themselves.

| Cost                       | Dependencies only (s) | Full workspace (s) |
| -------------------------- | --------------------: | -----------------: |
| Entire cache-producing job |                 208.5 |              212.7 |
| Cache post step            |                   6.2 |               10.6 |

## Cache modes with package installation first

With package installation moved earlier as in
[#2992](https://github.com/astral-sh/uv-dev/pull/2992), a separate six-commit comparison gives 187.6
→ 177.3 s for dependency-only → full workspace caching: 1.06× (90% CI 1.01–1.12×), or 10.3 s saved
(2.3–20.6 s), under the PR cache-save policy (16 pairs). Most of the saving comes from the final
workflow-only commit reusing the cache populated at the preceding commit.

This shorter window uses a new matched cohort.
[Individual runs, phase costs, uncertainty, and a reproducer](apt-first-cache/README.md) are
available separately. The [package-order comparison](apt-order/README.md) measures the effect of
ordering with full workspace caching in both variants.

## Reproduce the summary

`samples.csv` includes source and workflow revisions, cache keys and ages, individual durations, and
direct job links. `metadata.json` records the source selection, cache-state audit, and expected
summary.

```sh
uv run --script summarize.py
uv run --script summarize.py --read-only
uv run --script summarize.py --fresh-initial-cache
```

The script selects the appropriate read or write samples, averages repetitions within each source
commit, and calculates the paired effect and 90% intervals. It verifies the recomputed results
against `metadata.json`.
