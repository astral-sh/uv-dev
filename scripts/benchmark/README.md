# benchmark

Benchmarking scripts for uv and other package management tools.

The `uv-bench` CodSpeed workloads use immutable package artifacts and ecosystem lockfiles listed in
`fixtures.json`. From the repository root, run `python3 scripts/benchmark/prepare-fixtures.py` to
download them into `.cache/bench-fixtures`. The script verifies every fixture against its recorded
SHA-256, including files already present. CI prepares these inputs before timing and includes them
in the walltime runner artifact, so the measured workloads do not download fixtures.

`walltime-shards.py` assigns every built walltime suite to exactly one of up to eight independent
runs. The saved plan is checked against the extracted benchmark binaries before execution, so a
missing or stale artifact cannot silently reduce coverage. Each shard retains its own CodSpeed
profile for main-branch baseline imports.

Whole-command workloads also need `cargo build --locked --profile profiling --bin uv`. Run
`python3 scripts/benchmark/prepare-environments.py` to install the pinned CPython interpreter under
`.cache/bench-python` and prime the package cache with Prefect's frozen runtime dependencies. The
temporary environment is discarded; measured workloads reconstruct their own environments offline
from the same lockfile and cached package artifacts.

Pass `--discovery` to also install the pinned Python 3.10, 3.12, and 3.13 interpreters used by the
Python discovery workloads.

Network workloads use `serve-fixtures.py` with the pinned Python 3.11 interpreter. It serves the
prepared wheels, their actual core metadata, and Simple API listings derived from those wheels or an
immutable lockfile. The server binds an ephemeral loopback port and applies a fixed 20 ms request
delay to model an ordinary remote index without relying on live service timing. Wheel responses
support byte ranges; only locally prepared artifact bodies can be downloaded.

`qualify-concurrent-downloads.py` runs a finite cache-coordination check against a chosen uv binary
and Python interpreter. It verifies overlapping independent wheel downloads, one full download for
concurrent installs sharing a cache, offline reuse of the published files, metadata-request
amplification, and recovery after terminating an install partway through a wheel body. Its JSON
report includes binary and fixture hashes, request counts, transferred body bytes, and peak
concurrency. The reported command timings are local end-to-end measurements, not CodSpeed results.

The qualification only needs four of the immutable fixtures:

```shell
python3 scripts/benchmark/prepare-fixtures.py \
  --fixture click-8.4.2-py3-none-any.whl \
  --fixture flask-3.1.2-py3-none-any.whl \
  --fixture jupyterlab-4.4.7-py3-none-any.whl \
  --fixture sympy-1.14.0-py3-none-any.whl
python3 scripts/benchmark/qualify-concurrent-downloads.py \
  --uv target/debug/uv \
  --python /absolute/path/to/python3.11 \
  --fd-limit 128 \
  --output download-qualification.json
```

`--fd-limit` applies a hard descriptor limit to the uv child processes on POSIX systems. Use
`--preview-feature content-addressed-cache` to exercise the file store, or
`--expect-coalesced-metadata` when qualifying metadata-request coordination. Running with
`--downloads 1` is a negative control: the independent-download check must report serialization.
Repeat `--scenario` to select individual checks when comparing implementations with different cache
capabilities.

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
