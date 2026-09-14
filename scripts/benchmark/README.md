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
