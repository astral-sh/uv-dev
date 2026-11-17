# Unclear resolution error when `minimum-libc-version` is involved

Issue: astral-sh/uv#23272

Classification: bug

## Summary

The reporter uses uv 0.13.8 to initialize a project, add pandas, enable wheel-only
resolution, set a minimum supported glibc version of 2.20, and limit resolution to
Linux x86_64. Running `uv lock` fails with an explanation that pandas 3.1.1 has no
wheels compatible with the Linux/x86_64 environment marker. The reporter says
Linux/x86_64 wheels exist, but require a newer glibc.

The rejection can be correct while its explanation is misleading: the diagnostic
does not mention the libc baseline that makes those wheels unsuitable. Existing
source and integration snapshots independently establish that diagnostic gap.
The feature's introducing PR already contained the same explanation. No exact
duplicate or prior fix for this diagnostic was found.

## Report and distinct claims

The relevant configuration is:

```toml
[tool.uv]
no-build = true
preview-features = ["minimum-libc-version"]
minimum-libc-version = { glibc = "2.20" }
environments = [
    "sys_platform == 'linux' and platform_machine == 'x86_64'",
]
```

After `uv init .` and `uv add pandas`, the reporter adds this configuration and
runs `uv lock`. The reported cause includes:

```text
Because pandas==3.1.1 has no `platform_machine == 'x86_64' and sys_platform == 'linux'`-compatible wheels
```

The primary symptom is that a glibc-specific rejection is described using only
OS and architecture. The requested correction is to mention the configured glibc
2.20 baseline. A further requested improvement is to report the minimum glibc
version supported by the available wheels.

The affected subsystem is universal dependency resolution and its incompatibility
reporting. The triggering combination includes an explicit libc baseline,
Linux/x86_64 environment selection, and disabled source builds. The report does
not establish an incorrect package selection, a host-platform detection problem,
or a need to discard musl wheels from the lockfile.

## Draft response

You're right: the error omits the configured glibc requirement. With `no-build = true`, rejecting wheels that require a newer glibc is expected, but describing that only as a Linux/x86_64 mismatch is misleading. The tests added in astral-sh/uv#21651 already show this omission. The next step is to carry the configured libc baseline into the resolution diagnostic and update those snapshots; showing the available wheels' libc requirements would make the mismatch clearer.

## Classification

The user-facing explanation omits a constraint that actually determines wheel compatibility, misleadingly attributing rejection to OS/architecture alone. Source confirms that libc-aware coverage is reduced to MissingPlatform(MarkerTree), whose formatter only prints the marker; existing integration snapshots demonstrate the omission. This is a diagnostic correctness bug, even though reporting the available wheels' minimum libc requirements is an additional improvement. The introducing PR already contained this output, so the evidence does not establish a regression. No existing tracker found covers the same universal-resolution diagnostic closely enough to centralize discussion.

A preview designation does not make a misleading diagnostic correct. No further
reproduction is required to establish the general reporting defect: existing
tests show it with synthetic wheels independently of pandas. The exact pandas
release's wheel inventory and the reporter's full project were not independently
reproduced.

## Related

- astral-sh/uv#21651 — Add minimum libc versions for universal resolution (pull request, merged). Introduced minimum-libc-version for universal resolution on September 18, 2026. Its tests already emit the reported marker-only missing-wheel explanation when the configured glibc baseline is unsatisfied. Establishes the feature's origin and intended compatibility rules; it did not fix the diagnostic.
- astral-sh/uv#10581 — Hint user towards `--python-platform linux` targeting glibc 2.17 (issue, open). The linked maintainer discussion explicitly proposes explaining a glibc version mismatch. It concerns the implicit glibc 2.17 baseline of a concrete --python-platform target; this report concerns an explicit minimum-libc-version constraint omitted from a universal-resolution marker diagnostic. Related messaging work, but not the same failure path.
- astral-sh/uv#18169 — Wheel availability hint does not include musllinux tag in alpine image (issue, open). Another confirmed gap in communicating libc compatibility: a maintainer explains that Alpine requires musllinux and questions the missing hint. It concerns install-time musl-versus-glibc platform hints, whereas this report concerns a configured glibc version during uv lock. Its linked proposed fix, astral-sh/uv#18341, was closed without merging.
- astral-sh/uv#12597 — Markers for Glibc/manylinux version (issue, open). Tracks glibc-aware backtracking in project resolution and led to astral-sh/uv#21651. The original request was to select compatible package versions on older glibc systems; this report already uses that capability and identifies an incomplete explanation when no version satisfies it.

## Supporting evidence

