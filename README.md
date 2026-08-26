# uv 0.12.6 release artifacts are missing from releases.astral.sh

Issue: astral-sh/uv#21302

Classification: duplicate

## Summary

The reporter initially found that both the uv 0.12.6 release archives and their `.sha256` sidecars returned HTTP 404 from `releases.astral.sh` across release variants, despite being present on the GitHub release. A direct check reproduced the reported x86-64 GNU checksum failure while the same 0.12.5 mirror path returned 200.

The canonical incident is astral-sh/uv-dev#854. It records that the uv 0.12.6 `publish-mirror` job downloaded the release artifacts successfully, but every upload to the R2 release prefix failed with `AccessDenied` on three attempts. The exact authorization cause was not established in the available discussion. The missing artifacts have since been restored to the mirror.

## Maintainer status

A uv maintainer reported the issue fixed on August 26, 2026. A subsequent direct check confirmed HTTP 200 for both reported x86-64 GNU URLs: the release archive and its `.sha256` sidecar.

## Reported impact

During the incident, a `setup-uv` user reported that the missing mirror assets surfaced in CI as `Warning: Failed to download from mirror, falling back to GitHub Releases: Unexpected HTTP response: 404`. This shows that the failure was visible to downstream CI users and caused `setup-uv` to attempt its GitHub Releases fallback; the comment did not confirm whether that fallback subsequently completed.

## Classification

This is a `duplicate` of astral-sh/uv-dev#854, which was already open and independently tracks the same uv 0.12.6 mirror-publication failure. The current report describes missing archives and checksums across release variants, while the existing incident records the underlying failure of all uploads for the 0.12.6 mirror prefix.

The underlying behavior was an established correctness problem, not a support question or enhancement: GitHub listed 19 `.sha256` assets for uv 0.12.6, including the reported file; that GitHub asset was available; the 0.12.5 mirror equivalent returned 200; and the 0.12.6 mirror URL returned 404 during triage. Because an open issue already tracked this exact regression, `duplicate` took precedence over `bug`. The later restoration does not change that original classification.

## Related

- astral-sh/uv-dev#854 — **Release mirror uploads fail with R2 AccessDenied** (open). This is the canonical incident. It identifies the uv 0.12.6 `publish-mirror` job and records `AccessDenied` for every R2 upload on three attempts. The mirror assets are now available even though this internal incident remains open.
- astral-sh/uv#19282 — **Bug: Primary download URL returns 404 for uv 0.11.9 on x86_64 Linux** (closed). This is the closest historical recurrence. The 0.11.9 mirror artifacts and then their `.sha256` sidecars were missing until maintainers backfilled them.
- astral-sh/uv#19297 — **uv version 0.11.10 give a 404 error when runnning the installation script** (closed). The next release also initially returned 404 for every path under its mirror prefix and was repaired.
- astral-sh/uv#18159 — **Upload uv releases to a mirror** (merged pull request). This introduced the release-artifact upload workflow for the Astral mirror; the current incident identifies its `publish-mirror` job as the failing subsystem.

## Search and supporting evidence

The report was decomposed into the exact release (`0.12.6`), artifact types (release archives and `.sha256` sidecars), observable response (HTTP 404), scope (all release variants), and subsystem (`releases.astral.sh` release mirroring). Literal and conceptual searches covered the mirror hostname, version, checksum terminology, missing release artifacts, CDN and mirror publication, upload failures, and release-workflow terminology across open and closed issues and open, closed, and merged pull requests. Historical fix-oriented inspection followed references among astral-sh/uv#19282, astral-sh/uv#19297, astral-sh/uv-dev#854, and astral-sh/uv#18159.

Direct checks found:

- The GitHub uv 0.12.6 release contains 42 assets, including 19 `.sha256` files.
- `uv-x86_64-unknown-linux-gnu.tar.gz.sha256` is uploaded on GitHub with a 101-byte payload and a recorded SHA-256 digest.
- The reported 0.12.6 checksum URL returned HTTP 404 during initial triage.
- The corresponding GitHub release URL responds with a download redirect.
- The equivalent 0.12.5 Astral mirror URL returns HTTP 200.
- After the maintainer's fix confirmation, both the reported 0.12.6 archive and checksum URLs returned HTTP 200.
- astral-sh/uv-dev#854 records `AccessDenied` from both `PutObject` and `CreateMultipartUpload` across three attempts, while leaving the precise authorization cause open in the available discussion.

astral-sh/uv#18798 was a plausible checksum-title match but was ruled out. Its absent powerpc64 checksum resulted from an intentionally removed big-endian PPC64 build target and was resolved by removing that target from the downloads configuration; the supported uv 0.12.6 assets in this report exist on GitHub and failed only during mirror publication. astral-sh/uv#18503 was also inspected but is broader: it tracks adopting the Astral mirror as the primary source rather than a particular failed release upload.
