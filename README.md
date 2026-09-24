# Direct URL requirements with a `sig` query parameter are reinstalled on every run: `direct_url.json` stores the redacted URL (regression in 0.12.8)

Issue: astral-sh/uv#21969

Classification: bug

## Summary

Targeted testing reproduces the regression for direct wheel URLs whose query contains `sig`. uv 0.12.7 writes the requested value to `direct_url.json` and recognizes the installation on the next run. uv 0.12.8, the installed uv 0.12.13, and uv 0.12.18 instead write `sig=****`; a later `uv pip install --dry-run` plans an uninstall and reinstall. When redaction is active, the URL formatter also reserializes the full query, so otherwise unchanged values such as timestamps acquire percent encoding.

Repository evidence corroborates the reported mechanism:

- astral-sh/uv#21360 added `sig` to sensitive display parameters for the 0.12.8 release. Its stated contract and test require the original request URL to remain intact.
- `DisplaySafeUrl` masks sensitive query values when formatted. If it encounters one, it rebuilds the complete query with a form serializer, which accounts for the observed encoding changes in neighboring parameters.
- Before the fix, the `DirectUrl` conversion for a parsed archive called the wrapped URL's display conversion when populating the string later serialized to `direct_url.json`. This crossed the display/persistence boundary and stored the masked representation.
- Installer satisfaction checking parses the installed metadata URL and compares it with the requested canonical URL. Replacing a real query value with `****` makes those identities differ, so uv rejects its own installed distribution on every run.
- astral-sh/uv#21755 independently added the same `sig` display rule in 0.12.16 without changing metadata construction. It therefore did not address the regression, consistent with the report through 0.12.18.

No open issue or pull request already tracks this exact `sig` metadata regression.

## Reproduction

Outcome: **reproducible**.

The reproduction ran on Linux 6.17.0-1022-azure x86_64 with Python 3.12.3. The `uv` executable on `PATH` was uv 0.12.13; uv 0.12.7, 0.12.8, and 0.12.18 were run through `uvx`. All virtual environments, tool installations, downloads, and caches were isolated under a new `/tmp/uv-21969.*` directory. The URL used a synthetic `sig=reproduction-value`; no URL or credential from the report was used.

The essential commands for each version were:

```console
$ uvx --from uv==0.12.7 uv --no-config venv /tmp/uv-21969/venv-0.12.7 --python python3.12
$ uvx --from uv==0.12.7 uv --no-config pip install --python /tmp/uv-21969/venv-0.12.7/bin/python 'six @ https://files.pythonhosted.org/packages/b7/ce/149a00dd41f10bc29e5921b496af8b574d8413afcd5e30dfa0ed46c2cc5e/six-1.17.0-py2.py3-none-any.whl?sig=reproduction-value&st=2026-09-15T16:34:14Z'
$ uvx --from uv==0.12.7 uv --no-config pip install --python /tmp/uv-21969/venv-0.12.7/bin/python --dry-run 'six @ https://files.pythonhosted.org/packages/b7/ce/149a00dd41f10bc29e5921b496af8b574d8413afcd5e30dfa0ed46c2cc5e/six-1.17.0-py2.py3-none-any.whl?sig=reproduction-value&st=2026-09-15T16:34:14Z'
```

The same commands were repeated with `uv==0.12.8` and `uv==0.12.18`; the installed uv 0.12.13 was also tested directly. The cache directory was set separately for each version.

Observed results:

| Version | Installed `direct_url.json` | Second `uv pip install --dry-run` |
|---|---|---|
| 0.12.7 | Retained the synthetic `sig` value and the unencoded timestamp | `Would make no changes` |
| 0.12.8 | Stored `sig=****` and percent-encoded the timestamp colons | `Would uninstall 1 package` and `Would install 1 package` |
| 0.12.13 | Stored `sig=****` and percent-encoded the timestamp colons | `Would uninstall 1 package` and `Would install 1 package` |
| 0.12.18 | Stored `sig=****` and percent-encoded the timestamp colons | `Would uninstall 1 package` and `Would install 1 package` |

This directly confirms both the first-bad boundary and the repeated-install plan described in astral-sh/uv#21969.

Before the parent regression test was added, nearby tests did not cover this end-to-end case:

- `crates/uv-redacted/src/lib.rs::tests::redact_azure_sas_query_signature` checks display redaction only.
- `crates/uv/tests/pip_install/pip_install.rs::direct_url_json_direct_url` checks `direct_url.json` for a direct URL without query parameters and does not perform a second install.
- `crates/uv/tests/pip/pip_sync.rs::incompatible_direct_url_redacts_credentials` checks redaction in an error for an incompatible wheel; it never installs the wheel or reads `direct_url.json`.

## Fix

Outcome: **fixed**.

