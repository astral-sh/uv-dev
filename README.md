# Refresh Cargo.lock to pick up fixed zstd, zstd-safe, icu_normalizer and ctutils (RustSec advisories)

Issue: astral-sh/uv#23280

Classification: bug

## Summary

The reporter requests a targeted Cargo.lock refresh for uv 0.13.9 and main to replace Rust dependencies covered by published security advisories. They report scanner findings in distributed binaries and argue that some zstd decompression advisories affect .tar.zst extraction. The checkout confirms every listed locked version and the use of async-compression's ZstdDecoder for .tar.zst extraction. It does not establish the advisory-specific preconditions or exploitability in uv.

No duplicate was found within the completed searches. The closest items are the separate RSA/reqsign request, an earlier ICU-tree scanner report, and an older zstd update that was closed without merging. Further search queries encountered an API rate limit.

## Draft response

The checkout confirms the locked versions you listed. Please send a targeted Cargo.lock update covering both zstd/zstd-safe version lines, ICU and ctutils, with an advisory check of the resulting dependency graph. This can be reviewed separately from astral-sh/uv#23279. The .tar.zst extraction path does use ZstdDecoder, but the cited zstd-safe advisories require specific context or buffer usage; that alone does not establish that uv is affected. Please include evidence for those preconditions if you have it. Inclusion in the next release will depend on review and validation.

## Classification

Cargo.lock confirms the reported dependency versions, and the inspected RustSec records establish memory-safety defects in those zstd-safe versions and constant-time correctness defects in ctutils 0.4.2. This is corrective dependency maintenance, even though exploitability in uv remains unverified. The related reports concern different advisories or an older, unmerged update; no same-problem tracker or previously fixed regression was established.

The issue requests correction of dependencies with source-backed defects rather than new functionality. Treating it as a bug does not assert that every scanner finding is reachable in uv. In particular, the reporter's claim that all uv binaries are affected and the proposed lockfile-only resolution have not been independently demonstrated. There is no evidence that a previously completed fix has regressed.

## Related

- astral-sh/uv#23279 — Move to an rsa >= 0.10 (RUSTSEC-2023-0071) once reqsign supports it (issue, open). Companion report about vulnerable Rust dependencies flagged in uv binaries. It tracks rsa through reqsign and an upstream API migration, whereas this issue requests updates to different crates within their existing version lines; it is not the same remediation.
- astral-sh/uv#22936 — Please update 'quinn-proto',  'zerovec-derive', 'zerovec' (issue, closed). Earlier binary-scanner dependency report involving the ICU dependency tree. Maintainers established that those specific findings affected uncompiled code or parsing of embedded ICU data and chose periodic updates. That conclusion does not establish reachability or safety for the different normalizer, zstd and ctutils advisories reported here.
- astral-sh/uv#22182 — Update Rust crate zstd to 0.14.0 (pull request, closed). Prior zstd update found through the dependency dashboard. Closed without merging on 2026-10-05; it changed Cargo.toml and Cargo.lock to zstd 0.14.0, itself one of the versions flagged by this report. It predates the inspected October 16 zstd-safe advisories and does not supply the requested patched versions.

## Report decomposition

- zstd and zstd-safe: replace both locked version lines; reported memory-safety advisories include RUSTSEC-2026-0391 and RUSTSEC-2026-0396 through RUSTSEC-2026-0406. The claimed trigger is decompression of .tar.zst archives through async-compression.
- icu_normalizer: update the ICU/IDNA dependency tree for GHSA-3jjw-63vg-v3hh.
- ctutils: update constant-time utilities for RUSTSEC-2026-0385.
- Requested maintenance: use semver-compatible dependency updates to remove scanner findings, preferably in the next release.
- Companion report: assess the relationship to astral-sh/uv#23279 independently, since its RSA migration has an upstream dependency constraint.

## Supporting evidence

### Locked dependencies and source paths

| Dependency | Confirmed in Cargo.lock | Requested fixed release |
| --- | --- | --- |
| zstd | 0.13.3 and 0.14.0 | 0.13.4 and 0.14.3 |
| zstd-safe | 7.2.4 and 8.0.0 | 7.4.0 and 8.2.0 |
| icu_normalizer | 2.1.1 | 2.3.0 |
| ctutils | 0.4.2 | 0.4.4 |

The requested releases are taken from the report. The inspected RustSec records independently confirm the zstd-safe and ctutils patched ranges; the other requested release numbers remain to be validated.

- Cargo.lock also contains async-compression 0.4.50 and compression-codecs 0.4.45. The latter depends on zstd 0.14.0 and zstd-safe 8.0.0. zstd 0.13.3 depends on zstd-safe 7.2.4.
- Cargo.toml requests zstd 0.13.3 with default features disabled and async-compression 0.4.12 with gzip, tokio and zstd enabled. A compatible 0.13.x direct zstd update is permitted by that requirement. All transitive requirements and the complete resolution still need validation.
- Cargo.lock shows idna_adapter 1.2.1 depending on icu_normalizer and digest 0.11.3 depending on ctutils. Cargo.lock does not contain the transitive version requirements. The report's assertion that the locked idna_adapter requires icu_normalizer ^2.2 is not established by the lockfile and should be checked against the actual package manifest.
- crates/uv-extract/src/stream.rs:936 defines untar_zst; line 942 constructs async_compression::tokio::bufread::ZstdDecoder and passes it to untar_in.
- crates/uv-python-managed/src/downloads.rs imports zstd::stream::read::Decoder. crates/uv-python-managed/build.rs imports zstd::stream::write::Encoder.
- crates/uv/Cargo.toml declares version 0.13.9. The separate release tag's lockfile was not inspected.

