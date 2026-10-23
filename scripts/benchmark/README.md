# benchmark

Benchmarking scripts for uv and other package management tools.

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

## Preparation concurrency

`preparation-concurrency.py` compares download and install concurrency limits against an immutable
loopback index of generated wheels. Wheel sizes vary, and the index advertises their sizes and
metadata hashes. Each sample installs the same hashed requirements into a new environment, first
with a cold uv cache and then with the populated cache. Environment creation and installed-payload
verification are outside the timed interval. Configuration order is shuffled with a recorded seed.
The client bypasses environment proxies for loopback addresses, and the server's listen backlog is
sized to the corpus and recorded in the results.

```shell
python scripts/benchmark/preparation-concurrency.py \
  --uv /absolute/path/to/uv \
  --output target/preparation-concurrency \
  --downloads 1 4 16 --installs 1 4 16 --runs 10
```

Repeat `--uv` to compare binaries. Use a new output directory for each invocation. `--latency-ms`
adds a fixed delay before wheel responses; `--chunk-delay-ms` adds a delay per 64 KiB of wheel data.
These model a controlled server, not a particular external network. The workload contains wheels
only; it does not measure source builds. Cold refers to uv's cache, not the OS page cache.

`--compression deflated` enables ZIP deflate, while `--payload repeated` generates compressible
payloads from a repeated seeded block. The defaults are stored ZIP entries and random payloads.
Compare stored and deflated runs with the same payload kind, seed, package count, and maximum size
to hold installed bytes constant. With deflate enabled, compare random and repeated payloads at the
same expanded size to vary transfer volume. Each output records both wheel bytes and expanded bytes;
a smaller download does not imply less extraction work. These controls change the workload and do
not measure extraction time directly.

`results.json` records binary versions and digests, every sample, p50/p95 latency, request
intervals, download bytes, and peak simultaneous wheel requests. The corpus manifest and command
logs are retained. Sample failures retain completed samples and report the error with binary
identities. Virtual environments and caches are removed after successful samples. Request intervals
show server-side overlap; they do not identify time spent in extraction, installation, or scheduler
queues. Use profiles or tracing to attribute changes before modifying scheduling policy.

This experiment follows the end-to-end measurement motivation of
[Coz](https://arxiv.org/abs/1608.03676) and the measurement-variability concerns of
[Stabilizer](https://plasma.cs.umass.edu/emery/stabilizer.html). It does not implement their causal
profiling or layout-randomization techniques. Use optimized binaries and a quiet host for
performance comparisons; development builds are sufficient to exercise the harness.
