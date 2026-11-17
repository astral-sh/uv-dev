# Refresh Cargo.lock to pick up fixed zstd, zstd-safe, icu_normalizer and ctutils (RustSec advisories)

Issue: astral-sh/uv#23280

Classification: bug

## Summary

The reporter requests a targeted Cargo.lock refresh for uv 0.13.9 and main to replace Rust dependencies covered by published security advisories. A targeted cargo-audit 0.23.0 scan reproduces advisory findings for zstd, both zstd-safe version lines, and ctutils in both the checkout and the public uv 0.13.9 release lockfile. Both scans exit with status 1. The reported icu_normalizer version is present, but its cited advisory could not be independently validated. These observations establish dependency advisory findings, not exploitability in uv or scanner results for every distributed binary.

No duplicate was found within the completed searches. The closest items are the separate RSA/reqsign request, an earlier ICU-tree scanner report, and an older zstd update that was closed without merging. Further search queries encountered an API rate limit.

Maintainer woodruffw has supplied a reachability assessment covering every advisory in the report and considers them unreachable in uv because of disabled features or the way uv uses the affected APIs. Patches remain conditional on evidence of a reachable vulnerability. The reproduced lockfile findings establish dependency advisory matches; they do not contradict that assessment or demonstrate uv exposure. The explanations below are attributed maintainer findings, not independent verification of every call path or feature setting.

## Maintainer position

In the November 17, 2026 comment at 18:29:53 UTC on astral-sh/uv#23280 (comment ID 6957176660), woodruffw, identified as a repository member in the supplied discussion, compared this report to astral-sh/uv#23279. They stated that reachable vulnerabilities can be patched, asked for evidence of reachability, and directed reports of unreachable vulnerabilities to the scanner vendor. They also explained that maintainers already see and triage all RustSec advisories, so additional reports should establish confidence that the affected code is reachable.

The maintainer's assessment on astral-sh/uv#23280 at 19:00:38 UTC on November 17, 2026 (comment ID 6957900939, explicitly marked AI-assisted) expands the earlier check of RUSTSEC-2026-0400 and RUSTSEC-2026-0403 to all reported advisories. Its explanations group into disabled functionality and API usage:

| Basis | Advisories | Maintainer's explanation |
| --- | --- | --- |
| Disabled seekable support | RUSTSEC-2026-0396, RUSTSEC-2026-0398, RUSTSEC-2026-0405 | uv does not enable seekable support. |
| Disabled experimental support | RUSTSEC-2026-0399, RUSTSEC-2026-0404 | uv does not enable experimental support. |
| Disabled multithreading or training | RUSTSEC-2026-0401, RUSTSEC-2026-0402, RUSTSEC-2026-0406 | Dictionary training is disabled for RUSTSEC-2026-0401; experimental and zstdmt are disabled for RUSTSEC-2026-0402; zstdmt is disabled for RUSTSEC-2026-0406. |
| Disabled legacy decoding | RUSTSEC-2026-0403 | Bundled zstd does not enable legacy decoding, which the advisory requires. |
| Stream context lifecycle | RUSTSEC-2026-0391, RUSTSEC-2026-0400 | uv creates a new context for each stream and uses only streaming calls, avoiding the required cross-stream reuse or mixing of one-shot and streaming operations. |
| Initialized output | RUSTSEC-2026-0397 | The output offset remains within initialized memory. |
| ICU API selection | GHSA-3jjw-63vg-v3hh | URL normalization uses a character iterator instead of the affected APIs. |
| Cryptographic API usage | RUSTSEC-2026-0385 | Cloud signing generates HMAC tags without comparing them. |

These are attributed maintainer findings. The supplied comment gives advisory links and concise explanations, but no code locations, feature-graph output, or runtime reproduction. This handoff has not independently verified each exclusion. The assessment covers all reported advisories; the earlier description of the other zstd, ICU and ctutils findings as unaddressed by maintainers is superseded. The existing scans remain valid evidence of dependency advisory matches without establishing an exploitable uv path.

The comment also supplies a more specific upstream ICU advisory location: https://github.com/unicode-org/icu4x/security/advisories/GHSA-3jjw-63vg-v3hh. This is a new investigation lead, not a successfully retrieved advisory record in this handoff; the earlier public lookup failures remain recorded below. No patch, complete lockfile resolution, release commitment, or issue closure is established by the supplied discussion.

## Classification

Classification: bug, concerning corrective dependency maintenance. Actual cargo-audit runs confirm advisory findings for the locked zstd, zstd-safe and ctutils versions. The RustSec records describe memory-exposure and memory-safety defects in the zstd crates and constant-time correctness defects in ctutils 0.4.2. Exploitability in uv remains unverified, and the ICU advisory remains unvalidated. The related reports concern different advisories or an older, unmerged update; no same-problem tracker or previously fixed regression was established.

