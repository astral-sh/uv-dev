# Direct URL requirements with a `sig` query parameter are reinstalled on every run: `direct_url.json` stores the redacted URL (regression in 0.12.8)

Issue: astral-sh/uv#21969

Classification: bug

## Summary

The report shows a repeatable regression for direct wheel URLs whose query contains `sig`. uv 0.12.7 writes the requested value to `direct_url.json` and recognizes the installation on the next run. Starting with 0.12.8 and continuing through 0.12.18, the installed metadata contains `sig=****`; every later `uv pip install` or dry run consequently plans an uninstall and reinstall. When redaction is active, the URL formatter also reserializes the full query, so otherwise unchanged values such as timestamps acquire percent encoding.

Repository evidence corroborates the reported mechanism:

- astral-sh/uv#21360 added `sig` to sensitive display parameters for the 0.12.8 release. Its stated contract and test require the original request URL to remain intact.
- `DisplaySafeUrl` masks sensitive query values when formatted. If it encounters one, it rebuilds the complete query with a form serializer, which accounts for the observed encoding changes in neighboring parameters.
- The `DirectUrl` conversion for a parsed archive currently calls the wrapped URL's display conversion when populating the string later serialized to `direct_url.json`. This crosses the display/persistence boundary and stores the masked representation.
- Installer satisfaction checking parses the installed metadata URL and compares it with the requested canonical URL. Replacing a real query value with `****` makes those identities differ, so uv rejects its own installed distribution on every run.
- astral-sh/uv#21755 independently added the same `sig` display rule in 0.12.16 without changing metadata construction. It therefore did not address the regression, consistent with the report through 0.12.18.

No open issue or pull request already tracks this exact `sig` metadata regression.

## Draft response

Thanks for the clear reproduction. This is a bug introduced by the `sig` display-redaction change in astral-sh/uv#21360. The current metadata path formats the wrapped URL when constructing `direct_url.json`, so the display-only redaction is persisted as `sig=****` and the query is reserialized. The next satisfaction check compares that stored URL with the requested URL and treats the installed distribution as mismatched.

astral-sh/uv#11082 and astral-sh/uv#11088 addressed a related path percent-encoding comparison, but not replacement of a query value. The next step is to preserve the original URL serialization when writing `direct_url.json` while retaining redaction in logs and user-facing output, with an integration test that installs the same signed direct URL twice and verifies the second install and dry run are no-ops.

## Classification

This is a source-corroborated correctness regression and should be classified as a `bug`. uv persists a display-safe representation as installation identity metadata, then treats that altered identity as different from the original requirement. The first-bad-release boundary matches astral-sh/uv#21360 and the 0.12.8 changelog.

This is not a duplicate. astral-sh/uv#11082 covered a different URL-representation mismatch and was fixed by path percent-decoding in astral-sh/uv#11088. The repeated-install issues astral-sh/uv#17711 and astral-sh/uv#15832 arise from inconsistent wheel filename and internal `WHEEL` tags. No open item covers redacted `sig` values persisted to `direct_url.json`.

## Related

- astral-sh/uv#21360 — Merged pull request, **Redact Azure shared access signatures in URLs**. This is the release-timed trigger: it added `sig` to display redaction for 0.12.8 while explicitly requiring that formatting not alter the request URL.
- astral-sh/uv#21755 — Merged pull request, **Redact Azure shared access signatures in displayed URLs**. This independently added the same display rule in 0.12.16 without changing `direct_url.json` construction, so it did not fix the persistence and satisfaction failure.
- astral-sh/uv#11082 — Closed issue, **uv encodes `direct_url.json` content differently than pip**. This is the closest prior metadata-identity symptom: differing percent-encoding caused an unnecessary reinstall. It involved a local file path and pip-to-uv interoperability, rather than repeated `sig` redaction after uv's own installation.
- astral-sh/uv#11088 — Merged pull request, **Percent-decode URLs in canonical comparisons**. This fixed astral-sh/uv#11082 by normalizing percent-encoding in URL paths. It does not make a real query value equivalent to `****`.

## Search coverage

Searches covered open and closed issues and open, closed, and merged pull requests. Literal searches used `direct_url.json`, `sig=****`, 0.12.8, astral-sh/uv#21360, repeated reinstall wording, and `Would make no changes`. Conceptual searches covered direct-URL satisfaction and idempotence, Azure SAS and shared-access URLs, `DisplaySafeUrl`, credential and query redaction, URL normalization, and percent encoding. Fix-oriented checks included the 0.12.x changelog, merged redaction changes, and the resolution chain for astral-sh/uv#11082.

Plausible candidates were inspected and ruled out: astral-sh/uv#17711 and astral-sh/uv#15832 concern mismatched wheel tags; astral-sh/uv#10359 concerns Azure index cache headers and SAS cache churn; astral-sh/uv#13560 and astral-sh/uv#13791 concern the broader display-safe URL migration and export behavior, not installed direct-URL identity.
