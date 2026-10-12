# v0.13.0 fails to install pybabel-json==0.2.0

Issue: astral-sh/uv#22492

Classification: bug

## Summary

The reporter cannot complete `uv sync` with uv 0.13.0 because extraction of
`pybabel-json==0.2.0` fails before its build. The dependency is included through
`fm-data-api:dev==4.7.75`. The reported environment is Linux
7.0.0-1017-azure, x86_64, with Python 3.14.8.

The decisive diagnostic is:

```text
error: Failed to download and build `pybabel-json==0.2.0`
  cause: Failed to extract archive: pybabel-json==0.2.0
  cause: Invalid tar file
  cause: at byte 25088: extraction policy rejected input: hard-link members are not allowed
```

astral-sh/uv#22094 explains the uv 0.13.0 hard-link rejection and provides a legacy-backend workaround. The earlier same-package failure in astral-sh/uv#11223 was fixed by astral-sh/uv#11221 in uv 0.5.28.

No existing open issue or pull request tracking this regression was found.
The issue had no comments when inspected.

## Draft response

uv 0.13.0 switched to tar-codec in astral-sh/uv#22094, and its extraction policy rejects hard links. The earlier failure in astral-sh/uv#11223 was fixed by astral-sh/uv#11221 in uv 0.5.28; this is a new compatibility regression with the default backend. Please try `UV_LEGACY_TAR_BACKEND=1 uv sync`, the documented workaround, and let us know whether it succeeds.

## Classification

A previously supported package fails because uv 0.13.0's default tar extraction policy rejects hard links. The earlier same-package failure was fixed in uv 0.5.28 by astral-sh/uv#11221, so this is a new compatibility regression, not a duplicate of astral-sh/uv#11223. The documented stricter policy explains the failure but does not negate the compatibility bug. No open issue or PR tracking this regression was found.

This classification concerns the loss of package-installation compatibility.
The backend change was deliberate and its limitation is documented, but the
reported installation failure is established by the error and matching source
policy. It does not require an independent reproduction to classify.

The historical failure and the current failure share a hard-linked source
archive, but have different immediate mechanisms: the old backend constructed
an incorrect target path; the new backend rejects hard-link members outright.
The merged default-backend change explains the regression and is not a fix for it.

## Related

- astral-sh/uv#22094 — Use tar-codec by default (pull request, merged). Changed the default tar backend for uv 0.13.0. Its release notes explicitly describe rejection of previously accepted hard-link archives and document UV_LEGACY_TAR_BACKEND=1. The current extraction policy confirms hard-link rejection, matching the reported error.
- astral-sh/uv#11223 — v0.5.27 fails to install pybabel-json==0.2.0 (issue, closed). The same package version previously failed during hard-link extraction. A maintainer confirmed that a fix had merged; it shipped in uv 0.5.28. That failure involved incorrect link-target resolution, whereas this report shows explicit rejection by the new backend.
- astral-sh/uv#11221 — Fix hardlinks in tar unpacking (pull request, merged). Historical fix released in uv 0.5.28 for hard-link targets incorrectly resolved relative to the member's parent directory. Establishes that hard-link extraction support had been restored before the uv 0.13.0 backend change.

## Supporting evidence

- `CHANGELOG.md:6` identifies the uv 0.13.0 release.
  `CHANGELOG.md:66` attributes the default-backend switch to astral-sh/uv#22094.
  `CHANGELOG.md:70` explicitly says that archives containing hard links can now
  be rejected; `CHANGELOG.md:72` documents `UV_LEGACY_TAR_BACKEND=1`.
- astral-sh/uv#22094 merged on October 1, 2026, before this report opened on
  October 12, 2026. Its body covers extraction, source-distribution creation,
  publishing metadata, and the legacy-backend opt-out. Its discussion and review
  comments do not establish a later hard-link compatibility fix.
- `crates/uv-extract/src/stream.rs:791` defines `tar_extract_policy()`.
  It retains the default hard-link rejection policy; the Unix branch uses
  `ExtractPolicy::default()`. This directly matches the reporter's diagnostic.
