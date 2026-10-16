# Resolver fork clone measurement

Build uv, then run:

```sh
python scripts/research/fork_clone.py --uv "$PWD/target/debug/uv" --work-dir "$PWD/work" --forks 8 --depth 32
```

The fixture resolves a shared dependency chain before splitting on disjoint `sys_platform` markers.
Increasing `--depth` grows shared state; increasing `--forks` grows the number of branches. All
metadata is held in deterministic local wheels. The command disables indexes, network access,
builds, and persistent caches, and emits JSON with a fixture fingerprint, per-clone durations,
tracked-package counts, incompatibility-reference counts, and wall time.

The instrumentation covers the two split sites in `ResolverState::forks_to_fork_states` and
`ResolverState::version_forks_to_fork_states`. It is active only at debug log level. Reference
counts can include the same incompatibility more than once and are not allocation or byte counts.
Debug logging and a development build affect timings; these results are a measurement scaffold, not
a production performance claim. The synthetic platform labels deliberately exercise marker
partitioning rather than enumerate supported operating systems.

[Ultimately Incremental SAT](https://ofers.dds.technion.ac.il/publications/sat14.pdf) motivates
studying retained solver state. Sharing learned incompatibilities between uv forks additionally
requires proving their environment and metadata assumptions. This experiment measures cloning before
attempting such sharing and does not alter solver state ownership or reuse.
