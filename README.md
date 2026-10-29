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

Reproduced on 2026-10-29 with the installed uv 0.13.5 on Ubuntu 24.04.5,
x86_64 Linux, and CPython 3.12.3. A fresh project containing only
`certifi==2024.12.14` produces the same two PYSEC-2026-4320 entries, conflicting
fixed versions, and count of two vulnerabilities. The reported uv 0.13.6 and
Fedora/Python combination were not run.

Fresh OSV and GitHub advisory reads confirm that the fixed-version conflict is
already present upstream: PYSEC specifies 2026.10.19, while GHSA and GitHub's
advisory specify 2026.10.18. The duplicate presentation/counting and inconsistent
upstream fixed events are separate concerns.

No matching duplicate or historical deduplication fix found. astral-sh/uv#22292 explains the identical display IDs; astral-sh/uv#20399 addresses a different fix-version bug; astral-sh/uv#19492 provides upstream data-quality guidance.


The fix investigation confirmed the parent regression passes with the duplicate
output and fails when changed to expect one finding. No production fix was
retained: reconciling conflicting advisory metadata needs an explicit policy.
The parent fixture also assigns the two fix versions to the opposite sources
from the live records, so choosing the preferred display-ID source would not
establish a correct fix. See the Fix section for validation and limitations.

## Draft response

You're seeing two OSV records for the same vulnerability. uv currently keeps both findings and displays their preferred PYSEC identifier, so it also counts the vulnerability twice. That is a reporting bug; this issue can track deduplicating aliased findings and correcting the counts.

The fix-version mismatch is present upstream: OSV's PYSEC-2026-4320 record specifies 2026.10.19, while its GHSA-6q48-6c7w-9gw2 record and GitHub's advisory specify 2026.10.18. uv reads those fixed-version fields directly. Please report the incorrect fixed event in vulns/certifi/PYSEC-2026-4320.yaml to pypa/advisory-database so the source data can be corrected as well.

## Classification

Bug: the executed audit counts two mutually aliased records for the same
package/version as two vulnerabilities and displays the same preferred PYSEC
identifier twice. This is misleading even though the audit command is
experimental. The reproducer and inspected implementation support treating
alias deduplication/counting as a uv reporting issue.

The conflicting fix versions are present in OSV's source data, and uv displays
those values. GitHub's advisory and the upstream YAML's prose identify
2026.10.18 as removing the affected roots, while the PYSEC fixed event specifies
2026.10.19. Correcting that event is an upstream data concern; no evidence shows
uv inventing the later version.

The prior related-item search found no existing item tracking this same problem.
There is no demonstrated regression of a previous alias-deduplication fix;
the related display-ID and package-scoping changes address different behavior.
The reproduction on 0.13.5 also establishes that the symptom is not exclusive
to the reported 0.13.6 release.

## Reproduction

Outcome: **reproducible**, observed on 2026-10-29 against the live default PyPI
index and OSV service. No mocked advisory responses were used.

### Inputs and environment

- Reported commands: `uv audit` and `uv audit --verbose` from the root of
  rust-lang/fls at `2f683b0678541a1900e8b6fa2877c4eeff27344a`.
- Reported environment: uv 0.13.6, Fedora 44 Kinoite,
  Linux 7.2.13-200.fc44.x86_64, Python 3.11.18.
- The public project metadata at that commit has `requires-python = ">= 3.10"`
  and dependencies `sphinx ~= 8.0` and `sphinx-autobuild >= 0`, with no
  `[tool]` section. The report's lock resolves certifi to 2024.12.14.
  The report also mentions user configuration without providing its contents;
  the minimal reproduction succeeds with configuration discovery disabled.
- Executed environment: installed `uv 0.13.5 (x86_64-unknown-linux-gnu)` at
  `/opt/hostedtoolcache/uv/0.13.5/x86_64/uv`, Ubuntu 24.04.5 LTS,
  Linux 7.0.0-1017-azure x86_64, `/usr/bin/python3` (CPython 3.12.3).
- Fresh standalone project, no dependency groups or workspace members, no
  existing lockfile or environment. The audit creates a two-package lockfile
  and audits its sole registry dependency, certifi. No package build or
  installation is needed.

