# benchmark

Benchmarking scripts for uv and other package management tools.

The `uv-bench` CodSpeed workloads use immutable package artifacts and ecosystem lockfiles listed in
`fixtures.json`. From the repository root, run `python3 scripts/benchmark/prepare-fixtures.py` to
download them into `.cache/bench-fixtures`. The script verifies every fixture against its recorded
SHA-256, including files already present. CI prepares these inputs before timing and includes them
in the walltime runner artifact, so the measured workloads do not download fixtures.

Source-distribution extraction compares the normal and preview tar implementations on published
Flask, Django, and NumPy archives. The inputs retain their real documentation, tests, Python
modules, and native source files while spanning roughly 0.7–20 MB of compressed data.

Requirements parsing also uses vLLM's immutable prerelease, CUDA, and development requirement
graphs. Their real index options and include directives exercise paths absent from ordinary compiled
requirements. Included contents are loaded before timing so CPU simulation measures parsing without
conflating it with filesystem access.

`walltime-shards.py` assigns every built walltime suite to exactly one of up to eight independent
runs. It places longer suites first into the lightest shard, using approximate five-minute runtime
weights for the Linux walltime runners. New suites receive one unit; increase their weight if
recorded runs show that they need more time. These weights are scheduling hints, not changes to
benchmark sampling. The saved plan is checked against the extracted benchmark binaries before
execution, so a missing or stale artifact cannot silently reduce coverage. Each shard retains its
own CodSpeed profile for main-branch baseline imports.

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

Run `python3 scripts/benchmark/prepare-python-archives.py` to prepare the host-native interpreter
archives in `python-archives.json`. These records are copied from uv's pinned download metadata and
retain their original URLs and SHA-256 values. The prepared files use uv's actual archive-cache
names, and `serve-fixtures.py --python-archives .cache/bench-python-archives` can replay them as a
Python installation mirror. Python-management workloads select this frozen download metadata so
later interpreter rebuilds do not silently change their inputs.

The installation workload installs one, two, or four real interpreter versions into an empty
directory. It compares a populated archive cache with a fresh cache and the delayed loopback mirror.
Both cases include extraction and installation; fixture downloads happen before timing. The Unix
uninstall workload prepares the same installed versions and their executable links, then removes
either one version or the entire installation set. Concurrent installation uses one, two, or four uv
processes with independent installation directories and one empty archive cache. Separate cases
request the same version or distinct versions, exposing cache-publication contention without
conflating it with installation-directory locking.

Executable-link workloads repeat `uv python install` after installing one, two, or four versions,
including their real platform-specific links or launchers. Each timed command runs in a fresh uv
process against an already-populated installation directory.

Pass `--project-caches` to also prepare separate caches for each frozen environment and one shared
cache containing all three environments. Cache maintenance workloads copy these caches and
reconstruct their environments before timing, retaining the real cache layout and links between
installed files and cached wheel contents. Shared-cache cleanup requests one, eight, or twenty
installed package names, or the environment's complete dependency set. This models clearing one
project's cached packages while retaining the packages used by other projects.

Retained-source pruning uses the published Flask 3.1.1 and Django 5.2.6 source archives. It builds
each archive through its pinned PEP 517 backend, checks that ordinary pruning retains the complete
source payload, and copies the resulting cache before each measured `uv cache prune` invocation.
Local archive and replay-server URL cases exercise both source-revision pointer formats. Cache
preparation and copying are excluded from the timing.

