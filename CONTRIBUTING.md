# Contributing

## Finding ways to help

We label issues that we think are a good opportunity for subsequent contributions as
[`help wanted`](https://github.com/astral-sh/uv/issues?q=is%3Aopen+is%3Aissue+label%3A%22help+wanted%22).
These require varying levels of experience with Rust and uv. Often, we want to accomplish these
tasks but do not have the resources to do so ourselves.

You don't need our permission to start on an issue we have labeled as appropriate for community
contribution as described above. However, it's a good idea to indicate that you are going to work on
an issue to avoid concurrent attempts to solve the same problem.

Please check in with us before starting work on an issue that has not been labeled as appropriate
for community contribution. We're happy to receive contributions for other issues, but it's
important to make sure we have consensus on the solution to the problem first.

Outside of issues with the labels above, issues labeled as
[`bug`](https://github.com/astral-sh/uv/issues?q=is%3Aopen+is%3Aissue+label%3A%22bug%22) are the
best candidates for contribution. In contrast, issues labeled with `needs-decision` or
`needs-design` are _not_ good candidates for contribution. Please do not open pull requests for
issues with these labels.

Please do not open pull requests for new features without prior discussion. While we appreciate
exploration of new features, we will almost always close these pull requests immediately. Adding a
new feature to uv creates a long-term maintenance burden and requires strong consensus from the uv
team before it is appropriate to begin work on an implementation.

## Use of AI

We **require all use of AI in contributions to follow our
[AI Policy](https://github.com/astral-sh/.github/blob/main/AI_POLICY.md)**.

If your contribution does not follow the policy, it will be closed.

## Setup

[Rust](https://rustup.rs/) (and a C compiler) are required to build uv.

On Ubuntu and other Debian-based distributions, you can install a C compiler with:

```shell
sudo apt install build-essential
```

On Fedora-based distributions, you can install a C compiler with:

```shell
sudo dnf install gcc
```

On Windows, [NASM](https://www.nasm.us/) is required for building the TLS backend (`aws-lc-sys`). If
it is not present, a prebuilt blob provided by `aws-lc-sys` will be used instead. WinGet can be used
to install NASM:

```shell
winget install NASM.NASM
```

After installation, add `C:\Program Files\NASM` to your `PATH`. While the prebuilt blob will not be
used when NASM is found, you can guarantee this behavior by setting `AWS_LC_SYS_PREBUILT_NASM=0`.

## Testing

For running tests, we recommend [nextest](https://nexte.st/).

To run a specific test by name:

```shell
cargo nextest run -E 'test(test_name)'
```

To run all tests and accept snapshot changes:

```shell
cargo insta test --accept --test-runner nextest
```

To update snapshots for a specific test:

```shell
cargo insta test --accept --test-runner nextest -- <test_name>
```

### Python

Testing uv requires multiple specific Python versions; they can be installed with:

```shell
cargo run python install
```

The storage directory can be configured with `UV_PYTHON_INSTALL_DIR`. (It must be an absolute path.)

### Resolver scenarios

Small dependency graphs in `test/scenarios` can be checked against an independent, exhaustive
resolver oracle:

```shell
cargo build --package uv --bin uv
cargo dev check-scenarios --uv target/debug/uv test/scenarios/backtracking/wrong-backtracking-basic.toml
```

Use `--python-version` and `--python-platform` (`linux`, `macos`, or `windows`) to select concrete
marker environments. Comma-separated values check their Cartesian product, with a fresh cache for
each fixed-environment projection. The checker verifies satisfiability and the exact reachable
dependency closure, not a particular preferred version. It uses a closed-world local index and
rejects unsupported policies, including pre-releases, yanked candidates, non-universal wheels, and
non-additive extras. `--max-states` bounds the exhaustive search and fails explicitly when a graph
is too large. The root's Python range is enforced in full, while dependency `Requires-Python`
follows uv's
[lower-bound-only policy](https://docs.astral.sh/uv/pip/compatibility/#requires-python-upper-bounds).
Empty dependency Python ranges are not modeled.

Pass `--lock` to check one universal project lock, its canonical round trip, and a frozen
requirements export in every selected environment. Without a whole-domain certificate, an
unsatisfiable lock is only confirmed when one selected environment provides an unsatisfiable
witness; successful samples cannot prove the entire marker universe is satisfiable.

```shell
cargo dev check-scenarios --uv target/debug/uv --lock --python-version 3.12,3.13,3.14 --python-platform linux,macos,windows test/scenarios/fork/basic.toml
```

Add `--lock-without-metadata` to exercise the metadata-free lockfile preview. The selected format is
used for the initial lock, both read-only round trips, frozen exports, and any diagnostic refresh.
Failure captures record the format and exhaustive-search bound alongside the command trace.

Explicit Packse `resolution` and `fork_strategy` settings are passed to the initial resolution,
read-only lock checks, and frozen exports. The oracle still checks dependency correctness instead of
prescribing the preferred solution. Explicit lock `environments` restrict the independent marker
domain and are retained in the generated project for all lock and export commands. Requested
projections outside that domain are rejected. Environment entries must be disjoint, intersect the
root Python range, and contain no extra or PEP 751 list markers. The marker witness proves
satisfiability over that restricted domain. Original-derivation `structured-v1` classification binds
the ordered initial environments and checks that the failed effective environment lies inside the
certified domain. Its bounded marker comparison rejects `in`, `contains`, extra, and list markers,
and comparisons that combine more than one of `os_name`, `sys_platform`, and `platform_system`.
Those three OS identities have native cross-variable exclusions that the capture consumer does not
yet model.

Add `--project-selections` for project optional dependencies and PEP 735 dependency groups. The
universal lock is checked against all roots together. Frozen exports disable default groups and
cover the base project, individual and combined extras, individual and combined groups, groups-only
selections, and all roots together. This is a bounded selection matrix, not every possible subset.
Group includes remain in the temporary project so uv's expansion is checked against the oracle's
independent expansion. Project self-references, `extra` markers on project or group requirements,
and conflict declarations are not modeled.

```shell
cargo dev check-scenarios --uv target/debug/uv --lock --project-selections --python-version 3.12,3.13 --python-platform linux,macos,windows test/scenarios/project/selection-projections.toml
```

For a reproducible set of generated graphs, provide a starting seed and an output directory:

```shell
cargo dev check-scenarios --uv target/debug/uv --seed 0 --cases 100 --output-dir generated-scenarios
```

Every input is saved as a complete Packse TOML file before uv runs. Re-run a saved file directly to
replay a failure; the checker refuses to overwrite a different input with the same name. Use
`--packages`, `--versions`, and `--max-states` to bound the graph and exhaustive search. Add
`--lock` to run the same generated graphs through the lockfile checks.

Add `--markers` for Python and platform conditions, per-version `Requires-Python`, and additive
extra markers. These graphs cover three Python minor lines starting with the first selected Python
version. Their saved expectation describes that first target, while each additional projection is
checked independently. The ordinary generator retains its one-minor-line replay format.

```shell
cargo dev check-scenarios --uv target/debug/uv --seed 0 --cases 100 --markers --python-version 3.12,3.13,3.14 --python-platform linux,macos,windows --output-dir generated-markers
```

Combine `--seed` with `--project-selections` to generate project graphs with two extras and two
dependency groups, including repeated group includes. These also cover three Python minor lines. The
saved expectation describes all project roots at the first target; it does not describe a particular
exported subset.

```shell
cargo dev check-scenarios --uv target/debug/uv --lock --project-selections --seed 0 --cases 100 --python-version 3.12,3.13,3.14 --python-platform linux,macos,windows --output-dir generated-projects
```

Add `--satisfiable` to construct project graphs around a compatible version assignment. An
independent certificate checks every possible root and selected-version dependency, without
discarding inactive markers, and requires the selected versions to cover the project's entire Python
range. The oracle also checks the assignment's exact reachable closure for every selected target and
project selection before uv runs. The assignment, whole-domain certificate, and checked targets are
saved in a neighboring `.witness.json` file. The assignment does not prescribe which versions uv
should prefer; alternative candidates retain their generated constraints, and uv's actual exports
are still checked by the exhaustive oracle.

```shell
cargo dev check-scenarios --uv target/debug/uv --lock --project-selections --satisfiable --seed 0 --cases 100 --python-version 3.12,3.13,3.14 --python-platform linux,macos,windows --output-dir satisfiable-projects
```

Use `--witness path/to/scenario.witness.json` with `--lock --project-selections` to re-certify a
saved assignment against one actual scenario file. The checker recomputes a marker-conditioned
whole-domain proof; stored certificate fields are not trusted. `--max-witness-work` bounds that
proof's requirement evaluations. `--satisfiable` uses the same witnessed check for each generated
graph. A witnessed lock uses a fresh cache and an online, closed-world index for its initial
resolution. A no-solution result is classified only when its raw derivation has a recognized
semantic form and every registry package claimed absent has no listed scenario candidates. Printed
version ranges are lossy, so narrower absence claims about packages that do have candidates remain
unclassified. Inventories containing local-version candidates are also unclassified because their
absence leaves can be omitted from the displayed derivation. Transport, build, metadata,
unsupported-policy, and exhausted proof-budget errors are not resolver counterexamples.

For witnessed checks and reductions, `--lock-evidence structured-v1` selects an internal
original-derivation capture instead of the default `printed-v1` grammar. This requires a uv binary
with the matching capture protocol. The checker binds one online lock command to its actual child
process and executable digest, checks the exact local index and generated project, and tests every
original absence range against the raw scenario versions using uv's native range representation. It
rejects authentication failures, unsupported availability or source policy, custom resolver claims,
and incomplete or mismatched captures. It never falls back to printed diagnostics or starts a
diagnostic second resolution. The whole-domain witness is still required; the captured derivation is
not a separate satisfiability certificate or a verified PubGrub proof. The reducer requires a saved
witness and re-certifies its assignment before checking each candidate deletion with the selected
evidence mode.

```shell
cargo dev check-scenarios --uv target/debug/uv --lock --project-selections --lock-evidence structured-v1 --witness scenario.witness.json scenario.toml
cargo dev minimize-scenario --uv target/debug/uv --lock --project-selections --lock-evidence structured-v1 --witness scenario.witness.json --output reduced.toml scenario.toml
```

When a generated check fails, the checker also saves the commands, output, and exact served
distributions in a neighboring `.failure` directory. Lock checks additionally retain the temporary
project and the lockfile after each command. If uv rejects its freshly written lockfile, the capture
also refreshes the temporary project, saves the lockfile diff, and checks the refreshed lock. Use
`--failure-dir` to choose a new directory when replaying an existing fixture. The capture includes
the uv binary's SHA-256 digest and the advertised distribution hashes; existing evidence directories
are never overwritten. Witnessed failures also retain the newly computed proof and recognized
derivation summary. An unsatisfiable universal lock without a sampled unsatisfiable environment or a
sufficient whole-domain proof is captured but remains unclassified.

Use the same binary and target to reduce a fixed-environment counterexample:

```shell
cargo dev minimize-scenario --uv target/debug/uv --output reduced.toml scenario.toml
```

For a universal lockfile counterexample, pass `--lock` and the same target matrix used by the
checker. Add `--project-selections` to reduce optional dependencies and dependency groups too. If
the failure used the metadata-free preview, also pass `--lock-without-metadata`; every candidate and
the final confirmation use the selected lockfile representation:

```shell
cargo dev minimize-scenario --uv target/debug/uv --lock --project-selections --python-version 3.12,3.13,3.14 --python-platform linux,macos,windows --output reduced.toml scenario.toml
```

For a witnessed project counterexample, also pass `--witness scenario.witness.json`. The reducer
retains the original fixed versions, removing assignment entries only when their packages are
deleted. Every candidate must pass a fresh whole-domain certificate before uv runs;
`--max-witness-work` bounds each proof. A rejected or budget-exhausted proof is not an
unsatisfiability claim. The final TOML is checked again and receives a neighboring `.witness.json`
with its restricted assignment and recomputed certificate. Interrupted witnessed reductions also
save independent witness files for the candidate and last reproducer. These files can be replayed
with `check-scenarios --witness` without trusting any stored certificate fields. Deletion minimality
is relative to that fixed assignment and the configured proof budget; the reducer does not search
for alternative witnesses.

The reducer removes root requirements, packages, versions, extras, and dependency edges while
retaining the original kind of semantic mismatch. Each candidate uses a fresh cache. Unrelated
command failures stop the reduction, and `--max-attempts` bounds the number of candidate checks. If
an unclassified candidate stops the reduction, a neighboring `.interrupted` directory retains that
raw candidate, the last reproducing input, and the failure. Re-run the candidate with
`check-scenarios` and a new `--failure-dir` to capture its commands and served distributions. A
deletion-minimal result only means that no supported single deletion retains the mismatch, not that
the graph is globally minimal. The output records the concrete Python and platform target and avoids
claiming a preferred solution or universal satisfiability from one environment.

To measure version duplication in an existing universal lockfile, use:

```shell
cargo dev score-lock path/to/uv.lock
```

This reads the lockfile without resolving or modifying it and prints a JSON inventory. Its
`excess_versions` score sums the number of versions beyond the first for each normalized package
name and exact serialized source. Distinct registries, Git revisions, and local source kinds are
kept separate. Unversioned records are reported separately. Compare scores only after checking the
locks' correctness and using the same project, index inputs, and resolver policy; a lower score is
not a proof of global optimality or equivalent marker coverage.

### Snapshot testing

uv uses [insta](https://insta.rs/) for snapshot testing. It's recommended (but not necessary) to use
`cargo-insta` for a better snapshot review experience. See the
[installation guide](https://insta.rs/docs/cli/) for more information.

In tests, you can use `uv_snapshot!` macro to simplify creating snapshots for uv commands. For
example:

```rust
#[test]
fn test_add() {
    let context = TestContext::new("3.12");
    uv_snapshot!(context.filters(), context.add().arg("requests"), @"");
}
```

To run and review a specific snapshot test:

```shell
cargo test --package <package> --test <test> -- <test_name> -- --exact
cargo insta review
```

A script is available to update the snapshots based on results in CI. This is useful for updating
snapshots without re-running the test suite and for updating platform-specific snapshots.

```shell
./scripts/apply-ci-snapshots.sh
```

### Git and Git LFS

A subset of uv tests require both [Git](https://git-scm.com) and [Git LFS](https://git-lfs.com/) to
execute properly.

These tests can be disabled by turning off either `git` or `git-lfs` uv features.

### Local testing

You can invoke your development version of uv with `cargo run -- <args>`. For example:

```shell
cargo run -- venv
cargo run -- pip install requests
```

## Formatting

```shell
# Rust
cargo fmt --all

# Python
uv run --only-group=check ruff format .

# Markdown, YAML, and other files (requires Node.js)
npx prettier@3.9.0 --write .
# or in Docker
docker run --rm -v .:/src/ -w /src/ node:alpine npx prettier@3.9.0 --write .
```

## Linting

Linting requires [shellcheck](https://github.com/koalaman/shellcheck) to be installed separately.
Validating `pyproject.toml` against the checked-in uv schema also requires
[jq](https://jqlang.org/).

```shell
# Rust
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

# Python
uv run --only-group=check ruff check .

# Python type checking
uv run --only-group=check ty check python/uv

# Python project metadata and uv schema
./scripts/validate-pyproject.sh

# Generated files
cargo dev generate-all --mode dry-run

# Shell scripts
shellcheck <script>

# Spell checking
uv run --only-group=check typos

# Unused Rust dependencies
uv run --only-group=check cargo-shear
```

### Compiling for Windows from Unix

To run clippy for a Windows target from Linux or macOS, you can use
[cargo-xwin](https://github.com/rust-cross/cargo-xwin):

```shell
# Install cargo-xwin
cargo install --locked cargo-xwin@0.21.4

# Add the Windows target
rustup target add x86_64-pc-windows-msvc

# Run clippy for Windows
cargo xwin clippy --workspace --all-targets --all-features --locked -- -D warnings
```

## Crate structure

Rust does not allow circular dependencies between crates. To visualize the crate hierarchy, install
[cargo-depgraph](https://github.com/jplatte/cargo-depgraph) and graphviz, then run:

```shell
cargo depgraph --dedup-transitive-deps --workspace-only | dot -Tpng > graph.png
```

## Running inside a Docker container

Source distributions can run arbitrary code on build and can make unwanted modifications to your
system
(["Someone's Been Messing With My Subnormals!" on Blogspot](https://moyix.blogspot.com/2022/09/someones-been-messing-with-my-subnormals.html),
["nvidia-pyindex" on PyPI](https://pypi.org/project/nvidia-pyindex/)), which can even occur when
just resolving requirements. To prevent this, there's a Docker container you can run commands in:

```console
$ docker build -t uv-builder -f crates/uv-dev/builder.dockerfile --load .
# Build for musl to avoid glibc errors, might not be required with your OS version
cargo build --target x86_64-unknown-linux-musl --profile profiling
docker run --rm -it -v $(pwd):/app uv-builder /app/target/x86_64-unknown-linux-musl/profiling/uv-dev resolve-many --cache-dir /app/cache-docker /app/scripts/popular_packages/pypi_10k_most_dependents.txt
```

We recommend using this container if you don't trust the dependency tree of the package(s) you are
trying to resolve or install.

## Profiling and Benchmarking

Please refer to Ruff's
[Profiling Guide](https://github.com/astral-sh/ruff/blob/main/CONTRIBUTING.md#profiling-projects),
it applies to uv, too.

We provide diverse sets of requirements for testing and benchmarking the resolver in
`test/requirements` and for the installer in `test/requirements/compiled`.

You can use `scripts/benchmark` to benchmark predefined workloads between uv versions and with other
tools, e.g., from the `scripts/benchmark` directory:

```shell
uv run resolver \
    --uv-pip \
    --poetry \
    --benchmark \
    resolve-cold \
    ../test/requirements/trio.in
```

### Analyzing concurrency

You can use [tracing-durations-export](https://github.com/konstin/tracing-durations-export) to
visualize parallel requests and find any spots where uv is CPU-bound. Example usage, with `uv` and
`uv-dev` respectively:

```shell
RUST_LOG=uv=info TRACING_DURATIONS_FILE=target/traces/jupyter.ndjson cargo run --features tracing-durations-export --profile profiling -- pip compile test/requirements/jupyter.in
```

```shell
RUST_LOG=uv=info TRACING_DURATIONS_FILE=target/traces/jupyter.ndjson cargo run --features tracing-durations-export --bin uv-dev --profile profiling -- resolve jupyter
```

### Trace-level logging

You can enable `trace` level logging using the `RUST_LOG` environment variable, i.e.

```shell
RUST_LOG=trace uv
```

## Documentation

To preview any changes to the documentation locally:

1. Install the [Rust toolchain](https://www.rust-lang.org/tools/install).

2. Install [Node](https://nodejs.org/en/download) - needed to run Prettier to format the docs

3. Run `cargo dev generate-all`, to update any auto-generated documentation.

4. Run the development server with:

   ```shell
   uv run --only-group docs mkdocs serve -f mkdocs.yml
   ```

The documentation should then be available locally at
[http://127.0.0.1:8000/uv/](http://127.0.0.1:8000/uv/).

Documentation is deployed automatically on release by publishing to the
[Astral documentation](https://github.com/astral-sh/docs) repository, which itself deploys via
Cloudflare Pages.

After making changes to the documentation, [format the markdown files](#formatting) using Prettier.

## Development code signing on macOS

Code signing can only be performed by Astral team members.

Code signing on macOS can improve developer experience when running tests, e.g., when running tests
that access the macOS keychain, a signed binary can be approved once but an unsigned binary will
need to be approved on each re-compile.

### Acquiring a development certificate

1. Generate a
   [request for the certificate](https://developer.apple.com/help/account/certificates/create-a-certificate-signing-request)
2. Create a certificate in the
   [Apple Developer portal](https://developer.apple.com/account/resources/certificates/list)
3. Download and install the certificate to your login keychain

   ```shell
   security import ~/Downloads/mac_development.cer -k ~/Library/Keychains/login.keychain-db
   ```

4. Identify your code signing identity

   ```shell
   security find-identity -v -p codesigning
   ```

5. If the above fails to find your identity, install the intermediate certificates

   ```shell
   curl -sLO "https://www.apple.com/certificateauthority/AppleWWDRCAG3.cer"
   security import AppleWWDRCAG3.cer -k ~/Library/Keychains/login.keychain-db
   rm AppleWWDRCAG3.cer
   ```

6. Set `UV_TEST_CODESIGN_IDENTITY`

   ```shell
   export UV_TEST_CODESIGN_IDENTITY="Mac Developer: Your Name (TEAM_ID)"
   ```

Note `UV_TEST_CODESIGN_IDENTITY` is only supported via `nextest`.

## Releases

Releases can only be performed by Astral team members.

Changelog entries and version bumps are automated. First, run:

```shell
./scripts/release.sh
```

If release preparation detects a new workspace crate, add it to
[`astral-sh/crates-policies`](https://github.com/astral-sh/crates-policies).

Then, editorialize the `CHANGELOG.md` file to ensure entries are consistently styled.

Then, open a pull request, e.g., `Bump version to ...`.

Binary builds will automatically be tested for the release.

After merging the pull request, run the
[release workflow](https://github.com/astral-sh/uv/actions/workflows/release.yml) with the version
tag. **Do not include a leading `v`**. The release will automatically be created on GitHub after
everything else publishes.