The issue requests correction of dependencies with source-backed defects rather than new functionality. Treating it as a bug does not assert that every scanner finding is reachable in uv. In particular, the reporter's claim that all uv binaries are affected and the proposed lockfile-only resolution have not been independently demonstrated. There is no evidence that a previously completed fix has regressed.

The bug classification records the existing triage assessment of corrective dependency maintenance; it is not a maintainer confirmation that uv is vulnerable. It is retained as prior triage context. The maintainer now provides reasons for considering every reported advisory unreachable in uv, and no reachable uv vulnerability has been established. Dependency audit reproducibility must not be read as confirmation of an affected uv runtime path.

## Reproduction

Outcome: **reproducible for the dependency advisory findings**, with limits described below.

### Environment and inputs

- Linux x86_64 (`Linux 7.0.0-1014-aws`); Python 3.12.3 was used to copy files, fetch public records, and parse TOML/JSON. No Python project configuration is involved.
- Checkout commit: `1266c354d3758051aac2c01db279ab6ed7f17b9a`; `crates/uv/Cargo.toml` declares uv 0.13.9.
- Installed executable on PATH: `/opt/hostedtoolcache/uv/0.13.7/x86_64/uv`, reporting `uv 0.13.7 (x86_64-unknown-linux-gnu)`. It was used only for version identification; no claim is made that it represents a uv 0.13.9 binary.
- Scanner: official cargo-audit 0.23.0 prebuilt `x86_64-unknown-linux-musl` executable, downloaded from the public rustsec/rustsec release. No uv or Rust package was built.
- RustSec database commit: `233155b4025b854c68a4b199c0e32d7fd53291a2` (2026-11-17), containing 1,527 advisories.
- Inputs: an unchanged temporary copy of the checkout's Cargo.lock and the public `0.13.9` tag's Cargo.lock fetched from `https://raw.githubusercontent.com/astral-sh/uv/0.13.9/Cargo.lock`.
- All downloaded files, scanner outputs, and cache locations are under `/tmp/uv-23280-pnfh988p`.

### Commands and observed results

With the downloaded scanner, cloned advisory database, and two lockfiles in that directory, the explicit audit invocations were:

```sh
repro=/tmp/uv-23280-pnfh988p
CARGO_HOME="$repro/cargo-home" XDG_CACHE_HOME="$repro/cache" \
  "$repro/cargo-audit" audit --no-fetch --no-yanked \
  --db "$repro/advisory-db" --file "$repro/Cargo.lock" --json \
  > "$repro/checkout-audit.json"

CARGO_HOME="$repro/cargo-home-release" XDG_CACHE_HOME="$repro/cache-release" \
  "$repro/cargo-audit" audit --no-fetch --no-yanked \
  --db "$repro/advisory-db" --file "$repro/uv-0.13.9-Cargo.lock" --json \
  > "$repro/release-audit.json"
```

Both commands exited **1**, each reporting 12 vulnerability entries across the complete lockfile, plus informational warnings. The relevant entries were identical:

| Locked dependency | Observed scanner finding | Patched range reported by scanner |
| --- | --- | --- |
| zstd 0.13.3 and 0.14.0 | RUSTSEC-2026-0391 | `^0.13.4` or `>=0.14.3` |
| zstd-safe 7.2.4 and 8.0.0 | RUSTSEC-2026-0398 and RUSTSEC-2026-0403; informational `unsound` warnings include RUSTSEC-2026-0400 and the other reported IDs through RUSTSEC-2026-0406 | `^7.4.0` or `>=8.2.0` |
| ctutils 0.4.2 | RUSTSEC-2026-0385 | `>=0.4.4` |
| icu_normalizer 2.1.1 | No finding in this RustSec database | Not independently established |

The remaining vulnerability entries concern other crates and are outside this issue's requested refresh. The lockfile SHA256 values are `44e089f82bc4f6eb3da7a8642da941ff39986047bc959d0146e63fa369afb3fa` (checkout) and `dab86594a946ebcfa50f8d90b417f0e73679a82e73d0dcfd746184fba7dd278d` (release).

### Scope, missing evidence, and existing tests

The report supplies no scanner name, scanner command/output, binary artifact hashes, failing uv invocation, malicious archive, or runtime failure. A Cargo.lock audit meaningfully reproduces the dependency-maintenance concern, but it does not prove that every distributed uv binary includes affected code or that extracting an archive reaches an advisory's preconditions. No exploit was reproduced. The full targeted Cargo resolution was not attempted, so a lockfile-only fix is not yet demonstrated.

