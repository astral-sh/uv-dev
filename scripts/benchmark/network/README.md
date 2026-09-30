# Network replay

`bench.py` serves hash-pinned public distributions over loopback. It can impose a shared
response-body bandwidth cap, response latency, deterministic per-request jitter,
transient HTTP failures, truncated downloads, missing PEP 658 metadata, and missing
range support. Each trial gets a fresh cache and working directory. `--cache-mode
warm` primes that cache before timing; `refresh` additionally forces revalidation.

Prepare the immutable artifacts once, outside the measured interval:

```sh
uv run --no-project python scripts/benchmark/network/bench.py \
  --directory "$HOME/code/tmp/uv-network-fixtures" prepare
```

Use `--manifest scripts/benchmark/network/source-fixtures.json` before the
subcommand to prepare or replay source distributions. These fixtures include
static `PKG-INFO` metadata, so a `pip compile --no-binary :all:` workload
can exercise archive transfer and extraction without requiring a build backend.
`python-fixtures.json` contains a pinned managed CPython archive. Manifest entries
with `"kind": "raw"` are served as files without parsing package metadata.
An entry's optional `paths` array adds exact aliases for clients that construct
release URLs. `tool-fixtures.json` uses this to replay a pinned Ruff release
through `UV_ASTRAL_MIRROR_URL`.
`make_scheduling_fixtures.py --directory DIR --manifest MANIFEST --profiles PROFILES`
creates deterministic wheels for a small synthetic backtracking graph. Requiring
`uv-bench-choice` and `uv-bench-pin==1.0` rejects releases 30 through 16 and selects
release 15. The `slow-unused` profile delays metadata for older, unused releases
to expose speculative requests that outlive the solution. Report this workload
as synthetic; it measures request scheduling rather than a representative package
installation. Replay waits for outstanding fixture handlers after timing ends,
so cancelled responses are recorded and cannot overlap the next trial.

Compare optimized binaries built from an exact parent and candidate commit:

```sh
uv run --no-project python scripts/benchmark/network/bench.py \
  --directory "$HOME/code/tmp/uv-network-fixtures" run \
  --parent /absolute/path/to/parent/uv --parent-sha PARENT_COMMIT \
  --head /absolute/path/to/head/uv --head-sha HEAD_COMMIT \
  --profile range-slow --pairs 20 \
  --requirement six==1.17.0 \
  --work-dir "$HOME/code/tmp/uv-network-trials" \
  --output "$HOME/code/tmp/uv-network-results.json" \
  -- pip compile --no-header --no-annotate \
  --python-version 3.12 --default-index '{index}' '{work}/requirements.in'
```

The JSON retains binary hashes, source revisions, fixture hashes, command outputs,
per-request timing and byte counters, every paired measurement, and a pointwise
95% paired-bootstrap interval. A result passes the 5% threshold only when the upper
end of the candidate/parent interval is at most `0.95`. Re-run relevant workloads
under `fast`, `range-fast`, warm-cache, and failure profiles before accepting a
change. A successful command is required, and normalized standard output must
match within each pair. Installation workloads should pass `--verify-tree
'{work}/site'` (and install into that directory) to compare every installed file,
executable bit, directory, and symlink after the timed command.
For installation files that embed their destination, `--normalize-tree-file GLOB`
replaces the trial URL and directory before hashing only matching relative paths.
Managed Python workloads use this for `**/_sysconfigdata_*.py`.

`make_requirements_fixtures.py --directory DIR --manifest MANIFEST` creates a
chain of six remote requirements files ending in a pinned `iniconfig` requirement.
`requirements_oracle.py` follows that chain and reads the known package metadata
on one persistent HTTP/1.1 connection. Run both the oracle and paired command under
the same kernel network profile to measure connection setup and remaining headroom.

For commands that require an installed environment or project files, pass
`--setup-commands PATH` with a JSON array of uv argument arrays. Those commands
run before timing with the same binary and isolated cache. `--config-template`
and `--project-template` copy text templates to the trial's `uv.toml` and
`pyproject.toml`; the former is selected with `--config-file`. Templates and
arguments support `{base}`, `{index}`, `{work}`, `{python}`, and `{fixtures}`.
Use `--env KEY=VALUE` for settings such as `UV_CONCURRENT_DOWNLOADS`, and repeat
`--verify-file '{work}/uv.lock'` to compare generated files after normalizing the
temporary origin and work directory. Inputs and environment overrides are saved
in the result. Each `/flat/NAME` endpoint serves a PEP 503 page for the manifest;
profiles may set exact-path response delays in `path_latency_ms`.

