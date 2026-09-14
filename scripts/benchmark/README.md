# benchmark

Benchmarking scripts for uv and other package management tools.

The `uv-bench` CodSpeed workloads use immutable package artifacts and ecosystem lockfiles listed in
`fixtures.json`. From the repository root, run `python3 scripts/benchmark/prepare-fixtures.py` to
download them into `.cache/bench-fixtures`. The script verifies every fixture against its recorded
SHA-256, including files already present. CI prepares these inputs before timing and includes them
in the walltime runner artifact, so the measured workloads do not download fixtures.

`walltime-shards.py` assigns every built walltime suite to exactly one of up to eight independent
runs. The saved plan records the tracked source, compiler and CodSpeed versions, and each built
benchmark's SHA-256. Each runner checks the complete suite inventory and its selected binaries
before execution, so missing, stale, or mixed build artifacts cannot silently reduce coverage. Each
shard retains its own CodSpeed profile for main-branch baseline imports.

The `run` subcommand executes the selected suites in plan order and writes `result.json` to a new
directory selected with `--output`. Its producer and consumer tool identities are separate, and each
suite records its command, artifact, process ID, start, exit, and timeout state. The consumer
freezes the child environment and invokes its observed absolute `cargo` path. It records the
resolved target and hash separately so Rustup's multicall dispatch keeps the `cargo` basename, and
rechecks the tracked source, both executable identities, suite inventory, and selected binary before
each launch. These checks detect changed inputs; they do not lock the filesystem against an
uncooperative replacement after verification. Use `--timeout-seconds` for a relative limit or
`--deadline` for a Unix timestamp. A failure stops the shard, leaving unstarted suites pending; only
a successful exit from every suite completes the run. On POSIX systems, cleanup targets the child's
own process group after every exit, including surviving descendants. Group probes and termination
signals retain their syscall results; only `ESRCH` confirms that the group is absent. A denied probe
remains unknown until a later probe or the bounded cleanup deadline. Repeated cancellation cannot
interrupt cleanup or replace the first signal. An external hard kill can leave the last record
marked as running, which is partial evidence rather than a completed result. The recorded elapsed
times describe process liveness, not benchmark performance.

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
