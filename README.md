# `uv` fails to download Python 3.15 in CI for Windows, but works fine on Ubuntu

Issue: astral-sh/uv#22756

Classification: question

Status: closed on 2026-10-21 at 10:01:02 UTC; reporter reports success with `UV_SYSTEM_CERTS=1`.

## Summary

The reporter reports that several tests with `UV_SYSTEM_CERTS=1` appear to work and closed astral-sh/uv#22756. This provides a reported resolution through system-certificate selection, not evidence of a Python-version bug or an upgrade fix. No detailed successful-run logs, successful-run uv version, or explicit `UV_SYSTEM_CERTS=0` result were supplied.

The reporter runs `uv sync --no-build-isolation-package internal_package` with uv 0.13.0, installed from an internal PyPI mirror. On Azure DevOps `windows-latest` / Windows Server 2025, downloading the x86_64 CPython 3.15.0 build from the python-build-standalone 20261009 release fails with `client error (Connect)` and `invalid peer certificate: UnknownIssuer`. Ubuntu successfully downloads Python and creates the environment. The reporter asks whether uv 0.13.1 fixes this.

Earlier local reproduction outcome: **needs_more_information** (unchanged; separate from the reporter's subsequent successful tests). On the available Linux x86_64 runner, installed uv 0.13.1 successfully completed a minimal Python 3.15.0 sync and downloaded the exact Windows archive named in the report, with both bundled and system certificates. Adding a deliberately nonexistent `SSL_CERT_FILE` produced the reported `UnknownIssuer` chain even with `--system-certs`. This confirms a possible configuration-dependent mechanism, but that override is not established on the reporter's runner. Linux download success does not establish Windows certificate behavior or an upgrade fix.

Closest matches concern Windows certificate trust and certificate-file overrides, including a Windows CI configuration fix. Python 3.15 support was already present in uv 0.13.0; no matching 0.13.1 TLS fix or confirmed duplicate was found. The reporter clarified that Python downloads were deliberately configured to use GitHub because network policy blocks an Astral server URI. The exact configuration and blocked endpoint were not supplied. The reporter now reports success with system certificates enabled; a corporate TLS proxy, certificate overrides, and the original effective trust settings remain unknown; a network-policy block does not by itself establish TLS interception or explain the GitHub certificate error. Installing uv from an internal package mirror does not identify the certificate chain presented by the separate GitHub download request.

## Reproduction

**Earlier local outcome: needs_more_information.** The reporter subsequently reported success with `UV_SYSTEM_CERTS=1`; no new local reproduction was performed for that follow-up. The Windows-only failure has not been established in this environment. The report omits the failing runner's certificate-source and proxy configuration, and no Windows execution environment was available. An intentionally invalid certificate override can produce the same error, but it cannot be attributed to this issue without evidence from the failing runner.

### Environment and isolation

- Tested on 2026-10-20 using the installed executable `/opt/hostedtoolcache/uv/0.13.1/x86_64/uv`: `uv 0.13.1 (x86_64-unknown-linux-gnu)`.
- Host: Ubuntu Linux x86_64, kernel `7.0.0-1017-azure`. The reported host is Azure DevOps `windows-latest`, Windows Server 2025, with uv 0.13.0. No uv 0.13.0 binary or Windows certificate verifier was exercised.
- All fixture files, caches, and Python installations are under `/tmp/uv-22756.QpIgOp`. Commands used `env -i`, an explicit executable search path, `TMPDIR`, `UV_CACHE_DIR`, `UV_PYTHON_INSTALL_DIR`, `UV_CREDENTIALS_DIR`, and `--no-config`. Each download case had fresh cache and installation directories; no proxy or certificate overrides were inherited.
- The private project and package were not needed to isolate a failure that occurs before the interpreter download completes. The minimal sync fixture does not reproduce the private dependency graph.

### Linux sync control

Created `.python-version` containing `3.15.0` and this `pyproject.toml`:

```toml
[project]
name = "python-download-repro"
version = "0.0.0"
requires-python = "==3.15.*"
dependencies = []
```

Ran `uv --no-config sync --no-build-isolation-package internal_package` with `UV_PYTHON_DOWNLOADS=automatic` in the isolated fixture. It exited 0, downloaded CPython 3.15.0 for Linux (35.9 MiB), reported `Using CPython 3.15.0`, created `.venv`, and resolved the one-package fixture. There was no certificate error.

### Windows archive checks from Linux

First ran these commands with separate empty cache and installation directories:

```sh
uv --no-config python install --no-bin --no-registry cpython-3.15.0-windows-x86_64-none
uv --no-config --system-certs python install --no-bin --no-registry cpython-3.15.0-windows-x86_64-none
```

Both exited 0 and downloaded the 23.3 MiB Windows archive. Catalog inspection showed that uv 0.13.1 selects build `20261013`, whereas the report names `20261009`.

To test the reported URL exactly, obtained the public download metadata from `https://raw.githubusercontent.com/astral-sh/uv/0.13.0/crates/uv-python-managed/download-metadata.json` and saved only its `cpython-3.15.0-windows-x86_64-none` entry to `/tmp/uv-22756.QpIgOp/windows-20261009.json`. The entry specifies CPython 3.15.0, Windows, x86_64, libc `none`, build `20261009`, the URL below, and SHA-256 `0943854634fa0fc4faf2a37d900bc7e1a6f89afd7ac576b22d17f39142c63b55`:

```text
https://github.com/astral-sh/python-build-standalone/releases/download/20261009/cpython-3.15.0%2B20261009-x86_64-pc-windows-msvc-install_only_stripped.tar.gz
```

The exact-archive invocation was:

```sh
env -i \
  PATH=/opt/hostedtoolcache/uv/0.13.1/x86_64:/usr/bin:/bin \
  TMPDIR=/tmp/uv-22756.QpIgOp \
  UV_CACHE_DIR=/tmp/uv-22756.QpIgOp/exact-windows-bundled/cache \
  UV_PYTHON_INSTALL_DIR=/tmp/uv-22756.QpIgOp/exact-windows-bundled/python \
  UV_CREDENTIALS_DIR=/tmp/uv-22756.QpIgOp/credentials \
  uv --no-config python install --no-bin --no-registry \
  --mirror https://github.com/astral-sh/python-build-standalone/releases/download \
  --python-downloads-json-url /tmp/uv-22756.QpIgOp/windows-20261009.json \
  cpython-3.15.0-windows-x86_64-none
```

This exited 0. A separate invocation adding `--system-certs`, with fresh `exact-windows-system/cache` and `exact-windows-system/python` directories, also exited 0. Both downloaded and installed the archive, and both installed `BUILD` files contained `20261009`. The Windows interpreter was not executed. These observations establish that the reported archive is downloadable with this Linux client's trust configuration.

The reporter's follow-up confirms that selecting GitHub was intentional. The exact-archive checks above explicitly used the GitHub `--mirror`, so they remain relevant to the chosen download source, but they did not reproduce the reporter's network policy or unspecified mirror configuration. Further isolated attempts must retain the intended GitHub download source: `--no-config` or a cleared environment can remove the setting that selects it. Success or failure against another endpoint would not test the reported request.

### Certificate-override diagnostic

Repeated the exact-archive system-certificate invocation with fresh `exact-windows-invalid-cert` directories and the deliberately added environment variable `SSL_CERT_FILE=/tmp/uv-22756.QpIgOp/missing.pem`. It exited 1 with:

```text
warning: Invalid `SSL_CERT_FILE`. Path does not exist: /tmp/uv-22756.QpIgOp/missing.pem. No default certificates will be trusted.
error: Failed to install cpython-3.15.0-windows-x86_64-none
  cause: Failed to download `https://github.com/astral-sh/python-build-standalone/releases/download/20261009/cpython-3.15.0%2B20261009-x86_64-pc-windows-msvc-install_only_stripped.tar.gz`
  cause: error sending request for url (https://github.com/astral-sh/python-build-standalone/releases/download/20261009/cpython-3.15.0%2B20261009-x86_64-pc-windows-msvc-install_only_stripped.tar.gz)
  cause: client error (Connect)
  cause: invalid peer certificate: UnknownIssuer
```

The same controlled override also failed against uv 0.13.1's default `20261013` build. This is documented trust-source precedence, and explains why `--system-certs` alone cannot correct an unintended explicit override. It is not evidence that the reporter has such an override or that this is a Python 3.15 defect.

### Existing test coverage

Read the relevant setup, helper assertions, feature gates, and snapshots; did not run the Rust suite or add tests.

- `crates/uv-client/tests/it/ssl_certs.rs::test_ssl_cert_file_nonexistent_overrides_default_roots` uses a generated local CA/server and a missing file path, then asserts a fatal request failure and no system-certificate hint. It covers the invalid-file mechanism, not this Windows runner or GitHub chain.
- `crates/uv-client/tests/it/ssl_certs.rs::test_system_certs_with_ssl_cert_file_replaces_system_roots` enables system certificates, supplies an unrelated generated CA through `SSL_CERT_FILE`, and asserts failure connecting to `pypi.org`. It is gated by `test-pypi` and covers override precedence. The corresponding `test_system_certs_with_ssl_cert_dir_replaces_system_roots` covers directory overrides.
- `crates/uv/tests/it/network.rs::invalid_ssl_cert_file_warns_default_roots_are_disabled` snapshots the missing-file warning while a local HTTP server returns 500. It covers warning rendering; its HTTP fixture does not test TLS validation.
- `crates/uv/tests/python/python_install.rs::python_install`, enabled by `test-python-managed` in `crates/uv/tests/python/main.rs`, installs the default Python 3.15, checks executable aliases, executes the interpreter, and checks reinstall/uninstall behavior, including Windows-specific paths. It has no custom certificate/proxy setup and does not cover the reported TLS failure.

No test reproducing this exact Windows/Azure/GitHub certificate condition was identified. The successful Windows tests are reporter-supplied and lack fresh-download logs and effective certificate/proxy settings. These limits prevent identifying the precise trust-chain difference, but do not negate the reported resolution. No last-known-good Windows run under identical settings was supplied to establish a regression.

## Archived draft response

The following draft predates the reporter's successful system-certificate tests and closure. Its diagnostic requests are no longer pending; it is retained only as prior triage context and is not a proposed follow-up response.

Python 3.15 is supported in uv 0.13.0 through astral-sh/uv#22400. Your log shows a TLS certificate trust failure while connecting to GitHub; the 0.13.1 release notes do not list a fix for this. A similar Windows Python-download failure was resolved by using system certificates in astral-sh/uv#8832.

Please try `uv --system-certs python install 3.15.0` on the failing runner with fresh temporary cache and Python-install directories so the command performs a download. Also check whether `SSL_CERT_FILE` or `SSL_CERT_DIR` is set: nonempty values override system certificates (see astral-sh/uv#22011), and missing paths leave no trusted certificates. Clear unintended overrides, or ensure an intentional bundle contains the required CAs. If it still fails, please share sanitized output from `uv -vv --system-certs python install 3.15.0` and say whether the runner uses a TLS-inspecting proxy and whether either certificate override is set.

## Classification

Classification remains question. The reporter reports success after enabling system certificates and closed the issue, supporting resolution through trust configuration. The specific certificate-chain difference, any TLS-inspecting proxy, and any certificate-source override remain unconfirmed. The reported success does not establish a uv correctness defect, a regression, or a 0.13.1 fix. Python 3.15 support already shipped in uv 0.13.0.

The local reproduction findings remain unchanged: the exact archive downloaded on Linux, and a matching failure was observed only after deliberately adding a certificate override. That artificial override has not been established on the reporter's runner. The similar resolution in astral-sh/uv#8832 is relevant, but its confirmed corporate-proxy mechanism is not established here. The reporter's secondary preference for `pyproject.toml` configuration does not change the classification of this resolved download-support question.

## Related

- astral-sh/uv#8832 — uv unable to download python due to corp proxy certificate (issue, closed). The same Windows managed-Python download failure ended in UnknownIssuer. The reporter confirmed that enabling system certificates with the then-current --native-tls flag resolved it. Unlike this report, a corporate proxy was confirmed.
- astral-sh/uv#6649 — Cannot install packages or download Python in Windows with Corporate Cisco Umbrella Certificate (issue, closed). Python downloads failed with UnknownIssuer on Windows while Ubuntu under WSL worked. Follow-up comments confirmed success after removing a certificate-file override and using native TLS. This report does not yet establish either a proxy or an override.
- astral-sh/uv#22011 — Can't install packages: UnknownIssuer (issue, open). Recent Windows UnknownIssuer failures persisted with --system-certs; the reporter confirmed SSL_CERT_FILE was overriding it. Useful diagnostic precedent, but it concerns PyPI package requests and a confirmed override, neither of which establishes the cause here.
- astral-sh/uv#20807 — Fix Windows conda system tests with invalid certificate override (pull request, merged). Fixed Windows CI UnknownIssuer failures by clearing a nonexistent SSL_CERT_FILE path set by Conda activation. This changed uv's test workflow, not runtime TLS behavior; the new report does not mention Conda.
- astral-sh/uv#22400 — Sync latest Python releases (pull request, merged). Added the 20261009 CPython 3.15.0 builds for uv 0.13.0. Establishes that this release already supports the requested Python version; catalog availability is separate from the reported TLS connection failure.

## Supporting evidence

- In the 2026-10-21 resolution comment on astral-sh/uv#22756 (comment ID 6313262894), the reporter states that `UV_SYSTEM_CERTS=1` appears to work after several tests and says they are closing the issue. Read-only GitHub issue metadata confirms state `CLOSED` and closure at 2026-10-21T10:01:02Z. The successful-run details and explicit disabled-mode result were not supplied.
- In the reporter's 2026-10-20 follow-up on astral-sh/uv#22756 (comment ID 6288444697), they state that GitHub was selected explicitly because network policy blocks an Astral server URI. This is reporter-supplied configuration context, not an independently verified policy or a confirmed cause of `UnknownIssuer`. No setting name, exact blocked URI, or comparison of the two runners' download-source settings was provided.
- `crates/uv-settings/src/settings.rs` defines `python-install-mirror`, and `crates/uv-static/src/env_vars.rs` documents `UV_PYTHON_INSTALL_MIRROR` and its precedence over `UV_ASTRAL_MIRROR_URL` for CPython downloads. These are relevant configuration mechanisms to check; the comment does not identify which mechanism the reporter used.
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
- astral-sh/uv#9243 and astral-sh/uv#17355 discussed Windows certificate-chain failures despite enabling native/system certificates. Follow-up users reported success with uv 0.11.0. astral-sh/uv#18550 introduced the platform verifier and the `--system-certs` name, merged on 2026-03-23 and documented in `changelogs/0.11.x.md`. This reporter now reports success with system certificates enabled, not recurrence of that failure mechanism. The reqwest version discussion in astral-sh/uv#17427 is about a dependency upgrade, not a TLS fix newly shipping in uv 0.13.1.
- astral-sh/uv#16474 concerns a specific certificate bundle rejected on both Windows and Linux; the reporter confirmed system certificates worked after clearing `SSL_CERT_FILE`. That condition remains unconfirmed here. Its comments link astral-sh/uv#16473, which documents override precedence and adds `SSL_CERT_DIR` support.

## Search scope

Searched astral-sh/uv open and closed issues for UnknownIssuer, invalid peer certificate, the CPython 3.15.0/20261009 identifiers, Windows/Azure CI, and Python downloads. Broadened to system/native TLS, trust stores, certificate overrides, corporate proxies, and area:network. Searched PRs across all states and supplemented search with a paginated title/body scan of open, closed, and merged PRs for the errors, TLS terminology, reqwest, and release identifiers. Inspected candidate comments and linked fixes, plus 0.13.0/0.13.1 release notes and certificate-selection source. Ruled out astral-sh/uv#22439 (missing catalog entry before 0.13.0), astral-sh/uv#16696 (prerelease executable aliases), and a demonstrated recurrence of astral-sh/uv#9243 or astral-sh/uv#17355: their system-verification fix, astral-sh/uv#18550, shipped in 0.11.0, and this report does not establish failure with system certificates enabled. Also inspected astral-sh/uv#16474 and astral-sh/uv#20741 for explicit certificate-override behavior.

PR search queries did not yield usable PR candidates, so the review also scanned the complete paginated REST pull-request listing across all states and inspected relevant candidates directly with authenticated `gh`.

## Resolution and remaining uncertainty

The maintainer requested a comparison of `UV_SYSTEM_CERTS=1` and `UV_SYSTEM_CERTS=0`. The reporter supplied a successful result for the enabled setting and closed the issue; the disabled-setting result remains unspecified. There is no outstanding diagnostic request needed to record this reported resolution. The original certificate issuer, proxy involvement, certificate-source overrides, exact GitHub mirror configuration, and uv version used in the successful tests remain unknown. The evidence does not establish which certificate authority or chain element accounts for the difference.

If the failure recurs, useful diagnostics remain separate fresh-download attempts with `UV_SYSTEM_CERTS=1` and `UV_SYSTEM_CERTS=0`, retaining the intended GitHub download source and otherwise identical settings, with sanitized error chains and the uv version. Explicit certificate-file/directory overrides take precedence over either mode and must be accounted for when interpreting results. The earlier Linux checks do not substitute for a failing Windows request.

## Persistent configuration clarification

The reporter would prefer `system-certs = true` in `pyproject.toml` and is considering setting `UV_SYSTEM_CERTS` in `Taskfile.yml` to cover pipeline stages. That Taskfile change is a proposal, not a reported implementation or tested result.

Repository evidence supports the `pyproject.toml` limitation: `crates/uv-settings/src/settings.rs:288` marks `system-certs` as `uv_toml_only`, and `docs/concepts/authentication/certificates.md` documents the environment variable, command-line flag, and `uv.toml` setting. However, `uv.toml` is not restricted to system-wide configuration. `docs/concepts/configuration-files.md` documents discovery of project-local `uv.toml` in the current or nearest parent directory, as well as user- and system-level files. A project-local `uv.toml` is therefore an available persistent configuration location for project commands such as `uv sync`. When both files are present in the same directory, `uv.toml` takes precedence and the accompanying `[tool.uv]` section in `pyproject.toml` is ignored. These are source-backed configuration facts, not additional tests on the reporter's pipeline.

## Scope of work

Read-only repository and public metadata investigation, plus isolated command-line reproduction checks on Linux. No GitHub changes, checkout edits, builds, or execution on the reporter's Windows runner. This handoff README was updated directly; all reproduction fixtures, caches, and Python installations were written under `/tmp/uv-22756.QpIgOp`. Existing Rust tests were inspected rather than executed.
