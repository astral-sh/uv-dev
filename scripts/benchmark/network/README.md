# Network replay

`bench.py` serves hash-pinned public wheels over loopback. It can impose a shared
response-body bandwidth cap, response latency, deterministic per-request jitter,
transient HTTP failures, truncated downloads, missing PEP 658 metadata, and missing
range support. Each trial gets a fresh cache and working directory. `--cache-mode
warm` primes that cache before timing; `refresh` additionally forces revalidation.

Prepare the immutable artifacts once, outside the measured interval:

```sh
uv run --no-project python scripts/benchmark/network/bench.py \
  --directory "$HOME/code/tmp/uv-network-fixtures" prepare
```

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

`--required-bytes` and `--required-waves` record independently established lower
bounds for a workload. The optimistic time floor is the maximum of required bytes
divided by bottleneck bandwidth and dependent response waves multiplied by minimum
response latency. This deliberately permits perfect transfer/compute overlap and
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

The `calibrate --profile slow --output PATH` subcommand measures one `HEAD` and
two concurrent, byte-checked range transfers independently of uv. Use the same
fixture directory and network namespace as the actual benchmark.

The `oracle` subcommand takes one or more `--filename` arguments and fetches the
known package graph with unlimited concurrency. `--route metadata` uses PEP 658;
`--route wheel` downloads whole wheels. It reports a realizable reference time,
actual traffic, and an optimistic metadata-byte/request-wave lower bound. The
oracle assumes all selected versions and dependencies are already known, so its
advantage includes dependency-discovery work that a cold resolver may need to do.

Run protocol and bottleneck checks with:

```sh
uv run --no-project python scripts/benchmark/network/test_bench.py
```
