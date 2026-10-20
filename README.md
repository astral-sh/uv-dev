# `uv` fails to download Python 3.15 in CI for Windows, but works fine on Ubuntu

Issue: astral-sh/uv#22756

Classification: question

## Summary

The reporter runs `uv sync --no-build-isolation-package internal_package` with uv 0.13.0, installed from an internal PyPI mirror. On Azure DevOps `windows-latest` / Windows Server 2025, downloading the x86_64 CPython 3.15.0 build from the python-build-standalone 20261009 release fails with `client error (Connect)` and `invalid peer certificate: UnknownIssuer`. Ubuntu successfully downloads Python and creates the environment. The reporter asks whether uv 0.13.1 fixes this.

Closest matches concern Windows certificate trust and certificate-file overrides, including a Windows CI configuration fix. Python 3.15 support was already present in uv 0.13.0; no matching 0.13.1 TLS fix or confirmed duplicate was found.

The issue has no follow-up comments. A corporate TLS proxy, certificate overrides, and use of system certificates are not established by the report. Installing uv from an internal package mirror does not identify the certificate chain presented by the separate GitHub download request.

## Draft response

Python 3.15 is supported in uv 0.13.0 through astral-sh/uv#22400. Your log shows a TLS certificate trust failure while connecting to GitHub; the 0.13.1 release notes do not list a fix for this. A similar Windows Python-download failure was resolved by using system certificates in astral-sh/uv#8832.

