# Populated-cache resolver error measurements

Matched offline measurements for [#1785](https://github.com/astral-sh/uv-dev/pull/1785) and [#1788](https://github.com/astral-sh/uv-dev/pull/1788), using the Rooster, HTTPX, and NumPy fixtures from [#1581](https://github.com/astral-sh/uv-dev/pull/1581).

These are populated-cache Python-incompatibility conflicts. The seed executable and all three measured sources use `simple-v25`; every root package has compatible cached metadata. Full diagnostic specimens establish the intended Python-version conflict for every source and fixture. The separate [original measurements](../2026-09-23/README.md) include immediate offline cache-miss controls produced by a `simple-v22`/`simple-v25` mismatch and nine synthetic reporting endpoints. Those original observations and intervals are unchanged and are not pooled with this study.

Machine: Linux 5.15.0-1111-azure x86_64; AMD EPYC 9V74 80-Core Processor; 16 logical CPUs; 67419009024 bytes RAM. The hardware record was taken for the original study; the same retained benchmark executables were used here. Rust 1.98.1; profiling profile, optimization level 3, LTO off, debug info off for the measured artifacts.

Apply the matching overlay from this directory to each immutable product commit in `manifest.json`. Build the real target with `cargo +1.98.1 bench --locked --package uv-bench --bench resolver_errors --profile profiling --no-run --message-format=json --jobs 2 --config profile.profiling.debug=0`. The manifest retains all six original executable identities; only the three real-target executables are measured in this study.

Use the exact normal `uv` seed source and executable identity in `manifest.json` to create a fresh cache and virtual environment. For each fixture, write its listed requirements to `<requirements.in>` and run:

```sh
<seed-uv> --no-config --cache-dir <fresh-absolute-cache> venv --python <python-3.12> --no-managed-python --no-python-downloads <fresh-absolute-venv>
<seed-uv> --no-config --cache-dir <fresh-absolute-cache> pip compile --python-version <fixture-version> --python-platform aarch64-apple-darwin --exclude-newer 2024-12-01T00:00:00Z <requirements.in>
```

Each compile must exit with the complete Python-incompatibility diagnostic, not a missing-package, offline, network-disabled, or download failure. Require all three `simple-v25` root metadata files, copy the complete cache to a fresh runtime cache, and inspect every measured source before timing. The exact seed, cache inventories, semantic gates, complete specimens, source order, and original raw-file identities are recorded in the manifest.

For each schedule row, run from that source's `crates/uv-bench` directory, with fresh Criterion output and the same prepared cache and environment:

```sh
UV_RESOLVER_ERROR_BENCH_MODE=measure UV_RESOLVER_ERROR_BENCH_CACHE=<absolute-runtime-cache> UV_RESOLVER_ERROR_BENCH_ENVIRONMENT=<absolute-venv> UV_RESOLVER_ERROR_BENCH_OUTPUT=<new-absolute-output> <retained-real-executable>
```

For `render`, fresh offline resolution is outside the timer; first `Display` and report/error destruction are inside.
For `resolve_and_render`, offline resolution, first `Display`, and report/error destruction are inside.

Each endpoint uses twelve matched process pairs with alternating order, 20 Criterion samples per process, one second of warm-up, and three seconds of measurement. Candidate / parent elapsed time below one is faster. The pointwise 95% percentile bootstrap uses paired log ratios, 20,000 resamples, and the PR number as the seed. The intervals are not simultaneous. Every endpoint, including slower and inconclusive results, is retained.

The independently identified Git fsmonitor remains running.
Effects apply to this host with the identified background service running; neither a quiet host nor absence of delayed cache or I/O effects is established.
Pair-interval counters include the existing validation overhead. RUSAGE_CHILDREN is cumulative runner-child accounting, not isolated native-command CPU accounting.
The manifest retains all 24 before/after-pair activity observations as descriptive covariates; they do not select or reweight samples.

Run `python3 analyze.py <absolute-bundle-directory> --manifest-sha256 <trusted-manifest-sha256>` to reconstruct all twelve results without running products or accessing the network. The archive omits only byte-identical Criterion `base` copies; the manifest records both original identities.

## #1785: populated-cache Python conflicts

| Endpoint | Candidate / parent | Pointwise 95% CI | Conclusion |
| --- | ---: | ---: | --- |
| `resolver_errors/render/rooster` | 0.992813 | [0.982068, 1.003447] | inconclusive |
| `resolver_errors/resolve_and_render/rooster` | 1.002931 | [0.998411, 1.007763] | inconclusive |
| `resolver_errors/render/httpx` | 0.991862 | [0.980601, 1.006510] | inconclusive |
| `resolver_errors/resolve_and_render/httpx` | 1.000436 | [0.993303, 1.006494] | inconclusive |
| `resolver_errors/render/numpy` | 1.037046 | [1.019814, 1.056801] | slower |
| `resolver_errors/resolve_and_render/numpy` | 1.008918 | [0.991879, 1.024875] | inconclusive |

## #1788: populated-cache Python conflicts

| Endpoint | Candidate / parent | Pointwise 95% CI | Conclusion |
| --- | ---: | ---: | --- |
| `resolver_errors/render/rooster` | 1.034153 | [1.023360, 1.046620] | slower |
| `resolver_errors/resolve_and_render/rooster` | 1.002892 | [0.997705, 1.007699] | inconclusive |
| `resolver_errors/render/httpx` | 1.008458 | [1.002914, 1.013245] | slower |
| `resolver_errors/resolve_and_render/httpx` | 0.999532 | [0.991681, 1.006811] | inconclusive |
| `resolver_errors/render/numpy` | 1.034098 | [1.011738, 1.061570] | slower |
| `resolver_errors/resolve_and_render/numpy` | 0.986358 | [0.971808, 1.002136] | inconclusive |

No successful-resolution, network, end-to-end CLI, whole-application, or pooled historical/populated-cache claim.
