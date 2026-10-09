# gcs-endpoint: unparseable GOOGLE_APPLICATION_CREDENTIALS silently falls back to VM metadata identity

Issue: astral-sh/uv#22273

Classification: bug

## Summary

With `UV_GCS_ENDPOINT_URL` targeting Google Artifact Registry, the reporter ran `uv sync` with `GOOGLE_APPLICATION_CREDENTIALS` pointing to a workload identity federation `external_account` configuration. A missing `credential_source.format` caused reqsign to reject the configuration with `data did not match any variant of untagged enum Source`. The reporter observed fallback to the GKE node's service account and wheel-download failures with `403 Forbidden`. The credential-loading diagnostic appeared with `RUST_LOG=reqsign_core=debug`, but not with ordinary output or `-v`.

Reported environment: uv 0.12.23, Python 3.11, Linux x86_64 musl in a Debian trixie Docker container on GKE CI runners. The issue was opened October 7, 2026; its issue thread currently has no comments.

astral-sh/uv#22389 proposes fixing silent identity fallback but remains open and excludes -v diagnostics. apache/reqsign#910 fixes the parsing trigger from apache/reqsign#909 upstream. astral-sh/uv#19611 and astral-sh/uv#19894 provide adjacent credential-selection context.

The report contains three distinct concerns: accepting omitted external-account source formats, handling an explicitly configured credential file that cannot be loaded, and exposing credential-loading diagnostics under normal or verbose output.

A local behavioral reproduction confirmed all three observations on the installed uv 0.12.13 (Linux x86_64 GNU, CPython 3.12.3): an omitted source format caused fallback to mocked VM metadata, ordinary output and `-v` hid the credential-loading error, and `RUST_LOG=reqsign_core=debug` exposed it. Adding the explicit text format avoided metadata entirely and let the fixture sync successfully. This establishes the behavior independently of source inspection; the reporter's exact uv 0.12.23 musl/GKE environment was not run.

## Reproduction

Outcome: **reproducible**, using local mock metadata, token-exchange, and wheel endpoints. The HTTP 403 is deliberately supplied by the fixture; the independently observed behavior is rejection of the configured external-account file, successful metadata fallback, an authenticated wheel retry, and suppressed diagnostics.

### Environment and isolation

- Installed executable on PATH: `/opt/hostedtoolcache/uv/0.12.13/x86_64/uv`, reporting `uv 0.12.13 (x86_64-unknown-linux-gnu)`.
- Linux x86_64; `/usr/bin/python3`, CPython 3.12.3. The reported environment instead uses uv 0.12.23 musl and Python 3.11 in Debian trixie on GKE.
- Each scenario used a separate temporary project, virtual environment, cache, configuration directory, and authentication directory. Subprocesses received a fresh environment without inherited credentials or proxy settings; keyring and Python downloads were disabled.
- All runtime HTTP endpoints were `127.0.0.1` listeners. `GCE_METADATA_HOST` redirected metadata discovery to the mock server. No real credential files, metadata credentials, or cloud identities were accessed. The fixture used synthetic markers and recorded only request paths and whether an Authorization header existed, never its value.
- No repository files or GitHub state were changed. Existing checkout changes were left intact. No build or repository test suite was run.

### Fixture and commands

The retained standard-library Python harness is `reproduction/reproduce.py`, relative to this README. It resolves the installed uv from PATH, generates a minimal wheel and project, starts the local server, runs one scenario, and writes its output and request summary under a new temporary scenario directory. Run explicitly from the issue-context directory:

```sh
python3 reproduction/reproduce.py default
python3 reproduction/reproduce.py verbose
python3 reproduction/reproduce.py diagnostic
python3 reproduction/reproduce.py format-control
```

The project has no workspace or dependency groups:

```toml
[project]
name = "fallback-probe"
version = "0.1.0"
requires-python = ">=3.11"
dependencies = ["demo @ http://127.0.0.1:PORT/wheels/demo-0.1.0-py3-none-any.whl"]
```

`GOOGLE_APPLICATION_CREDENTIALS` points to a newly generated, non-secret `external_account` configuration. Its relevant fields are:

```json
{
  "type": "external_account",
  "audience": "//iam.googleapis.com/projects/123/locations/global/workloadIdentityPools/test/providers/test",
  "subject_token_type": "urn:ietf:params:oauth:token-type:jwt",
  "token_url": "http://127.0.0.1:PORT/sts",
  "credential_source": {"file": "/TEMP/synthetic-subject.txt"}
}
```

The file source contains only a synthetic test marker. The initial configuration omits `credential_source.format`; the control adds only `"format": {"type": "text"}`. The fake metadata endpoint returns a synthetic successful response for `/computeMetadata/v1/instance/service-accounts/default/token`. The wheel endpoint returns 403 until the mock `/sts` exchange occurs, after which it serves the generated wheel. This models different registry access for the two flows without implementing real IAM authorization.

The underlying command, with `PORT` and `/TEMP` supplied by the harness, is:

```sh
UV_GCS_ENDPOINT_URL=http://127.0.0.1:PORT/wheels \
GOOGLE_APPLICATION_CREDENTIALS=/TEMP/external-account.json \
GCE_METADATA_HOST=127.0.0.1:PORT \
uv sync --no-index --no-config --python /usr/bin/python3 \
  --no-python-downloads --keyring-provider disabled --preview-features gcs-endpoint
```