### Advisory findings and reachability limits

The public RustSec advisory files were read through the GitHub contents API:

- https://github.com/RustSec/advisory-db/blob/main/crates/zstd-safe/RUSTSEC-2026-0400.md describes a dictionary use-after-free when one context mixes an unfinished stream with one-shot operations without a reset. The patched ranges are ^7.4.0 and >=8.2.0. A streaming decoder call by itself does not prove these conditions.
- https://github.com/RustSec/advisory-db/blob/main/crates/zstd-safe/RUSTSEC-2026-0403.md describes uninitialized output after a corrupt legacy frame, with a growable Vec destination and buffer positions committed on error. It explicitly excludes zstd::stream::read::Decoder, zstd::decode_all and the bulk API. The patched ranges are ^7.4.0 and >=8.2.0. The async-compression buffer/error path for the locked release was not fully traced.
- https://github.com/RustSec/advisory-db/blob/main/crates/ctutils/RUSTSEC-2026-0385.md describes branches in supposedly constant-time mask generation and u128 comparisons. It identifies >=0.4.4 as patched. Which affected operations uv invokes on secrets was not established.

Retrieving GHSA-3jjw-63vg-v3hh from the global advisories API returned HTTP 403 (resource inaccessible to this integration); no independent conclusion about that advisory's mechanism is made. A lookup for an icu_normalizer directory in RustSec/advisory-db returned HTTP 404. Other zstd advisories in the reported range were not individually validated.

An upstream default-branch compression-codecs decoder source was also inspected. It uses a raw streaming zstd decoder, but it was not pinned to the locked release and is insufficient to establish uv's exposure.

### Prior discussions and timing

In astral-sh/uv#22936, maintainers explained on October 26–27 that quinn-proto was not compiled and that the reported zerovec issues involved parsing ICU data embedded in uv. They chose periodic dependency updates for those findings. These comments apply to those specific advisories and do not dismiss a separate runtime decompression or normalizer defect.

astral-sh/uv#22182 was created and closed without merging on October 5, before the October 16 dates in the inspected zstd-safe advisory records. Its target was zstd 0.14.0 and it included a Cargo.toml change; it is neither a completed fix nor the requested patched-version refresh.

astral-sh/uv#23279 is an open sibling report about RSA through reqsign. Its upstream lead, apache/reqsign#941, is an open requirement update to rsa 0.10.0. The reporter also commented there about API migration failures; this inspection did not independently diagnose those failures. That work does not centralize the four-crate lockfile request here.

## Search coverage and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed and merged PRs using authenticated gh. Separate literal searches covered zstd-safe/zstd, icu_normalizer, ctutils, RUSTSEC-2026-0391/0400/0403/0385 and GHSA-3jjw-63vg-v3hh. Conceptual searches covered decompression vulnerabilities, Unicode security, constant-time operations, RustSec, and Cargo.lock/lockfile security advisories. PR searches included zstd, ICU, ctutils, RustSec, security and Cargo dependency updates; supplemented empty PR search results by inspecting 700 recent PR titles across all states (2026-10-09 through 2026-11-17). Read candidate bodies and comments, followed the dependency dashboard to astral-sh/uv#22182 and the RSA lead to apache/reqsign#941. No existing tracker for the same advisory set was found. Ruled out merged astral-sh/uv#22693 as a security fix: it addresses gzip header validation and zstd no-progress errors. Also inspected astral-sh/uv#18287 and astral-sh/uv#20542, which concern different JWT/quick-xml advisories. Later follow-up searches were rate-limited, so coverage is not exhaustive.

The general dependency dashboard, astral-sh/uv#2658, was a discovery source for the closed zstd PR rather than a canonical tracker for this advisory set. astral-sh/uv#18287 concerned JWT decoding when uv used encoding, and astral-sh/uv#20542 concerned quick-xml vulnerabilities; neither covers these crates or mechanisms. Merged astral-sh/uv#22693 updated async-compression to 0.4.48 for gzip validation and zstd no-progress errors, rather than the reported memory-safety fixes.

## Suggested maintainer follow-up

Review a narrowly scoped lockfile update that covers both zstd/zstd-safe lines and validates ICU and ctutils compatibility. Follow the repository guidance to use precise Cargo updates rather than refreshing unrelated dependencies. Check the resulting advisory status and enabled feature graph, then run relevant compilation and extraction checks. Assess the zstd context, legacy-frame and output-buffer preconditions before characterizing impact or release urgency.

This handoff does not contain a dependency patch or a reproduced exploit. No builds or tests were needed for the read-only investigation. Only this temporary issue-context README was authored; no checkout files or GitHub objects were changed.
