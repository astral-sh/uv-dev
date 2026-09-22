# Pinned test distributions

These upstream distributions are used when a test needs a real build backend, an older tool, or
specific archive contents. Ordinary dependency-resolution tests should use generated packages in
`test/scenarios` instead.

`crates/uv-test/src/vendor.rs` records the distribution filenames and SHA-256 digests. The local
test servers read only these checked-in files and verify their digests before serving them. Missing
or changed files are errors; the servers do not download replacement artifacts. The archives retain
their upstream metadata and license files.