GHSA-3jjw-63vg-v3hh was absent from the cloned RustSec database. Public lookups at `https://api.osv.dev/v1/vulns/GHSA-3jjw-63vg-v3hh` and `https://github.com/advisories/GHSA-3jjw-63vg-v3hh` both returned HTTP 404. The maintainer now supplies an upstream ICU4X security-advisory URL and states that uv bypasses the affected APIs through character-iterator normalization; see Maintainer position. That URL has not been independently retrieved here. Earlier failed lookups and absence from this database do not establish that icu_normalizer is safe.

Searches of `crates/uv/tests/` and `crates/uv-client/tests/it/` found no regression test for these Rust advisory findings or the described zstd memory-exposure preconditions. The setup and assertions of `crates/uv/tests/sync/sync.rs::sync_deprecated_zstd_wheel` were read: it installs an ordinary wheel with `uv sync --frozen` and asserts zero requests for its obsolete `.whl.tar.zst` alternative, so it does not exercise zstd decompression. `crates/uv/tests/it/network.rs::python_install_http_500` checks retry failure before downloading a `.tar.zst` body; it also does not exercise these advisory conditions. These tests were not run because they do not validate Cargo.lock advisory status. No tests or source files were added or modified.

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

The table records the report's requested releases. The RustSec records and actual scans independently confirm the zstd, zstd-safe and ctutils patched ranges. The ICU advisory and requested fixed release remain to be validated; compatibility of the complete proposed resolution was not tested.

- Cargo.lock also contains async-compression 0.4.50 and compression-codecs 0.4.45. The latter depends on zstd 0.14.0 and zstd-safe 8.0.0. zstd 0.13.3 depends on zstd-safe 7.2.4.
- Cargo.toml requests zstd 0.13.3 with default features disabled and async-compression 0.4.12 with gzip, tokio and zstd enabled. A compatible 0.13.x direct zstd update is permitted by that requirement. All transitive requirements and the complete resolution still need validation.
- Published crate archives were downloaded from static.crates.io and verified against their Cargo.lock SHA256 checksums before reading their manifests. idna_adapter 1.2.1 actually requires icu_normalizer `2` and icu_properties `2`, with default features disabled, rather than the report's claimed `^2.2`. digest 0.11.3 permits ctutils `0.4` (optional). zstd 0.13.3 permits zstd-safe `7.1.0`, while zstd 0.14.0 permits zstd-safe `8.0.0`; compression-codecs 0.4.45 permits zstd-safe `8`. These individual requirements allow compatible patched versions, but they do not establish a complete workspace resolution.
- crates/uv-extract/src/stream.rs:936 defines untar_zst; line 942 constructs async_compression::tokio::bufread::ZstdDecoder and passes it to untar_in.
- crates/uv-python-managed/src/downloads.rs imports zstd::stream::read::Decoder. crates/uv-python-managed/build.rs imports zstd::stream::write::Encoder.
- crates/uv/Cargo.toml declares version 0.13.9. The separate public 0.13.9 tag's lockfile was downloaded and audited; it contains the same reported versions.

### Advisory findings and reachability limits

The following public RustSec advisory files were read; their version findings were also checked by cargo-audit against the cloned database:

- https://github.com/RustSec/advisory-db/blob/main/crates/zstd/RUSTSEC-2026-0391.md describes cross-stream data leakage when borrowed contexts are reused after an abandoned stream. The patched ranges are ^0.13.4 and >=0.14.3. Both locked zstd versions produce this finding. Independent context-lifecycle tracing was not completed; the maintainer reports that uv creates a fresh context for each stream, excluding the required reuse.
- https://github.com/RustSec/advisory-db/blob/main/crates/zstd-safe/RUSTSEC-2026-0400.md describes a dictionary use-after-free when one context mixes an unfinished stream with one-shot operations without a reset. The patched ranges are ^7.4.0 and >=8.2.0. A streaming decoder call by itself does not prove these conditions. Maintainer woodruffw reports that uv only uses streaming contexts and concludes that the required one-shot decoding/stream reuse appears unreachable.
- https://github.com/RustSec/advisory-db/blob/main/crates/zstd-safe/RUSTSEC-2026-0403.md describes uninitialized output after a corrupt legacy frame, with a growable Vec destination and buffer positions committed on error. It explicitly excludes zstd::stream::read::Decoder, zstd::decode_all and the bulk API. The patched ranges are ^7.4.0 and >=8.2.0. Maintainer woodruffw reports that zstd-sys/legacy is disabled in uv, excluding the required legacy-frame support. The async-compression buffer/error path for the locked release was not fully traced independently; the maintainer's exclusion relies on the separate legacy-feature prerequisite.
- https://github.com/RustSec/advisory-db/blob/main/crates/ctutils/RUSTSEC-2026-0385.md describes branches in supposedly constant-time mask generation and u128 comparisons. It identifies >=0.4.4 as patched. Independent tracing of affected operations on secrets was not completed; the maintainer states that cloud signing generates HMAC tags without comparing them and assesses this advisory as unreachable in uv.

