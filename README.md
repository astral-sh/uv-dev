# Can't install packages: UnknownIssuer

Issue: astral-sh/uv#22011

Classification: resolved configuration issue; optional warning enhancement suggested

## Summary

On Windows 11 25H2 with uv 0.12.19 and Python 3.12.12, both `uv pip install isodate` and `uv add isodate` reportedly fail while fetching `https://pypi.org/simple/isodate/` with `invalid peer certificate: UnknownIssuer`. `--system-certs` and its deprecated alias `--native-tls` do not change the result. Exporting GlobalSign certificates from the current user's Windows trusted-root store to a PEM bundle and passing it via `uv pip install --cert` succeeds.

The reporter has now confirmed that `SSL_CERT_FILE` was set. Since uv 0.12.0, a non-empty `SSL_CERT_FILE` or `SSL_CERT_DIR` intentionally replaces all default roots, including roots selected by `--system-certs`; `--cert` in turn overrides those environment sources. This precedence exactly explains why system-certificate mode failed while the explicit GlobalSign bundle succeeded. The report therefore does not demonstrate a Windows trusted-root or platform-verifier defect.

The reporter suggested that uv warn when `SSL_CERT_FILE` overrides `--system-certs`, for example by identifying the precedence conflict. This is a usability enhancement suggestion, not a maintainer decision or evidence that the current precedence is incorrect.

PyCharm is not required to trigger the observed failure. Its inability to supply extra uv arguments is context rather than part of the minimal reproduction.

## Reproduction

Outcome: reproduced and explained by the reporter's confirmed certificate-override configuration.

All files, targets, tools, and caches were isolated under `$RUNNER_TEMP`. The runner was Linux x86_64 with Python 3.12.3. The uv executable on `PATH` was 0.12.13; it was used to install the exact reported uv 0.12.19 x86_64 Linux wheel into the temporary directory for the version-specific checks.

With both certificate override variables removed, uv 0.12.19 succeeded with both bundled and system roots:

```console
$ env -u SSL_CERT_FILE -u SSL_CERT_DIR uv pip install --python /usr/bin/python3 --target site-default isodate
Resolved 1 package
Installed 1 package
 + isodate==0.7.2

$ env -u SSL_CERT_FILE -u SSL_CERT_DIR uv --system-certs pip install --python /usr/bin/python3 --target site-system isodate
Resolved 1 package
Installed 1 package
 + isodate==0.7.2
```

A non-empty but unusable override reproduced the report exactly:

