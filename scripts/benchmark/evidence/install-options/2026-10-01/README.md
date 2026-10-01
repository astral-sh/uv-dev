# Install-package filter evidence

This is the complete 2026-10-01 component study for
[#252](https://github.com/astral-sh/uv-dev/pull/252), not a new benchmark result. It compares the
literal parent `0ded765b4df49fe2dbe7e0497908edb80d9c8fbc` with
`018919a0813966ce887628b3170be0f03625e282`. The PR's current base may differ.

Run `python3 -I -B analyze.py` from this directory. No Rust build, network access, product
execution, or timing is required. The standard-library replay authenticates the manifest and bounded
archive, checks the public-field allowlist, replays all calibration decisions and ordered process
pairs, and prints the exact original 24-outcome report. To inspect individual files, unpack
`data.tar.gz` into a separate directory.

The archive includes the unchanged `calibration.json`, `aa.json`, `fixed-design.json`, `ab.json`,
and `report.json`; the unchanged estimator and strict fixed-design guard; the six source-pinned
public TOML inputs from the existing
[#1559 fixture owner](https://github.com/astral-sh/uv-dev/pull/1559); its manifest/environment
records; and a portable census generator. Only the census's six input-path strings are changed. The
original census digest remains on every historical sample. The portable census has a different
digest and must not be substituted into those samples.

The `overlays/base.patch` and `overlays/head.patch` files reconstruct the exact common benchmark
overlay against their respective commits, including the measured production-API example, benchmark
endpoint, and common lockfile. The normalized trees differ only in
`crates/uv-configuration/src/install_options.rs`. A new measurement using the portable fixture is a
separate study: it requires fresh source/build/executable identities, behavior admission,
calibration, A/A, sealed design, and A/B records.

`construct_and_lookup` includes owned input cloning, production `InstallOptions::new`, one target
pass, and drop. `amortized_lookup` uses a preconstructed production value. Ratios are head/base
elapsed time, with pointwise—not simultaneous—95% Student-t intervals on paired log ratios. The
sealed A/A design chose 12 or 24 A/B pairs, or no A/B when A/A was inconclusive. The actual
calibration used a 2 ms fast-arm target and a conservative 6 s slow-arm guard; the retained 1 ms–10
s limits and 15 s command cap were unchanged. All attempted calibration records and all selected
pairs remain present. No sample filtering, top-up, or whole-command speedup is claimed.

The public provenance allowlist retains source, toolchain, executable, fixture, estimator, and
timing identities. Original native records are represented by immutable hashes and relative
references; their private host/transport records remain in the original archive. This evidence-only
directory changes no production implementation.
