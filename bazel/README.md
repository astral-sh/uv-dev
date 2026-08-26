# Bazel and BuildBuddy experiment

This is an isolated experiment for development builds in `uv-dev`. Cargo manifests, `Cargo.lock`,
existing CI, and release workflows remain unchanged. Bazel 9.0.0 and Rust 1.98.0 are pinned; the C
toolchain and macOS SDK are downloaded by Bazel.

The recorded Linux build took **191.706 seconds to seed the cache** and **28.388 seconds to replay
from a fresh output base**, with all 1,712 remote actions cached. This measures reuse of an
unchanged build, with dependency downloads already warm. It does not establish typical CI time or a
speedup over Cargo.

## Reproduce the remote experiment

### Prerequisites

- Bazelisk available as `bazel`, or Bazel **9.0.0**. The runner honors `.bazelversion` when using
  Bazelisk; `--bazel bazelisk` selects a differently named executable.
- `uv` and an installed Python **3.11 or newer**. The Python helpers have no third-party
  dependencies and run offline.
- Network access to download the pinned Rust, LLVM, native-library, and Bazel dependencies. macOS
  also needs system `make` and access to the pinned Apple SDK download.
- Your own BuildBuddy credential, authorized for uv and the chosen tenant, with remote execution
  access and **Action Cache write permission**. Uploading content alone is insufficient. Ask the
  tenant administrator for access; do not reuse someone else's key.

The original client was macOS ARM64; compilation and tests ran on Linux x86-64 workers. Linux x86-64
and Intel macOS clients are configured but have not been verified. Windows and Linux ARM64 clients
are outside this prototype's tested scope.

### Checkout and run

```sh
git clone --single-branch --branch zsol/uv-bazel-experiment \
  https://github.com/astral-sh/uv-dev.git uv-bazel-experiment
cd uv-bazel-experiment
git rev-parse HEAD
```

Record that commit with your results. Use the exact commit supplied with the experiment when
comparing runs; the branch name can move. The original uv base was
`a3343a269d6b5fe3289128d1030235bc5f905c0b`, before this branch's Bazel additions.

Save the key using your approved secret-management workflow to a regular file **outside the
checkout**, owned by you and readable only by you (`chmod 600 /absolute/path/to/buildbuddy.key`).
Never paste a real key into a command, chat, log, or committed file. Then inspect and run:

```sh
uv run --offline scripts/bazel/reproduce.py \
  --host remote.buildbuddy.io --key-file /absolute/path/to/buildbuddy.key --dry-run

uv run --offline scripts/bazel/reproduce.py \
  --host remote.buildbuddy.io --key-file /absolute/path/to/buildbuddy.key
```

Use `--host openai.buildbuddy.io` instead for credentials authorized for that tenant. The host must
match the key's tenant; the helper rejects other hosts, insecure connections, and unsafe key-file
permissions. `--dry-run` does not read the key, create files, or contact BuildBuddy.

The runner requires a clean checkout and:

1. Prefetches dependencies and toolchains, recording this setup time separately.
2. Builds `//:uv` into a new output base to seed a fresh remote Action Cache namespace.
3. Builds the same target into a second new output base, with the local disk cache disabled.
4. Compares the downloaded binaries' SHA-256 hashes and runs all 67 PEP 440 tests on a remote worker
   with cached test results disabled.

It succeeds only when the seed has remote executions and no remote cache hits, the replay has
matching cache hits and no remote executions, the binaries match, and the 67 tests pass uncached.
Action counts and elapsed times can differ across client platforms; they are not fixed assertions.

Each run gets a unique namespace under `uv-dev-experiment/`. To coordinate a particular run, provide
a new `--instance uv-dev-experiment/your-unique-run`. Reusing a populated namespace is not a cold
seed, and the runner reports that condition. A namespace separates cache names; it does not enforce
permissions, isolate worker capacity, or guarantee a cold content store or worker.

### Results and privacy

The runner prints the location of `summary.json`, containing only selected measurements: source
revision, client platform, conditions, Bazel elapsed time, process wall time, execution/cache
counts, binary hashes, and test outcomes. Use that file for comparisons. Review even a summary
before sharing it publicly, especially if you chose an identifying namespace.

Raw console logs and Build Event Protocol (BEP) files stay in a private directory outside the
checkout. They can contain local paths, command lines, and build metadata; **do not commit them or
upload them without review**. `--run-dir /path/to/new-directory` selects a durable location; it must
not already exist or be inside the checkout. No credentials are copied into that directory.

The runner ignores system, home, and private workspace Bazel configuration, retains the pinned
repository settings, and overrides caches and endpoints explicitly. Build-event uploads are
disabled; remote execution still sends build inputs to the explicitly selected service and writes to
its cache. The original measurements included private build-event reporting, so reporting overhead
can differ. No service credentials, roles, global settings, or other repository settings are
changed.

If replay executes remotely again, check Action Cache write permission first. The original read-only
credential produced successful remote builds but zero replay cache hits. An error with
`--lockfile_mode=error` means the pinned graph needs investigation; do not silently regenerate the
lockfile and present the result as the same experiment.

## Local build

Install Bazelisk, and make sure `make` is available on macOS. Then run from the repository root:

```sh
bazel build //:uv
bazel run //:uv -- --version
bazel test //:pep440-tests
```

After a native build, reproduce the offline CLI smoke with:

```sh
uv run --offline scripts/bazel/smoke.py --binary bazel-bin/crates/uv/uv
```

It checks version/help, creates a temporary virtualenv, resolves and installs two generated local
wheels with a transitive dependency, and verifies imports. It disables downloads and cleans up its
fixtures. Run this against a binary for your machine, not the remote Linux binary on macOS.

