# Reproducible release experiment

This recipe builds `uv` and `uvx` for `x86_64-unknown-linux-musl`, with the release profile,
`self-update` feature, and embedded `cargo-auditable` dependency information. It does not yet
replace the published release artifacts or promise that older releases can be reproduced.

Commit changes to the recipe before running it. With Python 3.11+ and Docker available, run from the
repository root:

```shell
python3 scripts/reproducible-builds/build.py --check --out target/reproducible-release
```

The output directory must not already exist. Use `--revision <commit-or-tag>` to select a different
committed revision containing the recipe, or omit `--check` to perform one build. The source archive
and builder recipe both come from that revision; uncommitted files are not included. Temporary build
directories are created under `~/code/tmp`.

The builder pins the `linux/amd64` native-toolchain image used by Maturin's musl builds, an official
Rust image, and the musl standard-library archive. Update all three Rust pins in the Dockerfile when
changing `rust-toolchain.toml`. Keep the `cargo-auditable` version synchronized with
`scripts/install-cargo-extensions.sh`.

Dependencies are vendored from `Cargo.lock` before compilation. Each build then runs in a fresh,
network-disabled container with its own Cargo home and target directory. The comparison deliberately
changes the source path and modification times. Rust and native source paths are remapped, and the
release archive has normalized metadata. `build-inputs.json` records the source digest, builder
image ID, and explicit Git version metadata. Each build retains its binaries, archive, hashes,
toolchain details, and ELF inspection output, including on a comparison failure.

The workflow runs for changes to this recipe and its Rust dependency/toolchain inputs, and can also
be dispatched manually. A passing comparison establishes reproducibility for these two builds, not
for every host, platform, or existing release. Adopting the recipe in the release workflow and
publishing the builder image and inputs are separate steps. PGO targets additionally need the exact
training profile preserved as an input.