- `crates/uv-static/src/lib.rs:19` defines `TarBackend::from_env()`.
  A true `UV_LEGACY_TAR_BACKEND` selects `TokioTar`; otherwise it selects
  `TarCodec`. The extraction dispatch in `crates/uv-extract/src/stream.rs`
  applies that choice.
- astral-sh/uv#11223 describes the same package version succeeding with uv
  0.5.26 and pip, then failing with uv 0.5.27 while resolving the README hard
  link. A maintainer said the hard-link fix was already merged. The issue was
  closed on February 5, 2025.
- astral-sh/uv#11221 merged on February 4, 2025. It explains the incorrect
  hard-link target base, updates the tar dependency, and adds regression tests.
  `changelogs/0.5.x.md:1504` and `changelogs/0.5.x.md:1513` place that fix in
  uv 0.5.28. The PR closes astral-sh/uv#11213, whose maintainer discussion
  confirms that including cached hard-linked files in a source distribution
  exposed the same historical extraction regression.

## Search coverage and excluded candidates

The report was decomposed before searching into one installation-failure claim:
`uv sync`; source-distribution extraction; the exact package and version;
the exact policy-rejection and invalid-tar errors; uv 0.13.0 timing; and the
reporter's proposed historical relationship. There is no separate request for
new functionality or evidence of a Python-version-specific failure.

Used authenticated gh to search open and closed issues and open, closed, and merged PRs for pybabel-json, the exact hard-link policy error, Invalid tar file, 0.13.0 extraction failures, hardlink/hard-link archive handling, tar-codec, and UV_LEGACY_TAR_BACKEND. Supplemented PR keyword searches, which omitted known matches, with recent all-state PR listings and changelog-linked PRs. Inspected candidate bodies, comments, reviews, and historical references, including astral-sh/uv#11213. Ruled out astral-sh/uv#21641 (connection resets), astral-sh/uv#12163 and astral-sh/uv#1858 (different symlink failures), and astral-sh/uv#21128 (missing diagnostic hint). astral-sh/uv#22425 was closed without merging and only proposed compatibility documentation. No existing open tracker for this regression was found.

Literal symptom searches were kept separate from mechanism searches. Conceptual
queries removed the package, version, and platform and used hard links,
hardlinks, archive extraction, unpacking, source distributions, and tar policy.
Fix-oriented checks included the historical release changelog and explicit
closed/merged PR searches. Because keyword PR searches did not return known
matching PRs, four pages of recent all-state PR listings were also inspected,
and the strongest PRs were retrieved directly.

The following candidates were examined but do not centralize this regression:

- astral-sh/uv#21641 reports changing extraction failure locations and
  connection resets; a maintainer identifies a download failure, and changing
  networks worked for the reporter.
- astral-sh/uv#12163 concerns accepting links outside the extraction directory.
  It does not track rejection of archive-internal hard links.
- astral-sh/uv#1858 concerns querying a symbolic-link target before extraction
  and Windows symbolic-link privileges.
- astral-sh/uv#21128 concerns a missing virtual-environment diagnostic hint for
  an unsafe absolute symbolic link.
- astral-sh/uv#22425 proposes documentation of the same compatibility change
  and fallback. It was closed without merging on October 9, 2026, and contains
  no extraction fix or open regression discussion.
- astral-sh/uv#22143 reduces archive-decoder specialization by changing the
  reader interface; its body and discussion do not describe a hard-link policy fix.

## Next step and validation limits

Ask the reporter to retry with `UV_LEGACY_TAR_BACKEND=1 uv sync`. This is the
documented compatibility option and selects the older extraction implementation.
Its success for the reporter's full project remains unverified.

No installation reproduction or test suite was run. The conclusion rests on
the supplied diagnostic, repository source, release notes, and inspected GitHub
discussions. No fix release for this regression was identified. Only this
handoff README was written; no checkout files or GitHub state were changed.
