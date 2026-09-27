# Can't install packages: UnknownIssuer

Issue: astral-sh/uv#22011

Classification: configuration-dependent behavior reproduced; Windows system-certificate bug unconfirmed

## Summary

On Windows 11 25H2 with uv 0.12.19 and Python 3.12.12, both `uv pip install isodate` and `uv add isodate` reportedly fail while fetching `https://pypi.org/simple/isodate/` with `invalid peer certificate: UnknownIssuer`. `--system-certs` and its deprecated alias `--native-tls` do not change the result. Exporting GlobalSign certificates from the current user's Windows trusted-root store to a PEM bundle and passing it via `uv pip install --cert` succeeds.

The same command/result pattern is reproducible with uv 0.12.19 when a non-empty `SSL_CERT_FILE` points to an unusable certificate source. Since uv 0.12.0, a non-empty `SSL_CERT_FILE` or `SSL_CERT_DIR` intentionally replaces all default roots, including roots selected by `--system-certs`; `--cert` in turn overrides those environment sources. The report does not state whether either environment variable is set, so this reproduction confirms the reported symptom but does not establish that Windows failed to consult or honor its current-user trusted-root store.

PyCharm is not required to trigger the observed failure. Its inability to supply extra uv arguments is context rather than part of the minimal reproduction.

## Reproduction

Outcome: reproducible under an evidence-backed certificate-override configuration; the Windows-specific cause needs confirmation.

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

This Linux fixture cannot exercise Windows `CertGetCertificateChain` or the reporter's served certificate chain. To distinguish the reproduced override behavior from a Windows verifier defect, the reporter should rerun in the same Windows shell after removing `SSL_CERT_FILE` and `SSL_CERT_DIR` for that process and using `--no-config`, then confirm only whether those variables had been set (not their values). If the failure persists, a credential-redacted `uv -vv --no-config --system-certs pip install isodate` log and the server certificate chain are needed, along with confirmation of whether the required root is in the current-user or local-machine certificate store.

Existing tests cover the reproduced override semantics:

- `crates/uv-client/tests/it/ssl_certs.rs`, `test_system_certs_with_ssl_cert_file_replaces_system_roots`, verifies that `SSL_CERT_FILE` replaces system roots and causes a public PyPI connection to be rejected when the override lacks the public root.
- The same file's `test_system_certs_with_ssl_cert_file_valid` verifies that a valid `SSL_CERT_FILE` is honored when system certificates are enabled, and `test_system_certs_trusts_pypi` verifies that system roots can validate PyPI on the test host. These do not exercise the Windows current-user store specifically.
- `crates/uv/tests/project/run.rs`, `run_remote_pep723_script_with_nonexistent_ssl_cert_file`, snapshots the same warning followed by `invalid peer certificate: UnknownIssuer` when a missing `SSL_CERT_FILE` disables default roots.

## Classification

The reported symptom is reproducible, but the current evidence does not confirm a uv defect. Under the reproduced configuration, the result is intentional: non-empty certificate environment overrides replace system roots even if the configured path is missing or contains no usable certificates. This behavior shipped in uv 0.12.0 and is documented in that release's breaking changes.

If both override variables are absent and the command still fails on the reported Windows host, `--system-certs` is expected to delegate validation to Windows, and rejection of a chain that Windows itself accepts would be a likely bug. Until that isolated Windows result and the served chain are available, the Windows trust-store explanation remains a hypothesis.

## Draft response

Thanks for the detailed command comparisons. I could reproduce the same `UnknownIssuer` result with uv 0.12.19 when `SSL_CERT_FILE` is non-empty but unusable. In that configuration, `SSL_CERT_FILE` intentionally replaces the roots selected by `--system-certs`, while `--cert` overrides the environment setting, matching the failure/success pattern in the report.

Could you check whether `SSL_CERT_FILE` or `SSL_CERT_DIR` is present in the failing shell, without sharing either value? Please then remove both variables for that process and retry `uv --no-config --system-certs pip install isodate`. If it still fails, please provide a credential-redacted verbose log and the certificate chain served for `pypi.org`, and confirm whether the required root is installed for the current user or the local machine. That will determine whether this is the expected override behavior or a Windows system-verifier problem.

## Related

- astral-sh/uv#20741 and astral-sh/uv#20767 — Merged for uv 0.12.0. They made non-empty `SSL_CERT_FILE` and `SSL_CERT_DIR` sources override default roots even when the source is missing, inaccessible, empty, or invalid. This is the change directly exercised by the reproduction.
- astral-sh/uv#20418 — Merged for uv 0.12.0. It added pip-compatible `uv pip --cert`; an explicit PEM bundle replaces system and environment certificate sources.
- astral-sh/uv#18550 — Merged for uv 0.11.0. It upgraded the TLS stack to reqwest 0.13 and `rustls-platform-verifier`, delegating Windows system-certificate validation to the operating system.
- astral-sh/uv#9243 — Closed historical system-trust discussion. Reporters confirmed that uv 0.11.0 fixed earlier native/system-certificate failures.
- astral-sh/uv#17355 — Closed historical Windows `UnknownIssuer` report involving a trusted company root. Its reporter also confirmed the uv 0.11.0 fix.

## Search evidence

The source and integration-test search covered `UnknownIssuer`, `system-certs`, `native-tls`, `SSL_CERT_FILE`, `SSL_CERT_DIR`, and `rustls-platform-verifier`. The uv 0.11.0 and 0.12.0 release notes and the merged certificate changes in astral-sh/uv#18550, astral-sh/uv#20418, astral-sh/uv#20741, and astral-sh/uv#20767 were inspected. No uv 0.12.13 through 0.12.19 release note describes another TLS trust-source change.
