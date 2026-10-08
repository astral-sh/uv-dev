# Frozen package workloads

`profiles.json` records the exact runtime graph and isolated build requirements for each workload.
`artifacts.tsv` records the release filenames, URLs, SHA-256 digests, and index metadata. The
fixture index exposes only the releases admitted by the selected profile and verifies archive bytes
before serving them. It retains real registry wheels and source distributions, including the larger
Linux Home Assistant workload. Source-build checks use the pinned backend graph as well as the
runtime constraints.

The archive cache is independent of uv's cache. A first run downloads the selected archives into
that directory; `--offline` requires those verified files to exist and performs no public downloads.
Each workload starts with an empty environment and uv cache. Both uv versions use the same loopback
index throughout the wheel, refresh, audit, and source-build operations.

For example, on an Apple Silicon machine with Python 3.14.2:

```console
python scripts/cache_baseline.py select --manifest cache-baseline.json
python scripts/cache_baseline.py download --manifest cache-baseline.json --target aarch64-apple-darwin --output ./uv-previous
python scripts/check_cache_compat.py --uv-current ./target/debug/uv --uv-previous ./uv-previous --fixture-cache ./fixture-cache --identity cache-workloads.json
python scripts/check_cache_compat.py --uv-current ./target/debug/uv --uv-previous ./uv-previous --fixture-cache ./fixture-cache --identity cache-replay.json --offline
```

CI selects the latest stable baseline once per workflow, then shares its version and archive digests
across the platform jobs. Save `cache-baseline.json` to replay that exact selection; pass
`--version` to `select` to choose a particular release. The workload identity file records the
Python version, both executable versions and digests, the profiles, the archive catalog digest, and
the archives actually served. CI uploads these identity files even when an operation fails. The
external preparation dependencies are GitHub release availability and the recorded PyPI archive
URLs.

To refresh a workload deliberately, edit its exact `roots` and `build_roots`, then run:

```console
python scripts/update_package_fixtures.py --uv ./target/debug/uv --resolve cache-anyio
```

The generator resolves the runtime and backend closures for the profile's recorded Python and
platform, then refreshes archive identities for all pinned releases. Review the graph and catalog
diff together and run the affected wheel and source-build workloads. Running the generator without
`--resolve` refreshes only the archive catalog. New interpreter or platform families require their
own reviewed graph when dependency markers select different packages.

## Interpreter installation workloads

The system profiles freeze the `pylint` closure for each Python minor version and admit the
`pydantic-core` release used by the existing CPython native-wheel phase. They also pin the packages
used by `uv venv --seed`. The native-extension fixture retains its checked-in interpreter-specific
`setuptools` and `wheel` requirements. Windows-only dependencies such as `colorama` are recorded in
the same profile; package metadata still controls whether they are installed on another platform.
The existing interpreter/version gates determine which native phases run.

`check_system_python.py` uses one profile across the system, uv-created virtual environment, and
stdlib-created virtual environment phases. `check_embedded_python.py` uses the explicit Windows
Python 3.11 profile with `pylint` and `numpy`. Both scripts retain interpreter and Conda discovery
inputs, give each run a fresh uv cache, and log the workload and served archive identities even on
failure. The interpreter's preinstalled or bundled `pip`, used by `ensurepip` and stdlib `venv`,
remains part of the interpreter under examination; registry packages and uv's seed packages come
from the frozen index.

To retain archives and repeat the same package phases without public registry access, add
`--fixture-cache PATH` on the first invocation and `--offline` on subsequent invocations. Pass
`--identity PATH` to save the logged identity as JSON. Use a fresh disposable interpreter for each
system run, since the script deliberately installs into that interpreter. An offline run still uses
its real compiler and native libraries. Refresh an interpreter profile with the same
`update_package_fixtures.py --resolve PROFILE` command used for cache profiles, and exercise the
relevant interpreter families before accepting the updated graph.
