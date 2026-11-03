# Incorrect checksum for `uv-x86_64-pc-windows-msvc.zip`?

Issue: astral-sh/uv#23074
Issue URL: https://github.com/astral-sh/uv/issues/23074
Classification: question

## Summary

The reporter asks whether the individual SHA-256 sidecar for the uv 0.13.7
Windows x86_64 ZIP is incorrect, while saying the corresponding entry in
`sha256.sum` is fine. The report provides the sidecar URL, platform, and
version, but no observed digest, verification command, or error output. The
issue was opened on 2026-11-03 at 12:24:15 UTC. The reporter subsequently
confirmed that the checksum is correct and retracted the original claim.

astral-sh/uv#18736 covers a WheelNext installer checksum mismatch, and astral-sh/uv#13074 covers installer verification support. Neither tracks this alleged release-sidecar discrepancy. A targeted reproduction is **not_reproducible**: an independent public download of the exact 0.13.7 ZIP passes verification against both checksum files.

## Resolution status

The issue author, 0a5d8, confirmed on 2026-11-03 at 12:44:34 UTC that the
checksum is correct and the initial report was mistaken. This agrees with the
independent artifact verification below. The clarification resolves the
reported discrepancy; it supplies no explanation of the original observation
or verification tool. No release defect or code change is established.

Source: https://github.com/astral-sh/uv/issues/23074#issuecomment-6604107425

## Classification

Classification remains **question**; reproduction outcome is **not_reproducible**. The report identifies the exact release assets, so their consistency can be tested despite the missing verification command.

The currently published 0.13.7 ZIP hashes to the value recorded in both its individual .sha256 file and sha256.sum, and GitHub's asset digest agrees. The checkout also validates archive bytes against sidecar checksums before publication. No incorrect checksum, source-confirmed correctness defect, or matching existing discussion was established. The reporter now confirms the checksum is correct, resolving the original question. The checks do not establish what they received earlier, and the comment does not identify the source of their initial mistake.

This classification reflects the checked artifacts, rather than the issue's
question-mark title. An established incorrect checksum would be a bug. There
is no evidence here of a previously fixed checksum bug returning, and the
adjacent installer reports do not justify duplicate classification. Do not
infer that the reporter's earlier download necessarily matched the current
responses, or that caching, signing, or a verification tool caused their result.

## Reproduction

Outcome: **not_reproducible** for the currently published assets named in the report. Expected behavior is that the individual sidecar and the matching `sha256.sum` entry both record the ZIP's SHA-256. That is the observed behavior; no checksum discrepancy or verification failure was reproduced.

### Environment and commands

Reported target: Windows x86_64, uv release 0.13.7. No Python version, uv command, checksum-verification tool, observed digest, or error output was supplied. The report does not describe an upgrade regression or a last known-good release.

Reproduction host: Linux x86_64; GNU coreutils `sha256sum` 9.4; Python 3.12.3. The installed executable on `PATH` reports `uv 0.13.6 (x86_64-unknown-linux-gnu)` and was used only for the version check. This is a check of the exact Windows 0.13.7 archive bytes, which does not require running uv or a Windows executable. The reporter's Windows verification tool was not reproduced.

All downloads were placed in `/tmp/uv-23074-reproduction.zVvnqb`. Equivalent minimal commands, run from a temporary directory:

```sh
reproduction_dir=$(mktemp -d /tmp/uv-23074-reproduction.XXXXXX)
cd "$reproduction_dir"
curl --fail --silent --show-error --location --output uv-x86_64-pc-windows-msvc.zip https://github.com/astral-sh/uv/releases/download/0.13.7/uv-x86_64-pc-windows-msvc.zip
curl --fail --silent --show-error --location --output uv-x86_64-pc-windows-msvc.zip.sha256 https://github.com/astral-sh/uv/releases/download/0.13.7/uv-x86_64-pc-windows-msvc.zip.sha256
curl --fail --silent --show-error --location --output sha256.sum https://github.com/astral-sh/uv/releases/download/0.13.7/sha256.sum
sha256sum uv-x86_64-pc-windows-msvc.zip
sha256sum --check uv-x86_64-pc-windows-msvc.zip.sha256
sha256sum --check --ignore-missing sha256.sum
```

Both verification commands exit 0 and print:

```text
uv-x86_64-pc-windows-msvc.zip: OK
```

Only the reported archive was downloaded, so `--ignore-missing` skips other release archives listed in `sha256.sum`; it still verifies the Windows ZIP. An independent Python `hashlib` calculation also matched both recorded values. No project, dependency groups, Python environment, build, or cache was needed.

### Observed artifacts

Earlier context collection used `gh release download` and recorded the GitHub asset digests below. The independent reproduction on 2026-11-03 downloaded all three assets from their public URLs with `curl`, without extracting or executing the ZIP. GNU `sha256sum` and Python 3.12.3 `hashlib` computed the same archive digest. Both checksum-file verification commands succeeded. These observations agree with the earlier artifact checks.

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

### Existing test coverage and limits

Searched `crates/uv/tests/` and `crates/uv-client/tests/it/` for the archive name, `.zip.sha256`, `sha256.sum`, and release-checksum assembly references; no test directly covering agreement between the published release archive, sidecar, and combined checksum file was found. Read `crates/uv/tests/it/self_update.rs`: `check_self_update` installs and updates uv through `axoupdater::test::helpers::perform_runtest`, then asserts the resulting binary's `--version` succeeds. It does not compare these two published checksum files or pin release 0.13.7. The other tests there use offline errors or mocked update metadata and dry-run output, rather than validating these asset contents. No existing test is claimed to cover the reported discrepancy. The reproduction used direct artifact verification; no Rust tests were run or added.

These checks establish consistency of the assets returned at verification time, not what the reporter previously downloaded. The reporter has since confirmed that the checksum is correct, so further reproduction details are no longer needed to resolve this report. No cause such as caching, signing, or tool behavior has been established.

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

The reporter has withdrawn the checksum concern, and independent verification
found no discrepancy. The issue can be closed as resolved without a code
change; no further reproduction request is needed unless new contradictory
evidence is supplied. The supplied discussion does not establish whether the
issue has already been closed.

No GitHub changes were made, and no checkout files were modified.
