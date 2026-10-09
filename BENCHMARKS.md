# Benchmarks

The package-management benchmarks below were computed on macOS using Python 3.12.4 (for non-uv
tools), and come with a few important caveats:

- Benchmark performance may vary dramatically across different operating systems and filesystems. In
  particular, uv uses different installation strategies based on the underlying filesystem's
  capabilities. (For example, uv uses reflinking on macOS, and hardlinking on Linux.)
- Benchmark performance may vary dramatically depending on the set of packages being installed. For
  example, a resolution that requires building a single intensive source distribution may appear
  very similar across tools, since the bottleneck is tool-agnostic.

This document benchmarks against Trio's `docs-requirements.in`, as a representative example of a
real-world project.

In each case, a smaller bar (i.e., lower) is better.

## Warm Installation

Benchmarking package installation (e.g., `uv sync`) with a warm cache. This is equivalent to
removing and recreating a virtual environment, and then populating it with dependencies that you've
installed previously on the same machine.

![install-warm](https://github.com/user-attachments/assets/84118aaa-d030-4e29-8f1e-9483091ceca3)

## Cold Installation

Benchmarking package installation (e.g., `uv sync`) with a cold cache. This is equivalent to running
`uv sync` on a new machine or in CI (assuming that the package manager cache is not shared across
runs).

![install-cold](https://github.com/user-attachments/assets/e7f5b203-7e84-452b-8c56-1ff6531c9898)

## Warm Resolution

Benchmarking dependency resolution (e.g., `uv lock`) with a warm cache, but no existing lockfile.
This is equivalent to blowing away an existing `requirements.txt` file to regenerate it from a
`requirements.in` file.

![resolve-warm](https://github.com/user-attachments/assets/e1637a08-8b27-4077-8138-b3849e53eb04)

## Cold Resolution

Benchmarking dependency resolution (e.g., `uv lock`) with a cold cache. This is equivalent to
running `uv lock` on a new machine or in CI (assuming that the package manager cache is not shared
across runs).

![resolve-cold](https://github.com/user-attachments/assets/b578c264-c209-45ab-b4c3-54073d871e86)

## Reproduction

All benchmarks were generated using the `scripts/benchmark` package, which wraps
[`hyperfine`](https://github.com/sharkdp/hyperfine) to facilitate benchmarking uv against a variety
of other tools.

The benchmark script itself has a several requirements:

- A local uv release build (`cargo build --release`).
- An installation of the production `uv` binary in your path.
- The [`hyperfine`](https://github.com/sharkdp/hyperfine) command-line tool installed on your
  system.

To benchmark resolution against pip-compile, Poetry, and PDM:

```shell
uv run resolver \
    --uv-project \
    --poetry \
    --pdm \
    --pip-compile \
    --benchmark resolve-warm --benchmark resolve-cold \
    --json \
    ../requirements/trio.in
```

To benchmark installation against pip-sync, Poetry, and PDM:

```shell
uv run resolver \
    --uv-project \
    --poetry \
    --pdm \
    --pip-sync \
    --benchmark install-warm --benchmark install-cold \
    --json \
    ../requirements/compiled/trio.txt
```

Both commands should be run from the `scripts/benchmark` directory.

After running the benchmark script, you can generate the corresponding graph via:

```shell
cargo run -p uv-dev --all-features render-benchmarks resolve-warm.json --title "Warm Resolution"
cargo run -p uv-dev --all-features render-benchmarks resolve-cold.json --title "Cold Resolution"
cargo run -p uv-dev --all-features render-benchmarks install-warm.json --title "Warm Installation"
cargo run -p uv-dev --all-features render-benchmarks install-cold.json --title "Cold Installation"
```

You need to install the [Roboto Font](https://fonts.google.com/specimen/Roboto) if the labels are
missing in the generated graph.

## Acknowledgements

The inclusion of this `BENCHMARKS.md` file was inspired by the excellent benchmarking documentation
in
[Orogene](https://github.com/orogene/orogene/blob/472e481b4fc6e97c2b57e69240bf8fe995dfab83/BENCHMARKS.md).

## Troubleshooting

### Flaky benchmarks

If you're seeing high variance when running the cold benchmarks, then it's likely that you're
running into throttling or DDoS prevention from your ISP. In that case, ISPs forcefully terminate
TCP connections with a TCP reset. We believe this is due to the benchmarks making the exact same
requests in a very short time (especially true for `uv`). A possible workaround is to connect to VPN
to bypass your ISPs filtering mechanism.

## Cold marker parsing

Marker parsing uses a process-global interner. Repeating a marker in one process reuses cached
operations and can hide the cost of constructing intermediate decision diagrams. The `marker_parse`
example reads one marker from standard input and reports only the time spent in
`MarkerTree::from_str`, excluding input reading and process startup.

Build the same example in each checkout with identical settings, copying the example into older
checkouts when necessary:

```shell
cargo build --release --locked -p uv-pep508 --example marker_parse
```

Keep a copy of each `target/release/examples/marker_parse` binary, then compare them:

```shell
python3 scripts/benchmark/marker_parsing.py /path/to/base-marker-parse /path/to/head-marker-parse \
    --samples 31 > marker-parsing.json
```

The driver starts a fresh process for every measurement, alternates revision order, and reports
individual samples, medians, and interquartile ranges. On Linux, `--cpu` can pin both binaries to
the same CPU. The long conjunctions exclude distinct `os_name` values; the long disjunctions include
those values, so each term contributes to the resulting tree.

A Linux x86-64 comparison with Rust 1.99.0, 31 samples per revision, and one CPU compared the parser
before balancing (`3b3a7b5ed3ea`) with `2334517e699f`. Only `crates/uv-pep508/src/marker/parse.rs`
differed between the builds; both used the latter revision's `Cargo.lock` and the same example.
Median parse times were:

| Input           | Before (ms) | Balanced (ms) | Balanced / before |
| --------------- | ----------: | ------------: | ----------------: |
| `short-version` |     0.02038 |       0.02070 |             1.016 |
| `short-and`     |     0.02712 |       0.02712 |             1.000 |
| `short-or`      |     0.96356 |       0.97579 |             1.013 |
| `and-32`        |     1.18433 |       1.05588 |             0.892 |
| `and-256`       |    12.18408 |       4.02127 |             0.330 |
| `and-1024`      |   163.00854 |      36.37917 |             0.223 |
| `or-32`         |     1.07474 |       0.96834 |             0.901 |
| `or-256`        |    11.79689 |       3.95835 |             0.336 |
| `or-1024`       |   162.30928 |      36.33062 |             0.224 |

The short-marker interquartile ranges overlap. In this comparison, balancing reduces the cost of
256-term chains by about three times and 1,024-term chains by about four and a half times.
