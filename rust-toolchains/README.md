# Rust toolchain manifests

This directory contains unmodified Rust distribution manifests used by `Dockerfile` to pin the
toolchain's component archives. The manifest for each version contains the upstream component URLs
and SHA-256 hashes for all supported hosts and targets.

`1.97.1.toml` is the upstream
[Rust 1.97.1 distribution manifest](https://static.rust-lang.org/dist/channel-rust-1.97.1.toml).

The Docker build copies the checked-in manifest into a local distribution mirror and generates its
`.sha256` sidecar from those bytes. `rustup` checks downloaded archives against that manifest before
extracting them; it never downloads a replacement manifest or an expected checksum from the
distribution server.

When updating Rust, update `rust-toolchain.toml` and `RUST_VERSION` in `Dockerfile`, and add the
complete distribution manifest for that version. Review the manifest and its artifact hashes as part
of the version update. Keep upstream manifests verbatim rather than formatting or editing them.

The Docker build currently mirrors `rustc`, `cargo`, the host `rust-std`, and the additional musl
`rust-std`. Supporting another component requires mirroring its archive from the same checked-in
manifest.