### Minimal commands

```sh
repro=$(mktemp -d /tmp/uv-22985-XXXXXX)
mkdir "$repro/project"
cd "$repro/project"
cat > pyproject.toml <<'TOML'
[project]
name = "audit-certifi-repro"
version = "0.1.0"
requires-python = ">=3.10"
dependencies = ["certifi==2024.12.14"]
TOML
UV_CACHE_DIR="$repro/cache" UV_PYTHON=/usr/bin/python3 \
  uv audit --no-config --no-build --no-python-downloads
```

The actual invocation used a clean subprocess environment, with temporary,
initially empty uv cache/configuration/data directories and Python-download
storage. The executable was the installed `uv` on PATH. Reproduction files,
raw stdout/stderr, exit status, and downloaded public advisory records are
retained under `/tmp/uv-22985-j9462k1n`.

### Observed result

Exit status: **1**. Stdout contains:

```text
certifi 2024.12.14 has 2 known vulnerabilities:

- PYSEC-2026-4320: certifi bundles emSign root CAs with compromised intermediate CA keys

  Fixed in: 2026.10.18

  Advisory information: https://github.com/certifi/python-certifi/security/advisories/GHSA-6q48-6c7w-9gw2

- PYSEC-2026-4320: certifi bundles emSign root CAs with compromised intermediate CA keys

  Fixed in: 2026.10.19

  Advisory information: https://github.com/advisories/GHSA-6q48-6c7w-9gw2
```

Stderr reports `Resolved 2 packages` and
`Found 2 known vulnerabilities and no adverse project statuses in 1 package`.
This reproduces both the duplicate entry/count and conflicting fix versions.
The expected distinct-vulnerability count for these mutually aliased records
is one; GitHub's advisory identifies 2026.10.18 as the first patched version.
The fresh upstream record checks are detailed below.

### Existing test coverage

Read the setup, mock responses, command invocations, and snapshots in
`crates/uv/tests/build/audit.rs`:

- `audit_best_id_selection`: one CVE record with GHSA and PYSEC aliases displays
  the PYSEC identifier and counts one vulnerability. It does not return two
  records linked by aliases.
- `audit_multiple_vulnerabilities_same_package`: two distinct records,
  VULN-A and VULN-B, with no alias relationship, display separate fix versions
  and count two vulnerabilities. This does not test duplicate identities.
- `audit_ignore_until_fixed_with_fix_for_other_package`: a matching package
  without a fix is ignored even when another package or a package-less affected
  entry has fixed events. This covers package scoping, not conflicting records
  for the same package.

The initial reproduction search found no test for deduplicating alias-linked
records. The checkout now includes
`crates/uv/tests/build/audit.rs::audit_duplicate_vulnerability_aliases` from
astral-sh/uv-dev#3028. This test uses mutually aliased PYSEC and GHSA records
with conflicting fix versions and snapshots the undesirable duplicate output.
Its baseline and temporary desired-behavior assertion were executed during the
fix investigation, as detailed below. The module already has the
`test-python` and `test-pypi` feature gates in `crates/uv/tests/build/main.rs`.

## Related

- astral-sh/uv-dev#3028 — Parent regression pull request. Checkout commit
  `cc087ffaf16f2089004930961c1d2cb4466c7046` adds
  `audit_duplicate_vulnerability_aliases`, which asserts the duplicate output.

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

## Fix

Outcome: **not_fixed**. The checkout is unchanged after the investigation;
no production patch or modified regression was retained.

### Confirmed cause and scope

`Osv::query_batch` deduplicates fetches by exact advisory ID but creates a
finding for each dependency/advisory-ID pair. `Vulnerability::best_id` only
selects the displayed identifier. Consequently, two mutually aliased records
remain two findings with the same displayed PYSEC ID. Text output counts and
prints both, as demonstrated by the integration test.