These repository defaults do not select a remote endpoint. Existing system, home, or `user.bazelrc`
settings can override them. Local verification used the startup flags `--nohome_rc --nosystem_rc`
and did not select a remote configuration.

Configurations are provided for macOS ARM64, macOS x86-64, and Linux x86-64; see the verification
section below for platforms actually tested.

The first-party BUILD files are generated from Cargo package metadata. After changing a manifest,
regenerate them with:

```sh
uv run scripts/bazel/generate.py
uv run scripts/bazel/generate.py --check
```

`rules_rs` resolves dependency edges and features directly from Cargo manifests and the lockfile.
Native libraries, including AWS-LC and jemalloc, retain their locked sources and Cargo build
scripts. GNU Make is a declared build tool on Linux; macOS keeps its existing system make. LLVM's
`nm` is declared on both platforms. The Bazel-only patches preserve jemalloc's linker flags, honor
the declared make executable, and supply header paths to GNU Make's preprocessor checks. AWS-LC's
CPU-jitter entropy implementation is explicitly enabled; missing SDK headers must fail the build
instead of silently disabling it.

## BuildBuddy

Remote access is optional and requires an account and credentials authorized for this repository.
The default endpoint is `remote.buildbuddy.io`. Nothing here creates keys or changes roles. Keep
credentials in an existing Bazel credential helper or the ignored `user.bazelrc`; never commit them.
For a different supported tenant, use the reproduction runner above. The manual configuration below
is scoped to the generic endpoint; changing only the credential does not select a new tenant.

For a key already supplied securely through `BUILDBUDDY_API_KEY`, the optional helper passes it
directly to Bazel without putting it on the command line:

```sh
# Compile locally and use the remote action cache.
bazel build --config=buildbuddy --config=buildbuddy-env //:uv

# Compile and test Linux x86-64 on remote workers.
bazel build --config=remote-linux --config=buildbuddy-env //:uv
bazel test --config=remote-linux --config=buildbuddy-env //:pep440-tests
```

Omit `--config=buildbuddy-env` when using an existing authentication mechanism. The helper requires
`uv` and an installed Python 3.11 or newer; it runs offline. Do not invoke the helper directly with
a real key: its stdout contains credentials. Its tests use fake keys and make no network requests:

```sh
uv run --offline scripts/bazel/test_buildbuddy_credentials.py
uv run --offline scripts/bazel/test_reproduce.py
```

The `uv-dev-experiment` remote instance separates cache names but is not an access control boundary.
A remote cache read permission alone is insufficient to upload locally computed action results.
Check account permissions before measuring cache population or remote execution. Do not publish
invocation data without approval.

## Scope and measurement

- The initial test target runs the existing `uv-pep440` unit tests. The full uv integration suite,
  Python fixtures, and platform-specific tests are not wired.
- Remote execution currently targets Linux x86-64 with Linux workers. macOS builds run locally.
- `rules_rs` 0.0.96 unifies workspace features, including those enabled by other members and their
  development dependencies. This is not the exact feature graph of `cargo build -p uv`.
- Rust compilation in Bazel's default `fastbuild` mode has no optimization or debug information;
  native build scripts can select their own compiler flags. This does not reproduce uv's Cargo CI
  profiles, including their panic and optimization settings. Do not compare these timings as
  equivalent builds without aligning them.
- The sandbox does not expose `.git` to `uv-cli`'s build script. Package versions are preserved, but
  the executable has no Git revision stamp.
- To verify remote cache reuse, use a fresh Bazel output base with the local disk cache disabled
  (`--disk_cache=`), then inspect remote action-cache hits. A second build in the same output base
  only demonstrates local incremental reuse.

## Verification

Verified on 2026-08-26 against uv base `a3343a269d6b`:

| Check               | Result                                                                                          |
| ------------------- | ----------------------------------------------------------------------------------------------- |
| macOS ARM64 binary  | Builds; `bazel run //:uv -- --version` reports `uv 0.12.6 (aarch64-apple-darwin)`               |
| PEP 440 tests       | 67 passed locally and on a remote Linux worker, with `--nocache_test_results`                   |
| Offline CLI smoke   | Version/help, virtualenv creation, transitive resolution, wheel installation, and import passed |
| AWS-LC archive      | Contains the six jitter-entropy implementation objects                                          |
| Linux x86-64        | Remote binary build passed; repeated build produced an identical binary                         |
| Remote cache replay | Fresh output base, disk cache disabled: 1,712 remote-cache hits, zero remote executions         |
| Python helpers      | Generation check passed for 70 crates; credential-helper checks passed                          |
| Formatting and lint | Buildifier, Ruff, and Prettier passed                                                           |

Remote execution and action-cache reuse were verified with an authorized writer credential in the
`uv-dev-experiment` namespace. A prior check without action-cache write permission rebuilt all
remote actions. No Cargo-versus-Bazel performance comparison has been run.

[Recorded measurements](results/2026-08-26.json) preserve the original counts, elapsed times, and
binary hash without private invocation links or raw logs. Counts come from BEP
`buildMetrics.actionSummary.runnerCount` (including the `remote cache hit` runner), not the separate
`remoteCacheHits` scalar, which was absent. Elapsed time means BEP finish minus start; it excludes
some client startup overhead. The runner also records process wall time separately.

Service references: [BuildBuddy remote execution setup](https://www.buildbuddy.io/docs/rbe-setup/),
[BuildBuddy authentication](https://www.buildbuddy.io/docs/guide-auth/), and
[Bazel Build Event Protocol](https://bazel.build/remote/bep).
