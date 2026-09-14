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
`.cache/bench-python` and prime the package cache with frozen project dependencies. The
`environments.json` manifest selects Packse's runtime, uv's documentation environment, and Prefect's
runtime as small, medium, and large package graphs. These workloads share one pinned interpreter.
The temporary environments are discarded; measured workloads reconstruct their own environments
offline from the same lockfiles and cached package artifacts.

Pass `--discovery` to also install the pinned Python 3.10 and 3.13 interpreters used by the Python
discovery workloads.

Pass `--project-caches` to also prepare a separate cache for each frozen environment. Cache
maintenance workloads copy these caches and reconstruct their environments before timing, retaining
the real cache layout and links between installed files and cached wheel contents.

Run `python3 scripts/benchmark/prepare-tools.py` to prime the package cache for the CLI tools in
`tools.json`. Each tool has its own hashed dependency constraints, refreshed explicitly with
`--refresh-locks`. Tool workloads reconstruct isolated installations offline and use one, five, or
fifteen actual tools as their size dimension.

Pass `--individual-caches` to also prepare separate Ruff, Black, and Poetry caches for cached-tool
execution. Each measured invocation starts with its own populated cache and a prepared tool
environment, so recreating an environment cannot accumulate abandoned environments across samples.

Network workloads use `serve-fixtures.py` with the pinned Python 3.11 interpreter. It serves the
prepared wheels, their actual core metadata, and Simple API listings derived from those wheels or an
immutable lockfile. The server binds an ephemeral loopback port and applies a fixed 20 ms request
delay to model an ordinary remote index without relying on live service timing. Wheel responses
support byte ranges; only locally prepared artifact bodies can be downloaded.

Run `python3 scripts/benchmark/prepare-git.py` to prepare the Git sources in `git.json` under
`.cache/bench-git`. These repositories retain upstream commit and tree objects for PyPA's sample
project, Flask, Django, and the small pip regression fixture. Each captured ref points to its pinned
commit. Their complete reachable history lets the ordinary Git client fetch branches and tags
without requiring special handling for a shallow remote. Git workloads use these local repositories
without fetching live upstream data while timing.

The Git fetch workload uses source-tree size as its main dimension: 12 files in `sampleproject`, 234
in Flask, and 6,901 in Django. Its cold case starts with an empty uv Git cache; the warm cases reuse
a populated cache with a precise commit, a full commit-like reference, or an upstream branch/tag.
These cases do not claim to evict the operating system's filesystem cache.

## Workload selection

Prefer immutable artifacts and dependency graphs from real projects. Include small, medium, and
large workloads where different sizes can exercise different paths, and identify the relevant
dimension: package count, archive entries, artifact bytes, or dependency-graph complexity. A larger
input is not automatically more representative.

Use CPU simulation for CPU-bound work such as parsing and graph transformations. Use walltime for
filesystem operations, subprocesses, network requests, and concurrency effects. Keep fixture setup
outside the measured region, control external services, and inspect the timing distribution and
run-to-run noise before relying on a benchmark to detect regressions. An ablation should produce a
repeatable effect that is meaningfully larger than the observed noise.

Register walltime targets in `.github/workflows/bench.yml`. Expensive filesystem and whole-command
targets use `common::walltime_criterion()` to bound sampling cost; adjust that policy only after
checking the resulting distributions.

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
