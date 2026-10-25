# uv Linux package installation order

The complete Linux test job is replayed across six consecutive source commits. Both variants retain full workspace caches and use the same action, active compiler, Cargo flags, package commands, and test commands. The treatment moves APT installation before Rust cache restoration and Python installation.

| PR cache-save policy | Current order (s) | APT first (s) | Speedup (90% CI) | Saved (s; 90% CI) |
| --- | ---: | ---: | ---: | ---: |
| Normal policy, 16 pairs | 197.2 | 188.9 | 1.04× (1.03–1.06×) | 8.2 (6.2–10.3) |
| Writes disabled, 18 pairs | 194.8 | 186.1 | 1.05× (1.04–1.06×) | 8.7 (6.9–10.4) |

Times include setup and post steps and exclude queues. Repetitions are averaged within each source commit, and source commits receive equal weight.

For each source change, use one measured cache-producing pair when the uv PR planner enables saves and the cache key is not an exact hit; otherwise use the mean of three read-only pairs. Each source change has equal weight in the aggregate.

90% percentile intervals from 50,000 circular moving-block bootstrap resamples of paired per-commit means, block length 2 commits. Repetitions are averaged within each commit; they are not treated as independent source changes. Intervals describe the observed histories, not arbitrary future workloads.

Source window: [`f158ef25`](https://github.com/astral-sh/uv-dev/commit/f158ef25a3cf2fb9bc089ca38cc06f659066f566) through [`6fe4f0f5`](https://github.com/astral-sh/uv-dev/commit/6fe4f0f5cfa8322e97af4479cbfb3d53a3ce9596). The preceding cache-key change at [`c559c3bd`](https://github.com/astral-sh/uv-dev/commit/c559c3bd8a3d92e03e022dd56ec29eb7a5131bce) supplies the initial cache. Two intervening commits do not change its key. Later cache-key changes are followed by a successful cache-producing run before the next source commit.

Action: [`8139eb47`](https://github.com/astral-sh/github-actions/tree/8139eb474ac22fe39bb1e33b50afa426dcdaf455/rust-workspace-cache). Workflow: [`f862312d`](https://github.com/astral-sh/uv-dev/blob/f862312d40231f2b7731dc99cce01e35fc5bc2af/.github/workflows/ci.yml). Runner: `depot-ubuntu-24.04-16`.

## Setup-only I/O diagnostic

A separate experiment runs six paired rounds at one fixed source revision. It compares the current order, APT first, and an explicit filesystem flush after Rust cache restoration. These jobs stop after setup and can finish with background writes still pending. The complete-job replay above measures the end-to-end effect.

| Cache mode / treatment | Current order (s) | Treatment (s) | Speedup (90% CI) | Saved (s; 90% CI) |
| --- | ---: | ---: | ---: | ---: |
| Dependencies only / APT first | 55.3 | 49.3 | 1.12× (1.04–1.19×) | 6.0 (2.1–9.2) |
| Dependencies only / Explicit flush | 55.3 | 57.7 | 0.96× (0.90–1.02×) | -2.4 (-6.1–1.1) |
| Full workspace / APT first | 59.2 | 51.4 | 1.15× (1.11–1.18×) | 7.8 (5.8–9.2) |
| Full workspace / Explicit flush | 59.2 | 65.8 | 0.90× (0.87–0.93×) | -6.5 (-8.6–-4.4) |

These diagnostic intervals use 50,000 paired bootstrap resamples of the six rounds. They estimate uncertainty in mean timing at the fixed source revision, not variation across source changes.

With full caches in the current order, about 5.7 GiB of filesystem pages are dirty immediately before APT. Moving APT first reduces that to about 0.6 GiB. Explicitly flushing the restored files reduces APT time but adds more time to setup overall. These observations support disk contention as an explanation; they do not identify a particular blocking syscall.

`io-diagnostic.json` retains the individual job links, timestamps, memory counters, disk counters, and I/O pressure snapshots.

## Reproduce the full-job summary

```sh
uv run --script summarize.py
uv run --script summarize.py --read-only
```

The script selects the appropriate read or write samples from `samples.csv`, averages repetitions within each source commit, and checks the recomputed estimates against `metadata.json`.
