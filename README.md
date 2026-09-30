# `uv audit` reports "no known vulnerabilities" (exit 0) when the OSV querybatch response has fewer results than queries

Issue: astral-sh/uv#22102

Classification: bug

## Summary

The report demonstrates that `uv audit` can return a false clean result when an OSV
`/v1/querybatch` request receives HTTP 200 with fewer `results` than submitted `queries`. The
unmatched dependencies are not audited, but the command counts them as audited, prints `Found no
known vulnerabilities ...`, and exits 0.

The current source confirms the behavior. `QueryBatchResponse.results` is an unconstrained vector,
and `query_identifiers` pairs it with `pending_batch` using `zip`. Rust's `zip` ends with the shorter
iterator, so no error is produced for missing response entries. The resulting finding set can be
empty, after which the text renderer reports success for the original package count. OSV's API
documentation states that batch-response ordering is guaranteed to match the input, making a
query/result cardinality mismatch an invalid service response rather than evidence that the omitted
packages have no vulnerabilities.

No existing issue or pull request tracks this exact false-success case. The closest history is the
batch-query implementation and its later chunking refactor, together with earlier work that
intentionally surfaces malformed OSV data instead of skipping it.

## Draft response

Confirmed: this is a bug. The current batch-response path accepts any `results` length and zips it
with the pending queries, so a short response drops unmatched packages and can incorrectly render a
clean audit with exit 0. OSV documents that response ordering matches the input, and
astral-sh/uv#19515 likewise establishes that malformed OSV data should be surfaced rather than
skipped.

I don't see an existing issue tracking this cardinality mismatch. The concrete next step is to
reject unequal query/result counts as a service error and add regression coverage for short
responses, including paginated and chunked batches.

## Classification

This is a `bug`, not an enhancement or question. The repository source confirms a correctness
failure: a structurally decodable but incomplete OSV response silently removes dependencies from
the vulnerability lookup while `uv audit` presents a successful result for the full package count.
That is especially misleading for a command used as a CI gate. The trigger may be an invalid
response from OSV or an intermediary, but uv should not interpret missing audit results as negative
results.

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

- `crates/uv-audit/src/service/osv.rs` deserializes `results` without a length constraint and pairs
  `pending_batch.iter()` with `batch_response.results.iter()` using `zip`.
- The same file's current tests cover correct mappings, pagination, and batches over the OSV limit,
  but every mock returns one result per query; there is no short-response regression case.
- `crates/uv/src/commands/project/audit.rs` renders success when the collected findings contain no
  vulnerability, while its reported package count comes from the full auditable dependency set.
- OSV's `/v1/querybatch` documentation guarantees response ordering matching the input.
- astral-sh/uv#19515 records the existing maintainer decision to surface malformed OSV responses
  because they indicate a service/data problem.
