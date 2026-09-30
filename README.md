# `uv audit` reports "no known vulnerabilities" (exit 0) when the OSV querybatch response has fewer results than queries

Issue: astral-sh/uv#22102

Classification: bug

## Summary

The reported behavior is reproducible: `uv audit` returns a false clean result when an OSV
`/v1/querybatch` request receives HTTP 200 with fewer `results` than submitted `queries`. In the
targeted reproduction, the one locked dependency has known vulnerabilities according to the real
OSV service, but a mock service that receives one query and returns `{"results": []}` makes uv
report that the package has no known vulnerabilities and exit 0.

The prior implementation deserialized `QueryBatchResponse.results` as an unconstrained vector and
paired it with `pending_batch` using `zip`. Rust's `zip` ends with the shorter iterator, so no error
was produced for missing response entries. The resulting finding set could be empty, after which
the text renderer reported success for the original package count. The fix now verifies the
response cardinality before pairing results with queries and returns a service error for any
mismatch.

No existing issue or pull request tracks this exact false-success case. The closest history is the
batch-query implementation and its later chunking refactor, together with earlier work that
intentionally surfaces malformed OSV data instead of skipping it.

## Reproduction

Outcome: **reproducible** with the installed `uv 0.12.13 (x86_64-unknown-linux-gnu)` on Linux
x86_64 (`6.17.0-1022-azure`). Python 3.12.3 was used only to run the local HTTP mock. All project,
cache, managed-Python, and mock-server files were isolated under `$RUNNER_TEMP`.

The minimal project was:

```toml
[project]
name = "repro"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = ["python-multipart==0.0.6"]
```

After creating `uv.lock` with `uv lock`, a local mock accepted `POST /v1/querybatch`, counted the
request's `queries`, and returned HTTP 200 with `{"results": []}`. The audit was run with isolated
configuration and cache directories; reduced to its material arguments, the command was:

```console
$ uv audit --frozen --preview-features audit-command \
    --service-url http://127.0.0.1:<PORT>
Found no known vulnerabilities and no adverse project statuses in 1 package
$ echo $?
0
```

The mock recorded `{"path":"/v1/querybatch","query_count":1}`, confirming that the response had
zero results for one submitted query. As a control, the same frozen lock audited against the real
OSV service reported 18 known vulnerabilities for `python-multipart 0.0.6` and exited 1. The exact
number of current OSV records is time-dependent, but it confirms that this fixture was vulnerable
at reproduction time and that the short response changed the outcome to a false clean result.

Before the parent regression was added, existing tests did not cover unequal query/result
cardinality. In `crates/uv/tests/build/audit.rs`, `audit_no_vulnerabilities` verifies a valid
one-query/one-empty-result response and exit 0, while `audit_vulnerability_found` verifies a valid
one-query/one-result response and exit 1. In `crates/uv-audit/src/service/osv.rs`,
`test_query_identifiers` checks positional mapping with two queries and two results,
`test_query_identifiers_batch_limit` checks chunking with one generated result per query, and
`test_query_batch_pagination` checks equal-length initial and paginated responses. The parent
`audit_missing_batch_result` integration test now covers the missing-result response and requires
exit 2.

## Fix

Outcome: **fixed**. `query_identifiers` now compares the number of decoded OSV results with the
number of queries in each submitted batch before using positional pairing. A mismatch returns a
dedicated `InvalidBatchResponse` error containing both counts, so the command reports the invalid
service response and exits 2 instead of presenting an incomplete audit as clean. This applies to
initial, chunked, and paginated batch requests at the common response-consumer boundary.

The parent integration test `audit_missing_batch_result` was updated from the reproduced exit-0
snapshot to require the new exit-2 error. No separate command-level tests were added because project,
script, and tool audits all reuse this same OSV consumer, and no distinct producer/consumer path or
additional manifestation was found. Focused validation passed for the parent regression, valid
empty and vulnerable end-to-end audit responses, all seven OSV service tests (including mapping,
batch limits, pagination, and malware filtering), and Rust formatting.

## Scope and open question

A maintainer asked whether the reporter encountered a short OSV response in production or only
demonstrated it with the synthetic service in the issue reproduction. The discussion does not yet
contain the reporter's answer, so a real-world occurrence is not established. The local targeted
reproduction establishes the uv behavior independently, but it does not establish that OSV or an
intermediary has emitted such a response in practice.