Reviewed the adjacent ID-selection, distinct-vulnerability, ignore-by-alias,
and package-scoped fix tests in `crates/uv/tests/build/audit.rs`, plus the
vulnerability fixture and assertions in `crates/uv/tests/tool/tool_audit.rs`.
The neighboring fixtures do not contain a second aliased record or prescribe
how conflicting metadata should be reconciled. Both project and tool commands
call `uv_audit_operations::audit_lock`; additional command tests would reuse
the same affected path. JSON serializes the same findings and SARIF emits a
result per finding, with source IDs, aliases, and fix versions exposed to
consumers. These paths were inspected, not independently claimed as newly
demonstrated failures. No speculative variant tests were added.

### Limitation requiring a decision

The parent fixture assigns fixed version 2026.10.18 to the PYSEC record and
2026.10.19 to GHSA. The live records captured during reproduction assign those
versions the other way around. Both fixture records also have the same
modification timestamp. Thus a patch preferring the PYSEC record could satisfy
the fixture while selecting the incorrect 2026.10.19 value in the real case.

The existing PYSEC/GHSA/CVE preference concerns identifier presentation, not
metadata authority. Neither the implementation nor the inspected tests define
a source-precedence or reconciliation policy for conflicting fix versions,
links, summaries, and timestamps. Keeping an arbitrary first record would lose
information; taking the earliest fixed event or combining versions would
introduce a new interpretation of contradictory upstream data. The flattened
`fix_versions` report field does not retain each record's affected ranges and
provenance. Resolving this without silently choosing a security recommendation
requires an explicit metadata policy or a report representation for conflicts.
That decision is outside the authorized scope of this fix attempt.

The discrepancy in the upstream PYSEC fixed event remains a separate data
correction. No GitHub or upstream database changes were made.

### Focused validation

- Ran `cargo test -p uv --test build audit::audit_duplicate_vulnerability_aliases -- --exact`
  on the original parent regression in the debug/test profile: **1 passed**,
  230 filtered out. Its snapshot asserts two entries and two vulnerabilities.
- Temporarily updated that original test's snapshot to expect one entry,
  fixed in 2026.10.18, and counts of one. Ran
  `INSTA_UPDATE=no cargo test -p uv --test build audit::audit_duplicate_vulnerability_aliases -- --exact`:
  **failed for the reported reason**, showing the extra 2026.10.19 entry and
  both counts remaining two. No production code had been changed.
- Restored the original regression byte-for-byte. `git diff --exit-code`
  succeeded and `git status --short` was empty. No Rust source changes remain,
  so no formatting changes were needed or retained.

The temporary desired-behavior diff and original test backup are under
`/tmp/uv-22985-fix-lucbfdts`. The passing check establishes the parent test's
baseline, not a successful production fix. No entire test suite, release build,
or additional command matrix was run.

## Recommended next steps

Decide how a single vulnerability should retain or select metadata from
conflicting alias-linked records, including what to expose in text, JSON, and
SARIF. Correct the regression fixture's source/version association to match
the captured live records before relying on it to validate metadata selection.
Retain a regression asserting a single vulnerability and accurate counts under
the agreed policy. Separately request correction of the PYSEC fixed event
upstream.

## Verification limits

The certifi symptom was observed by executing the installed uv 0.13.5 in a
fresh temporary project. The full rust-lang/fls audit, its other duplicate
advisories and total of 48 findings, uv 0.13.6, Fedora, and Python 3.11.18 were
not independently exercised. The minimal reproduction is sufficient to
establish the reported certifi behavior without that project's unrelated
dependencies or omitted user configuration.

The original reproduction inspected checkout commit
`c76ff10bab78a44da9cd26f81d15c1e5ad018e03`. The fix investigation used the
parent regression checkout at `cc087ffaf16f2089004930961c1d2cb4466c7046` and
built and exercised its uv 0.13.6 debug binary against the mocked OSV service
in the parent integration test. This does not constitute a fresh full-project
or live-service reproduction on 0.13.6. Advisory data can change after the
recorded live observations.

Related-issue/PR search history is retained from the existing investigation.
No GitHub objects, Git configuration, or dependency manifests/lockfiles were
changed. The temporary test edit was restored, leaving no checkout changes.
Only this issue-context README was updated within the issue-context directory;
build artifacts and temporary diagnostic files were produced separately.