Please try `uv --system-certs python install 3.15.0` on the failing runner. Also check whether `SSL_CERT_FILE` or `SSL_CERT_DIR` is set: nonempty values override system certificates (see astral-sh/uv#22011), and missing paths leave no trusted certificates. Clear unintended overrides, or ensure an intentional bundle contains the required CAs. If it still fails, please share sanitized output from `uv -vv --system-certs python install 3.15.0` and say whether the runner uses a TLS-inspecting proxy and whether either certificate override is set.

## Classification

The report establishes a TLS trust failure and asks whether upgrading resolves it, but does not establish that uv rejects a valid certificate chain under the intended trust configuration. Python 3.15 support already shipped in 0.13.0. Related reports have confirmed proxy or certificate-override conditions absent here, so neither an exact duplicate nor a regression of the historical TLS fix is established.

The existing GitHub bug label is not evidence of a confirmed correctness defect. The proposed classification reflects the current support question and unresolved trust configuration. A failure with a valid, intended trust configuration would warrant investigating a bug; reproduction of a previously fixed mechanism would warrant regression classification. No such mechanism is established here.

## Related

- astral-sh/uv#8832 — uv unable to download python due to corp proxy certificate (issue, closed). The same Windows managed-Python download failure ended in UnknownIssuer. The reporter confirmed that enabling system certificates with the then-current --native-tls flag resolved it. Unlike this report, a corporate proxy was confirmed.
- astral-sh/uv#6649 — Cannot install packages or download Python in Windows with Corporate Cisco Umbrella Certificate (issue, closed). Python downloads failed with UnknownIssuer on Windows while Ubuntu under WSL worked. Follow-up comments confirmed success after removing a certificate-file override and using native TLS. This report does not yet establish either a proxy or an override.
- astral-sh/uv#22011 — Can't install packages: UnknownIssuer (issue, open). Recent Windows UnknownIssuer failures persisted with --system-certs; the reporter confirmed SSL_CERT_FILE was overriding it. Useful diagnostic precedent, but it concerns PyPI package requests and a confirmed override, neither of which establishes the cause here.
- astral-sh/uv#20807 — Fix Windows conda system tests with invalid certificate override (pull request, merged). Fixed Windows CI UnknownIssuer failures by clearing a nonexistent SSL_CERT_FILE path set by Conda activation. This changed uv's test workflow, not runtime TLS behavior; the new report does not mention Conda.
- astral-sh/uv#22400 — Sync latest Python releases (pull request, merged). Added the 20261009 CPython 3.15.0 builds for uv 0.13.0. Establishes that this release already supports the requested Python version; catalog availability is separate from the reported TLS connection failure.

## Supporting evidence

- The reported failure occurs while connecting to the Python archive URL, before Python can be downloaded or the internal package can be built. The log does not show a missing download entry, HTTP 404, archive extraction failure, or build-isolation failure.
- `docs/concepts/authentication/certificates.md` documents bundled Mozilla roots by default, Windows system verification with `--system-certs` / `UV_SYSTEM_CERTS=true`, and complete replacement of the trust source by nonempty `SSL_CERT_FILE` / `SSL_CERT_DIR` values. `crates/uv-client/src/base_client.rs:630` implements the custom-certificate, system-certificate, and bundled-root precedence.
- astral-sh/uv#8832 contains a maintainer recommendation to enable the historical `--native-tls` flag and reporter confirmation that it resolved the Windows Python download. The modern flag is `--system-certs`.
- In astral-sh/uv#22011, the maintainer asked about certificate environment variables, and the reporter confirmed on 2026-09-28 that `SSL_CERT_FILE` had taken precedence over `--system-certs`. The issue remains open; it does not establish the same configuration on this runner.
- astral-sh/uv#20807 merged on 2026-07-30 and changed only `.github/workflows/test-system.yml`. Its confirmed cause was Windows Conda activation setting a nonexistent certificate path. This supports checking the runner configuration, but does not establish Conda use here.
- astral-sh/uv#20741 fixed the fallback behavior tracked by astral-sh/uv#18526: explicit nonempty certificate sources remain authoritative even when no certificates can be loaded. The change is documented in `changelogs/0.12.x.md` and predates uv 0.13.0.
- astral-sh/uv#22400 merged on 2026-10-09, adding the Python build release named in the report. `CHANGELOG.md` records uv 0.13.0, released the same day, making Python 3.15 the default stable version. Its 0.13.1 section, released on 2026-10-14, lists no matching TLS fix.

## Candidates ruled out and historical fixes

- astral-sh/uv#22439 concerned `No download found` for final Python 3.15.0 in uv 0.12.24, before the catalog update and uv 0.13.0 release. This reporter already reaches the correct version's archive URL.
- astral-sh/uv#16696 concerned missing default executable aliases for Python 3.15 prereleases after installation, not certificate validation or downloads.
- astral-sh/uv#9243 and astral-sh/uv#17355 discussed Windows certificate-chain failures despite enabling native/system certificates. Follow-up users reported success with uv 0.11.0. astral-sh/uv#18550 introduced the platform verifier and the `--system-certs` name, merged on 2026-03-23 and documented in `changelogs/0.11.x.md`. This report neither confirms use of system certificates nor demonstrates recurrence of that mechanism. The reqwest version discussion in astral-sh/uv#17427 is about a dependency upgrade, not a TLS fix newly shipping in uv 0.13.1.
- astral-sh/uv#16474 concerns a specific certificate bundle rejected on both Windows and Linux; the reporter confirmed system certificates worked after clearing `SSL_CERT_FILE`. That condition remains unconfirmed here. Its comments link astral-sh/uv#16473, which documents override precedence and adds `SSL_CERT_DIR` support.

## Search scope

Searched astral-sh/uv open and closed issues for UnknownIssuer, invalid peer certificate, the CPython 3.15.0/20261009 identifiers, Windows/Azure CI, and Python downloads. Broadened to system/native TLS, trust stores, certificate overrides, corporate proxies, and area:network. Searched PRs across all states and supplemented search with a paginated title/body scan of open, closed, and merged PRs for the errors, TLS terminology, reqwest, and release identifiers. Inspected candidate comments and linked fixes, plus 0.13.0/0.13.1 release notes and certificate-selection source. Ruled out astral-sh/uv#22439 (missing catalog entry before 0.13.0), astral-sh/uv#16696 (prerelease executable aliases), and a demonstrated recurrence of astral-sh/uv#9243 or astral-sh/uv#17355: their system-verification fix, astral-sh/uv#18550, shipped in 0.11.0, and this report does not establish failure with system certificates enabled. Also inspected astral-sh/uv#16474 and astral-sh/uv#20741 for explicit certificate-override behavior.

PR search queries did not yield usable PR candidates, so the review also scanned the complete paginated REST pull-request listing across all states and inspected relevant candidates directly with authenticated `gh`.

## Next information needed

An isolated Python-install attempt on the failing Windows runner with system certificates enabled, whether certificate-source overrides are set and intentional, whether the runner uses a TLS-inspecting proxy, and sanitized verbose output if the isolated attempt still fails. These distinguish an interpreter-download trust problem from project configuration without requiring the internal package.

## Scope of work

Read-only GitHub and repository investigation; no GitHub changes or reproduction on the reporter's runner. Only this handoff README was authored. No builds or tests were needed for this triage.
