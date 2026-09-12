# benchmark

Benchmarking scripts for uv and other package management tools.

## Python environment identity

`qualify-python-environment-identity.py` checks immutable cached environments across a replacement
of their base interpreter at the same canonical executable path. It uses real CPython 3.13.1 and
3.13.2 distributions on macOS arm64, with frozen Ruff and Ruff-plus-Click resolutions. Each binary
gets its own copy of the prepared package cache. The script records interpreter and archive digests,
installed metadata graphs, physical environment identities, cache publications, and the executed
Python version. Elapsed times are diagnostic, not a matched performance benchmark.

The fixture reuses the tool-environment qualifier and frozen constraints from
[this pinned benchmark stack](https://github.com/astral-sh/uv-dev/tree/1146b3b6b99a1649f97fe541c846738a99ce4706/scripts/benchmark).
Use that stack's `prepare-tools.py` to populate a package cache with the pinned Ruff and Black
workloads; Black supplies the Click version used by the changed-resolution control. Download the
[CPython 3.13.1 archive](https://github.com/astral-sh/python-build-standalone/releases/download/20250115/cpython-3.13.1%2B20250115-aarch64-apple-darwin-install_only_stripped.tar.gz)
and
[CPython 3.13.2 archive](https://github.com/astral-sh/python-build-standalone/releases/download/20250317/cpython-3.13.2%2B20250317-aarch64-apple-darwin-install_only_stripped.tar.gz).
The qualifier, constraints, and interpreter archives are checked against their pinned SHA-256
digests before execution.

Run with Python 3.12 or later and an unused output directory:

```shell
python3 scripts/benchmark/qualify-python-environment-identity.py \
  --base /path/to/base/uv --base-revision BASE_COMMIT_SHA \
  --candidate /path/to/candidate/uv --candidate-revision CANDIDATE_COMMIT_SHA \
  --before-archive /path/to/cpython-3.13.1.tar.gz \
  --after-archive /path/to/cpython-3.13.2.tar.gz \
  --qualifier /path/to/benchmark/qualify-tool-environment-reuse.py \
  --constraints-directory /path/to/benchmark/tool-locks \
  --package-cache /path/to/prepared-ruff-and-click-cache \
  --output /path/to/python-environment-identity
```

Both revision arguments must be full commit SHAs. The baseline is expected to use the executable
path alone for the environment key; the candidate must also include the full Python version. The
fixture replaces only its newly created Python installation and leaves the input package cache
untouched. Its results do not qualify Windows replacement behavior or swaps between different Python
builds reporting the same version.

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
