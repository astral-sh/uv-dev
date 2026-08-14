# `uv export --format cyclonedx1.5` omits component hashes that `uv.lock` already carries

Issue: astral-sh/uv#21122

Classification: enhancement

## Summary

The reporter shows that uv 0.12.4 on macOS 14 arm64 records SHA-256 hashes for registry distributions in `uv.lock` and includes them in a `requirements.txt` export, but a CycloneDX 1.5 export contains no hashes on either components or distribution external references. `--hashes` does not change that output, and `--no-hashes` therefore has nothing to suppress. The dependency graph, package URLs, and uv marker properties are otherwise present.

This is the concrete follow-up anticipated during the original SBOM design discussion. In astral-sh/uv#6012, participants discussed the same lockfile distribution hashes, a maintainer agreed that adding them was reasonable, and the implementation author recorded that they would be left out of the initial pull request and could be added later. astral-sh/uv#16523 then shipped the initial CycloneDX exporter and closed that broad feature issue. No open issue or pull request currently tracks the deferred hash support.

In the current discussion, maintainer @zanieb confirmed that the omission was deferred from the original implementation for simplicity and stated that supporting hashes is acceptable. This establishes maintainer support for the enhancement, while leaving the representation and implementation details to be settled.

## Draft response

Thanks for the detailed reproduction. Distribution hashes were discussed in astral-sh/uv#6012 and intentionally deferred from the initial CycloneDX implementation in astral-sh/uv#16523, so this is a valid scoped enhancement rather than a regression. The current exporter does not consume the hash setting and emits neither component hashes nor distribution external references. The next step is to settle the artifact representation—astral-sh/uv#6012 identified distribution external references as the likely fit—and add coverage for hashes being included by default and suppressed by `--no-hashes`.

## Classification

This should be classified as an enhancement. CycloneDX hash support has never been implemented: the discussion in astral-sh/uv#6012 explicitly deferred it from the initial exporter rather than establishing that the first release would include it. The request would add that missing capability and make the existing hash-control option meaningful for CycloneDX output. It is not a regression and is not a duplicate because the broad feature issue is closed and no separate issue or active pull request already tracks the deferred work.

The maintainer response on astral-sh/uv#21122 corroborates this classification: the omission was a simplification in the original implementation, not a newly introduced failure, and adding support is acceptable.

The current source matches that history:

- The export command passes its hash setting to `RequirementsTxtExport`, but the CycloneDX `from_lock` call has no hash argument.
- CycloneDX components are constructed with both `hashes` and `external_references` absent.
- The command-line help describes `--no-hashes` generally as omitting hashes from generated output, making support for suppression part of the requested follow-up once CycloneDX hashes are emitted.

## Related

- astral-sh/uv#6012 — “Software Bill of Materials (SBOM) output” (closed). This is the original feature discussion and the closest historical record. It explicitly considered registry-distribution hashes, identified distribution external references as the likely representation, and deliberately deferred hashes to future work.
- astral-sh/uv#16523 — “Add SBOM export support” (merged). This implemented CycloneDX 1.5 export and closed astral-sh/uv#6012. The implementation shipped without the deferred hashes and does not apply the export hash setting to CycloneDX generation.

## Search and supporting evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries included `cyclonedx1.5`, CycloneDX with `hash` or `hashes`, `component hashes`, `externalReferences`, `--hashes`, and `--no-hashes`. Conceptual and fix-oriented queries included SBOM verification, artifact or distribution checksums, lockfile hash export, and the original SBOM implementation. No dedicated prior tracker, active implementation, or historical fix for CycloneDX hashes was found.

Several plausible hash issues were inspected and ruled out:

- astral-sh/uv#6944 requested suppressing hashes in `requirements.txt` exports.
- astral-sh/uv#9225 concerned ordering hashes already emitted to `requirements.txt`.
- astral-sh/uv#10987 was closed as a duplicate of astral-sh/uv#7064; those reports concern `--find-links` artifacts whose hashes are absent from `uv.lock` and consequently absent from `requirements.txt` exports.

Those differ materially from astral-sh/uv#21122, where the hashes already exist in `uv.lock` and are lost only by the CycloneDX exporter.
