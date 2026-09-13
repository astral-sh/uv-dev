# benchmark

Benchmarking scripts for uv and other package management tools.

## Getting Started

From the `scripts/benchmark` directory:

```shell
uv run resolver \
    --uv-pip \
    --poetry \
    --benchmark \
    resolve-cold \
    ../requirements/trio.in
```

## Installed-package indexing

The `fs_metadata` Criterion target measures the complete installed-package index, including
directory enumeration, optional JSON decoding, and index construction. The generated wheels have
valid `METADATA` and `WHEEL` files and missing, present, or mixed optional sidecars. Its file-count
sweep includes the 1,024-wheel parallel-read boundary. Fixture creation, interpreter discovery, and
ordered-name checks happen only for selected rows and remain outside the measured loop.

From the repository root, use a separate process and Criterion output directory for each installer
concurrency setting:

```shell
env -u RUST_LOG -u RAYON_NUM_THREADS \
    UV_BENCH_CONCURRENT_INSTALLS=4 \
    CRITERION_HOME="$HOME/code/tmp/uv-installed-index-workers-4" \
    cargo bench --locked --profile profiling -p uv-bench --bench fs_metadata -- \
    installed_package_sidecars --sample-size 10 --warm-up-time 1 --measurement-time 2
```

Set `UV_BENCH_PYTHON` to a fixed interpreter when comparing builds. An unset
`UV_BENCH_CONCURRENT_INSTALLS` uses Rayon's default; `1` selects the production serial path. The
benchmark sets `RAYON_PARALLELISM` before the global pool can initialize. Changing
`RAYON_NUM_THREADS` alone does not select the explicitly single-threaded installed-index path. For
an explicit width above one, the first selected row initializes the pool outside timing and checks
its observed width. Width one and the unset/default setting stay lazy; the default's reported CPU
parallelism is only an inference, not an observed pool size. The untimed index check warms the pool
when a row needs it, so these are steady-state, warm-filesystem measurements, not process-startup or
cold-cache measurements.

For a fresh-process comparison, create the same fixture once with the benchmark's shared generator:

```shell
cargo run --locked --profile profiling -p uv-bench --example installed_packages -- \
    "$HOME/code/tmp/uv-installed-index-present-1024" --packages 1024 --sidecars present
```

The example requires a new directory and prints the expected `uv pip list --format json` graph. Run
each compared `uv` executable against that directory with `--target`, the same `--python`, and the
CLI's `UV_CONCURRENT_INSTALLS` setting. Compare parsed output before timing, keep the fixture
read-only, and use matched source trees, toolchains, profiles, feature sets, and separate processes.
The read-backend comparison in
[astral-sh/uv-dev#1493](https://github.com/astral-sh/uv-dev/pull/1493) excludes JSON decoding and
index construction; its I/O-only timings are a separate result.
