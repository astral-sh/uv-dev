# Incorrect checksum for `uv-x86_64-pc-windows-msvc.zip`?

Issue: astral-sh/uv#23074
Issue URL: https://github.com/astral-sh/uv/issues/23074
Classification: question

## Summary

The reporter asks whether the individual SHA-256 sidecar for the uv 0.13.7
Windows x86_64 ZIP is incorrect, while saying the corresponding entry in
`sha256.sum` is fine. The report provides the sidecar URL, platform, and
version, but no observed digest, verification command, or error output. The
issue was opened on 2026-11-03 at 12:24:15 UTC and had no comments when inspected.

astral-sh/uv#18736 covers a WheelNext installer checksum mismatch, and astral-sh/uv#13074 covers installer verification support. Neither tracks this alleged release-sidecar discrepancy; the current 0.13.7 ZIP and both checksum files agree.

## Draft response

I checked the published 0.13.7 Windows ZIP: its SHA-256 is `0cdc5b38fe5a8168ed6a25814f5ba43c21d313dd3187314fccb37b7cec46e021`, matching both `uv-x86_64-pc-windows-msvc.zip.sha256` and the entry in `sha256.sum`. GitHub's recorded asset digest agrees too. Could you share the checksum value you received, the exact verification command or tool, and its full output? That would help explain the discrepancy you're seeing.

## Classification

The currently published 0.13.7 ZIP hashes to the value recorded in both its individual .sha256 file and sha256.sum, and GitHub's asset digest agrees. The checkout also validates archive bytes against sidecar checksums before publication. No incorrect checksum, source-confirmed correctness defect, or matching existing discussion was established. Clarification requires the value and verification output the reporter received; these checks do not establish what they received earlier.

This classification reflects the checked artifacts, rather than the issue's
question-mark title. An established incorrect checksum would be a bug. There
is no evidence here of a previously fixed checksum bug returning, and the
adjacent installer reports do not justify duplicate classification. Do not
infer that the reporter's earlier download necessarily matched the current
responses, or that caching, signing, or a verification tool caused their result.

## Related

- astral-sh/uv#18736 — Installer script throws checksum mismatch when trying to use the wheelnext download URL on Linux (open issue). Closest observable symptom: checksum verification fails for a uv download. That report uses the official shell installer with INSTALLER_DOWNLOAD_URL pointing to WheelNext on Linux; comments describe replacing the installer's embedded checksum. It does not report disagreement between checksum assets for an official GitHub release, so it is adjacent rather than a duplicate.
- astral-sh/uv#13074 — Verify checksum in installer script (open issue). Tracks installer checksum verification, rather than incorrect published checksum values. Maintainer comments identify cargo-dist as the installer generator and state that shell installers embed archive checksums since 0.11.0, while PowerShell verification was still missing. Useful subsystem context, but it does not track the reported 0.13.7 sidecar discrepancy.

## Supporting evidence

### Claims and search decomposition

The report has one discrepancy claim with two independently checked parts:
the individual `.zip.sha256` file allegedly has an incorrect archive digest,
while the same archive's `sha256.sum` entry is correct. The subsystem is release
artifact publication and verification. The triggering identifiers are uv
0.13.7, Windows x86_64, and `uv-x86_64-pc-windows-msvc.zip`. No uv command,
verification tool, observed error, or requested new capability is specified.

### Published artifact checks

On 2026-11-03, downloaded the archive and both checksum assets using
authenticated `gh release download`, without executing or extracting the ZIP.
Computed SHA-256 with Python's `hashlib`. Also fetched both checksum files from
their public download URLs independently. Both retrieval methods returned the
same digest for the archive.

Release: https://github.com/astral-sh/uv/releases/tag/0.13.7
Published: 2026-11-03 at 02:45:04 UTC.

| Evidence | Observed value |
| --- | --- |
| Downloaded ZIP size | 15,236,317 bytes |
| Computed ZIP SHA-256 | `0cdc5b38fe5a8168ed6a25814f5ba43c21d313dd3187314fccb37b7cec46e021` |
| Individual sidecar's recorded archive SHA-256 | `0cdc5b38fe5a8168ed6a25814f5ba43c21d313dd3187314fccb37b7cec46e021` |
| `sha256.sum` entry's recorded archive SHA-256 | `0cdc5b38fe5a8168ed6a25814f5ba43c21d313dd3187314fccb37b7cec46e021` |
| GitHub ZIP asset digest | `sha256:0cdc5b38fe5a8168ed6a25814f5ba43c21d313dd3187314fccb37b7cec46e021` |

