# Resolver error reporting measurements

Matched offline measurements for [#1785](https://github.com/astral-sh/uv-dev/pull/1785) and [#1788](https://github.com/astral-sh/uv-dev/pull/1788), using requirements from [#1581](https://github.com/astral-sh/uv-dev/pull/1581).

The requirements are taken from [#1581](https://github.com/astral-sh/uv-dev/pull/1581)'s Rooster, HTTPX, and NumPy fixtures. The cache was primed by uv 0.11.28 in the `simple-v22` namespace, while the measured sources use `simple-v25`. In these admitted offline runs, each root package was absent from the cache namespace read by the measured sources, so the recorded reports are immediate cache-miss failures with network-disabled hints. The rows labeled `real` therefore measure that error path, not populated-cache dependency conflicts or the fixtures' intended small/medium/large graph complexity. Separate measurements with a compatible populated cache are needed for the intended Python-incompatibility conflicts.

Machine: Linux 5.15.0-1111-azure x86_64; AMD EPYC 9V74 80-Core Processor; 16 logical CPUs; 67419009024 bytes RAM. Hardware was read after final admission. cargo 1.98.1 (797e8a9bc 2026-08-05); profiling profile; the actual Cargo artifacts report optimization level 3 and no debug information. The source requests LTO off, and the build uses two jobs.

The two patches and all source, executable, fixture and raw-file identities are in `manifest.json`. Apply the matching overlay to each immutable product commit. Build the real target with `cargo +1.98.1 bench --locked --package uv-bench --bench resolver_errors --profile profiling --no-run --message-format=json --jobs 2 --config profile.profiling.debug=0`, and the stress target with `cargo +1.98.1 test --locked --package uv-resolver --lib --profile profiling --no-run --message-format=json --jobs 2 --config profile.profiling.debug=0`. Prime the three requirements with `uv pip compile --python-version <fixture-version> --python-platform aarch64-apple-darwin --exclude-newer 2024-12-01T00:00:00Z <requirements.in>`; the expected result is unsatisfiable. Measurement uses that exact prepared cache offline.

Fresh offline resolution is outside the timer; first Display and report/error destruction are inside.
Offline resolution, first Display, and report/error destruction are inside.
Fixture and fresh-error construction are outside the timer; first Display, rendered hints, and output/error destruction are inside.

Each endpoint uses twelve matched process pairs with alternating order. The ratio is candidate / parent elapsed time: below one is faster. Intervals are pointwise 95% percentile bootstrap intervals over paired log ratios, with 20,000 resamples and the PR number as the seed. They are not simultaneous intervals. Real and synthetic endpoints are reported separately; no aggregate speedup is inferred.

Run `uv run --no-project --no-config --offline analyze.py <absolute-bundle-directory> --manifest-sha256 <trusted-manifest-sha256>` to reconstruct the complete result without running products.

For each manifest schedule row, use the retained executable for its exact source and kind. The harness fixes 20 samples, one second of warm-up, and three seconds of measurement per endpoint. Run from that source's `crates/uv-bench` directory with fresh Criterion output directories and the same prepared cache and virtual environment.

```sh
UV_RESOLVER_ERROR_BENCH_MODE=measure UV_RESOLVER_ERROR_BENCH_CACHE=<absolute-cache> UV_RESOLVER_ERROR_BENCH_ENVIRONMENT=<absolute-venv> UV_RESOLVER_ERROR_BENCH_OUTPUT=<new-absolute-output> <retained-real-executable>
UV_NO_SOLUTION_BENCH_MODE=measure UV_NO_SOLUTION_BENCH_SUITE=<exclude_newer-or-shared_derivation> UV_NO_SOLUTION_BENCH_OUTPUT=<new-absolute-output> <retained-stress-executable> error::report_performance::no_solution_error_benchmark --exact --ignored --nocapture --test-threads=1
```

The manifest supplies the complete alternating source/order schedule. Preserve the exact raw `new` and `base` files; this export omits only byte-identical `base` copies. Local paths in the reproduction commands are placeholders, not measured-host paths.

The independently identified Git fsmonitor remains running.
Effects apply to this host with the identified background service running; neither a quiet host nor absence of delayed cache or I/O effects is established.
Pair-interval counters include the existing validation overhead. RUSAGE_CHILDREN is cumulative runner-child accounting, not isolated native-command CPU accounting.
The manifest retains the initial observation and all 48 before/after-pair activity records as descriptive covariates. They do not select or reweight samples.

## #1785: real (offline cache-miss diagnostics)

| Endpoint | Candidate / parent | Pointwise 95% CI | Conclusion |
| --- | ---: | ---: | --- |
| `resolver_errors/render/rooster` | 0.981947 | [0.927245, 1.033720] | inconclusive |
| `resolver_errors/resolve_and_render/rooster` | 0.998879 | [0.994826, 1.003720] | inconclusive |
| `resolver_errors/render/httpx` | 0.991426 | [0.986036, 0.996853] | faster |
| `resolver_errors/resolve_and_render/httpx` | 1.003222 | [0.998842, 1.007602] | inconclusive |
| `resolver_errors/render/numpy` | 0.993052 | [0.982408, 1.005643] | inconclusive |
| `resolver_errors/resolve_and_render/numpy` | 1.002280 | [0.997996, 1.007077] | inconclusive |

## #1785: stress

| Endpoint | Candidate / parent | Pointwise 95% CI | Conclusion |
| --- | ---: | ---: | --- |
| `comb_plain_32` | 0.911086 | [0.898387, 0.920152] | faster |
| `comb_plain_1024` | 0.453217 | [0.450339, 0.455722] | faster |
| `balanced_plain_1024` | 0.963870 | [0.960950, 0.966983] | faster |
| `comb_excluded_256` | 0.026472 | [0.026271, 0.026642] | faster |
| `balanced_excluded_256` | 0.193416 | [0.192689, 0.194110] | faster |

## #1788: real (offline cache-miss diagnostics)

| Endpoint | Candidate / parent | Pointwise 95% CI | Conclusion |
| --- | ---: | ---: | --- |
| `resolver_errors/render/rooster` | 1.126802 | [1.067778, 1.200728] | slower |
| `resolver_errors/resolve_and_render/rooster` | 1.006839 | [1.003901, 1.009729] | slower |
| `resolver_errors/render/httpx` | 1.087700 | [1.080098, 1.096273] | slower |
| `resolver_errors/resolve_and_render/httpx` | 1.005623 | [1.001915, 1.008920] | slower |
| `resolver_errors/render/numpy` | 1.094437 | [1.085252, 1.104091] | slower |
| `resolver_errors/resolve_and_render/numpy` | 1.005016 | [1.001938, 1.007909] | slower |

## #1788: stress

| Endpoint | Candidate / parent | Pointwise 95% CI | Conclusion |
| --- | ---: | ---: | --- |
| `comb_plain_32` | 0.774518 | [0.770161, 0.778670] | faster |
| `shared_identified_12` | 0.003236 | [0.003221, 0.003250] | faster |
| `shared_identified_16` | 0.000218 | [0.000216, 0.000219] | faster |
| `shared_unidentified_10` | 0.205680 | [0.204166, 0.207085] | faster |

No successful-resolution, network, end-to-end CLI, or whole-application claim; no real/stress aggregate.
