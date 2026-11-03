# Compare build dependency lock times

`run.py` launches a single named project or the pinned `uv-ecosystem-testing` lock workload. Run it
from a non-root Linux account with Docker and Python 3.11 or later. Pull the digest-pinned Python
images listed in the script first. The corpus uses CPython 3.14 with the harness default universal
range of `>=3.13`. The named workloads use 3.12, except for Warehouse, whose project requires 3.11.
The `transformers` project includes all 39 optional dependency groups in the lock. The separate
`transformers-torch-vision` project retains the upstream `torch` and `vision` groups from the same
revision. Use the same native image for both cases. Airflow uses the requirements in
`test/requirements/airflow.in`.

## Seed

Use a dedicated internal IPv4 Docker bridge with `gateway_mode_ipv4=isolated`. The only service on
the bridge should be a credential-free proxy restricted to the public PyPI index and file hosts. The
workload mounts one binary, the required fixture or harness inputs, and dedicated writable cache and
result directories. Keep the cache and run output in disjoint directory trees. The container writes
under `OUTPUT/results`, and the host launcher records provenance in `OUTPUT/run.json`. Each
index-backed command uses the same absolute cutoff.

For example, with that network and proxy available:

```sh
python3 scripts/benchmark/lock/run.py black --uv "$BASELINE" \
    --output "$RUN/base-seed" --cache "$RUN/base-cache" \
    --network "$BENCH_NETWORK" --proxy "$BENCH_PROXY"
```

For the corpus, pass `corpus --harness "$HARNESS" --wheels "$WHEELS"` instead of `black`. Check out
harness commit `50ed1c0b195e773da684d72db6e863aba32ffcde` and supply the `tomli-w` and `tqdm` wheels
from its lockfile. The launcher checks their hashes before importing the resolver harness inside
Docker. The checked-in 15k list is the default. Use `--limit` for a pilot. Eight packages are listed
in the upstream harness's exclusions. Each corpus subprocess has a ten-minute time limit.

Archive a seed after its container exits. This reads the package-created cache through a read-only
mount. `SEED_IMAGE` is the digest-pinned image for the run.

```sh
docker run --rm --network none --cap-drop ALL --security-opt no-new-privileges \
    --read-only --user "$(id -u):$(id -g)" -e UV_EXCLUDE_NEWER=2026-10-01T00:00:00Z \
    --mount "type=bind,src=$RUN/base-cache,dst=/cache,readonly" \
    "$SEED_IMAGE" tar -C /cache -cf - . > "$RUN/base.tar"
```

To include candidate-only inputs, repeat the online seed with the candidate binary and its preview
settings, passing `--seed "$RUN/base.tar"` and fresh directories. Archive the resulting cache as
`union.tar`. Keep the online outcomes so cache misses in subsequent offline runs can be
distinguished from resolution failures.

For several online corpus passes on the same shard, keep the dedicated cache directory and pass
`--continue-seed-from` with the preceding run's output directory. Each pass records its predecessor.
The launcher verifies the cache identity and workload and allows one writer at a time. Archive the
union after the last pass and use a fresh cache directory for each timed run.

## Measure and compare

Run the two binaries serially on the same host with the same seed archive. Pass
`--source-pair SEPARATE_COMMIT INTEGRATED_COMMIT` to every arm, including the baseline, to record
the two full source hashes. Comparison, resume, and continued cache population check that the pair
matches. Each invocation restores an independent writable cache in a network-disabled container and
starts the project without a lockfile. For repeats, use new output and cache directories. Record the
source commits and build profile for both binaries. Use identical `--workers` for both sides of a
corpus comparison.

```sh
python3 scripts/benchmark/lock/run.py black --uv "$BASELINE" \
    --output "$RUN/base" --cache "$RUN/base-measured-cache" --seed "$RUN/union.tar"
python3 scripts/benchmark/lock/run.py black --uv "$CANDIDATE" \
    --output "$RUN/candidate" --cache "$RUN/candidate-measured-cache" \
    --seed "$RUN/union.tar" --preview build-dependency-locking
mkdir "$RUN/report"
docker run --rm --network none --cap-drop ALL --security-opt no-new-privileges \
    --read-only --user "$(id -u):$(id -g)" -e UV_EXCLUDE_NEWER=2026-10-01T00:00:00Z \
    --mount "type=bind,src=$RUN/base,dst=/base,readonly" \
    --mount "type=bind,src=$RUN/candidate,dst=/candidate,readonly" \
    --mount "type=bind,src=$RUN/report,dst=/out" \
    --mount "type=bind,src=$PWD/scripts/benchmark/lock/compare.py,dst=/compare.py,readonly" \
    "$SEED_IMAGE" python -I /compare.py /base /candidate /out/comparison
```

