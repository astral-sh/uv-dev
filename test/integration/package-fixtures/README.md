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
