# gcs-endpoint: unparseable GOOGLE_APPLICATION_CREDENTIALS silently falls back to VM metadata identity

Issue: astral-sh/uv#22273

Classification: bug

## Summary

With `UV_GCS_ENDPOINT_URL` targeting Google Artifact Registry, the reporter ran `uv sync` with `GOOGLE_APPLICATION_CREDENTIALS` pointing to a workload identity federation `external_account` configuration. A missing `credential_source.format` caused reqsign to reject the configuration with `data did not match any variant of untagged enum Source`. The reporter observed fallback to the GKE node's service account and wheel-download failures with `403 Forbidden`. The credential-loading diagnostic appeared with `RUST_LOG=reqsign_core=debug`, but not with ordinary output or `-v`.

Reported environment: uv 0.12.23, Python 3.11, Linux x86_64 musl in a Debian trixie Docker container on GKE CI runners. The issue was opened October 7, 2026; its issue thread currently has no comments.

astral-sh/uv#22389 proposes fixing silent identity fallback but remains open and excludes -v diagnostics. apache/reqsign#910 fixes the parsing trigger from apache/reqsign#909 upstream. astral-sh/uv#19611 and astral-sh/uv#19894 provide adjacent credential-selection context.

The report contains three distinct concerns: accepting omitted external-account source formats, handling an explicitly configured credential file that cannot be loaded, and exposing credential-loading diagnostics under normal or verbose output.

## Draft response

The source confirms your report: reqsign's default credential chain logs loading errors and tries the next provider, while uv's -v filter excludes those diagnostics.

astral-sh/uv#22389 proposes failing when explicitly configured Google credentials cannot be loaded. It is still open and does not address warning visibility in -v. Separately, apache/reqsign#910 merged the missing-format parsing fix; uv 0.12.23 and the current checkout still pin reqsign-google 3.1.1.

For plain-text credential sources, adding "format": {"type": "text"} remains a workaround for the parsing failure. The next step is to review astral-sh/uv#22389; the reqsign warning-visibility request still needs separate handling.

## Classification

Source confirms that the pinned Google provider rejects omitted credential_source.format, its credential chain logs errors and continues to other providers, and uv's -v filter excludes reqsign logs. Silently discarding explicitly configured credentials and potentially selecting another identity is incorrect behavior. astral-sh/uv#22389 was created in response to this report, so it does not make the issue a duplicate. The upstream parsing fix postdates the report and is absent from the pinned dependency; there is no evidence of a regression from a previously shipped fix.

The upstream issue is a direct relationship, but its closed parser fix covers only the omitted-format trigger. The broader uv fallback and diagnostic behavior remains relevant. The open native Artifact Registry provider proposal uses a separate authentication path and does not replace the GCS endpoint provider.

## Related

- astral-sh/uv#22389 (pull request, open) — Fail when `GOOGLE_APPLICATION_CREDENTIALS` is set but cannot be loaded. Direct proposed fix for astral-sh/uv#22273, opened afterward and explicitly referencing it. Uses EnvCredentialProvider directly when the variable is nonempty so loading errors propagate instead of selecting another identity. It explicitly excludes reqsign warning visibility in -v.
- apache/reqsign#909 (issue, closed) — reqsign-google: `external_account` configs without `credential_source.format` fail to parse (Google defaults it to `text`). Tracks the exact parsing trigger and resulting metadata fallback, confirmed in the pinned reqsign-google 3.1.1 source. Closed by apache/reqsign#910; that parser fix does not resolve uv's broader handling of unloadable explicit credentials or hidden diagnostics.
- apache/reqsign#910 (pull request, merged) — fix(google): default external account credential format to text. Merged October 8, 2026, after this report. Defaults omitted formats to text for file and URL sources, with mocked default-provider coverage against metadata fallback. Does not change the general error-skipping credential chain or uv logging; uv 0.12.23 and this checkout still pin reqsign-google 3.1.1.
- astral-sh/uv#19611 (pull request, open) — Add Google Artifact Registry credential retrieval. Adjacent implementation of authoritative explicit Google credentials in a new Artifact Registry provider. Its diff propagates explicit credential-file errors, but leaves the existing GCS endpoint signer using the default chain, so it does not fix this reported path.
- astral-sh/uv#19894 (issue, open) — Consider erroring if `NETRC` is set to a missing file. Related policy discussion about falling back after an explicitly selected credential file cannot be found. Maintainer comments favor at most a warning for missing NETRC files. Different provider and trigger; it does not settle handling of rejected Google credential configurations.

## Supporting evidence

