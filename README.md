# [Antivirus interception]UV has been blacklisted by ESET as a PUA.

Issue: astral-sh/uv#22456

Classification: question

## Summary

The reporter cannot update uv through Scoop because ESET Internet Security blocks the uv 0.13.0 Windows x64 release download. The supplied log identifies a blocked URL and a PUA blacklist entry. The expected outcome is a successful release download; the actual outcome is interception before the update can complete.

Affected URL: https://github.com/astral-sh/uv/releases/download/0.13.0/uv-x86_64-pc-windows-msvc.zip

Related vendor-support guidance and antivirus reports exist, but no exact ESET URL-blacklist duplicate was found. Windows signing predates the affected release. Independent reproduction needs more information about the Windows/ESET setup and access to that environment; the available Linux runner cannot exercise ESET Internet Security’s URL filtering.

## Reproduction

**Outcome: `needs_more_information`.** The supplied log documents the reporter’s ESET interception, but no independent reproduction of the block was performed. Neither an ESET false positive nor a uv defect has been confirmed. Successful execution of uv on Linux would not test this report.

### Reported operation and environment

- Operation: update uv through Scoop. The exact command, Scoop version, bucket/manifest revision, and previously installed uv version are not supplied. The report does not show `uv self update` or a Python/project command.
- Target: uv 0.13.0, archive `uv-x86_64-pc-windows-msvc.zip`, at the affected URL in the summary.
- Platform: Windows, requesting the x64 MSVC artifact. The log’s application path names the Microsoft PowerShell 7.6.6.0 x64 package. The Windows edition/build and actual host architecture are not provided.
- Protection: ESET Internet Security. At `2026/10/10 15:02:39`, the log records `阻止的 URL` (blocked URL) and `PUA 黑名单` (PUA blacklist). The unlabeled log fields do not reliably identify the ESET product or detection database version. Web/HTTPS filtering and potentially unwanted application settings are not supplied.
- Expected: Scoop downloads the release archive and completes the update. Actual, as reported: ESET blocks the URL and the update cannot complete. This occurs during archive retrieval, before running the newly downloaded uv executable.
- Python version and project configuration are absent, but are not needed to test this download-stage interception. No last known-good uv release under the same ESET settings is identified, so an upgrade regression is not established.

### Local observations and scope

Read-only environment checks used the installed uv executable on `PATH`:

```console
$ command -v uv
/opt/hostedtoolcache/uv/0.12.13/x86_64/uv
$ uv --version
uv 0.12.13 (x86_64-unknown-linux-gnu)
$ uname -srm
Linux 6.17.0-1022-azure x86_64
```

Scoop and ESET scanner commands `ecls` and `esets_scan` are not on `PATH`, and `/opt/eset` is absent. PowerShell is available at `/usr/bin/pwsh`, but this is a Linux runner without the reported Windows/ESET setup. No Scoop update, archive download, antivirus scan, or installation/version comparison was attempted. A download without the relevant ESET filtering would not confirm or refute its classification. No reproduction fixture or cache was needed; no checkout files or existing user state were modified.

Reviewed `CHANGELOG.md` for uv 0.13.0 and `changelogs/0.12.x.md` for uv 0.12.12. The release notes confirm that Windows signing predates the affected release; they do not establish ESET’s verdict on this URL or confirm the signature of the blocked archive. `docs/getting-started/installation.md` documents Scoop as a separate installation route. `crates/uv/src/commands/self_update.rs` implements uv’s standalone-installer update path, which is not the reported operation. There is no evidence-backed project, dependency-group, or uv command variant that can substitute for ESET URL filtering.

### Existing test coverage

Searched `crates/uv/tests/` and `crates/uv-client/tests/it/` for ESET, PUA, blacklist, antivirus, Authenticode, and Scoop; no matching coverage was found. Read the setup and assertions in `crates/uv/tests/it/self_update.rs`: `check_self_update` uses axoupdater to install/update uv and asserts that the resulting executable accepts `--version`; `self_update_offline_error` checks uv’s explicit offline error; the dry-run tests use mocked release metadata. These tests do not exercise Scoop or ESET’s URL blacklist and cannot validate this report. They were not run.

### Information needed for a targeted reproduction

Obtain the exact Scoop update command and sanitized terminal output, Scoop version and uv bucket/manifest revision, previously installed uv version, Windows version/architecture, and explicit ESET Internet Security product and detection database/module versions. Include the relevant web/HTTPS and PUA protection settings and a fresh detection log with field headings, timestamp/time zone, blocked URL, and detection name; omit personal identifiers and credentials.

A meaningful reproduction must retry retrieval of the specified uv 0.13.0 archive through Scoop on a Windows machine with the matching ESET configuration, recording whether ESET emits the same URL/PUA event and prevents the update. If an older release is known to succeed under those same conditions, identify it for comparison. ESET’s support ticket and response can clarify whether its classification has changed or whether a uv-side action is warranted; no cause should be inferred from related antivirus reports alone.

## Draft response

Your log shows ESET blocking the uv 0.13.0 download URL during the Scoop update. Please share the exact Scoop command and error, Windows and Scoop versions, and the ESET product/detection database versions and web/PUA protection settings, with personal information and credentials removed. Please also submit the URL and detection log to ESET for review as a suspected false positive, then share their support ticket ID and response here, as described in astral-sh/uv#20792. We have signed Windows release binaries since uv 0.12.12 (astral-sh/uv#10336), but signing does not guarantee that an antivirus vendor will allow a download.

