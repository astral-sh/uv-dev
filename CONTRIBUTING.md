# Contributing

## Finding ways to help

The
[`help wanted`](https://github.com/astral-sh/uv/issues?q=is%3Aopen+is%3Aissue+label%3A%22help+wanted%22)
label identifies issues that welcome community contributions. These issues require different levels
of experience with Rust and uv. The uv team wants to complete these tasks but does not have enough
resources.

You do not need permission to work on an issue with this label. Consider stating that you plan to
work on the issue so that other contributors do not duplicate your work.

Contact the uv team before you work on an issue without a community contribution label. The team
welcomes other contributions but must first agree on a solution.

Among issues without this label, those labeled
[`bug`](https://github.com/astral-sh/uv/issues?q=is%3Aopen+is%3Aissue+label%3A%22bug%22) are the
best candidates for contributions. Issues labeled `needs-decision` or `needs-design` are _not_ good
candidates. Do not open pull requests for issues with these labels.

Discuss new features before you open a pull request. New features increase long-term maintenance, so
the uv team must agree on a feature before implementation begins. The team closes most feature pull
requests that have not been discussed.

## Use of AI

**All use of AI in contributions must follow the
[AI Policy](https://github.com/astral-sh/.github/blob/main/AI_POLICY.md)**.

The uv team closes contributions that do not follow this policy.

## Setup

Install [Rust](https://rustup.rs/) and a C compiler to build uv.

On Ubuntu and other Debian-based distributions, install a C compiler with:

```shell
sudo apt install build-essential
```

On Fedora-based distributions, install a C compiler with:

```shell
sudo dnf install gcc
```

On Windows, the TLS backend (`aws-lc-sys`) uses [NASM](https://www.nasm.us/) to build from source.
If NASM is not available, `aws-lc-sys` uses a prebuilt file instead. Install NASM with WinGet:

```shell
winget install NASM.NASM
```

After you install NASM, add `C:\Program Files\NASM` to your `PATH`. When NASM is available,
`aws-lc-sys` does not use the prebuilt file. Set `AWS_LC_SYS_PREBUILT_NASM=0` to require this
behavior.

## Testing

Run tests with [nextest](https://nexte.st/).

Run a specific test by name:

```shell
cargo nextest run -E 'test(test_name)'
```

Run all tests and accept snapshot changes:

```shell
cargo insta test --accept --test-runner nextest
```

Update the snapshots for a specific test:

```shell
cargo insta test --accept --test-runner nextest -- <test_name>
```

### Python

uv tests require several specific Python versions. Install them with:

```shell
cargo run python install
```

Set `UV_PYTHON_INSTALL_DIR` to an absolute path to configure the storage directory.

### Snapshot testing

uv uses [insta](https://insta.rs/) for snapshot testing. Use the optional `cargo-insta` tool to make
snapshot review easier. See the [installation guide](https://insta.rs/docs/cli/) for more
information.

Use the `uv_snapshot!` macro in tests to create snapshots for uv commands. For example:

```rust
#[test]
fn test_add() {
    let context = TestContext::new("3.12");
    uv_snapshot!(context.filters(), context.add().arg("requests"), @"");
}
```

Run and review a specific snapshot test:

```shell
cargo test --package <package> --test <test> -- <test_name> -- --exact
cargo insta review
```

Run the following script to update snapshots from CI results without running the test suite again.
The script also updates platform-specific snapshots.

```shell
./scripts/apply-ci-snapshots.sh
```

### Git and Git LFS

Some uv tests require both [Git](https://git-scm.com) and [Git LFS](https://git-lfs.com/).

To disable these tests, turn off either the `git` or `git-lfs` uv feature.

### Local testing

Run your development version of uv with `cargo run -- <args>`. For example:

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

Install [shellcheck](https://github.com/koalaman/shellcheck) separately before you run the linters.
Install [jq](https://jqlang.org/) to validate `pyproject.toml` against the checked-in uv schema.

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

Use [cargo-xwin](https://github.com/rust-cross/cargo-xwin) to run Clippy for a Windows target from
Linux or macOS:

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
[cargo-depgraph](https://github.com/jplatte/cargo-depgraph) and graphviz. Then run:

```shell
cargo depgraph --dedup-transitive-deps --workspace-only | dot -Tpng > graph.png
```

## Running inside a Docker container

Source distributions can run arbitrary code when you build them or resolve requirements. This code
can change your system. For examples, see
["Someone's Been Messing With My Subnormals!" on Blogspot](https://moyix.blogspot.com/2022/09/someones-been-messing-with-my-subnormals.html)
and ["nvidia-pyindex" on PyPI](https://pypi.org/project/nvidia-pyindex/). Run commands in a Docker
container to isolate these operations:

```console
$ docker build -t uv-builder -f crates/uv-dev/builder.dockerfile --load .
# Build for musl to avoid glibc errors, might not be required with your OS version
cargo build --target x86_64-unknown-linux-musl --profile profiling
docker run --rm -it -v $(pwd):/app uv-builder /app/target/x86_64-unknown-linux-musl/profiling/uv-dev resolve-many --cache-dir /app/cache-docker /app/scripts/popular_packages/pypi_10k_most_dependents.txt
```

Use this container when you do not trust the dependencies of the packages that you resolve or
install.

## Profiling and Benchmarking

See Ruff's
[Profiling Guide](https://github.com/astral-sh/ruff/blob/main/CONTRIBUTING.md#profiling-projects),
which also applies to uv.

Use `test/requirements` to test and benchmark the resolver. Use `test/requirements/compiled` to test
and benchmark the installer.

Use `scripts/benchmark` to compare workloads across uv versions and other tools. For example, run
the following command from the `scripts/benchmark` directory:

```shell
uv run resolver \
    --uv-pip \
    --poetry \
    --benchmark \
    resolve-cold \
    ../test/requirements/trio.in
```

### Analyzing concurrency

Use [tracing-durations-export](https://github.com/konstin/tracing-durations-export) to visualize
parallel requests and find where uv is CPU-bound. These examples run `uv` and `uv-dev`,
respectively:

```shell
RUST_LOG=uv=info TRACING_DURATIONS_FILE=target/traces/jupyter.ndjson cargo run --features tracing-durations-export --profile profiling -- pip compile test/requirements/jupyter.in
```

```shell
RUST_LOG=uv=info TRACING_DURATIONS_FILE=target/traces/jupyter.ndjson cargo run --features tracing-durations-export --bin uv-dev --profile profiling -- resolve jupyter
```

### Trace-level logging

Set the `RUST_LOG` environment variable to enable `trace`-level logging:

```shell
RUST_LOG=trace uv
```

## Documentation

To preview documentation changes locally:

1. Install the [Rust toolchain](https://www.rust-lang.org/tools/install).

2. Install [Node](https://nodejs.org/en/download) to run Prettier and format the documentation.

3. Run `cargo dev generate-all` to update generated documentation.

4. Run the development server:

   ```shell
   uv run --only-group docs mkdocs serve -f mkdocs.yml
   ```

Open [http://127.0.0.1:8000/uv/](http://127.0.0.1:8000/uv/) to view the documentation locally.

Each release publishes the documentation to the
[Astral documentation](https://github.com/astral-sh/docs) repository. That repository deploys the
documentation with Cloudflare Pages.

After you edit the documentation, [format the Markdown files](#formatting) with Prettier.

## Development code signing on macOS

Only Astral team members can sign code.

Code signing on macOS can make tests easier to run. For tests that access the macOS keychain, you
can approve a signed binary once. You must approve an unsigned binary after each recompile.

### Acquiring a development certificate

1. Generate a
   [request for the certificate](https://developer.apple.com/help/account/certificates/create-a-certificate-signing-request).
2. Create a certificate in the
   [Apple Developer portal](https://developer.apple.com/account/resources/certificates/list).
3. Download the certificate and install it in your login keychain:

   ```shell
   security import ~/Downloads/mac_development.cer -k ~/Library/Keychains/login.keychain-db
   ```

4. Find your code-signing identity:

   ```shell
   security find-identity -v -p codesigning
   ```

5. If the command does not find your identity, install the intermediate certificates:

   ```shell
   curl -sLO "https://www.apple.com/certificateauthority/AppleWWDRCAG3.cer"
   security import AppleWWDRCAG3.cer -k ~/Library/Keychains/login.keychain-db
   rm AppleWWDRCAG3.cer
   ```

6. Set `UV_TEST_CODESIGN_IDENTITY`:

   ```shell
   export UV_TEST_CODESIGN_IDENTITY="Mac Developer: Your Name (TEAM_ID)"
   ```

Only `nextest` supports `UV_TEST_CODESIGN_IDENTITY`.

## Releases

Only Astral team members can create releases.

The release script updates changelog entries and version numbers automatically. Run:

```shell
./scripts/release.sh
```

If release preparation detects a new workspace crate, add it to
[`astral-sh/crates-policies`](https://github.com/astral-sh/crates-policies).

Edit `CHANGELOG.md` so that its entries use a consistent style.

Open a pull request with a title such as `Bump version to ...`.

CI automatically tests the binary builds for the release.

After you merge the pull request, run the
[release workflow](https://github.com/astral-sh/uv/actions/workflows/release.yml) with the version
tag. **Do not include a leading `v`**. GitHub creates the release after all other publishing steps
finish.