`--required-bytes` and `--required-waves` record independently established lower
bounds for a workload. The optimistic time floor is the maximum of required bytes
divided by bottleneck bandwidth and dependent response waves multiplied by minimum
response latency. For profiles with different delays per path,
`--required-latency-ms` supplies the minimum total application delay along a known
critical path; kernel RTT is still charged for each required wave. This deliberately
permits perfect transfer/compute overlap and
omits TCP startup, TLS, headers, extraction, and other CPU work. Report both this
floor and a realizable oracle client when assessing remaining headroom. Do not
derive the required-byte count from uv's observed traffic: redundant traffic is
one of the quantities being measured.

Application replay does not model packet loss, congestion control, DNS, or TLS.
On Linux, `netem.sh` runs the entire command in a private user/network namespace
with only loopback enabled. For example, use `netem.sh 200 10 1 42 -- COMMAND...`
for a nominal 200 ms RTT, shared 10 Mbit/s loopback rate, and seeded 1% packet loss.
Use `none` in place of the seed when the installed `tc` lacks seeded loss; record
that limitation with the measurements.
It requires `unshare`, `ip`, and `tc`. Combine it with the `fast` application
profile so latency and bandwidth are not applied twice. Calibrate the effective
rate and RTT with a direct transfer before interpreting results. The wrapper never
changes a host interface, route, or qdisc. On hosts that disable user namespaces,
invoke the wrapper with `sudo -n --preserve-env=HOME`; it drops back to the invoking
user with `setpriv` before running the measured command.

For TLS/HTTP2 measurements, generate a local test certificate for `127.0.0.1` and
pass `--http2-proxy /absolute/path/to/caddy --tls-certificate CERT.pem --tls-key
KEY.pem` to `run`. The proxy forwards to a Unix socket, so kernel shaping applies
only to the client connection. The driver verifies TLS and ALPN before timing,
trusts the supplied certificate only in the measured process, and checks Caddy's
access log to ensure the requests used HTTP/2. The JSON records the proxy binary
and certificate hashes. Use a certificate and key dedicated to this fixture.
`http2_oracle.py` accepts the same fixture, profile, proxy, certificate, and key
paths and a `--filename`. It measures a known index-and-artifact transfer using
`curl` on one verified HTTP/2 connection, checks both response bodies, and records
the transfer timings, traffic, and optimistic network floor. Run it inside the
same network namespace as the paired command when calibrating HTTP/2 results.

```sh
mkdir -p "$HOME/code/tmp/uv-network-tls"
openssl req -x509 -newkey rsa:2048 -nodes -sha256 -days 365 \
  -subj /CN=127.0.0.1 -addext subjectAltName=IP:127.0.0.1 \
  -addext basicConstraints=critical,CA:FALSE -addext extendedKeyUsage=serverAuth \
  -keyout "$HOME/code/tmp/uv-network-tls/key.pem" \
  -out "$HOME/code/tmp/uv-network-tls/cert.pem"
```

The `calibrate --profile slow --output PATH` subcommand measures one `HEAD` and
two concurrent, byte-checked range transfers independently of uv. Use the same
fixture directory and network namespace as the actual benchmark.

The `oracle` subcommand takes one or more `--filename` arguments and fetches the
known package graph with unlimited concurrency. `--route metadata` uses PEP 658;
`--route wheel` downloads whole wheels; `--route resume` immediately retries
interrupted bodies using byte ranges and checks the completed artifact. The `raw`
and `raw-resume` routes use known artifact URLs without an index lookup. It reports
a realizable reference time, actual traffic, and optimistic metadata and
full-artifact transfer bounds. The
oracle assumes all selected versions and dependencies are already known, so its
advantage includes dependency-discovery work that a cold resolver may need to do.
`flat_index_oracle.py` accepts a manifest, profiles file, profile, selected filenames,
and `--flat-package` names. It fetches the shared flat index once and the remaining
Simple API pages independently, then verifies the selected metadata sidecars. Its
lower bound accounts for different response delays along each index-to-metadata path.
`find_links_oracle.py` accepts repeated `--index-path` arguments, fetches every
configured flat index concurrently, and then verifies one selected metadata sidecar.
It reports both a known-package minimum and an all-index transfer reference.

Run protocol and bottleneck checks with:

```sh
uv run --no-project python scripts/benchmark/network/test_bench.py
```