The verbose scenario appends `-v`; the diagnostic scenario sets `RUST_LOG=reqsign_core=debug`. Each uses a fresh project and cache. Other isolation variables are explicit in the harness, including `UV_CACHE_DIR`, `UV_CREDENTIALS_DIR`, `XDG_CONFIG_HOME`, `APPDATA`, `NETRC`, and `TMPDIR`.

### Observed results

| Scenario | Exit | Mock metadata requests | Mock STS requests | Credential-loading diagnostic |
| --- | --- | --- | --- | --- |
| Missing format, default output | 1 | 1 | 0 | Absent; reports wheel download 403 |
| Missing format, `-v` | 1 | 1 | 0 | Absent; includes `Found GCS credentials` and the 403 |
| Missing format, `RUST_LOG=reqsign_core=debug` | 1 | 1 | 0 | Parse warning and successful metadata provider selection visible |
| Explicit text format, default output | 0 | 0 | 1 | No parsing failure; installs `demo==0.1.0` |

In each failing scenario, the request sequence was an unauthenticated wheel GET, a GET to the mock default-service-account metadata endpoint, then a wheel GET with an Authorization header. The diagnostic run printed:

```text
DEBUG Trying credential provider: EnvCredentialProvider { scope: None }
WARN Error loading credential from provider EnvCredentialProvider { scope: None }: Error { kind: Unexpected, message: "failed to parse credential file", source: data did not match any variant of untagged enum Source, retryable: false }
DEBUG Trying credential provider: WellKnownCredentialProvider { scope: None }
DEBUG No credential found in provider: WellKnownCredentialProvider { scope: None }
DEBUG Trying credential provider: VmMetadataCredentialProvider { scope: None, endpoint: None, service_account: None }
DEBUG Successfully loaded credential from provider: VmMetadataCredentialProvider { scope: None, endpoint: None, service_account: None }
```

The control performed the mock STS exchange, retried the wheel with an Authorization header, and installed it without contacting metadata. This confirms that the omitted format triggers the observed fallback in this fixture. It does not independently verify the reporter's live GKE service-account identity or Artifact Registry permissions, nor constitute a runtime check of uv 0.12.23.

Saved evidence is in `reproduction/default-result.json`, `reproduction/verbose-result.json`, `reproduction/diagnostic-result.json`, and `reproduction/format-control-result.json`, with corresponding `*-output.txt` files. Original runtime directories are under `/tmp/uv-22273-repro.AnmZ2U/`.

### Existing test coverage

Searched `crates/uv/tests/it/`, `crates/uv-client/tests/it/`, and `crates/uv-auth/` for GCS endpoints, `GOOGLE_APPLICATION_CREDENTIALS`, external-account parsing, reqsign, and metadata fallback. No existing uv test covering this credential-file failure, metadata fallback, or reqsign warning visibility was found.

The setup and assertion in `crates/uv/tests/it/auth.rs::invalid_cloud_endpoint_urls` were inspected: it passes `not-a-url` as each cloud endpoint and snapshots `Invalid UV_[CLOUD]_ENDPOINT_URL` / `relative URL without a base`. That test checks endpoint URL validation, not credential parsing or fallback. The endpoint path-prefix unit tests in `crates/uv-auth/src/providers.rs` and the AWS/Azure signer tests in `crates/uv-auth/src/credentials.rs` likewise do not cover this behavior.

## Draft response

I reproduced the credential fallback and diagnostic behavior with the installed uv 0.12.13 on Linux, using local mock endpoints and a non-secret external-account configuration. Omitting `credential_source.format` led to successful VM metadata fallback and a wheel request with an Authorization header. Default output and `-v` showed only the resulting 403; `RUST_LOG=reqsign_core=debug` exposed the parse error and provider selection. Adding the explicit text format used the external-account flow with no metadata request. The exact uv 0.12.23/GKE deployment was not tested.

astral-sh/uv#22389 proposes failing when explicitly configured Google credentials cannot be loaded. It remains open and does not address warning visibility in `-v`. Separately, apache/reqsign#910 merged the missing-format parsing fix; uv 0.12.23 and the current checkout still pin reqsign-google 3.1.1.

For plain-text credential sources, adding `"format": {"type": "text"}` is a verified workaround in the local fixture. The explicit-credential fallback policy and visibility of dependency warnings remain separate decisions.

## Classification

Retain the bug classification for the external-account parsing failure and missing credential-loading feedback. Runtime evidence confirms metadata fallback after the explicit file fails to parse and the absence of its diagnostic under default output and `-v`. Reproducibility alone does not decide whether uv should fail or warn when an explicit credential source cannot be loaded; that policy remains a maintainer decision. astral-sh/uv#22389 was created in response to this report, so it does not make the issue a duplicate. The upstream parsing fix postdates the report and is absent from the pinned dependency; there is no evidence of a regression from a previously shipped fix.

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

Source inspection supports the failure mechanism and diagnostic filtering, and the local runtime checks above independently demonstrate the parser failure, metadata fallback, and hidden diagnostics on installed uv 0.12.13. The reporter's live GKE identity selection and Artifact Registry permissions were not independently verified. Only generated non-secret fixture configurations and synthetic responses were used; no real credentials were accessed and no builds were run.

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

The reporter's explicit text-format workaround succeeded for the plain-text fixture. No private configuration or credential material is needed to reproduce the behavior. A future regression test should cover the chosen explicit-credential policy and relevant verbosity behavior; integrating the upstream parser fix alone does not settle either policy.

This handoff records the earlier read-only repository and GitHub investigation plus isolated runtime reproduction as of October 9, 2026. No GitHub changes were made.