- `docs/concepts/resolution.md:241` explains that environment markers cannot
  express libc implementation or version. Its example states that a
  `manylinux_2_17` wheel satisfies glibc 2.31, while a `manylinux_2_34` wheel
  does not. A usable source distribution can satisfy required coverage unless
  builds are disabled.
- `crates/uv-resolver/src/prioritized_distribution.rs:1002` computes wheel
  coverage using `minimum_libc_version.platform_coverage(tag)`.
  `crates/uv-resolver/src/resolver/mod.rs:1692` reports unsuccessful artifact
  coverage as `IncompatibleWheel::MissingPlatform(marker)`.
- `crates/uv-resolver/src/prioritized_distribution.rs:390` stores only a
  `MarkerTree` in that incompatibility variant. Its formatter at line 323
  renders a platform name or marker and has no libc-specific explanation.
- `crates/uv/tests/lock/minimum_libc.rs:629`,
  `minimum_libc_no_compatible_version`, creates a manylinux 2.34 wheel and a
  musllinux wheel, requires glibc 2.31 coverage, and snapshots a failure that
  names only the Linux/x86_64 marker. The preview warning mentions the setting
  name but does not explain the incompatible baseline.
- `crates/uv/tests/lock/minimum_libc.rs:778`,
  within `minimum_libc_allows_sdist_fallback`, repeats that marker-only failure
  after disabling builds for a release with an otherwise usable sdist.
- The file diff in merged astral-sh/uv#21651 contains these same diagnostic
  snapshots. The PR merged on September 18, 2026, before this November 17 report.
  A review reply confirms that wheels with an older manylinux baseline satisfy
  a newer configured glibc floor; retaining additional wheels is intentional.
- The discussion linked from astral-sh/uv#10581 explicitly asks whether an error
  should say the glibc version is too low or explain that
  `--python-platform linux` defaults to glibc 2.17. This is a useful adjacent
  design discussion, but its concrete-target tag diagnostics differ from the
  marker-only universal-resolution path here.

## Candidates ruled out as duplicates

astral-sh/uv#22913 and its open implementation astral-sh/uv#22942 concern choosing
a Linux libc family and excluding musl artifacts. Maintainer comments distinguish
requiring glibc coverage from pruning other wheels; neither tracks the omitted
glibc version in this failure.

astral-sh/uv#23037 and astral-sh/uv#23038 use similar missing-wheel wording but ask
for resolution to tolerate unsupported environments or limit individual
dependencies. This report accepts an unsatisfiable resolution and challenges its
explanation.

Closed astral-sh/uv#19566 concerns universal locking being independent of the
current host; its maintainer response explains that distinction. It does not
establish a previous fix for libc-aware error text. Closed astral-sh/uv#22716
concerns a dependency genuinely lacking Windows wheels, rather than an omitted
libc condition.

Historical merged astral-sh/uv#10739 adds install-time tag hints, and merged
astral-sh/uv#15484 validates already-installed wheels after a platform change.
Neither fixes this universal-resolution diagnostic. Closed, unmerged
astral-sh/uv#18341 proposed a musllinux platform hint for astral-sh/uv#18169.
Closed, unmerged astral-sh/uv#21671 proposed a different libc configuration
design. None establishes a regression of a previously corrected message.

## Search scope and validation

Used authenticated gh to search open and closed issues and open, closed, and merged PRs. Literal queries covered minimum-libc-version, compatible wheels, no-build, glibc, and error message. Separate conceptual queries covered libc/manylinux diagnostics, misleading wheel availability, supported minimum glibc versions, and area:error-messages; package names, versions, and architecture were removed. Fix-oriented searches covered closed reports and merged PRs; supplemented PR searches with repository PR listings and issue timelines. Inspected candidate bodies, comments, linked review discussions, and source/tests. Ruled out astral-sh/uv#10581 and astral-sh/uv#18169 as duplicates because they concern different concrete-platform/install-time hints. Also ruled out astral-sh/uv#22913 and astral-sh/uv#22942 (excluding musl), astral-sh/uv#23037 and astral-sh/uv#23038 (tolerating unsupported environments), and historical fixes astral-sh/uv#10739 and astral-sh/uv#15484 (sync hints and installed-wheel validation). No exact duplicate or prior fix for this diagnostic was found.

The issue body, candidate discussions, and linked review comments were treated
as evidence, not instructions. Repository guidance and the issue-triage schema
were read. This handoff is based on read-only GitHub queries and source/snapshot
inspection; no build or test execution was needed, and no GitHub changes were
made. Only this external issue-context README was updated. The checkout already
showed a modified `agents/codex/config.toml` and an untracked
`.issue-triage-event.json`; neither was changed.