The CSV records outcomes for both sides, wall time, lockfile size, package entries and distinct
versions. A paired success also reports additional versions to audit. The SVGs plot the absolute and
percentage change for paired successes. Failure causes are inferred from stderr. Raw stderr and
lockfiles remain in each run's output directory.

## Resume a stopped corpus run

Each package's summary, stdout, stderr and successful lockfile are flushed independently. Snapshot a
stopped run's cache as above. Mount its output and the original checked-in CSV read-only when
generating the remaining targets:

```sh
docker run --rm --network none --cap-drop ALL --security-opt no-new-privileges \
    --read-only --user "$(id -u):$(id -g)" -e UV_EXCLUDE_NEWER=2026-10-01T00:00:00Z \
    --mount "type=bind,src=$HARNESS/data/top-15k-pypi.csv,dst=/manifest.csv,readonly" \
    --mount "type=bind,src=$RUN/base,dst=/prior,readonly" \
    --mount "type=bind,src=$PWD/scripts/benchmark/lock/resume.py,dst=/resume.py,readonly" \
    "$SEED_IMAGE" python -I /resume.py /manifest.csv /prior > "$RUN/remaining.csv"
```

Pass `--input "$RUN/remaining.csv"` to a fresh corpus invocation. For an online seed, restore the
interrupted cache using `--seed`. For a paired measurement, use the original seed and use the same
shard on both sides. Keep each attempt in a separate directory. For a second interruption, supply
all prior run directories to `resume.py` in order. The command checks their manifest hashes and
experiment settings and retries partially written records.

## Transformers native build dependencies

For a separate Transformers run, gather the Debian packages for the pinned CPython 3.12 base image
using its signed apt indexes. Use `apt-get --download-only --no-install-recommends install` with
`pkg-config`, `gcc`, `libc6-dev`, `libavformat-dev`, `libavcodec-dev`, `libavdevice-dev`,
`libavutil-dev`, `libavfilter-dev`, `libswscale-dev` and `libswresample-dev`. Save the downloaded
packages and their hashes. Put the packages under `debs/` in a build context. The offline build
checks that pkg-config finds all seven FFmpeg libraries.

```sh
docker build --network none --provenance=false -t uv-lock-transformers \
    -f scripts/benchmark/lock/transformers.Dockerfile "$BUILD_CONTEXT"
docker image inspect uv-lock-transformers --format '{{.Id}}'
```

Pass the resulting digest to `run.py transformers --image-id "$IMAGE_ID"` for both binaries. The
runner verifies the image's Python minor and records its digest alongside the original fixture hash.

## Measure an excluded corpus root

For a separate bounded comparison of the five excluded roots present in the pinned CSV, supply one
original CSV row and its project name, for example `--input "$SHARD" --excluded-root emmet-api`. The
runner invokes the harness's lock command for that single name with the universal range `>=3.13` on
CPython 3.14. Use an identical union cache for the baseline and candidate. Keep these results
separate from the standard corpus series.

Each run gets four CPUs, 8 GiB of memory, 512 PIDs, and ten minutes for the lock command with ten
additional seconds for termination. On a cgroup v2 host the summary includes maximum RSS in KiB, OOM
kills and PID-limit hits. If the whole container is killed, inspect the Docker state in `run.json`
and the streamed stdout and stderr in the package directory.

## Compare encodings of the same graph

Build the uv binary first and record its digest. Copy `encoding_sizes.rs` into
`crates/uv-lock/examples/` in the prototype checkout. If the crate omits
`Lock::with_build_encoding`, append `reencode.rs` to `crates/uv-lock/src/lock/build.rs` for this
example build. The adapter selects the encoding on an already resolved graph. Build the example with
the same toolchain and restore the source file after the build:

```sh
cargo build --locked --profile profiling -p uv-lock --example encoding_sizes
```

In a network-disabled analysis container, mount the example binary and the measured locks read-only.
Pass one or more lockfile paths to the example. Each JSON line records the byte count, package rows,
explicit build-universe rows, and three timings of `Lock::to_toml` in nanoseconds for each encoding.
Parsing, encoding selection, and graph comparison happen outside the timed section. To measure the
writer's cost for a larger graph, pass the runtime-only and build-locked files to the same example
binary and compare their dependency-encoding medians. The round-trip check reparses each result and
compares its decoded graph to the original. Serialization takes place after timing the resolver.

For a prefetch A/B, use the same overlay binary and set `UV_INTERNAL__BUILD_REQUIREMENTS_PREFETCH`
to `0` and `1` in separate runs. Hold `UV_INTERNAL__BUILD_RUNTIME_PREFERENCES=first` fixed. Run the
cold pair online with fresh empty caches and the warm pair offline with copies of the same seed.
Pass `--prefetch-diagnostics` in additional named runs to capture the resolver's counters in saved
stderr. Exclude those verbose runs from the timing comparison and verify that successful off/on
locks have identical bytes.
