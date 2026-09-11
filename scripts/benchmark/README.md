# benchmark

Benchmarking scripts for uv and other package management tools.

The `uv-bench` CodSpeed workloads use immutable package artifacts and ecosystem lockfiles listed in
`fixtures.json`. From the repository root, run `python3 scripts/benchmark/prepare-fixtures.py` to
download them into `.cache/bench-fixtures`. The script verifies every fixture against its recorded
SHA-256, including files already present. CI prepares these inputs before timing and includes them
in the walltime runner artifact, so the measured workloads do not download fixtures.

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
