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

Source-build workloads use the published backend wheels in the same fixture manifest and pin their
versions with `source-build-constraints.txt`. This lets isolated builds use their real PEP 517
backends through the replay index or a local wheel directory without depending on current PyPI
releases.

Run `python3 scripts/benchmark/prepare-build-backend.py` after preparing the interpreter to build
the current `uv_build` wheel with the release workflow's pinned Maturin version. Native-backend
workloads can then compare direct builds with the ordinary isolated PEP 517 path using the same
checkout, rather than a previously released backend binary. The shared native-source fixtures change
only packaging configuration in real sampleproject, Flask, and Django trees; their Python modules,
data files, documentation, and tests remain intact.

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
support byte ranges; only locally prepared artifact bodies can be downloaded. Fixtures marked
`"replay": false` are used only by local-file workloads and do not change the replayed package
listings.

Run `python3 scripts/benchmark/prepare-git.py` to prepare the Git sources in `git.json` under
`.cache/bench-git`. These repositories retain upstream commit and tree objects for PyPA's sample
project, Flask, Django, and the small pip regression fixture. Each captured ref points to its pinned
commit. Their complete reachable history lets the ordinary Git client fetch branches and tags
without requiring special handling for a shallow remote. Git workloads use these local repositories
without fetching live upstream data while timing.

Run `scripts/benchmark/prepare-sources.py` with Python 3.12.11 or newer after the fixture and Git
preparation steps to export the project trees in `sources.json` under `.cache/bench-sources`. CI
uses the pinned managed interpreter so archive link handling is consistent across runners. These
contain the actual tracked files at pinned commits, including uv's own source tree, without a
changing checkout or Git object database in the measured directory.

Workspace-script discovery measures warm source trees and, on Linux, a second case that advises the
kernel to discard candidate file data with `POSIX_FADV_DONTNEED` before each invocation. This does
not evict directory entries or the executable, and the kernel may retain pages; treat it as an
advisory file-data-cache workload rather than a guaranteed cold-filesystem measurement.

Pass `--git-directory .cache/bench-git` to `serve-fixtures.py` to replay GitHub commit lookups and
the exact `pyproject.toml` contents stored in these repositories. The private test endpoint
overrides let source-metadata workloads use the normal GitHub fast path against that loopback
server.

The Git fetch workload uses source-tree size as its main dimension: 12 files in `sampleproject`, 234
in Flask, and 6,901 in Django. Its cold case starts with an empty uv Git cache; the warm cases reuse
a populated cache with a precise commit, a full commit-like reference, or an upstream branch/tag.
These cases do not claim to evict the operating system's filesystem cache.

GitHub metadata workloads compare empty source caches with real, already-materialized checkouts. The
sample project and Flask have static metadata; Django and the pip regression fixture require their
actual `setuptools` backend. Git transport is redirected to the pinned local repositories, while
commit and raw-content requests use the delayed replay server.

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
