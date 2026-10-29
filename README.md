# `uv audit` reports duplicate advisory with wrong fix version

Issue: astral-sh/uv#22985

Classification: bug

## Summary

The reporter runs `uv audit` and `uv audit --verbose` against rust-lang/fls at commit
`2f683b0678541a1900e8b6fa2877c4eeff27344a`, using uv 0.13.6 on Fedora 44
(x86_64 Linux) with Python 3.11.18. For certifi 2024.12.14, the output lists
PYSEC-2026-4320 twice, calls these two known vulnerabilities, and gives conflicting
fixed versions of 2026.10.18 and 2026.10.19. Similar duplicates appear for anyio,
h11, idna, jinja2, pygments, requests, starlette, and urllib3. The overall output
reports 48 known vulnerabilities in 34 packages.

Source inspection confirms that findings are not merged across advisory aliases.
Live OSV and GitHub advisory responses confirm the two certifi records have
conflicting fixed-version metadata. These are separate concerns: uv should not
present aliased records as distinct vulnerabilities, and the incorrect upstream
fixed event also needs correction.

No matching duplicate or historical deduplication fix found. astral-sh/uv#22292 explains the identical display IDs; astral-sh/uv#20399 addresses a different fix-version bug; astral-sh/uv#19492 provides upstream data-quality guidance.

## Draft response

You're seeing two OSV records for the same vulnerability. uv currently keeps both findings and displays their preferred PYSEC identifier, so it also counts the vulnerability twice. That is a reporting bug; this issue can track deduplicating aliased findings and correcting the counts.

The fix-version mismatch is present upstream: OSV's PYSEC-2026-4320 record specifies 2026.10.19, while its GHSA-6q48-6c7w-9gw2 record and GitHub's advisory specify 2026.10.18. uv reads those fixed-version fields directly. Please report the incorrect fixed event in vulns/certifi/PYSEC-2026-4320.yaml to pypa/advisory-database so the source data can be corrected as well.

## Classification

Source confirms that alias-linked OSV records remain separate findings and are counted as separate vulnerabilities, producing misleading output. Live OSV records also confirm the reported fixed-version conflict: PYSEC says 2026.10.19 while GHSA and GitHub's advisory say 2026.10.18. No existing item tracks the same problem, and the earlier display-ID and package-scoping fixes did not implement alias deduplication; this is not an established regression of those fixes.

The report is a correctness issue even though the audit command is experimental.
Counting multiple records for the same vulnerability overstates the number of
vulnerabilities. The upstream data problem does not make that count correct.
There is no evidence that a previously implemented alias-deduplication fix has
regressed. The earlier changes below address different behavior.

## Related

- astral-sh/uv#22292 — Honor advisory ID preference in `uv audit` (merged pull request). Merged October 7, before the reported uv 0.13.6 release. It makes PYSEC aliases take precedence over GHSA/CVE display IDs, explaining why separate OSV records now display the same PYSEC identifier. It changes display selection only; it does not deduplicate findings or reconcile fix versions.

- astral-sh/uv#20399 — Scope OSV fixes to matching packages (merged pull request). An earlier incorrect-fix-version correction, merged July 15. It filters OSV fixed events by package and ecosystem. This report instead involves two alias-linked records for the same PyPI package with conflicting fixed events; the earlier fix remains present and does not address this case.

- astral-sh/uv#19492 — uv audit fails with error decoding response body when OSV returns empty range events (closed issue). Related upstream advisory-data precedent: a maintainer specifically advises checking pypa/advisory-database for incorrect or missing fix information. That issue concerned malformed OSV events and was resolved by an upstream backfill; this report concerns successfully parsed, inconsistent records and duplicate counts.

## Supporting evidence

### Report decomposition

- Duplicate presentation and counts: one package/version has two findings with
  the same advisory identity; expected behavior is one finding per distinct
  vulnerability for that package/version.
- Incorrect fix metadata: the same certifi vulnerability is shown with two
  different fixed versions, although the reporter says both linked advisories
  identify 2026.10.18.
- Subsystems: OSV query and record conversion in `uv-audit`, preferred identifier
  selection, and text report rendering in `uv-audit-operations`.
- Trigger: the audit receives separate PYSEC and GHSA records that identify each
  other as aliases. The supplied verbose log shows both IDs being fetched.
  No evidence makes this specific to Fedora or Python 3.11.
- Exact identifiers: PYSEC-2026-4320 and GHSA-6q48-6c7w-9gw2.
  Relevant output fragments are `has 2 known vulnerabilities` and `Fixed in:`.

### Repository source

Inspected checkout commit `c76ff10bab78a44da9cd26f81d15c1e5ad018e03`:

- `crates/uv-audit/src/service/osv.rs:351`, `Osv::query_batch`, deduplicates
  exact IDs for fetching, then creates a finding for every dependency/advisory-ID
  pair. It does not merge records linked by aliases.
- `crates/uv-audit/src/types.rs:146`, `Vulnerability::best_id`, searches primary
  ID and aliases in PYSEC, GHSA, then CVE preference order. Both certifi records
  therefore display PYSEC-2026-4320.