- `crates/uv-auth/src/providers.rs:147`: `GcsEndpointProvider::create_signer` calls `reqsign::google::default_signer("storage.googleapis.com")` without overriding credential selection.
- `Cargo.lock:4024` pins `reqsign-google` 3.1.1; `Cargo.lock:3988` pins `reqsign-core` 3.3.1. The lockfile at the uv 0.12.23 tag has the same versions.
- Published `reqsign-google` 3.1.1 source, `src/credential.rs:128` and `:140`: `UrlSource` and `FileSource` require `format: Format` without a serde default. A missing format therefore rejects those variants of the untagged `Source` enum.
- Published `reqsign-google` 3.1.1 source, `src/provide_credential/default.rs:98`: the environment provider reads and parses the configured file, propagating failures to its caller. The default chain tries environment credentials, the well-known ADC file, and VM metadata in that order.
- Published `reqsign-core` 3.3.1 source, `src/api.rs:481`: `ProvideCredentialChain` catches each provider error, emits a warning, and continues. A later successful provider can therefore supply another identity.
- `crates/uv/src/lib.rs:197` maps one verbose flag to `DebugUv`; `crates/uv/src/logging.rs:40` uses `uv=debug` for that mode and disables logging by default. The 0.12.23 logging source has the same filters. These defaults exclude reqsign's warning target.
- The diff and description of astral-sh/uv#22389 agree: a nonempty explicit credential variable selects `EnvCredentialProvider` directly; absent or empty values retain the default chain. Its three unit tests cover unparseable files, missing files, and default-chain selection. The proposal does not change logging and has no submitted reviews.
- apache/reqsign#910 merged on October 8, 2026, at commit `92dd0187ba788bca2e380bce95c469c2b2557241`. Its diff adds serde defaults to file and URL formats and defaults `Format` to text. Its description reports mocked token-exchange tests that reject metadata fallback for the omitted-format case; malformed explicit formats still fail parsing.
- The diff of astral-sh/uv#19611 introduces an Artifact Registry credential provider that propagates explicit ADC failures. The existing GCS endpoint signer still calls the default signer; the change there only renames its type alias.
- In astral-sh/uv#19894, a maintainer favors at most a warning for a missing explicit NETRC file. The preceding astral-sh/uv#19865 was closed after discussion of compatibility concerns. This is provider-specific design context, not an established general rule for Google credentials.

Source inspection confirms the failure mechanism and diagnostic filtering. The reporter's live GKE identity selection and HTTP failure were not independently reproduced. No credential files, tokens, or metadata-service credentials were accessed, and no builds or runtime tests were run.

## Search coverage and comparisons

Used authenticated gh to search open and closed issues and open, closed, and merged PRs. Separate literal searches covered GOOGLE_APPLICATION_CREDENTIALS, UV_GCS_ENDPOINT_URL, gcs-endpoint, external_account, credential_source, reqsign_core, and the untagged Source error. Conceptual searches covered Artifact Registry 403s, workload identity, metadata servers, ignored credentials, credential fallback, silent authentication failures, and warnings/verbosity using authentication, tracing, and error-message labels. Fix-oriented searches covered reqsign/GCS changes and closed credential reports. PR search failed to surface known matching PRs, so it was supplemented with the latest 1,000 PRs across states, the open-PR listing, source history, and issue timelines. Inspected candidate comments, reviews, and relevant diffs, including the apache/reqsign#909 to apache/reqsign#910 chain. Ruled out astral-sh/uv#20850 (package-specific 403s through keyring) and astral-sh/uv#12716 (401 resolved by keyring configuration) as duplicates. Also inspected astral-sh/uv#10854, astral-sh/uv#7685, astral-sh/uv#8364, astral-sh/uv#4162, and astral-sh/uv#4343: netrc warnings and keyring stderr have different diagnostic paths. No earlier duplicate or previously shipped fix for this GCS behavior was found.

Additional comparisons:

- astral-sh/uv#20850 uses keyring and reports a package-specific Artifact Registry wheel failure while other packages work. Comments also distinguish universal resolution from platform-specific installation. It supplies no evidence of rejected external-account credentials or VM identity fallback.
- astral-sh/uv#12716 was resolved with `authenticate = "always"` and the keyring subprocess provider, before GCS signing was introduced in astral-sh/uv#17474.
- astral-sh/uv#10854 was closed with the explanation that netrc parsing diagnostics require `-v` because netrc discovery is not opt-in. In this report, credentials are explicitly selected and even `-v` hides the dependency warning.
- astral-sh/uv#7685 and its merged fix astral-sh/uv#8364 concern netrc parsing warnings; astral-sh/uv#4162 and its merged fix astral-sh/uv#4343 concern forwarding keyring subprocess stderr. Neither demonstrates a regression in GCS signing.
- astral-sh/uv#12280 concerns keyring lookup configuration and feedback; its comments lead to astral-sh/uv#12716 and unrelated keyring fixes.
- astral-sh/uv#22025 merged the reqsign 0.20.6 update on September 28, 2026, before this report and the upstream parser fix. It does not establish that the reported bug had previously been fixed.

Searches briefly hit GitHub's search rate limit; the affected searches were retried successfully. PR keyword search did not return known matching pull requests, so conclusions were supplemented with direct listings, timelines, and source history rather than treating empty results as proof of absence.

## Maintainer follow-up

Review the explicit-credential failure behavior proposed in astral-sh/uv#22389, retaining separate consideration of the requested reqsign diagnostics in `-v`. Track integration of the upstream missing-format fix independently: a parser correction prevents this specific trigger but does not change fallback for other credential-loading errors.

The reporter's explicit text-format workaround is appropriate for plain-text sources. No additional private configuration or credential material is needed to establish the bug.

This handoff records read-only repository and GitHub investigation as of October 9, 2026. No GitHub changes were made.