The global advisories API lookup for GHSA-3jjw-63vg-v3hh returned HTTP 403, and public GitHub/OSV lookups returned HTTP 404. The cloned RustSec database contains no corresponding advisory or icu_normalizer entry. The maintainer subsequently linked the upstream ICU4X security-advisory page and described an unaffected character-iterator normalization path; that record and call path have not been independently verified here. cargo-audit matched all reported zstd-safe IDs in RUSTSEC-2026-0396 through RUSTSEC-2026-0406. Their individual runtime preconditions were not all independently traced; the maintainer assessment above now supplies exclusions for the entire set.

The checksum-verified compression-codecs 0.4.45 archive's `src/zstd/decoder.rs` constructs a raw streaming zstd decoder and uses its streaming run/flush/finish methods. This supports the presence of a streaming decompression path, but does not establish the mixed one-shot/streaming context requirement of RUSTSEC-2026-0400 or the error-buffer exposure requirement of RUSTSEC-2026-0403. The complete async-compression buffer/error path was not traced.

### Prior discussions and timing

In astral-sh/uv#22936, maintainers explained on October 26–27 that quinn-proto was not compiled and that the reported zerovec issues involved parsing ICU data embedded in uv. They chose periodic dependency updates for those findings. These comments apply to those specific advisories and do not dismiss a separate runtime decompression or normalizer defect.

astral-sh/uv#22182 was created and closed without merging on October 5, before the October 16 dates in the inspected zstd-safe advisory records. Its target was zstd 0.14.0 and it included a Cargo.toml change; it is neither a completed fix nor the requested patched-version refresh.

astral-sh/uv#23279 is an open sibling report about RSA through reqsign. Its upstream lead, apache/reqsign#941, is an open requirement update to rsa 0.10.0. The reporter also commented there about API migration failures; this inspection did not independently diagnose those failures. That work does not centralize the four-crate lockfile request here.

## Search coverage and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed and merged PRs using authenticated gh. Separate literal searches covered zstd-safe/zstd, icu_normalizer, ctutils, RUSTSEC-2026-0391/0400/0403/0385 and GHSA-3jjw-63vg-v3hh. Conceptual searches covered decompression vulnerabilities, Unicode security, constant-time operations, RustSec, and Cargo.lock/lockfile security advisories. PR searches included zstd, ICU, ctutils, RustSec, security and Cargo dependency updates; supplemented empty PR search results by inspecting 700 recent PR titles across all states (2026-10-09 through 2026-11-17). Read candidate bodies and comments, followed the dependency dashboard to astral-sh/uv#22182 and the RSA lead to apache/reqsign#941. No existing tracker for the same advisory set was found. Ruled out merged astral-sh/uv#22693 as a security fix: it addresses gzip header validation and zstd no-progress errors. Also inspected astral-sh/uv#18287 and astral-sh/uv#20542, which concern different JWT/quick-xml advisories. Later follow-up searches were rate-limited, so coverage is not exhaustive.

The general dependency dashboard, astral-sh/uv#2658, was a discovery source for the closed zstd PR rather than a canonical tracker for this advisory set. astral-sh/uv#18287 concerned JWT decoding when uv used encoding, and astral-sh/uv#20542 concerned quick-xml vulnerabilities; neither covers these crates or mechanisms. Merged astral-sh/uv#22693 updated async-compression to 0.4.48 for gzip validation and zstd no-progress errors, rather than the reported memory-safety fixes.

## Suggested maintainer follow-up

Use the maintainer's complete advisory assessment as the current investigation baseline. Further work would be useful if it verifies a specific exclusion or supplies concrete evidence contradicting one, such as an affected build enabling a gated feature, unsafe context reuse, an output offset beyond initialized memory, or a call to an affected ICU or ctutils operation. Advisory-specific code references and an enabled-feature graph would make the assessment independently reviewable; a reachable call path or focused runtime reproduction would be needed to establish uv exposure. The upstream ICU4X advisory URL is the next record to consult if independently evaluating that claim. Repeating a Cargo.lock audit does not resolve these questions.

If a finding is demonstrated to be unreachable, the maintainer's direction is to report the false positive to the scanner vendor. If reachability is established, assess a narrowly scoped update covering the affected zstd/zstd-safe version lines and other validated findings. Follow repository guidance to use precise Cargo updates, validate compatibility and advisory status, and run relevant compilation and extraction checks. No patch or next-release inclusion has been committed to in the discussion.

Two actual cargo-audit scans reproduced the dependency findings; no exploit, build, integration-test run, or dependency update was performed. Reproduction files and scanner caches were confined to the temporary directory. Only the issue-context README was revised outside that directory; checkout files and GitHub objects were not changed.