- `crates/uv-audit-operations/src/lib.rs:206`, `render_text`, counts the findings
  vector for the total and each package group, then prints every finding. This
  confirms that duplicate records also inflate the reported counts.
- `crates/uv-audit/src/service/osv.rs:459` extracts `fixed` events from
  matching PyPI package ranges. `crates/uv-audit-operations/src/lib.rs:286`
  prints those versions. The advisory web page is not consulted to determine
  the fixed version.
- `crates/uv/tests/build/audit.rs:467` tests preferred ID selection within a
  single record. The nearby `audit_multiple_vulnerabilities_same_package`
  tests two distinct vulnerabilities. Neither scenario tests two records that
  alias the same vulnerability.

### Live advisory data checked on 2026-10-29

| Source | Alias relationship | Fixed version |
| --- | --- | --- |
| https://api.osv.dev/v1/vulns/PYSEC-2026-4320 | Aliases GHSA-6q48-6c7w-9gw2 | 2026.10.19 |
| https://api.osv.dev/v1/vulns/GHSA-6q48-6c7w-9gw2 | Aliases PYSEC-2026-4320 | 2026.10.18 |
| GitHub advisory API for GHSA-6q48-6c7w-9gw2 | certifi, pip ecosystem | first_patched_version: 2026.10.18 |

The PYSEC OSV record was last modified on October 20; the GHSA record was last
modified on October 26. These timestamps establish the observed record state,
not the cause of their divergence.

The upstream file
https://github.com/pypa/advisory-database/blob/main/vulns/certifi/PYSEC-2026-4320.yaml
also explicitly contains `fixed: 2026.10.19` and lists 2026.10.18 as affected.
Its own prose says 2026.10.18 removed all emSign roots and 2026.10.19 restored
unaffected roots. This independently confirms the discrepancy is in the source
metadata, rather than a uv conversion that invents 2026.10.19.

The GitHub advisory is available at
https://github.com/advisories/GHSA-6q48-6c7w-9gw2.

### Release timing and related history

uv 0.13.6 was published on October 27, before the October 29 report.
astral-sh/uv#22292 merged on October 7 and changed only display-ID preference;
its code remains present. astral-sh/uv#20399 merged on July 15 and its
package/ecosystem filter remains present. Both certifi records describe the
same PyPI package, so the latter fix does not resolve their conflicting data.

In astral-sh/uv#19492, the May 21 maintainer discussion distinguishes malformed
upstream records from uv error reporting and recommends checking
pypa/advisory-database for fix-version data problems. Its follow-up,
astral-sh/uv#19515, improved malformed-record diagnostics without changing
advisory data or deduplicating findings.

## Search coverage and exclusions

Searched astral-sh/uv open/closed issues and open/closed/merged PRs using authenticated gh. Separately searched duplicate advisories/counts and incorrect fix versions: exact PYSEC-2026-4320 and GHSA-6q48-6c7w-9gw2 identifiers, certifi/version literals, audit duplicate/repeated/same vulnerability, vulnerability count, aliases, deduplication, OSV wrong, fixed in, and fix/fixed version. Used area:uv-audit and package-independent queries; supplemented empty PR keyword results with an all-state listing of 5,000 recent PRs covering audit's introduction. Inspected candidate bodies, comments, reviews, relevant diffs, and roadmap references. Ruled out astral-sh/uv#20399 as the same bug, astral-sh/uv#18968 as configuration handling, and astral-sh/uv#19660 with astral-sh/uv#19872 as SARIF output work. astral-sh/uv#18506 is a broad roadmap without a matching deduplication item. No prior fix or open discussion tracking this same problem was found.

Also inspected the implementation chain referenced by astral-sh/uv#18506:
astral-sh/uv#18119, astral-sh/uv#18124, astral-sh/uv#18193,
astral-sh/uv#18392, and astral-sh/uv#18394. The batch-query change deduplicates
network fetches by exact ID; it does not establish a prior fix for merging
alias-linked findings. astral-sh/uv#18970 fixes script handling and extras
traversal, while astral-sh/uv#21153 updates vulnerable dependencies in repository
lockfiles. Neither tracks the reported output bug.

Keyword PR searches returned no results even for known audit work, so the
all-state PR listing was used to identify candidates across the audit
implementation's history. This handoff does not treat those empty keyword
results alone as evidence that no related PR exists.

## Recommended next steps

Track deduplication of alias-linked records and accurate per-package and total
counts in this issue. A future regression test should supply two OSV records
for the same package/version that alias each other; the existing single-record
ID-preference test does not cover this. Any policy for conflicting metadata
must avoid silently choosing an arbitrary fix version.

Separately, request correction of the PYSEC fixed event upstream. The live
records and upstream YAML already provide specific evidence; another full
project reproduction is not needed to classify the report.

## Verification limits

The conclusions use the supplied output, repository source and test inspection,
GitHub issue/PR discussions and diffs, and live advisory API/source reads.
The full rust-lang/fls audit was not executed, and no builds or tests were run.
Only this temporary handoff README was updated; no checkout files or GitHub
objects were changed.
