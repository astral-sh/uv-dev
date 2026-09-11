# Include license information in cyclonedx export

Issue: astral-sh/uv#21617

Classification: enhancement

## Summary

The issue requests that `uv export --format cyclonedx1.5` include license information for each
SBOM component. The current CycloneDX exporter constructs both package and synthetic-root
components with `licenses: None`. Although parsed distribution metadata can contain legacy
`License`, PEP 639 `License-Expression`, `License-File`, and classifier fields, the `PackageMetadata`
stored in `uv.lock` currently retains requirements, provided extras, and dependency groups rather
than licenses.

No existing issue tracks license population in CycloneDX output. The closest open discussion,
astral-sh/uv#8156, concerns acquiring the same dependency-license metadata but proposes a dedicated
license-audit command. The original SBOM issue and implementation established a minimal,
lockfile-based CycloneDX exporter. A later hash enhancement provides useful precedent, with the
important difference that artifact hashes were already present in the lockfile.

## Draft response

Thanks for raising this. The CycloneDX exporter introduced in astral-sh/uv#16523 currently leaves
component licenses unset, and `uv.lock` package metadata does not retain license fields, so this is
more than wiring an existing lockfile value into the output. astral-sh/uv#8156 is related, but it
tracks a dedicated dependency-license audit command rather than CycloneDX enrichment, so this
should remain a separate enhancement.

The next design step is to define the authoritative sources and fallbacks for PEP 639 license
expressions, legacy license fields or classifiers, and registry, direct-URL, and local
packages—especially whether export may perform additional index requests or artifact downloads
when the lockfile lacks that metadata.

## Classification

`enhancement` fits because the request adds optional component metadata that the existing exporter
has never populated. The source explicitly leaves licenses unset, and the lock model has no license
field to export. Implementing the request therefore requires a new policy for acquiring, storing,
normalizing, and omitting unavailable metadata; there is no evidence of a regression or violation
of an already-supported behavior. GitHub also currently labels astral-sh/uv#21617 as `enhancement`.

This is not a duplicate of astral-sh/uv#8156: that issue requests license auditing as a separate
user-facing capability. The metadata work may overlap, but either interface can be implemented or
designed independently.

## Related

- astral-sh/uv#8156 — **Open issue, “Add `uv license` or similar to audit dependency licenses.”**
  This is the closest license-specific discussion. It asks how uv can access license fields or
  classifiers and whether that data should be recorded in `uv.lock`, but its requested output is a
  dedicated audit command rather than a CycloneDX SBOM.
- astral-sh/uv#6012 — **Closed issue, “Software Bill of Materials (SBOM) output.”** This is the
  canonical parent discussion that selected CycloneDX as a `uv export` format. It described a
  minimal lockfile-derived SBOM and was completed without a license-specific requirement.
- astral-sh/uv#16523 — **Merged pull request, “Add SBOM export support.”** This implemented
  CycloneDX 1.5 JSON export for astral-sh/uv#6012. The resulting component construction leaves
  licenses unset, matching the behavior reported here.
- astral-sh/uv#21122 — **Closed issue, “`uv export --format cyclonedx1.5` omits component hashes
  that `uv.lock` already carries.”** This is the closest analogous component-enrichment request.
  Maintainers said hashes had been deferred from the initial implementation and classified the
  addition as an enhancement. Unlike licenses, those hashes already existed in `uv.lock`.
- astral-sh/uv#21131 — **Merged pull request, “Include hashes in cyclonedx exports.”** This fixed
  astral-sh/uv#21122 by mapping locked distribution URLs and hashes to CycloneDX external
  references. It is an implementation precedent, but it did not need a new metadata source.

## Supporting evidence

Literal searches covered `cyclonedx license`, `SBOM license`, `component license`, `license
information`, `license field`, `uv export license`, `Metadata23 license`, and `License-Expression`.
Conceptual searches covered dependency-license auditing, package and license metadata, classifiers,
lockfile storage, SBOM component enrichment, index metadata, and direct-URL metadata. Fix-oriented
searches covered open and closed issues and open, closed, and merged pull requests for the original
CycloneDX implementation and subsequent missing-field fixes.

The discussions, comments, references, and implementation history for the five related items above
were inspected. astral-sh/uv#8156 was the most plausible duplicate but differs in its user-facing
capability. astral-sh/uv#21122 and astral-sh/uv#21131 were inspected as the strongest prior fix but
concern lockfile data already available to the exporter. No issue or pull request predating
astral-sh/uv#21617 was found for populating CycloneDX component licenses.
