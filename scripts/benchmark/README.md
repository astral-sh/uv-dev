# benchmark

Benchmarking scripts for uv and other package management tools.

## Source preparation

`qualify-source-preparation.py` exercises a chosen `uv` binary with real, offline PEP 517 and PEP
660 builds. It creates an isolated workspace and standard-library build backends, applies a hard
file-descriptor limit to the uv child, records actual backend starts and retained source-cache
descriptors, and imports the installed packages. Python 3.11 or newer is required.

```shell
python3 scripts/benchmark/qualify-source-preparation.py \
  --uv target/debug/uv \
  --python /absolute/path/to/python3.11 \
  --scenario wide --packages 192 --fd-limit 128 --builds 1 \
  --output source-preparation.json
```

Use `--expect emfile`, `--expect timeout`, or `--expect cycle` for a known-failing control.
`--scenario nested` creates one source-built build requirement, `nested-wide` creates `--packages`
sibling source requirements, and `nested-chain` creates `--depth` recursive source requirements.
`siblings` runs `--graphs` independent source graphs with `--packages` build requirements each.
`unnamed-nested` starts from a direct URL whose version requires a metadata build; `cycle` creates
mutually recursive build requirements. `--fault failure` makes one backend fail, then retries with
the same cache; `--fault interrupt` terminates a running backend and retries. These scenarios
distinguish a preparation-width limit from a global descriptor ceiling: ancestors can retain their
cache locks while nested build requirements run.

Linux uses `/proc` for descriptor sampling. On macOS, the optional native observer captures short
descriptor bursts that `lsof` can miss:

```shell
mkdir -p .cache
cc -O2 -Wall -Wextra -Werror scripts/benchmark/observe-source-fds.c \
  -lproc -o .cache/observe-source-fds
python3 scripts/benchmark/qualify-source-preparation.py \
  --uv target/debug/uv \
  --python /absolute/path/to/python3.11 \
  --fd-observer .cache/observe-source-fds \
  --output source-preparation.json
```

## Git archives

`qualify-git-archives.py` compares two `uv` binaries using a real local Git repository containing
two immutable source archives. It reuses the source-preparation fixture's standard-library PEP 517
backend, records actual metadata and wheel-build events, and checks the installed module and build
settings. The fixture gates a running backend, so source-cache lock contention and independent
builds are observed directly instead of inferred from total command duration. The harness requires
Git, Python 3.11 or newer, and a POSIX system.

```shell
python3 scripts/benchmark/qualify-git-archives.py \
  --base /absolute/path/to/old-uv \
  --candidate /absolute/path/to/new-uv \
  --python /absolute/path/to/python3.11 \
  --candidate-policy recovery \
  --output git-archives.json
```

The default `record` policy for the base records duplicate builds and missing-source failures. The
`serialized` policy requires same-reference builds to coalesce and metadata requests and different
build settings to use the shared source lock. The `recovery` policy additionally requires an offline
installation to restore a displaced source directory while its cached revision hashes remain. Every
policy checks that distinct Git references can build independently, interrupted work can be
recovered, and completed caches can be reused offline. Mixed-version scenarios exercise both
writer/reader directions and concurrent publication; an older binary that does not participate in
the source lock may still perform a duplicate build.

Use `--scenario` to select an individual contract. The JSON receipt includes binary digests, Git
commits, archive digests, commands, exit status, and backend events. An interrupted backend has no
matching end event, so its event-derived unfinished count is evidence of interruption, not a count
of processes still alive after the harness stops its process group.

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