Sidecar URL:
https://github.com/astral-sh/uv/releases/download/0.13.7/uv-x86_64-pc-windows-msvc.zip.sha256

Combined checksum URL:
https://github.com/astral-sh/uv/releases/download/0.13.7/sha256.sum

The sidecar uses two spaces before the filename; the combined file uses a
space followed by `*`. These are the text and binary filename-marker forms
accepted by the checkout's checksum validator; their digest fields are equal.

The downloaded sidecar's own SHA-256 is
`b706970932643830577957ab4a282419341f6b1263877991ba8741960bcfad3b`,
and the downloaded combined file's own SHA-256 is
`3372371e521a7e179fe52a0d9a2736da40eea758d3d0bc017f7ddd96f80fab30`.
These also match their respective GitHub asset digests. They are hashes of
the checksum files themselves, distinct from the archive digest they record.

### Repository source

`scripts/assemble-release-artifacts.py:62` computes the archive SHA-256,
normalizes the text/binary filename marker, and rejects a mismatched digest or
filename. Its assembly loop validates and copies the selected archive and
sidecar together, selecting signed replacements for Windows and macOS.
`.github/workflows/release.yml` consumes those assembled artifacts and
`scripts/patch-dist-manifest-checksums.py` injects sidecar values into the local
dist manifest. This is evidence about the checkout's behavior; it does not
prove what the reporter downloaded earlier.

### Candidate comparisons and reference chains

astral-sh/uv#18736 concerns Linux installation with
`INSTALLER_DOWNLOAD_URL=https://wheelnext.astral.sh`. Its comments report
success after changing the installer's embedded checksum; no comment
establishes a mismatch between official GitHub sidecar and combined checksums.

Maintainers in astral-sh/uv#13074 identify cargo-dist as the installer generator
and link axodotdev/cargo-dist#439. That upstream discussion concerns checksum
verification in fetching installers and how signing can affect embedded
hashes; it does not establish a defect in the current 0.13.7 assets.

Inspected and excluded as duplicate candidates:

- astral-sh/uv#14839: self-update fails with `Text file busy`; its title's
  checksum wording does not identify an incorrect published digest.
- astral-sh/uv#19282 and astral-sh/uv#21302: archives and checksum assets were
  missing from `releases.astral.sh`, producing HTTP 404 responses. This report
  points to an available GitHub asset and alleges incorrect contents.
- astral-sh/uv#10336: Windows executable signing, completed as of 0.12.12
  according to the closing maintainer comment. Followed its reference to
  astral-sh/uv#18280, a closed, unmerged signing proposal. Neither establishes
  a stale checksum or a checksum regression in 0.13.7.
- astral-sh/uv#18261: enabling immutable GitHub releases, confirmed working for
  0.10.8. It concerns release mutability rather than checksum correctness.
- astral-sh/uv#11961: Docker installation received HTML instead of the installer;
  a later comment could no longer reproduce it. It does not concern ZIP hashes.

### Search coverage

Searched astral-sh/uv open and closed issues and open, closed, and merged pull requests using the exact archive name, sha256.sum, 0.13.7 with checksum, and broader checksum, sha256, checksum mismatch, incorrect hash, release digest, and sidecar terms. Separately searched possible causes and fixes using Windows signing, release signing, and internal:releases vocabulary. Inspected candidate bodies and comments, followed astral-sh/uv#13074 to axodotdev/cargo-dist#439 and astral-sh/uv#10336 to the closed, unmerged signing proposal astral-sh/uv#18280. No matching checksum-fix PR was found. Ruled out astral-sh/uv#14839 (Text file busy during self-update), astral-sh/uv#19282 and astral-sh/uv#21302 (missing mirror assets), and astral-sh/uv#18261 (release immutability). Independently compared the published ZIP, both checksum files, GitHub asset digests, and the checkout's archive-validation code.

## Next maintainer step

Request the digest the reporter received, their exact verification command or
tool, and its complete output. Compare that evidence with the currently
matching files before assigning a cause or treating this as a release bug.
No GitHub changes were made, and no checkout files were modified.