The parent regression test in `crates/uv/tests/pip_install/pip_install.rs::direct_url_json_direct_url` initially passed while asserting the undesirable redacted metadata and reinstall plan. It was changed first to require the original synthetic query in `direct_url.json` and `Would make no changes` from the second dry run; that desired assertion failed because the installed metadata still contained the redacted, reserialized URL.

The production change is limited to the archive `DirectUrl` producer in `crates/uv-pypi-types/src/parsed_url.rs`. It now serializes the parsed URL's credential-free underlying representation instead of formatting `DisplaySafeUrl`. This retains the original query, including its value and encoding, without persisting URL user information. Display and logging paths continue to redact sensitive query values.

Neighboring producers and consumers were inspected. Git URL parsing intentionally removes query parameters before producing VCS metadata, while local path and directory forms did not provide another demonstrated manifestation of this failure. No additional test case was added. The existing direct-URL Git metadata tests, the incompatible-wheel display-redaction test, the Azure query-redaction test, and the parsed/direct URL conversion tests continue to pass.

Successful focused validation:

- `cargo test --package uv --test pip_install direct_url_json_`
- `cargo test --package uv --test pip incompatible_direct_url_redacts_credentials`
- `cargo test --package uv-redacted redact_azure_sas_query_signature`
- `cargo test --package uv-pypi-types direct_url_`
- `cargo +stable fmt --all --check`
- `cargo +stable clippy --package uv-pypi-types --all-targets -- -D warnings`

## Draft response

Thanks for the clear reproduction. This is a bug introduced by the `sig` display-redaction change in astral-sh/uv#21360. The metadata path formatted the wrapped URL when constructing `direct_url.json`, so the display-only redaction was persisted as `sig=****` and the query was reserialized. The next satisfaction check compared that stored URL with the requested URL and treated the installed distribution as mismatched.

astral-sh/uv#11082 and astral-sh/uv#11088 addressed a related path percent-encoding comparison, but not replacement of a query value. The fix now preserves the credential-free original URL serialization when writing `direct_url.json` while retaining redaction in logs and user-facing output. The integration regression test verifies both the stored metadata and that a second dry run is a no-op.

## Classification

This is an observed correctness regression and should be classified as a `bug`. uv persists a display-safe representation as installation identity metadata, then treats that altered identity as different from the original requirement. The reproduced first-bad-release boundary matches astral-sh/uv#21360 and the 0.12.8 changelog.

This is not a duplicate. astral-sh/uv#11082 covered a different URL-representation mismatch and was fixed by path percent-decoding in astral-sh/uv#11088. The repeated-install issues astral-sh/uv#17711 and astral-sh/uv#15832 arise from inconsistent wheel filename and internal `WHEEL` tags. No open item covers redacted `sig` values persisted to `direct_url.json`.

## Related

- astral-sh/uv#21360 — Merged pull request, **Redact Azure shared access signatures in URLs**. This is the release-timed trigger: it added `sig` to display redaction for 0.12.8 while explicitly requiring that formatting not alter the request URL.
- astral-sh/uv#21755 — Merged pull request, **Redact Azure shared access signatures in displayed URLs**. This independently added the same display rule in 0.12.16 without changing `direct_url.json` construction, so it did not fix the persistence and satisfaction failure.
- astral-sh/uv#11082 — Closed issue, **uv encodes `direct_url.json` content differently than pip**. This is the closest prior metadata-identity symptom: differing percent-encoding caused an unnecessary reinstall. It involved a local file path and pip-to-uv interoperability, rather than repeated `sig` redaction after uv's own installation.
- astral-sh/uv#11088 — Merged pull request, **Percent-decode URLs in canonical comparisons**. This fixed astral-sh/uv#11082 by normalizing percent-encoding in URL paths. It does not make a real query value equivalent to `****`.

## Search coverage

Searches covered open and closed issues and open, closed, and merged pull requests. Literal searches used `direct_url.json`, `sig=****`, 0.12.8, astral-sh/uv#21360, repeated reinstall wording, and `Would make no changes`. Conceptual searches covered direct-URL satisfaction and idempotence, Azure SAS and shared-access URLs, `DisplaySafeUrl`, credential and query redaction, URL normalization, and percent encoding. Fix-oriented checks included the 0.12.x changelog, merged redaction changes, and the resolution chain for astral-sh/uv#11082.

Plausible candidates were inspected and ruled out: astral-sh/uv#17711 and astral-sh/uv#15832 concern mismatched wheel tags; astral-sh/uv#10359 concerns Azure index cache headers and SAS cache churn; astral-sh/uv#13560 and astral-sh/uv#13791 concern the broader display-safe URL migration and export behavior, not installed direct-URL identity.

Pull request: https://github.com/astral-sh/uv-dev/pull/2108
