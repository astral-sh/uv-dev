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