Run `python3 scripts/benchmark/prepare-file-caches.py` to prepare content-addressed caches for
Packse's runtime and Apache Airflow 3.0.6 with the Amazon, Google, and Microsoft Azure providers.
The Airflow requirements use its official Python 3.12 constraints at
[`b2e77273`](https://github.com/apache/airflow/blob/b2e77273cd65dd02e407997a97fe3584a14f1e96/constraints-3.12.txt)
and a 2025-08-30 release cutoff. The checked-in requirements pin every artifact hash; only
`dill==0.3.1.1` is built from source, with pinned build dependencies. Fixture manifests record
installed versions and internal hardlink groups. Pruning workloads reconstruct independent caches
and hardlinked environments before timing ordinary pruning and `--ci` pruning, with and without the
installed environment retained.

Concurrent-environment workloads create one, eight, or 32 independent Prefect runtime environments
from the same warm wheel cache. They measure the complete batch of frozen `uv sync` commands in
symlink and hardlink modes, including interpreter discovery and environment creation. Each sample
starts with fresh project directories, and cleanup happens outside the measured interval. This
models the actor startup contention reported in
[uv#12525](https://github.com/astral-sh/uv/issues/12525).

Run `python3 scripts/benchmark/prepare-tools.py` to prime the package cache for the CLI tools in
`tools.json`. Each tool has its own hashed dependency constraints, refreshed explicitly with
`--refresh-locks`. Tool workloads reconstruct isolated installations offline and use one, five, or
fifteen actual tools as their size dimension.

Pass `--individual-caches` to also prepare separate Ruff, Black, and Poetry caches for cached-tool
execution. Each measured invocation starts with its own populated cache and a prepared tool
environment, so recreating an environment cannot accumulate abandoned environments across samples.

Network workloads use `serve-fixtures.py` with the pinned Python 3.11 interpreter. It serves the
prepared wheels and source archives, wheel core metadata, and Simple API listings derived from
wheels or an immutable lockfile. The server binds an ephemeral loopback port and applies a fixed 20
ms request delay to model an ordinary remote index without relying on live service timing. Wheel
responses support byte ranges; only locally prepared artifact bodies can be downloaded. Fixtures
marked `"replay": false` are used only by local-file workloads and do not change the replayed
package listings.

The macOS certificate workload compares bundled and system certificate configuration during real
metadata resolution. It uses plain HTTP replay requests to isolate client initialization and native
certificate-store access from TLS handshake and live network variability. Its native walltime job
retains a separate profile stream, and its noise must be assessed independently from the Linux
bare-metal runner.

CodSpeed's runner currently supports Linux and macOS. Native Windows workloads use
`criterion-runs.py` to retain three independent Criterion runs, their raw samples, exact binary
hash, and within-run and between-run variation. CI uploads these separately as
`benchmarks-walltime-windows`; they are native walltime measurements, not CodSpeed uploads. Use
`--binary` to compare a different optimized uv build with the same workloads.

Run `python3 scripts/benchmark/prepare-git.py` to prepare the Git sources in `git.json` under
`.cache/bench-git`. These repositories retain upstream commit and tree objects for PyPA's sample
project, Flask, Django, and the small pip regression fixture. Each captured ref points to its pinned
commit. Their complete reachable history lets the ordinary Git client fetch branches and tags
without requiring special handling for a shallow remote. Git workloads use these local repositories
without fetching live upstream data while timing. The local upload-pack server allows object
filtering and subsequent object-ID requests, so partial-fetch implementations can avoid transferring
unneeded history while retaining the complete upstream repository as the benchmark input.

Use `--manifest scripts/benchmark/git-popular.json` to prepare Requests, Transformers, and Airflow.
Requests represents a common small source dependency, and Transformers represents users installing
unreleased model support or fixes, as described in its
[installation guide](https://github.com/huggingface/transformers/blob/d79b2d981f28b2730d402244ac3c2e9a8c054eee/docs/source/en/installation.md).
Airflow uses the same complete monorepo snapshot as the real-workspace benchmarks. These inputs are
separate from `git.json` because GitHub metadata replay has different fixture requirements.

Run `python3 scripts/benchmark/prepare-git-tags.py` to prepare the release refs in `git-tags.json`.
The manifest captures the real tags created by each pinned source snapshot's date: zero for
sampleproject, 67 for Flask, and 477 for Django. Git cache-key workloads use real checkouts and
compare commit-only keys with tag-sensitive keys in both loose-ref and packed-ref layouts.

Run `scripts/benchmark/prepare-sources.py` with Python 3.12.11 or newer after the fixture and Git
preparation steps to export the project trees in `sources.json` under `.cache/bench-sources`. CI
uses the pinned managed interpreter so archive link handling is consistent across runners. These
contain the actual tracked files at pinned commits, including uv's own source tree, without a
changing checkout or Git object database in the measured directory.

Workspace-script discovery measures warm source trees and, on Linux, a second case that advises the
kernel to discard candidate file data with `POSIX_FADV_DONTNEED` before each invocation. This does
not evict directory entries or the executable, and the kernel may retain pages; treat it as an
advisory file-data-cache workload rather than a guaranteed cold-filesystem measurement.

The real-workspace suite discovers the pinned Flask source and Airflow's complete 127-member
monorepo, including fresh and cached member discovery with a shared workspace cache. It also runs
frozen `export`, `tree`, and `workspace metadata` commands against Airflow's original 892-package
lockfile. These cases retain the actual project manifests, directory layout, and dependency graph
described in [uv#13796](https://github.com/astral-sh/uv/issues/13796). Airflow's source fixture uses
a shallow fetch and overrides `export-ignore` while exporting the pinned Git tree, since its
ordinary release archive omits required development workspace members.

Pass `--git-directory .cache/bench-git` to `serve-fixtures.py` to replay GitHub commit lookups and
the exact `pyproject.toml` contents stored in these repositories. The private test endpoint
overrides let source-metadata workloads use the normal GitHub fast path against that loopback
server.

The Git fetch workload uses source-tree size as its main dimension: 12 files in `sampleproject`, 234
in Flask, and 6,901 in Django. Its cold case starts with an empty uv Git cache; the warm cases reuse
a populated cache with a precise commit, a full commit-like reference, or an upstream branch/tag.
These cases do not claim to evict the operating system's filesystem cache.

The revision-history cases retain one cache across one, four, or ten actual Django releases. Run
`python3 scripts/benchmark/git-runtime.py prepare` with Python 3.12.11 or newer to build the pinned
Git 2.55 runtime, then use
`python3 scripts/benchmark/git-runtime.py run -- cargo bench -p uv-bench --bench git_fetch` to run
the suite. CI carries that runtime in the benchmark artifact and uses it for Git transport suites,
so the runner's system Git cannot silently select an older checkout implementation. The runtime
includes the local transport used by the fixtures; remote transports are excluded.

The `git_fetch_popular` suite measures fresh installs, four successive releases, and warm precise
reuse for Requests and Transformers. Every workload runs with ordinary clones, partial-fetch clones,
full-fetch worktrees, and partial-fetch worktrees. The `git_monorepo` suite runs offline
`uv pip compile --no-deps --no-build` for one or four real Airflow provider packages at the same
commit, both with a fresh cache and with populated source metadata. Its static package metadata
keeps package builds and registry resolution out of the measurement. Both suites run in CodSpeed.

Odoo's multi-gigabyte history from [uv#1737](https://github.com/astral-sh/uv/issues/1737) is
available as an extended workload. Prepare it with
`python3 scripts/benchmark/prepare-git.py --manifest scripts/benchmark/git-large.json`, then run
`python3 scripts/benchmark/git-runtime.py run -- cargo bench -p uv-bench --bench git_fetch_large`.
The same four settings cover a fresh checkout and four actual revisions of the 17.0 branch. This
suite is explicit because repeatedly fetching the complete history is too expensive for every pull
request's shared benchmark artifact and walltime budget.

GitHub metadata workloads compare empty source caches with real, already-materialized checkouts. The
sample project and Flask have static metadata; Django and the pip regression fixture require their
actual `setuptools` backend. Git transport is redirected to the pinned local repositories, while
commit and raw-content requests use the delayed replay server.

## Workload selection

Run `python3 scripts/benchmark/prepare-incremental-locks.py` to prime offline resolution of pinned
Flask, JupyterLab, and Airflow releases. Their checked-in initial locks use an October 2025 cutoff;
the measured command widens it to January 2026. Preparation also resolves without the initial lock
so both preference-preserving and full-resolution implementations have the required metadata in
cache. Regenerate these inputs explicitly with `--refresh-locks`.

Run `python3 scripts/benchmark/prepare-member-index-locks.py` for the same application graphs as
local path and workspace dependencies. The local package defines an explicit, query-qualified PyPI
index, giving it a stable index identity distinct from the workspace's default index without
requiring private credentials. The measured freshness checks are offline. These initial locks can
also be regenerated with `--refresh-locks`.

Release-runtime workloads use `prepare-release-binaries.py` to build with and without the production
PGO corpus in `scripts/build_uv_pgo.py`. Both binaries use the release profile and its fat LTO;
function symbols are retained for walltime profiling. The training corpus and its Python
installations live in `.cache/bench-release-corpus`, outside Cargo's cleaned target directory. The
build records the compiler, profile hash, training inputs, and both binary hashes. A separate
CodSpeed job resolves the pinned Flask, JupyterLab, and Airflow graphs plus full held-out JupyterLab
and Warehouse project manifests from warm metadata caches without an existing lock.
`prepare-release-workloads.py` verifies the held-out manifests against their frozen hashes and
primes their relocatable metadata caches. Release code generation is measured independently from the
ordinary non-LTO profiling build.

Run `python3 scripts/benchmark/prepare-resolver-errors.py` to prime the unsatisfiable requirements
in `resolver-errors.json`. The fixed December 2024 cutoff selects 8 Rooster, 14 HTTPX, and 31 NumPy
releases whose Python requirements exclude the requested interpreter. Diagnostic workloads solve
offline and measure both the first rendering of a fresh error and resolution plus rendering.

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
