# Resolver fork clone measurement

Build uv, then run:

```sh
python scripts/research/fork_clone.py --uv "$PWD/target/debug/uv" --work-dir "$PWD/work" --forks 8 --depth 32
python scripts/research/fork_clone.py --uv "$PWD/target/debug/uv" --work-dir "$PWD/work" --forks 8 --depth 32 --fork-kind requires-python
```

The fixture resolves a shared dependency chain before splitting on disjoint `sys_platform` markers.
Increasing `--depth` grows shared state; increasing `--forks` grows the number of branches. All
metadata is held in deterministic local wheels. The command disables indexes, network access,
builds, and persistent caches, and emits JSON with a fixture fingerprint, per-clone durations,
split kinds, and wall time.

`--fork-kind requires-python` uses release-specific Python 3.12 patch-version floors instead of
platform markers. Eight releases create eight Python regions and seven version-split clones; eight
platform markers also leave an unconstrained remainder and create eight dependency-split clones. The
JSON records which split site produced each event. `--forks` must be at least two. The driver fixes
`RUST_LOG=uv_resolver=debug`, independent of inherited logging filters.

Each measured clone separates the PubGrub state from the remaining uv fields. `pubgrub_us` and
`application_us` measure those two copies; `clone_us` includes timing overhead and rounds
separately. All fields are copied once. Normal logging uses the derived clone directly. Compare
component costs before deciding whether solver-specific sharing addresses the larger cost.

The instrumentation covers the two split sites in `ResolverState::forks_to_fork_states` and
`ResolverState::version_forks_to_fork_states`. It is active only at debug log level. Debug logging
and a development build affect timings; these results are a measurement scaffold, not a production
performance claim. The synthetic platform labels deliberately exercise marker partitioning rather
than enumerate supported operating systems.

[Ultimately Incremental SAT](https://ofers.dds.technion.ac.il/publications/sat14.pdf) motivates
studying retained solver state. Sharing learned incompatibilities between uv forks additionally
requires proving their environment and metadata assumptions. This experiment measures cloning before
attempting such sharing and does not alter solver state ownership or reuse.
