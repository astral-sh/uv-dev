# Network replay

`bench.py` serves hash-pinned public distributions over loopback. It can impose a shared
response-body bandwidth cap, response latency, deterministic per-request jitter, transient HTTP
failures, truncated downloads, missing PEP 658 metadata, and missing range support. It can also
close a connection before sending response headers. Each trial gets a fresh cache and working
directory. `--cache-mode warm` primes that cache before timing; `refresh` additionally forces
revalidation. For commands that always revalidate registry data, such as `pip list --outdated` and
`tool list --outdated`, pass `--refresh-mode implicit`. Their warm and refresh runs both use the
command's own revalidation policy, without appending an unsupported `--refresh` argument. The
selected mode and harness hash are recorded with the measurements.

Prepare the immutable artifacts once, outside the measured interval:

```sh
uv run --no-project python scripts/benchmark/network/bench.py \
  --directory "$HOME/code/tmp/uv-network-fixtures" prepare
```

Use `--manifest scripts/benchmark/network/source-fixtures.json` before the subcommand to prepare or
replay source distributions. These fixtures include static `PKG-INFO` metadata, so a
`pip compile --no-binary :all:` workload can exercise archive transfer and extraction without
requiring a build backend. An individual manifest entry can set `"pep658": false` to omit its
advertised metadata while other distributions still offer sidecars. `python-fixtures.json` contains
a pinned managed CPython archive. Manifest entries with `"kind": "raw"` are served as files without
parsing package metadata. An entry's optional `paths` array adds exact aliases for clients that
construct release URLs. `tool-fixtures.json` uses this to replay a pinned Ruff release through
`UV_ASTRAL_MIRROR_URL`.
`make_scheduling_fixtures.py --directory DIR --manifest MANIFEST --profiles PROFILES` creates
deterministic wheels for a small synthetic backtracking graph. Requiring `uv-bench-choice` and
`uv-bench-pin==1.0` rejects releases 30 through 16 and selects release 15. The `slow-unused` profile
delays metadata for older, unused releases to expose speculative requests that outlive the solution.
Report this workload as synthetic; it measures request scheduling rather than a representative
package installation. Replay waits for outstanding fixture handlers after timing ends, so cancelled
responses are recorded and cannot overlap the next trial.
`make_build_constraint_fixtures.py --directory DIR --constraints FILE` creates a 16-package
workspace whose local build backends emit deterministic wheels. Pass the Python 3.12 constraints
from
[Airflow 3.1.0 at its pinned commit](https://raw.githubusercontent.com/apache/airflow/3675eaba7aaedde3eb68bc947a8ec8958fbb0050/constraints-3.12.txt).
The generator checks the public file's hash and records every expected wheel hash. Use
`build --wheel --all-packages --no-index --out-dir '{work}/dist' --build-constraint '{base}/files/airflow-3.1.0-constraints-3.12.txt' '{fixtures}/workspace'`
with `--verify-tree '{work}/dist'` to measure repeated constraint reads. The workspace is synthetic;
the 17,974-byte constraints file is unmodified public data. A single selected package and a local
constraint path provide controls for work that cannot benefit from sharing remote reads.
`make_source_prefetch_fixtures.py --directory DIR` creates a source-only package with a large newer
release and a wheel that constrains it to a small older release. The generated manifests cover both
absent and advertised source sidecars. `source_graph_oracle.py` accepts selected filenames and
retrieves their advertised metadata or complete archives concurrently after each package index. It
verifies every response and reports the bound for that retrieval strategy. Use the same graph with
explicit version pins to measure the cost of deferring useful prefetches.
`make_wheel_prefetch_fixtures.py --directory DIR` first discovers that a registry lacks range
support, then introduces an unused wheel whose metadata follows a large payload. The
`uv-bench-wheel-root==1.0` case discards that wheel; root versions `2.0` and `3.0` select the small
and large choices, respectively. Its other manifest advertises wheel sidecars, and its `range`
profile restores range support.

`make_osv_fixtures.py --directory DIR` creates deterministic wheels, frozen direct-URL lockfiles,
and hash-pinned OSV response fixtures. Use the generated project and lockfile with
`--project-template` and `--lock-template`. The replay validates every dependency and pagination
token, records a canonical request digest, and supports delays and transient failures for individual
batches. `osv_oracle.py --packages N --concurrency C` retrieves those batches when their
prerequisites finish. Its optimistic bound assumes at most 1,000 queries per request and permits
arbitrary batching and overlap after each pagination token becomes available. Report this as a
synthetic audit workload. The pagination control exercises the API contract with deliberately small
response pages.

`make_osv_pagination_fixtures.py --directory DIR` adds dense and sparse page graphs plus a page
chain independent of a slow initial batch. These fixtures record each query's package and page
alongside the request digest, so studies can verify complete pagination when request grouping
changes. The `flaky-page` profile returns one transient failure during that independent chain.

`make_osv_record_fixtures.py --directory DIR` adds pinned vulnerability records, shared identifiers,
and record-specific delays and failures. Use `--expected-exit-code 1` for an audit that reports
vulnerabilities; both the measured command and any cache warmup must return that status.
`osv_record_oracle.py` starts each distinct full record once an identifying page completes and uses
one concurrency limit for pages and records. Pass `--records-cached` for a warm record-cache
reference. Its optimistic bound includes the required response bodies and identifier-to-record
dependencies. Set `osv_path_prefixes` in a network profile, such as `["/first", "/second"]`, to
serve the pinned API at multiple service URLs. The replay retains each actual request path and
tracks failures independently for each prefix.

`make_osv_service_fixtures.py --directory DIR` creates two service prefixes with shared advisory IDs
and setup commands that warm one service, alternate between services, or return to the first
service. The setup commands ignore the fixture advisories so they complete successfully; the
measured audit still reports the full findings.

`make_osv_modified_fixtures.py --directory DIR` creates matching, missing, malformed, and mismatched
modification timestamps in query summaries while keeping the full advisory records fixed. Its
`stale-*` profiles use a past response date to expire HTTP cache entries without a timed sleep. The
fixture's optional `summary_modified` map overrides the timestamp for each advisory ID; a null value
omits the field. Use the generated setup commands to populate the record cache before the measured
audit. The record oracle's `--records-revalidate` mode sends conditional requests for unchanged
cached records and counts their body-free responses in the network bound.

`make_tool_list_fixtures.py --directory DIR --packages N` creates two releases of each of `N`
console-script packages and commands that install the older releases locally. Set
`UV_TOOL_DIR={work}/tools` and `UV_TOOL_BIN_DIR={work}/bin` with `--env` to isolate each trial. The
`latest_index_oracle.py` reference fetches the known Simple API pages through a bounded pool of
persistent HTTP/1.1 connections. Its bound includes required response bytes and request waves, while
its measured reference also pays the configured connection delay and records any immediate retries.
`make_tool_filename_fixtures.py --directory DIR` creates an executable wheel, the equivalent
normalized source archive, and source-directory and unnamed-archive controls. Its setup commands
install each tool with an uncached flat index. Repeating the same archive request measures whether
name inference can finish without consulting that index; the required network bytes for a satisfied
local requirement are zero. Fresh installation controls still require their artifacts. Pass
`--build-dependency` to add a separate wheel required by the source backend, so installation-plan
studies can verify that source builds still resolve their build dependencies from the flat index.
`make_tool_audit_fixtures.py --directory DIR --packages N` creates locked local tools with
independent OSV queries. Its plain and paginated manifests exercise connection reuse across tool
audits. The OSV oracle can query the complete known dependency set in one batch when it contains at
most 1,000 packages, providing a realizable reference for further batching improvements. Pass
`--source registry` to install the tools from two saved index paths and exercise their
project-status lookups. `tool_audit_oracle.py` retrieves those complete Simple API pages while
batching the OSV queries across tools. It records an optimistic bound that permits the two services
to overlap. The `cached` profile keeps registry responses fresh; pair it with `--cached-registry` on
the oracle to exclude already available pages from the required network bytes.

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

The JSON retains binary hashes, source revisions, fixture hashes, command outputs, per-request
timing and byte counters, every paired measurement, and a pointwise 95% paired-bootstrap interval. A
result passes the 5% threshold only when the upper end of the candidate/parent interval is at most
`0.95`. Re-run relevant workloads under `fast`, `range-fast`, warm-cache, and failure profiles
before accepting a change. A successful command is required, and normalized standard output must
match within each pair. Installation workloads should pass `--verify-tree '{work}/site'` (and
install into that directory) to compare every installed file, executable bit, directory, and symlink
after the timed command. When repairing a harness after some cases have finished, retain the
original harness in the evidence archive. A verification case can name its `harness_file` and
`harness_sha256`; `requires_harness_hash` also requires the result to record that hash. The verifier
recomputes statistics and bounds with the selected file. Use `--compare-stderr` when the command's
diagnostic output is deterministic; it compares normalized standard error in addition to standard
output. For installation files that embed their destination, `--normalize-tree-file GLOB` replaces
the trial URL and directory before hashing only matching relative paths. For matching installed
files, their `RECORD` hashes and sizes are checked against the original contents and then updated
for the normalized comparison. Unselected files and their recorded entries remain exact comparisons.
Managed Python workloads use this for `**/_sysconfigdata_*.py`. `--normalize-tree-symlink GLOB`
similarly normalizes selected absolute symlink targets beneath the trial directory. Targets outside
that directory, relative targets, and unselected links remain exact comparisons. Managed Python
workloads can select the minor-version link after confirming it is the only remaining trial-specific
difference.

`make_requirements_fixtures.py --directory DIR --manifest MANIFEST` creates a chain of six remote
requirements files ending in a pinned `iniconfig` requirement. `requirements_oracle.py` follows that
chain and reads the known package metadata on one persistent HTTP/1.1 connection. Run both the
oracle and paired command under the same kernel network profile to measure connection setup and
remaining headroom.

`make_requirements_prefetch_fixtures.py` creates independent sibling includes, a dependency chain,
and repeated requirement/constraint includes. `requirements_graph_oracle.py --concurrency N`
discovers the graph and retrieves each unique file with at most `N` requests in flight. Repeat
`--root PATH` for multiple input URLs that are known before parsing begins. It then reads the pinned
package's index and metadata. Its optimistic bound also permits those metadata requests to overlap
remaining includes, so the measured oracle and the bound describe distinct, explicit assumptions.
Use `--route requirements` for a warm cache whose selected package metadata is already available, or
`--route revalidate` to conditionally validate an unchanged index that identifies the cached
sidecar.

For commands that require an installed environment or project files, pass `--setup-commands PATH`
with a JSON array of uv argument arrays. Those commands run before timing with the same binary and
isolated cache. `--config-template` and `--project-template` copy text templates to the trial's
`uv.toml` and `pyproject.toml`; the former is selected with `--config-file`. Use `--pylock-template`
to provide a locked `pylock.toml` input directly. Templates, arguments, and `--requirement` values
support `{base}`, `{index}`, `{work}`, `{python}`, and `{fixtures}`. Use `--env KEY=VALUE` for
settings such as `UV_CONCURRENT_DOWNLOADS`, and repeat `--verify-file '{work}/uv.lock'` to compare
generated files after normalizing the temporary origin and work directory. Use `--no-python-env` for
bare Python management commands, where `UV_PYTHON` would supply an explicit target. Inputs and
environment overrides are saved in the result. Each `/flat/NAME` endpoint serves a PEP 503 page for
the manifest; profiles may set exact-path response delays in `path_latency_ms`. A profile's
`path_aliases` mapping can expose the same Simple API response through several index URLs while
retaining each requested path in the trace.

`concurrency_sweep.py` compares download limits with one pinned source and binary. Give it a
manifest containing one wheel per selected package, a concrete Python executable, and either the
`resolve` or `install` workload. Each setting is paired with a fresh reference run, and setting
order is shuffled from a recorded seed. Verification reconstructs that order, including warmups, and
checks the recorded order within each pair. The results retain byte counts, request traces, output
or installed-tree equivalence, and an optimistic network bound for the selected protocol strategy.
These are exploratory configuration comparisons; selecting a promising limit requires fresh
confirmation and does not qualify a source optimization. `make_concurrency_fixtures.py` supplies a
deterministic set of independent wheels for exploring limits above the package count of a smaller
real-world fixture.

Set `head_ranges` to `false` to model a server that supports ranged `GET` but does not advertise
that support in `HEAD` responses. Set `artifact_etag` to `false` or `"weak"` to omit strong artifact
entity tags. `artifact_last_modified` and `response_date` supply HTTP dates for testing date-based
`If-Range` requests. The resume oracle uses a date only when no entity tag is present and the
response date is at least one minute later; otherwise it restarts interrupted transfers. Matching
dates permit ranges, while a changed validator returns the complete representation. `path_failures`
maps exact request paths to `status` and `count` values, overriding the profile-wide transient
failure settings for those paths. Add `body_bytes` and an optional `chunked: true` to model large or
unknown-length error pages sharing the download bandwidth. `retry_after` supplies the server's retry
advice. Use `{"disconnect": true, "count": 1}` instead of a status to close the first matching
connection before response headers. The trace records status `0`, zero response bytes, and
`injected_disconnect: "before-headers"`. Use direct HTTP/1.1 replay for this fault; a reverse proxy
can translate a backend disconnect into an HTTP error response. Use
`{"cut_after_bytes": 32, "count": 1}` to truncate a matching successful response after its headers
and first 32 body bytes. This also applies to index pages and metadata sidecars, whose traces retain
the advertised response length. Traces identify origin connections, including reuse across requests.
`auth_challenges` maps exact paths to an expected synthetic `authorization` header, an optional
`status` of `401`, `403`, or `404`, and `body_bytes`. Set `chunked` to `true` to send the error
without a known response length. Requests without the expected authorization header receive the
specified error body through the shared bandwidth limiter. Authenticated requests use the normal
fixture response. Traces record whether authentication was accepted without recording the header.
`make_auth_challenge_fixtures.py` supplies a pinned wheel and a synthetic netrc file; pass
`--env NETRC={fixtures}/auth.netrc` to isolate credential lookup. `auth_challenge_oracle.py`
retrieves the known package after cancelling unfamiliar authentication challenges at their headers.
It reports both the bound with those challenge waves and the more optimistic bound when credentials
can be sent on the first request. Its `--preauthenticated` mode also measures that latter strategy.
Set `connection_latency_ms` to add a one-time delay after each origin connection is accepted; this
isolates connection setup costs in application replay. Use kernel network shaping for measurements
of actual TCP handshakes and packet loss. `multi_index_oracle.py` requests those Simple API paths
concurrently and then reads the selected wheel's metadata. Its bound includes all configured
indexes, which is appropriate for an `unsafe-best-match` workload.

`--required-bytes` and `--required-waves` record independently established lower bounds for a
workload. The optimistic time floor is the maximum of required bytes divided by bottleneck bandwidth
and dependent response waves multiplied by minimum response latency. For profiles with different
delays per path, `--required-latency-ms` supplies the minimum total application delay along a known
critical path; kernel RTT is still charged for each required wave. This deliberately permits perfect
transfer/compute overlap and omits TCP startup, TLS, headers, extraction, and other CPU work. Report
both this floor and a realizable oracle client when assessing remaining headroom. Do not derive the
required-byte count from uv's observed traffic: redundant traffic is one of the quantities being
measured.

Application replay does not model packet loss, congestion control, DNS, or TLS. On Linux, `netem.sh`
runs the entire command in a private user/network namespace with only loopback enabled. For example,
use `netem.sh 200 10 1 42 -- COMMAND...` for a nominal 200 ms RTT, shared 10 Mbit/s loopback rate,
and seeded 1% packet loss. Use `none` in place of the seed when the installed `tc` lacks seeded
loss; record that limitation with the measurements. It requires `unshare`, `ip`, and `tc`. Combine
it with the `fast` application profile so latency and bandwidth are not applied twice. Calibrate the
effective rate and RTT with a direct transfer before interpreting results. The wrapper never changes
a host interface, route, or qdisc. On hosts that disable user namespaces, invoke the wrapper with
`sudo -n --preserve-env=HOME`; it drops back to the invoking user with `setpriv` before running the
measured command. Pass `--kernel-counters` to `run` to retain each command's qdisc configuration,
queue drops, and TCP retransmission counters. The snapshots run immediately before and after the
timed interval, within the private namespace. They include the loopback egress placement and queue
limit, which matter when interpreting a high-bandwidth, high-latency test. Loopback shaping remains
a transport simulation; its optimistic byte/RTT floor does not establish the maximum throughput of a
real TCP path.

For TLS/HTTP2 measurements, generate a local test certificate for `127.0.0.1` and pass
`--http2-proxy /absolute/path/to/caddy --tls-certificate CERT.pem --tls-key KEY.pem` to `run`. The
proxy forwards to a Unix socket, so kernel shaping applies only to the client connection. The driver
verifies TLS and ALPN before timing, trusts the supplied certificate only in the measured process,
and checks Caddy's access log to ensure the requests used HTTP/2. The JSON records the proxy binary
and certificate hashes. Use a certificate and key dedicated to this fixture. `http2_oracle.py`
accepts the same fixture, profile, proxy, certificate, and key paths and a `--filename`. It measures
a known index-and-artifact transfer using `curl` on one verified HTTP/2 connection, checks both
response bodies, and records the transfer timings, traffic, and optimistic network floor. Run it
inside the same network namespace as the paired command when calibrating HTTP/2 results.

```sh
mkdir -p "$HOME/code/tmp/uv-network-tls"
openssl req -x509 -newkey rsa:2048 -nodes -sha256 -days 365 \
  -subj /CN=127.0.0.1 -addext subjectAltName=IP:127.0.0.1 \
  -addext basicConstraints=critical,CA:FALSE -addext extendedKeyUsage=serverAuth \
  -keyout "$HOME/code/tmp/uv-network-tls/key.pem" \
  -out "$HOME/code/tmp/uv-network-tls/cert.pem"
```

The `calibrate --profile slow --output PATH` subcommand measures one `HEAD` and two concurrent,
byte-checked range transfers independently of uv. Use the same fixture directory and network
namespace as the actual benchmark.

The `oracle` subcommand takes one or more `--filename` arguments and fetches the known package graph
with unlimited concurrency. `--route metadata` uses PEP 658; `--route wheel` downloads whole wheels;
`--route resume` immediately retries interrupted bodies using byte ranges and checks the completed
artifact. The `raw` and `raw-resume` routes use known artifact URLs without an index lookup. It
reports a realizable reference time, actual traffic, and optimistic metadata and full-artifact
transfer bounds. The oracle assumes all selected versions and dependencies are already known, so its
advantage includes dependency-discovery work that a cold resolver may need to do.
`repeat.py --pilot PILOT --manifest MANIFEST --directory DIR --work-dir WORK --output RESULT --pairs 30`
reruns a completed pilot with its recorded binaries, Python executable, profile, command, setup,
cache mode, and output checks. It verifies their identities before timing and refuses to replace an
existing result. For an older pilot that records Python identity but omits the command timeout, pass
the original `--timeout` explicitly. Run kernel-network pilots inside the same `netem.sh` profile
and supply the original `--git-root` or TLS certificate and key when used. Pass
`--uv /absolute/path/to/uv` when the namespace launcher changes `PATH`. The output records the
pilot's SHA-256. A repeat remains separate evidence; retain the original pilot and every planned
control when reviewing a qualification study. `make_direct_wheel_fixtures.py --directory DIR`
creates direct-wheel requirements, an alternate version for registry overrides, a runtime
dependency, and a local source project with an isolated build dependency. Use the same fixtures with
and without `--no-deps` to check whether a configured flat index is necessary.
`flat_index_oracle.py` accepts a manifest, profiles file, profile, selected filenames, and
`--flat-package` names. It fetches the shared flat index once and the remaining Simple API pages
independently, then verifies the selected metadata sidecars. Its lower bound accounts for different
response delays along each index-to-metadata path. `find_links_oracle.py` accepts repeated
`--index-path` arguments, fetches every configured flat index concurrently, and then verifies one
selected metadata sidecar. It reports both a known-package minimum and an all-index transfer
reference. `make_duplicate_links_fixtures.py --directory DIR` creates a larger flat index for
repeated-location studies. Give the oracle each unique location once, and retain single-location and
distinct-location controls. Its `--route versions` mode stops after reading the index pages, as an
outdated version lookup does. Pass `--concurrency` to limit simultaneous index requests and include
that limit in the latency bound. `make_outdated_fixtures.py --directory DIR` creates two wheel
releases, setup commands that install the older release, and a nine-index latency profile for
`pip list --outdated`. `latest_version_oracle.py` measures concurrent version discovery across
specified Simple API and find-links paths without fetching distribution metadata.
`retry-after-profiles.json` supplies transient index errors with numeric, dated, missing, and
invalid retry advice. `retry_oracle.py` fetches a known distribution's index and sidecar with the
same retry budget and maximum delay. Its bound includes fixed numeric delays on the serial request
path. Pass the reported wait with `run --required-wait-ms` only when it cannot overlap another
required transfer. `first-transport-retry-profiles.json` adds single and repeated connection
closures at either metadata request, plus HTTP status controls. The retry oracle follows these
transport failures immediately and includes their request latency in the recorded serial path.
`first-body-retry-profiles.json` truncates Simple API pages and metadata sidecars after headers. The
retry oracle includes the lost prefixes in its byte bound; `--flat-index-path /flat/all` selects a
find-links page for the same experiment. `cached_wheel_oracle.py` verifies a complete wheel fixture
and measures a local metadata read. A direct URL already identified by that content hash has a zero
network-transfer bound; process startup, filesystem, and CPU costs remain. Use `--cache-state fresh`
for a fresh HTTP cache entry or `--cache-state revalidate` to measure one conditional request before
reading the unchanged cached metadata. `make_publish_fixtures.py --directory DIR` creates a batch of
distinct wheel builds for repeated `publish --dry-run --check-url` checks. `publish_check_oracle.py`
reads one current index per known package and verifies every selected local file against its
advertised SHA-256. Use `--route revalidate` to conditionally validate cached index bodies. The
replay serves already-published files, so this workload does not upload anything. Report it as a
synthetic repeat-publish workload. `make_frozen_fixtures.py --directory DIR` creates a wheel-only
project and setup commands that lock it before timing. Its profiles include a slow or intermittently
failing find-links source that is introduced only for the frozen installation.
`make_pylock_fixtures.py --directory DIR` creates direct wheel and local source-build `pylock.toml`
inputs. The source build imports a dependency available from the replay index and copies a valid
wheel, providing a control for indexes needed during builds. `sdist_metadata_oracle.py` measures a
gzip prefix through a known `PKG-INFO` entry when the index does not advertise a metadata sidecar.
This optimistic reference does not require full-archive integrity validation; use the full-artifact
oracle when the operation must check an archive hash. `cache_revalidation_oracle.py` measures
conditional index validation when the advertised strong hash matches an already validated local
source revision. Its one-response bound applies to an unchanged index that requires revalidation.
Use `--identity metadata` for a cached PEP 658 sidecar whose advertised hash is unchanged, or
`--identity wheel` for metadata in a fully verified cached wheel. `zip_metadata_oracle.py` reads a
known wheel metadata entry by its ZIP offset, verifies the received archive bytes, and reports its
compressed-byte lower bound. With ranges disabled, it reads the necessary archive prefix instead.

For mixed artifact hosts, a `run` profile can set `artifact_origins` to a mapping of origin names to
`filenames` arrays and optional `profile` overrides. The index links those files to separate
loopback listeners, each with its own range and failure behavior. All listeners share the root
response-body bandwidth cap. Traces identify the responding origin, and output comparison normalizes
each temporary URL separately. This mode currently uses HTTP/1.1.
`make_origin_fixtures.py --directory DIR` creates a small package that depends on the pinned NumPy
wheel, plus profiles for mixed and uniform range capabilities. Copy or prepare the target wheel
before timing. `artifact_origin_oracle.py` accepts repeated `--filename` arguments in
dependency-discovery order and reads each index and metadata entry using known artifact capabilities
and ZIP offsets.

For Git fetch scheduling, `make_git_fixtures.py --directory PATH --packages 8` creates a
deterministic bare monorepo, a hash-pinned bundle, and a project template that depends on eight
subdirectories at the same branch. Pass its `git-fixtures.json` to `--manifest`, its `git` directory
to `run --git-root`, and `git-project.toml` to `--project-template`. The replay delegates read-only
smart HTTP requests to `git http-backend`; its trace includes protocol headers, request body hashes
and sizes, response bytes, and timings. Per-path `status` and `count` failures also apply to Git
advertisements and upload-pack requests. Result files record the served refs and Git version.
Use `git_oracle.py` with the same fixture directory, profiles, and work directory for a single
verified branch fetch. Its time and traffic are a realizable reference. The optimistic cold network
floor assumes two dependent responses and gives no byte minimum, since negotiated Git pack contents
and compression can differ.

`make_tool_git_fixtures.py --directory PATH` creates dependency-free console tools in a
deterministic Git monorepo, separate-repository controls, and replayable GitHub commit responses.
Pass its `tool-git-fixtures.json`, `git` directory, and one of its `tool-git-*-setup.json` files to
`run` when measuring `tool upgrade --all`. The setup installs the selected tools before timing. Use
`UV_GITHUB_FAST_PATH_URL={base}/github` and a Git URL rewrite from
`https://github.com/uv-network-benchmark/` to `{base}/git/` so every request stays inside the
replay. `tool_git_oracle.py` fetches and verifies each distinct GitHub reference once. Its
optimistic bound also allows Git ref advertisement, conditional responses, and reuse of installed
immutable revisions.

`make_pool_fixtures.py --directory PATH --width 50` creates two wide groups of packages joined by
one shared dependency. Its `gate` profile delays that shared dependency's index response, leaving a
gap between request bursts. Use `pool-project.toml` as the project template and compare the trace's
`origin_connections` count as well as wall time. `connection_pool_oracle.py` reads the known graph
with one persistent HTTP/1.1 connection per package and reports byte, latency, and connection
counts. The origin accepts a backlog of 256 connections to keep the replay server's listen queue out
of these results.

Run protocol and bottleneck checks with:

```sh
uv run --no-project python scripts/benchmark/network/test_bench.py
```

`verify.py --evidence DIR --repository REPO --spec STUDY.json --output RESULT.json` checks an
archived paired study against exact local source revisions. The study specification lists `parent`,
`head`, `binary_sha256`, `scope`, `result_globs`, and every case's `file`, `pairs`, and `role`
(`primary`, `fast`, or `control`). Mark the primary qualifying case with `qualifying: true`; use
`requires_tree` or `required_files` for commands that produce installation trees or output files.
Use `requires_stderr: true` for studies that must compare diagnostic output. The verifier recomputes
confidence intervals and traffic totals and rejects omitted cases. Use `--retain-nonqualifying` to
verify a completed study whose predeclared primary misses the performance threshold. Its result
records the failed gate and retains every other evidence check. Review the workload and oracle
assumptions separately.
