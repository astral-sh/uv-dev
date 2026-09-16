# Linux nextest cache

These composite actions separate Cargo downloads from the compiled workspace cache for the Linux
`fast-build-nightly` nextest workload. The compiled cache has an exact source-commit key and a
fallback restricted to the same dependency, compiler, configuration, and workload identity. A
source-only change can therefore consume a compatible older entry and publish its own entry after
the complete workload succeeds.

`restore` and `save` use the official, separately pinned `actions/cache/restore` and
`actions/cache/save` actions. They do not copy native cache-service archives or import entries from
another repository. Adding these actions does not change any CI planner decision or existing cache
caller.

## Supported workload

The first contract is deliberately specific:

- Ubuntu 24.04, `x86_64-unknown-linux-gnu`, the source's numeric `rust-toolchain.toml` pin,
  Rustup-managed `cargo` and `rustc`, `cargo-nextest` 0.9.143, and `mold` as the selected `ld`.
- A clean, full source checkout inside `GITHUB_WORKSPACE`, with the expected repository and full
  commit supplied explicitly. The manifest records the actual commit, tree, checkout path, action
  source, and selected executable paths and hashes; `GITHUB_SHA` is recorded separately.
- `CARGO_INCREMENTAL=0`, `RUSTC_BOOTSTRAP=1`, `UV_LOCKED=1`, the default Cargo home, and
  `CARGO_TARGET_DIR=$GITHUB_WORKSPACE/target`. An explicit Cargo target, custom build directory,
  compiler wrapper, or custom target linker is outside this layout.
- The `fast-build-nightly` profile, checksum freshness, and the same features and `ci-linux` nextest
  profile as the Linux test job. The manifest's `workload.cargo_arguments` is the exact command
  suffix.

The target archive includes `target/.rustc_info.json` and the selected profile, including useful
workspace executables. It excludes incrementals and `.cargo-lock`. Downloads include Cargo's
registry/index/source and Git checkout stores, not credentials or installed tool binaries. Relevant
environment values and Cargo configuration contents contribute digests; the manifest does not dump
the environment or configuration contents.

## Caller contract

Call `restore` after installing the pinned Rust toolchain, nextest, and native build tools. Pass the
actual source checkout and its expected full commit. Use the returned absolute `cargo` invocation
path and `cargo-target-dir` for the workload. Invocation paths retain the `cargo` basename required
by Rustup; resolved executable paths are used only for identity checks.

The caller supplies `save-if` using its existing cache-publication policy. A writable cache miss or
fallback needs a freshness marker before the build and `CARGO_UNSTABLE_MTIME_ON_USE=true` during the
workload. After a successful complete workload, run `scripts/prune_cargo_workspace_cache.py` with
the selected profile directory and that marker, then pass `manifest` and `manifest-sha256` to
`save`. The save action rechecks the source, selected tools, paths, configuration, original
permission, and restore observations. It never turns a read-only restore into a writable one.

The normalized `cache-hit` output reports an exact compiled-target hit. `cache-matched-key` reports
the compatible entry actually restored, or an empty string. Downloads have their own exact-hit
output and key. A save step's successful outcome is not confirmation that the cache service
committed a new entry; a fresh consumer must restore the requested exact key before publication is
reported as confirmed.

An opt-in acceptance caller can set `key-namespace` to isolate its downloads and compiled entries.
The default empty namespace uses the ordinary version-1 keys. A nonempty namespace is a public,
lowercase ASCII identifier of at most 64 characters; its hash occupies a fixed-width key segment,
and neither restore family falls back to unnamespaced entries. The recorded namespace cannot be
overridden by `save` and does not grant permission to publish a cache.

Cargo's own freshness checks remain authoritative. Recording a different checkout path does not make
compiled artifacts hermetic or promise that Cargo can reuse them after relocation. The helper
detects ordinary source/tool changes at its observation boundaries; it cannot close races with an
uncooperative process that replaces and restores filesystem entries between those observations.

Run the source-only contract tests with:

```console
uv --no-config run --locked --python 3.12 --script scripts/tests/test_ci_rust_cache.py
```

## Opt-in full-operation acceptance

`linux-runner-acceptance.yml` is a manual-only caller in `astral-sh/uv-dev`. It uses the fixed
`f36ab068f6d834c7e13d5b7b10c7aca80d04cd65` and `65d6ba4fe450b7df59d8a1cda83083a02865ccd0` source
pair. Both contain the same native-auth test isolation; their only difference is an integration-test
source change. The controller and test source are separate clean checkouts, and the selected
source's toolchain, Linux test filesystems, Python versions, and nextest configuration remain part
of the recorded contract.

The `source` stage runs six sequential fresh jobs: downloads seeding, a cold baseline target, a
baseline exact hit, a candidate fallback from the baseline, a candidate exact hit, and that same
candidate at a different checkout path. Only the seed and the two complete source-cache producers
may save. Their following fresh consumers must observe the requested exact keys. The namespace is
derived from the workflow run and attempt, so this sequence cannot restore an ordinary CI entry.

The optional `source-and-malformed-cache` stage adds a fixture producer and fresh consumer in a
separate namespace. The producer restores the valid candidate entry read-only, records its payload
and original `.rustc_info.json`, and replaces only that generated metadata file. The consumer must
observe the isolated exact hit and finish the normal nextest workload with valid Cargo metadata. The
fixture is never represented as a successfully built production cache. An unsuccessful composite
restore remains an adverse result; an identity or setup failure is not a cache miss.

Each case has a 40-minute command deadline inside a 45-minute job on the normal 16-core Linux test
runner. Commands use the source-pinned process-group cleanup implementation from
`f1c904fb0930efeaf82232ea1884c41a18b93dd8`, with the Git blob and file digest checked before loading
it. The original failure, timeout, or signal remains authoritative while bounded cleanup confirms
that the owned group is absent. A successful leader that leaves descendants does not complete the
case.

The nextest command adds only `--cargo-message-format=json` to the manifest's workload arguments.
Evidence separates enclosing command time, Cargo's reported build interval, nextest's reported test
interval, actual `compiler-artifact` freshness records, raw configuration fingerprints, and a newly
written JUnit report. An artifact record is not a package count or a measure of equal compiler or
native-build work. Nextest's listed-binary count is retained as reported; listed binaries with no
reported tests need not have a JUnit suite. The aggregate downloads the exact returned artifact IDs,
requires successful digest-checked downloads, rehashes the case files, and rederives completion from
their source, command, cache, and test evidence. Partial or adverse runs remain visible.

This caller does not change production cache policy, grant cross-repository access, or establish a
speedup. A dependency-changing source pair, other targets, and production adoption require their own
qualification.

Run the source-only acceptance tests with:

```console
uv --no-config run --locked --python 3.12 --script scripts/tests/test_ci_rust_cache_acceptance.py
```

To include short real POSIX process controls, pass `--process-owner-repository` pointing to a local
Git repository containing the pinned cleanup commit. `--retain-directory` retains their command
records and output in a new directory. Neither option runs Cargo or accesses the cache service.