```console
$ env -u SSL_CERT_DIR SSL_CERT_FILE="$PWD/missing.pem" uv --system-certs pip install --python /usr/bin/python3 --target site-invalid isodate
warning: Invalid `SSL_CERT_FILE`. Path does not exist: .../missing.pem. No default certificates will be trusted.
error: Failed to fetch: `https://pypi.org/simple/isodate/`
  cause: error sending request for url (https://pypi.org/simple/isodate/)
  cause: client error (Connect)
  cause: invalid peer certificate: UnknownIssuer
```

The command exited with status 2. In a separate minimal project, the exact uv 0.12.19 command `env -u SSL_CERT_DIR SSL_CERT_FILE="$PWD/missing.pem" uv --system-certs add --no-sync isodate` produced the same warning and `UnknownIssuer` chain. Supplying a valid PEM explicitly while leaving the unusable environment override in place succeeded:

```console
$ env -u SSL_CERT_DIR SSL_CERT_FILE="$PWD/missing.pem" uv --system-certs pip install --python /usr/bin/python3 --target site-cert isodate --cert /etc/ssl/certs/ca-certificates.crt
Resolved 1 package
Installed 1 package
 + isodate==0.7.2
```

The Linux fixture could not exercise Windows `CertGetCertificateChain`, but that limitation is no longer material to this report: the reporter confirmed the same `SSL_CERT_FILE` condition used by the reproduction. No additional certificate-chain or `curl.exe` diagnostics are needed unless the failure can also be reproduced after removing both certificate override variables.

Existing tests cover the reproduced override semantics:

- `crates/uv-client/tests/it/ssl_certs.rs`, `test_system_certs_with_ssl_cert_file_replaces_system_roots`, verifies that `SSL_CERT_FILE` replaces system roots and causes a public PyPI connection to be rejected when the override lacks the public root.
- The same file's `test_system_certs_with_ssl_cert_file_valid` verifies that a valid `SSL_CERT_FILE` is honored when system certificates are enabled, and `test_system_certs_trusts_pypi` verifies that system roots can validate PyPI on the test host. These do not exercise the Windows current-user store specifically.
- `crates/uv/tests/project/run.rs`, `run_remote_pep723_script_with_nonexistent_ssl_cert_file`, snapshots the same warning followed by `invalid peer certificate: UnknownIssuer` when a missing `SSL_CERT_FILE` disables default roots.

## Classification

The reported symptom is reproducible and is explained by documented configuration precedence, not a uv correctness defect. The reporter confirmed that `SSL_CERT_FILE` was set. Non-empty certificate environment overrides replace system roots even if the configured source is missing or does not contain the roots needed for the request. This behavior shipped in uv 0.12.0 and is documented in the certificate documentation and that release's breaking changes.

The reporter's proposed warning when an environment certificate source overrides `--system-certs` would be an enhancement to diagnostics. There is no maintainer decision accepting that behavior change, and it should be evaluated separately from the resolved installation failure.

## Fix

Outcome: no production fix was made because the confirmed reproduction and reporter follow-up establish documented certificate-source precedence rather than a Windows verifier defect.

The parent regression in `crates/uv/tests/it/network.rs`, `system_certs_with_invalid_ssl_cert_file`, passed in the debug profile and confirmed that a non-empty missing `SSL_CERT_FILE` replaces the roots selected by `--system-certs`, producing the expected warning and `UnknownIssuer` failure. Updating that snapshot to expect a successful installation failed for exactly the reported reason before any production change.

The relevant implementation has two consistent layers. `NetworkSettings::resolve` loads non-empty `SSL_CERT_FILE` and `SSL_CERT_DIR` values as custom certificates, and `BaseClientBuilder` intentionally selects those custom certificates before system roots. On Unix, `rustls-platform-verifier` also treats these standard variables as the native certificate location. A trial settings-only precedence change therefore did not establish a valid cross-platform fix: after uv ignored the override, the Unix platform verifier still honored it and reported that no system CA certificates could be loaded.

Making an explicit `--system-certs` flag ignore standard certificate environment variables would change the documented uv 0.12 certificate contract and requires a product decision about command-line versus environment precedence. The trial changes were removed, leaving the checkout unchanged. A narrower warning that explains which certificate source won could improve diagnostics without changing precedence, but the reporter's suggestion has not been accepted or designed by maintainers.

## Resolution and possible follow-up

The reporter confirmed in astral-sh/uv#22011 that `SSL_CERT_FILE` was set and took precedence over `--system-certs`, matching both the documentation and the isolated reproduction. This resolves the original `UnknownIssuer` report as a configuration issue and rules out the suspected Windows system-certificate regression for this case.

The only proposed follow-up is the reporter's suggestion to emit a warning when `SSL_CERT_FILE` overrides `--system-certs`. No maintainer has yet classified or accepted that suggestion as an enhancement.

## Related

- astral-sh/uv#20741 and astral-sh/uv#20767 — Merged for uv 0.12.0. They made non-empty `SSL_CERT_FILE` and `SSL_CERT_DIR` sources override default roots even when the source is missing, inaccessible, empty, or invalid. This is the change directly exercised by the reproduction.
- astral-sh/uv#20418 — Merged for uv 0.12.0. It added pip-compatible `uv pip --cert`; an explicit PEM bundle replaces system and environment certificate sources.
- astral-sh/uv#18550 — Merged for uv 0.11.0. It upgraded the TLS stack to reqwest 0.13 and `rustls-platform-verifier`, delegating Windows system-certificate validation to the operating system.
- astral-sh/uv#9243 — Closed historical system-trust discussion. Reporters confirmed that uv 0.11.0 fixed earlier native/system-certificate failures.
- astral-sh/uv#17355 — Closed historical Windows `UnknownIssuer` report involving a trusted company root. Its reporter also confirmed the uv 0.11.0 fix.

## Search evidence

The source and integration-test search covered `UnknownIssuer`, `system-certs`, `native-tls`, `SSL_CERT_FILE`, `SSL_CERT_DIR`, and `rustls-platform-verifier`. The uv 0.11.0 and 0.12.0 release notes and the merged certificate changes in astral-sh/uv#18550, astral-sh/uv#20418, astral-sh/uv#20741, and astral-sh/uv#20767 were inspected. No uv 0.12.13 through 0.12.19 release note describes another TLS trust-source change.