The maintainer also noted an important trust boundary: cardinality validation detects a
structurally incomplete batch response, but it cannot make an arbitrarily manipulated
vulnerability-service response trustworthy. A service capable of returning fabricated data could
still return one empty result for every query and thereby produce a clean audit. The fix is scoped
to enforcing the API's query/result shape invariant and preventing this detectable incomplete
response from being interpreted as success; it is not a general defense against a compromised or
malicious service.

## Draft response

Confirmed by a targeted reproduction and fixed: this was a bug. The batch-response path now rejects
unequal query and result counts before positional pairing, so a short response produces a service
error with exit 2 instead of a clean audit. OSV documents that response ordering matches the input,
and astral-sh/uv#19515 likewise establishes that malformed OSV data should be surfaced rather than
skipped.

I don't see another issue tracking this cardinality mismatch. The implementation now rejects
unequal query/result counts at the shared batch-response boundary, including paginated and chunked
requests, and the parent regression covers the reported short-response failure.

## Classification

This is a `bug`, not an enhancement or question. The targeted reproduction observes the
correctness failure: a structurally decodable but incomplete OSV response removes a dependency from
the vulnerability lookup while `uv audit` presents a successful result for the full package count.
The prior source's unchecked, truncating `zip` was consistent with that observation. This is
especially misleading for a command used as a CI gate. The trigger may be an invalid response from
OSV or an intermediary, but uv should not interpret missing audit results as negative results.

It is not a duplicate. Searches found no open issue or pull request tracking this response-length
mismatch, and the related closed work covers different service failures or the implementation
history rather than this defect.

## Related

- astral-sh/uv#18394 — **Merged pull request, “Switch to batched OSV queries for `uv audit`.”**
  This introduced the positional query/result `zip` without checking response cardinality. It is
  the historical origin of the behavior, not an existing tracker for the reported failure.
- astral-sh/uv#20398 — **Merged pull request, “Batch large OSV queries.”** This refactored the same
  path into chunks of at most 1,000 queries and retained the truncating `zip`. Its test verifies
  valid equal-length responses and service-limit batching, but does not cover missing results.
- astral-sh/uv#19492 — **Closed issue, “uv audit fails with error decoding response body when OSV
  returns empty range events.”** This is an adjacent malformed-OSV-data case. It produced a hard
  decoding error rather than a false clean result and was resolved by an upstream OSV data change,
  so it is not the same bug.
- astral-sh/uv#19515 — **Merged pull request, “uv audit: specialize malformed OSV error.”** This
  followed astral-sh/uv#19492. Its maintainer discussion explicitly rejected skipping malformed OSV
  data because surfacing service problems is important, which supports fail-closed handling here;
  it did not add query/result cardinality validation.

## Search scope

Searches covered open and closed issues and open, closed, and merged pull requests. Literal searches
included `querybatch`, `query_identifiers`, `no known vulnerabilities`, `exit 0`, `results`,
`queries`, and `service-url`. Conceptual searches covered false negatives, incomplete or truncated
responses, empty results, response cardinality, malformed OSV data, proxy behavior, service errors,
and audit exit codes. Fix-oriented review covered the source history for the OSV client and the
pull requests that introduced batch querying, chunked large requests, malformed-record errors, and
nonzero audit exits.

The general roadmap in astral-sh/uv#18506 does not track response completeness. astral-sh/uv#19876
was also inspected because it mentions `/v1/querybatch` and a proxy, but it concerned a TLS
`UnknownIssuer` failure before any HTTP response and was withdrawn after the reporter could not
substantiate a uv defect. astral-sh/uv#18512 only defines a nonzero exit for actual findings; it
does not cover incomplete service responses.

## Supporting evidence

- `crates/uv-audit/src/service/osv.rs` now checks `batch_response.results.len()` against the current
  `pending_batch.len()` before pairing entries with `zip` and returns `InvalidBatchResponse` when
  they differ.
- `crates/uv/tests/build/audit.rs::audit_missing_batch_result` sends one query to a mock returning
  zero results and verifies the command reports the mismatched counts and exits 2.
- `crates/uv/src/commands/project/audit.rs` renders success when the collected findings contain no
  vulnerability, while its reported package count comes from the full auditable dependency set.
- OSV's `/v1/querybatch` documentation guarantees response ordering matching the input.
- astral-sh/uv#19515 records the existing maintainer decision to surface malformed OSV responses
  because they indicate a service/data problem.

Pull request: https://github.com/astral-sh/uv-dev/pull/2216
