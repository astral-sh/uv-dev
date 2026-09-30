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
arguments and `--requirement` values support `{base}`, `{index}`, `{work}`,
`{python}`, and `{fixtures}`.
Use `--env KEY=VALUE` for settings such as `UV_CONCURRENT_DOWNLOADS`, and repeat
`--verify-file '{work}/uv.lock'` to compare generated files after normalizing the
temporary origin and work directory. Inputs and environment overrides are saved
in the result. Each `/flat/NAME` endpoint serves a PEP 503 page for the manifest;
profiles may set exact-path response delays in `path_latency_ms`. A profile's
`path_aliases` mapping can expose the same Simple API response through several
index URLs while retaining each requested path in the trace.
Set `head_ranges` to `false` to model a server that supports ranged `GET` but
does not advertise that support in `HEAD` responses.
`path_failures` maps exact request paths to `status` and `count` values, overriding
the profile-wide transient failure settings for those paths.
Traces identify origin connections, including reuse across requests. Set
`connection_latency_ms` to add a one-time delay after each origin connection is
accepted; this isolates connection setup costs in application replay. Use kernel
network shaping for measurements of actual TCP handshakes and packet loss.
`multi_index_oracle.py` requests those Simple API paths concurrently and then
reads the selected wheel's metadata. Its bound includes all configured indexes,
which is appropriate for an `unsafe-best-match` workload.

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
Its `--route versions` mode stops after reading the index pages, as an outdated
version lookup does. `make_outdated_fixtures.py --directory DIR` creates two wheel
releases, setup commands that install the older release, and a nine-index latency
profile for `pip list --outdated`.
`latest_version_oracle.py` measures concurrent version discovery across specified
Simple API and find-links paths without fetching distribution metadata.
`make_frozen_fixtures.py --directory DIR` creates a wheel-only project and setup
commands that lock it before timing. Its profiles include a slow or intermittently
failing find-links source that is introduced only for the frozen installation.
`sdist_metadata_oracle.py` measures a gzip prefix through a known `PKG-INFO` entry
when the index does not advertise a metadata sidecar. This optimistic reference
does not require full-archive integrity validation; use the full-artifact oracle
when the operation must check an archive hash.
`zip_metadata_oracle.py` reads a known wheel metadata entry by its ZIP offset,
verifies the received archive bytes, and reports its compressed-byte lower bound.
With ranges disabled, it reads the necessary archive prefix instead.

For mixed artifact hosts, a `run` profile can set `artifact_origins` to a mapping
of origin names to `filenames` arrays and optional `profile` overrides. The index
links those files to separate loopback listeners, each with its own range and
failure behavior. All listeners share the root response-body bandwidth cap.
Traces identify the responding origin, and output comparison normalizes each
temporary URL separately. This mode currently uses HTTP/1.1.
`make_origin_fixtures.py --directory DIR` creates a small package that depends on
the pinned NumPy wheel, plus profiles for mixed and uniform range capabilities.
Copy or prepare the target wheel before timing. `artifact_origin_oracle.py`
accepts repeated `--filename` arguments in dependency-discovery order and reads
each index and metadata entry using known artifact capabilities and ZIP offsets.

For Git fetch scheduling, `make_git_fixtures.py --directory PATH --packages 8`
creates a deterministic bare monorepo, a hash-pinned bundle, and a project template
that depends on eight subdirectories at the same branch. Pass its
`git-fixtures.json` to `--manifest`, its `git` directory to `run --git-root`, and
`git-project.toml` to `--project-template`. The replay delegates read-only smart
HTTP requests to `git http-backend`; its trace includes protocol headers, request
body hashes and sizes, response bytes, and timings. Result files also record the
served refs and Git version. Use `git_oracle.py` with the same fixture directory,
profiles, and work directory for a single verified branch fetch. Its time and
traffic are a realizable reference. The optimistic cold network floor assumes two
dependent responses and gives no byte minimum, since negotiated Git pack contents
and compression can differ.

`make_pool_fixtures.py --directory PATH --width 50` creates two wide groups of
packages joined by one shared dependency. Its `gate` profile delays that shared
dependency's index response, leaving a gap between request bursts. Use
`pool-project.toml` as the project template and compare the trace's
`origin_connections` count as well as wall time. `connection_pool_oracle.py`
reads the known graph with one persistent HTTP/1.1 connection per package and
reports byte, latency, and connection counts. The origin accepts a backlog of
256 connections to keep the replay server's listen queue out of these results.

Run protocol and bottleneck checks with:

```sh
uv run --no-project python scripts/benchmark/network/test_bench.py
```