## Classification

This is a vendor-support question under the available classifications. The report concerns ESET’s external URL-classification decision and requests no new uv capability. The reporter’s log documents a blocked download; the block has not been independently reproduced, and neither a uv defect nor a false-positive mechanism is confirmed. A missing local reproduction is not the classification rationale.

No existing issue was found tracking this specific ESET URL block. The broad AV/EDR tracker explicitly distinguishes new vendors and failure modes, and the older ESET comment concerns CI execution rather than URL filtering. Those differences do not establish a duplicate. Windows signing already shipped as a mitigation, but was not a confirmed fix for this blacklist; this report does not establish that a previously fixed uv bug returned.

## Related

- astral-sh/uv#20792 — Windows antivirus/EDR issues (open). Maintainer guidance for antivirus interference recommends contacting the vendor and sharing its support ticket ID. The August 4 clarification treats new vendors or failure modes separately; this thread does not establish the cause of ESET's URL blacklist.
- astral-sh/uv#10079 — Antivirus detects `uv` installer as malicious on Windows (closed). Reports a BitDefender installer block and includes a September 2026 ESET report. That ESET comment concerns CI execution and hidden-file activity, whereas the new report concerns a release URL blocked during download. Maintainer advice is to submit the detection to the antivirus vendor.
- astral-sh/uv#21336 — Uv 0.12.7 and 0.12.6 removed by netskope (closed). Another vendor prevented installation of specific uv releases. Maintainers directed the reporter to vendor support and astral-sh/uv#20792, noting that signatures cannot prevent all false positives. Different vendor and older releases; no evidence of the same ESET blacklist.
- astral-sh/uv#10336 — Sign published executables for Windows (closed). Windows signing shipped in uv 0.12.12 on September 9, 2026, before the affected 0.13.0 release. This establishes the mitigation's status, not a fix for ESET URL blocking or evidence that signing has regressed.

## Supporting evidence

- The issue was opened on October 10, 2026, with no comments or labels at inspection. Its log names ESET's PUA blacklist and the exact GitHub release URL. It reports a Scoop update, not an invocation of uv's self-update command.
- In astral-sh/uv#20792, woodruffw's August 4, 2026 clarification describes the tracker as a place to aggregate vendor support IDs and says new vendors or failure modes should receive separate triage. The same thread's September 9 comment by zanieb confirms signing since uv 0.12.12 and explicitly says this will not immediately resolve antivirus false positives.
- In astral-sh/uv#10079, zanieb's December 21, 2024 comment recommends submitting the software to the antivirus vendor. The September 9, 2026 ESET comment is a user's report about hidden-file activity in CI; it does not diagnose the new release URL block.
- In astral-sh/uv#21336, woodruffw directs the Netskope reporter to the AV/EDR guidance and vendor support, explaining that false positives can occur despite signatures. That issue has question and external labels.
- The uv 0.12.12 release notes, published September 9, 2026, state that Windows executables in release archives and wheels have timestamped Authenticode signatures from Azure Artifact Signing. The affected uv 0.13.0 release was published October 9, 2026. These sources establish release policy and timing; the blocked archive was not downloaded or independently scanned.

Release evidence: https://github.com/astral-sh/uv/releases/tag/0.12.12 and https://github.com/astral-sh/uv/releases/tag/0.13.0

## Search coverage and exclusions

The report was decomposed before searching into one failure chain: a Scoop update on Windows requests a specific uv release archive, ESET classifies its URL under a PUA blacklist, and the update cannot complete. Search identifiers were ESET Internet Security, PUA blacklist, the corresponding Chinese log fragment, the Windows ZIP filename, and uv 0.13.0. Searches for observed blocking were kept separate from possible mitigations such as code signing.

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Literal searches covered ESET, PUA, PUA blacklist, PUA 黑名单, the Windows ZIP filename, Scoop blocked, and 0.13.0 antivirus/malware reports. Conceptual searches covered antivirus, blacklist, potentially unwanted, false positive, blocked URL, web protection, and download blocking, including external-label vocabulary. Fix-oriented PR searches covered signing, Authenticode, antivirus, malware, ESET, and PUA; no matching fix was returned. Inspected candidate comments and followed references through historical Defender reports and signing discussions. Ruled out astral-sh/uv#10428 as an installer-script/documentation request, and astral-sh/uv#1587 and astral-sh/uv#17344 as historical Defender detections. The referenced signing PR astral-sh/uv#18280 was closed unmerged, not an ESET fix. Verified signing timing against the uv 0.12.12 release notes.

Additional inspected chains included astral-sh/uv#10079 to astral-sh/uv#4300 and astral-sh/uv#9144, and astral-sh/uv#15011 to astral-sh/uv#17417 and astral-sh/uv#17344. The historical Defender reports concern file detections or quarantine and vendor definition changes. They do not establish an ESET regression. The signing discussion and its timeline were inspected; astral-sh/uv#18280 is a closed, unmerged proposal and must not be described as the deployed signing fix.

## Maintainer follow-up

Collect the Windows, Scoop, and ESET details listed in the reproduction section, including a fresh sanitized detection log. Ask the reporter to submit the exact URL and detection log to ESET for review and share the support ticket ID and vendor response. That response can establish whether ESET has corrected the classification or identified an actionable issue. The draft does not promise a uv change or claim the ESET detection has already been confirmed as a false positive.

This handoff and the proposed reply are for review only. No GitHub changes were made.
