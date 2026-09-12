# Rust support

The `rust-version` key under `[workspace.package]` in `Cargo.toml` lists the minimum Rust version
needed to compile uv. This version may change in any minor or patch release. It is never newer than
N-2, where N is the latest stable Rust version. For example, if the latest stable Rust version is
1.85, uv's minimum supported Rust version is at most 1.83.

This policy only affects users who build uv from source. Installation from the Python package index
usually provides a pre-built binary and does not require Rust.
