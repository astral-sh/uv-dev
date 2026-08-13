# Versioning

uv is widely used in production and is stable software.

uv increases its minor version for breaking changes. It increases its patch version for bug fixes,
enhancements, and other changes that preserve compatibility.

The care given to incompatible changes depends on their expected real-world impact, not on a version
numbering rule. uv develops new features quickly and groups potentially breaking changes into
clearly marked releases.

uv's changelog is [available on GitHub](https://github.com/astral-sh/uv/blob/main/CHANGELOG.md).

## Crate versioning

uv publishes its crates to [crates.io](https://crates.io). The following crates follow the normal uv
versioning policy:

- `uv`
- `uv-build`
- `uv-version`

The `uv` and `uv-build` crate versions describe the binary command-line interface. Their Rust
interfaces do not follow semantic versioning.

The remaining uv crates provide **no stability guarantees**. Their Rust interfaces are internal and
unstable, so their versions use `0.0.x`. The patch version increases with every uv release, even
when the crate does not change.

## Cache versioning

Cache versions are internal to uv and may change in a minor or patch release. The
[cache versioning documentation](../../concepts/cache.md#cache-versioning) provides more details.

## Lockfile versioning

The `uv.lock` schema version is part of the public API. It only increases in a minor release as a
breaking change. The
[lockfile versioning documentation](../../concepts/resolution.md#lockfile-versioning) provides more
details.
