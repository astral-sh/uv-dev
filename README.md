# Wheel variant preview implementation tracker

Issue: astral-sh/uv#22043

Classification: enhancement

## Summary

This tracking issue coordinates landing wheel-variant support in standard uv behind the
`wheel-variants` preview feature. It proposes decomposing the broad prototype in
astral-sh/uv#12203 into reviewable changes for PEP 825 variant labels, markers, static test input,
compatibility, providers, and eventually ABI dependencies.

The direct implementation chain is already visible: merged astral-sh/uv#22038 prepared generic
reading of `.dist-info` files for PEP 825 `variants.json`, and astral-sh/uv#22039 merged on
2026-10-12, adding the preview gate and parsing variant labels without selecting those wheels.
The remaining selection,
compatibility, provider execution, and ABI-dependency work is not implemented by that first scoped
pull request. Open astral-sh/uv#8639 is a historical design precursor, but it does not replace this
PEP 825 implementation tracker.

The updated checklist links three open metadata PRs: astral-sh/uv#22509 recognizes index metadata
entries, astral-sh/uv#22511 fetches and caches their contents, and astral-sh/uv#22512 parses and
validates them. Open astral-sh/uv#22490 adds variant fixtures to the Packse test server.
These PRs do not establish completed end-to-end variant selection support.

## Remaining implementation scope

The supplied issue body marks filename labels complete. It lists null-variant compatibility and
variant ordering/wheel preferences as separate pending steps, followed by static providers,
compatibility determination, marker evaluation, static file input, possible implied markers,
a provider frontend, a vendored CUDA provider, and CLI options for provider opt-in.
ABI dependencies remain explicitly low priority. These are tracker requirements, not claims that
the current scoped PRs implement them.

The opening sentence says PEP 815, while the checklist, linked pull request, and implementation
evidence consistently refer to PEP 825. This appears to be a typo worth correcting in the issue.

## Draft response

Thanks for setting up the tracker. astral-sh/uv#12203 is the broad prototype, while
astral-sh/uv#22038 and astral-sh/uv#22039 establish the first preparatory and preview-gated pieces.
Selection, compatibility, provider execution, and ABI-dependency work remain outstanding. The
opening sentence appears to mean PEP 825 rather than PEP 815. Please keep linking each split pull
request here as it is opened so this remains the canonical implementation checklist.

## Classification

Enhancement. The issue is explicitly a `tracking` issue for adding new preview functionality. It
does not describe incorrect current behavior, ask primarily for support, or duplicate an existing
implementation tracker. astral-sh/uv#8639 discusses an earlier metadata prototype, while
astral-sh/uv#12478 and astral-sh/uv#16522 describe user-facing hardware/backend-selection use cases;
none has the same scoped role of decomposing astral-sh/uv#12203 into the current PEP 825 and provider
work.

## Related

- astral-sh/uv#12203 — Open pull request, “Add prototype implementation of wheel variant
  specification.” This is the broad prototype the tracker explicitly intends to split. Its body
  covers providers, ordering, static configuration, markers, resolution, and lockfile support, and
  states that ABI dependencies are not implemented.
- astral-sh/uv#22038 — Merged pull request, “Refactor `.dist-info` reading.” This is a direct
  preparatory split from the prototype: it generalized `.dist-info` reads specifically to support
  PEP 825 `variants.json` metadata.
- astral-sh/uv#22039 — Merged pull request, “Parse wheel filenames with variant labels.” This is the
  first checklist pull request. It introduces the `wheel-variants` preview feature and parses PEP
  825 labels, but explicitly does not make variant wheels selectable yet.
- astral-sh/uv#22509 — Open pull request, “Recognize variant metadata in package indexes.” Recognizes
  `{name}-{version}-variants.json` entries in simple index pages. Downloading and parsing the
  contents are handled by subsequent PRs.
- astral-sh/uv#22511 — Open pull request, “Fetch index-level variant metadata.” Adds fetching with
  locking, retries, and caching based on index cache headers. Parsing and use happen separately.
- astral-sh/uv#22512 — Open pull request, “Parse and validate variant metadata.” Implements PEP 825
  metadata parsing and validation. Its description specifies backtracking on a version with invalid
  metadata instead of failing outright; the parsed data is not yet used for selection.
- astral-sh/uv#22490 — Open pull request, “Support wheel variants in the Packse test server.” Adds
  scenarios with variant properties, wheel tags, and dependencies, per-wheel metadata, and combined
  index metadata for each package version. This supplies test infrastructure for the implementation.
- astral-sh/uv#8639 — Open issue, “Variant metadata prototype ideas.” This historical design
  precursor proposes local static input, per-package index metadata, and ordered compatibility
  intersection. It predates PEP 825 and is not the current implementation checklist.

## Supporting evidence

The supplied comment dated 2026-10-14 quotes the expanded checklist in the supplied issue body;
it adds no independent reproduction or maintainer decision. The checklist nevertheless corrects
the handoff's stale filename-parsing status and identifies the next implementation PRs. Direct
GitHub PR reads on 2026-10-14 confirmed the merge of astral-sh/uv#22039 and the open status and
described scope of astral-sh/uv#22509, astral-sh/uv#22511, astral-sh/uv#22512, and astral-sh/uv#22490.
Merge status alone does not establish availability in a released uv version.

Literal searches covered “wheel variant,” “wheel variants,” “PEP 815,” “PEP 825,” “variant label,”
“variant markers,” “variant compatibility,” “static providers,” “ABI dependencies,”
`UV_VARIANT_LOCK`, `variants.json`, WheelNext, and the exact identifiers 12203 and 22039. Conceptual
searches covered hardware-aware, GPU/CPU backend, accelerator, provider-property, static-input, and
index-selection terminology. Open and closed issues and open, closed, and merged pull requests were
included; the strongest candidates and their comments and cross-reference timelines were inspected.

The cross-reference timeline for astral-sh/uv#12203 points directly to astral-sh/uv#22038,
astral-sh/uv#22039, and this tracker. The current body of astral-sh/uv#22039 limits its scope to
preview-gated filename parsing, confirming that it does not yet implement the later checklist items.

Plausible adjacent results were ruled out from the related list where the underlying request was
different. astral-sh/uv#16522 asks for automatic CPU/GPU selection across packages, and
astral-sh/uv#12478 asks for CUDA-specific indices beyond PyTorch; both are user-facing use cases
that the wheel-variant work may eventually serve, not implementation trackers. Closed
astral-sh/uv#15402 asks for another distribution channel for prototype builds; its maintainer reply
instead points toward shipping wheel variants in standard uv as a preview feature, so it supports
the direction but does not track the implementation.
