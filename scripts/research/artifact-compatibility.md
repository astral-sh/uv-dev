# Artifact compatibility matrix

Run `cargo run -p uv-platform-tags --example compatibility_matrix` to emit a TSV matrix of 72
artifact/environment pairs. Each row includes the actual `Tags::compatibility` result and the
expected preferred artifact. The command fails on an unexpected compatibility or priority.

The finite fixture separates package version choice from artifact eligibility: all artifacts
represent alternative wheels for one hypothetical version. It covers the glibc floor, musl,
architecture, Python minor version, stable ABI, free threading, macOS, and Windows. It calls uv's
tag implementation directly and neither downloads nor executes artifacts.

This provides a bounded starting point for studying artifact selection alongside
[Spack's binary reuse work](https://arxiv.org/abs/2509.07728). Wheel tags do not establish runtime
binary compatibility, and this matrix does not model external libraries or GPU drivers.
[PEP 825](https://peps.python.org/pep-0825/) proposes additional wheel-variant capabilities; the
example does not implement that proposal.
