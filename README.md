# uv 0.12.6 release artifacts are missing from releases.astral.sh

Issue: astral-sh/uv#21302

Classification: duplicate

## Summary

The reporter finds that both the uv 0.12.6 release archives and their `.sha256` sidecars return HTTP 404 from `releases.astral.sh` across release variants. The artifacts are present on the GitHub release. A direct check confirmed that the reported x86-64 GNU checksum returns 404 from the Astral mirror while the same 0.12.5 mirror path returns 200.

The canonical open incident is astral-sh/uv-dev#854. It records that the uv 0.12.6 `publish-mirror` job downloaded the release artifacts successfully, but every upload to the R2 release prefix failed with `AccessDenied` on two attempts. The exact authorization cause has not been established. Restoring upload access and backfilling the 0.12.6 mirror prefix are the concrete next steps.

## Maintainer status

A uv maintainer confirmed on August 26, 2026 that the team is aware the mirror upload failed and plans to remediate it on August 27, 2026. No completed remediation or restored mirror availability has been reported yet.

## Reported impact

A `setup-uv` user reports that the missing mirror assets surface in CI as `Warning: Failed to download from mirror, falling back to GitHub Releases: Unexpected HTTP response: 404`. This shows that the failure is visible to downstream CI users and causes `setup-uv` to attempt its GitHub Releases fallback; the comment does not confirm whether that fallback subsequently completed.

## Draft response

Confirmed: the 0.12.6 checksum exists on GitHub, but the Astral mirror publication failed. The release workflow's R2 uploads returned `AccessDenied` on two attempts; the exact authorization cause has not yet been established.

This incident is already tracked in astral-sh/uv-dev#854, so we'll centralize the investigation there. Once upload access is restored, the 0.12.6 mirror artifacts will need to be backfilled. In the meantime, the checksum is available from the GitHub release URL.

## Classification

This is a `duplicate` of astral-sh/uv-dev#854, which was already open and independently tracks the same uv 0.12.6 mirror-publication failure. The current report describes missing archives and checksums across release variants, while the existing incident records the underlying failure of all uploads for the 0.12.6 mirror prefix.

The underlying behavior is an established correctness problem, not a support question or enhancement: GitHub lists 19 `.sha256` assets for uv 0.12.6, including the reported file; that GitHub asset is available; the 0.12.5 mirror equivalent returns 200; and the 0.12.6 mirror URL returns 404. Because an open issue already tracks this exact regression, `duplicate` takes precedence over `bug`.

## Related

- astral-sh/uv-dev#854 — **Release mirror uploads fail with R2 AccessDenied** (open). This is the canonical current incident. It identifies the uv 0.12.6 `publish-mirror` job, records `AccessDenied` for every R2 upload on two attempts, and calls for restoring access and backfilling the release.
- astral-sh/uv#19282 — **Bug: Primary download URL returns 404 for uv 0.11.9 on x86_64 Linux** (closed). This is the closest historical recurrence. The 0.11.9 mirror artifacts and then their `.sha256` sidecars were missing until maintainers backfilled them.
- astral-sh/uv#19297 — **uv version 0.11.10 give a 404 error when runnning the installation script** (closed). The next release also initially returned 404 for every path under its mirror prefix and was repaired.
- astral-sh/uv#18159 — **Upload uv releases to a mirror** (merged pull request). This introduced the release-artifact upload workflow for the Astral mirror; the current incident identifies its `publish-mirror` job as the failing subsystem.

## Search and supporting evidence

The report was decomposed into the exact release (`0.12.6`), artifact types (release archives and `.sha256` sidecars), observable response (HTTP 404), scope (all release variants), and subsystem (`releases.astral.sh` release mirroring). Literal and conceptual searches covered the mirror hostname, version, checksum terminology, missing release artifacts, CDN and mirror publication, upload failures, and release-workflow terminology across open and closed issues and open, closed, and merged pull requests. Historical fix-oriented inspection followed references among astral-sh/uv#19282, astral-sh/uv#19297, astral-sh/uv-dev#854, and astral-sh/uv#18159.

Direct checks found:

- The GitHub uv 0.12.6 release contains 42 assets, including 19 `.sha256` files.
- `uv-x86_64-unknown-linux-gnu.tar.gz.sha256` is uploaded on GitHub with a 101-byte payload and a recorded SHA-256 digest.
- The reported 0.12.6 Astral mirror URL returns HTTP 404.
- The corresponding GitHub release URL responds with a download redirect.
- The equivalent 0.12.5 Astral mirror URL returns HTTP 200.
- astral-sh/uv-dev#854 records `AccessDenied` from both `PutObject` and `CreateMultipartUpload`, while explicitly leaving the precise authorization cause open.

astral-sh/uv#18798 was a plausible checksum-title match but was ruled out. Its absent powerpc64 checksum resulted from an intentionally removed big-endian PPC64 build target and was resolved by removing that target from the downloads configuration; the supported uv 0.12.6 assets in this report exist on GitHub and failed only during mirror publication. astral-sh/uv#18503 was also inspected but is broader: it tracks adopting the Astral mirror as the primary source rather than a particular failed release upload.
